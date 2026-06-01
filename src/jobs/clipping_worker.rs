// Background worker that polls for pending clipping jobs and executes them

use crate::agent::clipping_agent::GeminiClippingAgent;
use crate::clipping::uploader::ClipUploader;
use crate::jobs::clipping_job::{
    execute_clipping_job, fetch_destination_channel, infer_clipping_resume_from_nodes,
};
use crate::jobs::error_classifier::{classify, ErrorClass};
use crate::jobs::job_claimer::JobClaimer;
use crate::jobs::worker_config::WorkerConfig;
use crate::AppState;
use std::sync::Arc;
use tokio::task::JoinSet;
use tokio::time::{sleep, Duration};
use uuid::Uuid;

const ACTIVE_CLIPPING_JOB_STATUSES: &[&str] = &[
    "pending",
    "downloading",
    "analyzing",
    "extracting_clips",
    "posting",
    "fallback_rendering",
];

/// Run the clipping worker in a background loop (spawnable)
/// Supports true parallel job processing via JoinSet.
pub async fn run_clipping_worker_loop(app_state: Arc<AppState>) {
    // Load and validate configuration
    let config = WorkerConfig::from_env();
    if let Err(e) = config.validate() {
        tracing::error!("❌ Invalid worker configuration: {}", e);
        tracing::error!("Worker will NOT start. Please fix configuration.");
        return;
    }

    tracing::info!(
        "🔧 Clipping worker started (concurrency: {}, poll: {}s, id: {})",
        config.concurrency,
        config.poll_interval_secs,
        config.worker_id
    );
    tracing::info!(
        "📊 Recommended DB pool size: {} connections",
        config.recommended_db_pool_size()
    );

    // Optional startup delay — allows test binaries time to compile and enqueue jobs
    let startup_delay_secs: u64 = std::env::var("WORKER_STARTUP_DELAY_SECS")
        .ok()
        .and_then(|v| v.parse().ok())
        .unwrap_or(0);
    if startup_delay_secs > 0 {
        tracing::info!(
            "⏳ Worker startup delay: {}s (set via WORKER_STARTUP_DELAY_SECS)",
            startup_delay_secs
        );
        sleep(Duration::from_secs(startup_delay_secs)).await;
        tracing::info!("✅ Startup delay complete, worker is now active");
    }

    let mut interval =
        tokio::time::interval(tokio::time::Duration::from_secs(config.poll_interval_secs));

    loop {
        interval.tick().await;
        update_worker_heartbeat(&app_state, &config.worker_id, None).await;

        match process_clipping_jobs_parallel(&app_state, &config).await {
            Ok(_) => {}
            Err(e) => {
                tracing::error!("❌ Clipping worker error: {}", e);
            }
        }

        // Also process any pending manual clipping jobs
        if let Err(e) = process_manual_jobs(&app_state).await {
            tracing::error!("❌ Manual clipping worker error: {}", e);
        }
    }
}

// ============================================================================
// True parallel job processor — JoinSet-based
// ============================================================================

/// Process clipping jobs in parallel using JoinSet.
/// Fill JoinSet to concurrency limit, drain one slot before claiming next job.
/// If auto-clipping is disabled (via system_settings), skips all automatic processing.
/// Manual clipping jobs are processed separately in process_manual_jobs().
async fn process_clipping_jobs_parallel(
    app_state: &Arc<AppState>,
    config: &WorkerConfig,
) -> Result<(), String> {
    // Check the auto-clipping toggle before processing any automatic jobs
    let auto_clipping_enabled: bool = sqlx::query_scalar(
        "SELECT setting_value FROM system_settings WHERE setting_key = 'auto_clipping_enabled'"
    )
    .fetch_optional(&app_state.db_pool)
    .await
    .map_err(|e| format!("Failed to check auto-clipping setting: {}", e))?
    .map(|v: String| v == "true")
    .unwrap_or(false);

    if !auto_clipping_enabled {
        tracing::debug!("Auto-clipping is disabled — skipping automatic job processing this cycle");
        return Ok(());
    }

    crate::jobs::clipping_supervisor::run_clipping_supervisor_once(app_state).await?;
    detect_stuck_jobs(app_state).await?;
    auto_retry_failed_jobs(app_state).await?;
    recover_completed_jobs_with_unpublished_clips(app_state).await?;
    reconcile_fallback_delivery_job_statuses(app_state).await?;
    recover_completed_fallback_deliveries_to_youtube(app_state).await?;
    cleanup_stale_pending_jobs(app_state).await?;
    check_pending_too_long(app_state).await;

    // Compile-time assertion: GeminiClippingAgent must be Send + 'static for JoinSet::spawn
    #[allow(dead_code)]
    fn _assert_agent_send() {
        fn is_send<T: Send + 'static>() {}
        is_send::<GeminiClippingAgent>();
    }

    let pending_count: i64 = sqlx::query_scalar(
        "SELECT COUNT(*) FROM clipping_jobs WHERE status = 'pending' AND claimed_by IS NULL",
    )
    .fetch_one(&app_state.db_pool)
    .await
    .unwrap_or(0);

    if pending_count == 0 {
        tracing::debug!("No pending jobs available for processing");
        return Ok(());
    }

    tracing::info!(
        "📋 Found {} pending jobs, spawning up to {} concurrent workers",
        pending_count,
        config.concurrency
    );

    let mut join_set: JoinSet<Result<i32, String>> = JoinSet::new();
    let mut quota_exhausted = false;
    let mut total_completed = 0usize;
    let mut total_failed = 0usize;
    let worker_id = config.worker_id.clone();

    loop {
        // Claim new jobs up to concurrency limit
        while join_set.len() < config.concurrency && !quota_exhausted {
            let state = Arc::clone(app_state);
            let wid = worker_id.clone();

            match JobClaimer::new(wid, state.db_pool.clone())
                .claim_next_job()
                .await
            {
                Ok(Some(job_id)) => {
                    join_set.spawn(async move { execute_claimed_job(state, job_id).await });
                }
                Ok(None) => break, // no unclaimed pending jobs
                Err(e) => {
                    tracing::error!("Claim error: {}", e);
                    break;
                }
            }
        }

        // If nothing is running, we're done for this cycle
        if join_set.is_empty() {
            break;
        }

        // Wait for any one task to finish, then loop back to claim a new one
        match join_set.join_next().await {
            Some(Ok(Ok(job_id))) => {
                total_completed += 1;
                tracing::info!("✅ Job {} completed", job_id);
            }
            Some(Ok(Err(e))) => {
                total_failed += 1;
                if e.contains("RESOURCE_EXHAUSTED")
                    || e.contains("Resource has been exhausted")
                    || e.contains("quota")
                {
                    quota_exhausted = true;
                    tracing::warn!("⏸️  Gemini quota hit — stopping new claims this cycle");
                }
                tracing::error!("❌ {}", e);
            }
            Some(Err(join_err)) => {
                total_failed += 1;
                tracing::error!("❌ Task panicked: {}", join_err);
            }
            None => break,
        }
    }

    // Drain any remaining tasks (e.g., if quota_exhausted stopped intake mid-flight)
    while let Some(_) = join_set.join_next().await {}

    if quota_exhausted {
        let cfg = crate::jobs::worker_config::WorkerConfig::from_env();
        // Add ±20% jitter to avoid thundering herd when multiple workers hit quota simultaneously
        let jitter_pct = rand::random::<f64>() * 0.4 - 0.2; // -20% to +20%
        let pause_secs = (cfg.quota_pause_secs as f64 * (1.0 + jitter_pct)).round() as u64;
        tracing::warn!(
            "⏸️  Pausing {}s for Gemini quota recovery (base: {}s)",
            pause_secs,
            cfg.quota_pause_secs
        );
        sleep(Duration::from_secs(pause_secs)).await;
    }

    if total_completed > 0 || total_failed > 0 {
        tracing::info!(
            "📊 Cycle: {} completed, {} failed",
            total_completed,
            total_failed
        );
    }

    Ok(())
}

/// Execute a single claimed job. Used as the JoinSet task body.
/// Runs GeminiClippingAgent::process_job or falls back to execute_clipping_job.
/// On failure, classifies error and sets 'cancelled' for permanent failures.
async fn execute_claimed_job(app_state: Arc<AppState>, job_id: i32) -> Result<i32, String> {
    tracing::info!("🎬 Processing job {} (claimed)", job_id);

    let job_timeout_secs: u64 = std::env::var("JOB_EXECUTION_TIMEOUT_SECS")
        .ok()
        .and_then(|v| v.parse().ok())
        .unwrap_or(7200); // 2 hours

    let use_node_step_executor = std::env::var("CLIPPING_NODE_STEP_MODE")
        .map(|value| {
            !matches!(
                value.trim().to_ascii_lowercase().as_str(),
                "0" | "false" | "no" | "off" | "disabled"
            )
        })
        .unwrap_or(true);

    let execution_result = if use_node_step_executor {
        tracing::info!(
            "Durable clipping node-step executor enabled — executing job {} through checkpointed nodes",
            job_id
        );
        tokio::time::timeout(
            tokio::time::Duration::from_secs(job_timeout_secs),
            execute_clipping_job(job_id, app_state.clone()),
        )
        .await
    } else if app_state.gemini_client.is_some() {
        let agent = GeminiClippingAgent::new(app_state.clone());
        tokio::time::timeout(
            tokio::time::Duration::from_secs(job_timeout_secs),
            agent.process_job(job_id),
        )
        .await
    } else {
        tracing::warn!(
            "⚠️  GEMINI_API_KEY not configured — falling back to execute_clipping_job for job {}",
            job_id
        );
        tokio::time::timeout(
            tokio::time::Duration::from_secs(job_timeout_secs),
            execute_clipping_job(job_id, app_state.clone()),
        )
        .await
    };

    match execution_result {
        Ok(Ok(msg)) => {
            tracing::info!("✅ Job {} completed: {}", job_id, msg);
            Ok(job_id)
        }
        Ok(Err(e)) => {
            tracing::error!("❌ Job {} failed: {}", job_id, e);

            let new_status = match classify(&e) {
                ErrorClass::Permanent => {
                    tracing::warn!(
                        "🚫 Job {} permanently failed ({}), setting status=cancelled",
                        job_id,
                        e
                    );
                    "cancelled"
                }
                ErrorClass::OAuthExpired => {
                    tracing::warn!("🔑 Job {} failed: OAuth token expired — will retry automatically once channel owner reconnects their YouTube account", job_id);
                    "failed"
                }
                _ => "failed",
            };

            let fail_query = format!(
                "UPDATE clipping_jobs \
                 SET status = '{}', \
                     error_message = $1, \
                     completed_at = NOW(), \
                     claimed_by = NULL, \
                     worker_heartbeat_at = NULL, \
                     updated_at = NOW() \
                 WHERE id = $2",
                new_status
            );
            if let Err(db_err) = sqlx::query(&fail_query)
                .bind(&e)
                .bind(job_id)
                .execute(&app_state.db_pool)
                .await
            {
                tracing::error!(
                    "Failed to update job {} status to {}: {} — job may be stuck",
                    job_id,
                    new_status,
                    db_err
                );
            }

            if let Ok(Some(Some(workflow_id))) = sqlx::query_scalar::<_, Option<uuid::Uuid>>(
                "SELECT workflow_id FROM clipping_jobs WHERE id = $1",
            )
            .bind(job_id)
            .fetch_optional(&app_state.db_pool)
            .await
            {
                let workflow_runtime =
                    crate::services::WorkflowRuntime::new(app_state.db_pool.clone());
                let _ = if new_status == "cancelled" {
                    workflow_runtime
                        .mark_cancelled(workflow_id, Some(new_status), &e)
                        .await
                } else {
                    workflow_runtime
                        .mark_failed(workflow_id, Some(new_status), &e, None)
                        .await
                };
            }

            Err(format!("Job {} failed: {}", job_id, e))
        }
        Err(_timeout) => {
            let timeout_msg = format!(
                "Job execution timed out after {}s — API or external service did not respond",
                job_timeout_secs
            );
            tracing::warn!("⏰ Job {} timed out: {}", job_id, timeout_msg);

            let fail_query = "UPDATE clipping_jobs \
                 SET status = 'failed', \
                     error_message = $1, \
                     completed_at = NOW(), \
                     claimed_by = NULL, \
                     worker_heartbeat_at = NULL, \
                     updated_at = NOW() \
                 WHERE id = $2";

            match tokio::time::timeout(
                tokio::time::Duration::from_secs(10),
                sqlx::query(fail_query)
                    .bind(&timeout_msg)
                    .bind(job_id)
                    .execute(&app_state.db_pool),
            )
            .await
            {
                Ok(Ok(_)) => {}
                Ok(Err(e)) => {
                    tracing::error!(
                        "❌ Failed to reset timed-out job {}: {} — stuck-detection will handle it",
                        job_id,
                        e
                    );
                }
                Err(_) => {
                    tracing::error!("❌ Cleanup for timed-out job {} timed out after 10s — stuck-detection will handle it", job_id);
                }
            }

            if let Ok(Some(Some(workflow_id))) = sqlx::query_scalar::<_, Option<uuid::Uuid>>(
                "SELECT workflow_id FROM clipping_jobs WHERE id = $1",
            )
            .bind(job_id)
            .fetch_optional(&app_state.db_pool)
            .await
            {
                let workflow_runtime =
                    crate::services::WorkflowRuntime::new(app_state.db_pool.clone());
                let _ = workflow_runtime
                    .mark_failed(
                        workflow_id,
                        Some("timeout"),
                        &timeout_msg,
                        None,
                    )
                    .await;
            }

            Err(format!("Job {} timed out", job_id))
        }
    }
}

// ============================================================================
// Standalone functions called by main.rs tasks
// ============================================================================

/// Public wrapper — called by the main tokio runtime as an independent background task.
/// Runs every 60s regardless of whether the worker thread is busy processing a job.
pub async fn run_stuck_job_detection(app_state: &Arc<AppState>) -> Result<(), String> {
    detect_stuck_jobs(app_state).await
}

/// Update the worker_heartbeats table with current liveness data.
/// Called from the main loop and the independent heartbeat task.
pub async fn update_worker_heartbeat(
    app_state: &Arc<AppState>,
    worker_id: &str,
    current_job_id: Option<i32>,
) {
    if let Err(e) = sqlx::query(
        "INSERT INTO worker_heartbeats (worker_id, last_seen_at, updated_at, current_job_id)
         VALUES ($1, NOW(), NOW(), $2)
         ON CONFLICT (worker_id) DO UPDATE
           SET last_seen_at = NOW(), updated_at = NOW(), current_job_id = EXCLUDED.current_job_id",
    )
    .bind(worker_id)
    .bind(current_job_id)
    .execute(&app_state.db_pool)
    .await
    {
        tracing::error!("Failed to update heartbeat for worker {}: {}", worker_id, e);
    }
}

// ============================================================================
// V2: Heartbeat-aware stuck detection + pending-too-long alert
// ============================================================================

/// Detect jobs stuck in intermediate states and reset them to 'failed'.
///
/// Two-tier detection:
/// 1. Jobs WITH heartbeat: stuck if no heartbeat for 3 minutes (definitive crash signal).
/// 2. Jobs WITHOUT heartbeat (legacy): use conservative per-stage timeouts via updated_at.
async fn detect_stuck_jobs(app_state: &Arc<AppState>) -> Result<(), String> {
    // Release stale claims — pending jobs with claimed_by set from a possibly dead worker
    let stale_claim_query = String::from(
        "UPDATE clipping_jobs \
         SET claimed_by = NULL, claimed_at = NULL, updated_at = NOW() \
         WHERE status = 'pending' \
         AND claimed_by IS NOT NULL \
         AND claimed_at < NOW() - INTERVAL '5 minutes'",
    );
    match sqlx::query(&stale_claim_query)
        .execute(&app_state.db_pool)
        .await
    {
        Ok(result) if result.rows_affected() > 0 => {
            tracing::info!(
                "🔓 Released {} stale job claims (pending + claimed_by set for >5 min)",
                result.rows_affected()
            );
        }
        Ok(_) => {}
        Err(e) => {
            tracing::warn!("Failed to release stale claims (non-fatal): {}", e);
        }
    }

    // Heartbeat-aware stuck detection: 3-minute heartbeat timeout for jobs that have it,
    // legacy per-stage timeouts for jobs that pre-date the heartbeat column.
    // Timeouts are configurable via env vars (see WorkerConfig).
    let cfg = crate::jobs::worker_config::WorkerConfig::from_env();
    let stuck_query = format!(
        "SELECT id, status, COALESCE(worker_heartbeat_at, updated_at)::text AS last_seen \
         FROM clipping_jobs \
         WHERE status IN ('downloading', 'analyzing', 'extracting_clips', 'posting') \
           AND ( \
               (worker_heartbeat_at IS NOT NULL \
                AND worker_heartbeat_at < NOW() - INTERVAL '3 minutes') \
               OR \
               (worker_heartbeat_at IS NULL AND ( \
                   (status = 'downloading'      AND updated_at < NOW() - INTERVAL '{} minutes') OR \
                   (status = 'analyzing'        AND updated_at < NOW() - INTERVAL '{} minutes') OR \
                   (status = 'extracting_clips' AND updated_at < NOW() - INTERVAL '{} minutes') OR \
                   (status = 'posting'          AND updated_at < NOW() - INTERVAL '{} minutes') \
               )) \
           )",
        cfg.stuck_downloading_mins,
        cfg.stuck_analyzing_mins,
        cfg.stuck_extracting_mins,
        cfg.stuck_posting_mins,
    );
    let stuck_jobs: Vec<(i32, String, String)> = sqlx::query_as(&stuck_query)
        .fetch_all(&app_state.db_pool)
        .await
        .map_err(|e| format!("Failed to fetch stuck jobs: {}", e))?;

    if stuck_jobs.is_empty() {
        return Ok(());
    }

    tracing::warn!(
        "🔄 Found {} stuck jobs, resetting to failed",
        stuck_jobs.len()
    );

    for (job_id, status, last_seen) in stuck_jobs {
        let error_message = format!(
            "Job stuck/timed out in '{}' state. Last heartbeat/update: {}. Automatically reset by worker.",
            status, last_seen
        );

        let reset_stuck_query = String::from(
            "UPDATE clipping_jobs \
             SET status = 'failed', \
                 error_message = $1, \
                 completed_at = NOW(), \
                 updated_at = NOW(), \
                 claimed_by = NULL, \
                 worker_heartbeat_at = NULL, \
                 stuck_detection_count = COALESCE(stuck_detection_count, 0) + 1 \
             WHERE id = $2",
        );
        match sqlx::query(&reset_stuck_query)
            .bind(&error_message)
            .bind(job_id)
            .execute(&app_state.db_pool)
            .await
        {
            Ok(_) => {
                tracing::info!(
                    "✅ Job {} reset from '{}' to 'failed' (stuck for too long, last seen: {})",
                    job_id,
                    status,
                    last_seen
                );
            }
            Err(e) => {
                tracing::error!("Failed to reset stuck job {}: {}", job_id, e);
            }
        }
    }

    // Detect jobs in active states with NULL started_at — these were claimed but never
    // properly started (e.g., worker crashed after claim but before starting the job).
    // A job cannot be 'downloading' or 'analyzing' without a started_at timestamp,
    // so this is a definitive stuck signal after a 5-minute grace period.
    let null_start_result = sqlx::query_scalar::<_, i32>(
        "UPDATE clipping_jobs \
         SET status = 'failed', \
             error_message = 'Job stuck: claimed but never started (started_at IS NULL). Auto-reset by stuck detector.', \
             completed_at = NOW(), \
             updated_at = NOW(), \
             claimed_by = NULL, \
             worker_heartbeat_at = NULL, \
             stuck_detection_count = COALESCE(stuck_detection_count, 0) + 1 \
         WHERE status IN ('downloading', 'analyzing', 'extracting_clips', 'posting', 'processing') \
           AND started_at IS NULL \
           AND updated_at < NOW() - INTERVAL '5 minutes' \
         RETURNING id"
    )
    .fetch_all(&app_state.db_pool)
    .await;

    match null_start_result {
        Ok(ids) if !ids.is_empty() => {
            tracing::warn!(
                "🔄 Reset {} stuck jobs with started_at IS NULL: {:?}",
                ids.len(),
                ids
            );
        }
        Ok(_) => {}
        Err(e) => {
            tracing::warn!(
                "Failed to reset NULL started_at stuck jobs (non-fatal): {}",
                e
            );
        }
    }

    Ok(())
}

/// V2: Check for pending jobs that have been waiting too long without being claimed.
///
/// Does NOT fail these jobs — they are legitimately pending.
/// Only logs warnings/errors so operators can diagnose worker issues.
async fn check_pending_too_long(app_state: &Arc<AppState>) {
    let result: Option<(i64, Option<String>)> = sqlx::query_as(
        "SELECT COUNT(*) AS stuck_count, MIN(created_at)::text AS oldest \
         FROM clipping_jobs \
         WHERE status = 'pending' \
           AND claimed_by IS NULL \
           AND created_at < NOW() - INTERVAL '15 minutes'",
    )
    .fetch_optional(&app_state.db_pool)
    .await
    .ok()
    .flatten();

    if let Some((count, oldest_opt)) = result {
        if count > 0 {
            let oldest = oldest_opt.unwrap_or_default();

            // Determine how old the oldest job is
            let is_critical: bool = sqlx::query_scalar(
                "SELECT COUNT(*) > 0 FROM clipping_jobs \
                 WHERE status = 'pending' \
                   AND claimed_by IS NULL \
                   AND created_at < NOW() - INTERVAL '60 minutes'",
            )
            .fetch_one(&app_state.db_pool)
            .await
            .unwrap_or(false);

            if is_critical {
                tracing::error!(
                    "🚨 WORKER ALERT: {} jobs pending >15 min unclaimed (oldest: {}). \
                     Worker may be down or severely overloaded!",
                    count,
                    oldest
                );
            } else {
                tracing::warn!(
                    "⚠️  {} jobs pending >15 min unclaimed (oldest: {}). \
                     Worker may be slow or restarting.",
                    count,
                    oldest
                );
            }
        }
    }
}

/// Discard ancient unclaimed pending jobs so the queue reflects actionable work.
///
/// This is intentionally conservative:
/// - only `pending` jobs are touched
/// - only jobs with no active claim are touched
/// - default threshold is 24 hours via `CLIPPING_STALE_PENDING_DISCARD_MINS`
/// - jobs are moved to `discarded` so admins can still retry them manually
async fn cleanup_stale_pending_jobs(app_state: &Arc<AppState>) -> Result<(), String> {
    let cfg = crate::jobs::worker_config::WorkerConfig::from_env();
    let discard_query = format!(
        "UPDATE clipping_jobs \
         SET status = 'discarded', \
             error_message = COALESCE(error_message || ' ', '') || \
                 '[Auto-discarded after remaining pending/unclaimed for more than {} minutes. Use admin retry if still needed.]', \
             completed_at = NOW(), \
             updated_at = NOW() \
         WHERE status = 'pending' \
           AND claimed_by IS NULL \
           AND created_at < NOW() - INTERVAL '{} minutes'",
        cfg.stale_pending_discard_mins,
        cfg.stale_pending_discard_mins
    );

    let result = sqlx::query(&discard_query)
        .execute(&app_state.db_pool)
        .await
        .map_err(|e| format!("Failed to discard stale pending jobs: {}", e))?;

    if result.rows_affected() > 0 {
        tracing::warn!(
            "🧹 Discarded {} stale pending jobs older than {} minutes",
            result.rows_affected(),
            cfg.stale_pending_discard_mins
        );
    }

    Ok(())
}

// ============================================================================
// V3 + V4: Auto-retry with 7-day window, exponential backoff, discard at 10 retries
// ============================================================================

/// Automatically retry failed jobs that meet retry criteria.
///
/// V3: Extended retry window to 7 days (was 6 hours).
/// V4a: Exponential backoff — 2^retry_count minutes cooldown.
/// V4b: Error classification — permanent failures are already 'cancelled' (not retried).
/// V4c: Discard after 10 retries — moves to dead-letter queue.
async fn auto_retry_failed_jobs(app_state: &Arc<AppState>) -> Result<(), String> {
    let cfg = crate::jobs::worker_config::WorkerConfig::from_env();
    // First: discard exhausted jobs (>= max_retries) — move to dead-letter
    let discard_query = format!(
        "UPDATE clipping_jobs \
         SET status = 'discarded', \
             error_message = COALESCE(error_message || ' ', '') || '[Discarded after {} retries — use admin API to retry]', \
             updated_at = NOW() \
         WHERE status = 'failed' \
           AND COALESCE(retry_count, 0) >= {} \
           AND updated_at > NOW() - INTERVAL '1 hour'",
        cfg.max_retries, cfg.max_retries
    );
    if let Err(e) = sqlx::query(&discard_query)
        .execute(&app_state.db_pool)
        .await
    {
        tracing::error!("Failed to discard exhausted clipping jobs: {}", e);
    }

    // Warn about jobs approaching discard threshold
    let exhausted: Vec<(i32, Option<String>)> = sqlx::query_as(
        "SELECT id, error_message FROM clipping_jobs \
         WHERE status = 'discarded' \
         AND updated_at > NOW() - INTERVAL '1 hour'",
    )
    .fetch_all(&app_state.db_pool)
    .await
    .unwrap_or_default();

    for (job_id, err) in exhausted {
        tracing::error!(
            "🚨 CLIPPING JOB {} DISCARDED (exhausted 10 retries) — admin review required. \
             Use POST /api/admin/clipping/jobs/{}/retry to reset. Last error: {:?}",
            job_id,
            job_id,
            err
        );
    }

    // Fetch failed jobs eligible for retry.
    // Exponential backoff: cooldown = 2^retry_count minutes (capped at 256 min = ~4h).
    // Quota errors always get an extra 30-minute floor via OR clause.
    //
    // OAuth-aware gate: jobs that failed due to an expired YouTube token are only
    // re-queued once the destination channel has been reconnected (requires_reauth=false).
    // Without this gate, a disconnected channel burns through all 10 retries in hours,
    // gets discarded, and can never be retried automatically even after reconnection.
    let retry_jobs: Vec<(i32, Option<String>, Option<uuid::Uuid>)> = sqlx::query_as(
        "SELECT cj.id, cj.current_step, cj.workflow_id \
         FROM clipping_jobs cj \
         LEFT JOIN youtube_channel_linkages ycl ON ycl.id = cj.linkage_id \
         LEFT JOIN connected_youtube_channels cyc ON cyc.id = ycl.destination_channel_id \
         WHERE cj.status = 'failed' \
         AND cj.completed_at > NOW() - INTERVAL '7 days' \
         AND COALESCE(cj.retry_count, 0) < 10 \
         AND NOT ( \
             (cj.error_message LIKE '%authorization expired%' \
              OR cj.error_message LIKE '%Token refresh failed%' \
              OR cj.error_message LIKE '%needs reconnection%' \
              OR cj.error_message LIKE '%invalid_grant%') \
             AND cyc.requires_reauth = true \
         ) \
         AND ( \
             (cj.error_message NOT LIKE '%RESOURCE_EXHAUSTED%' \
              AND cj.error_message NOT LIKE '%429%' \
              AND cj.error_message NOT LIKE '%Too Many Requests%' \
              AND cj.completed_at < NOW() - (INTERVAL '1 minute' * POWER(2, LEAST(COALESCE(cj.retry_count, 0), 8)))) \
             OR \
             ((cj.error_message LIKE '%RESOURCE_EXHAUSTED%' \
               OR cj.error_message LIKE '%429%' \
               OR cj.error_message LIKE '%Too Many Requests%') \
              AND cj.completed_at < NOW() - INTERVAL '30 minutes') \
         ) \
         ORDER BY cj.completed_at ASC \
         LIMIT 10"
    )
    .fetch_all(&app_state.db_pool)
    .await
    .map_err(|e| format!("Failed to fetch failed jobs for retry: {}", e))?;

    if retry_jobs.is_empty() {
        return Ok(());
    }

    tracing::info!(
        "🔄 Found {} failed jobs eligible for automatic retry",
        retry_jobs.len()
    );

    for (job_id, current_step, workflow_id) in retry_jobs {
        let node_resume_from = infer_clipping_resume_from_nodes(&app_state.db_pool, workflow_id)
            .await
            .unwrap_or(None);
        let resume_from_owned = node_resume_from.or_else(|| {
            crate::jobs::JobPhase::from_step(current_step.as_deref().unwrap_or(""))
                .resume_from()
                .map(str::to_string)
        });
        let resume_from = resume_from_owned.as_deref();

        if let Some(phase) = resume_from {
            tracing::info!(
                "✅ Job {} reset to pending, will resume from '{}' (was at: {:?})",
                job_id,
                phase,
                current_step
            );
        } else {
            tracing::info!(
                "✅ Job {} reset to pending, will restart from Phase A (current_step: {:?})",
                job_id,
                current_step
            );
        }

        let reset_query = String::from(
            "UPDATE clipping_jobs \
             SET status = 'pending', \
                 resume_from = $1, \
                 error_message = NULL, \
                 progress_percent = 0, \
                 current_step = 'queued', \
                 started_at = NULL, \
                 completed_at = NULL, \
                 claimed_by = NULL, \
                 claimed_at = NULL, \
                 worker_heartbeat_at = NULL, \
                 supervisor_status = 'healthy', \
                 supervisor_reason = NULL, \
                 supervisor_last_action = 'auto_retry_requeued', \
                 supervisor_last_run_at = NOW(), \
                 blocked_by_job_id = NULL, \
                 updated_at = NOW(), \
                 retry_count = COALESCE(retry_count, 0) + 1, \
                 last_retry_at = NOW() \
             WHERE id = $2 \
             AND status = 'failed' \
             AND NOT EXISTS ( \
                 SELECT 1 \
                 FROM clipping_jobs sibling \
                 WHERE sibling.linkage_id = clipping_jobs.linkage_id \
                   AND sibling.source_video_id = clipping_jobs.source_video_id \
                   AND sibling.id <> clipping_jobs.id \
                   AND sibling.status = ANY($3) \
             )",
        );
        match sqlx::query(&reset_query)
            .bind(resume_from)
            .bind(job_id)
            .bind(ACTIVE_CLIPPING_JOB_STATUSES)
            .execute(&app_state.db_pool)
            .await
        {
            Ok(result) if result.rows_affected() == 0 => {
                tracing::info!(
                    "⏭️ Skipping auto-retry for job {} because another active job for the same source video already exists",
                    job_id
                );
            }
            Ok(_) => {}
            Err(e) => {
                tracing::error!("Failed to reset job {} for retry: {}", job_id, e);
            }
        }
    }

    Ok(())
}

/// Recover jobs that were incorrectly left as completed even though YouTube
/// upload did not publish every extracted clip.
///
/// This intentionally excludes fallback deliveries: those are completed because
/// a generated summary workflow was created, not because clips should be posted.
async fn recover_completed_jobs_with_unpublished_clips(
    app_state: &Arc<AppState>,
) -> Result<(), String> {
    let jobs: Vec<(i32, i64, i64)> = sqlx::query_as(
        "SELECT cj.id,
                COUNT(ec.id) AS total_clips,
                COUNT(ec.id) FILTER (WHERE ec.upload_status = 'published') AS published_clips
         FROM clipping_jobs cj
         JOIN extracted_clips ec ON ec.clipping_job_id = cj.id
         LEFT JOIN youtube_channel_linkages ycl ON ycl.id = cj.linkage_id
         LEFT JOIN connected_youtube_channels cyc ON cyc.id = ycl.destination_channel_id
         WHERE cj.status = 'completed'
           AND cj.fallback_delivery_id IS NULL
           AND cj.updated_at > NOW() - INTERVAL '7 days'
           AND COALESCE(cyc.requires_reauth, false) = false
         GROUP BY cj.id
         HAVING COUNT(ec.id) > 0
            AND COUNT(ec.id) FILTER (WHERE ec.upload_status = 'published') < COUNT(ec.id)
         ORDER BY cj.updated_at ASC
         LIMIT 10",
    )
    .fetch_all(&app_state.db_pool)
    .await
    .map_err(|e| format!("Failed to fetch completed upload-recovery jobs: {}", e))?;

    if jobs.is_empty() {
        return Ok(());
    }

    tracing::warn!(
        "Found {} completed clipping jobs with unpublished saved clips; requeueing Phase E upload",
        jobs.len()
    );

    for (job_id, total_clips, published_clips) in jobs {
        let result = sqlx::query(
            "UPDATE clipping_jobs
             SET status = 'pending',
                 resume_from = 'clips_extracted',
                 error_message = NULL,
                 progress_percent = 60,
                 current_step = 'queued_for_upload_recovery',
                 completed_at = NULL,
                 claimed_by = NULL,
                 claimed_at = NULL,
                 worker_heartbeat_at = NULL,
                 supervisor_status = 'healthy',
                 supervisor_reason = NULL,
                 supervisor_last_action = 'completed_upload_recovery_requeued',
                 supervisor_last_run_at = NOW(),
                 updated_at = NOW(),
                 retry_count = COALESCE(retry_count, 0) + 1,
                 last_retry_at = NOW()
             WHERE id = $1
               AND status = 'completed'
               AND fallback_delivery_id IS NULL
               AND NOT EXISTS (
                   SELECT 1
                   FROM clipping_jobs sibling
                   WHERE sibling.linkage_id = clipping_jobs.linkage_id
                     AND sibling.source_video_id = clipping_jobs.source_video_id
                     AND sibling.id <> clipping_jobs.id
                     AND sibling.status = ANY($2)
               )",
        )
        .bind(job_id)
        .bind(ACTIVE_CLIPPING_JOB_STATUSES)
        .execute(&app_state.db_pool)
        .await
        .map_err(|e| format!("Failed to requeue completed upload-recovery job {job_id}: {e}"))?;

        if result.rows_affected() > 0 {
            tracing::warn!(
                "Requeued completed job {} for upload recovery ({}/{} clips already published)",
                job_id,
                published_clips,
                total_clips
            );
        }
    }

    Ok(())
}

async fn reconcile_fallback_delivery_job_statuses(
    app_state: &Arc<AppState>,
) -> Result<(), String> {
    let failed = sqlx::query(
        "UPDATE clipping_jobs cj
         SET status = 'failed',
             progress_percent = 0,
             current_step = 'fallback_delivery_failed',
             error_message = CONCAT(
                 COALESCE(cj.error_message, 'Fallback delivery failed.'),
                 CASE
                   WHEN d.error_message IS NULL OR d.error_message = '' THEN ''
                   ELSE CONCAT(' Delivery error: ', d.error_message)
                 END
             ),
             completed_at = NULL,
             updated_at = NOW()
         FROM deliveries d
         WHERE d.id = cj.fallback_delivery_id
           AND cj.fallback_strategy = 'generated_summary_delivery'
           AND cj.status IN ('completed', 'fallback_rendering')
           AND d.status = 'failed'
           AND d.output_r2_url IS NULL",
    )
    .execute(&app_state.db_pool)
    .await
    .map_err(|e| format!("Failed to reconcile failed fallback deliveries: {e}"))?;

    if failed.rows_affected() > 0 {
        tracing::warn!(
            "Reconciled {} falsely-completed clipping jobs whose fallback deliveries failed",
            failed.rows_affected()
        );
    }

    let still_rendering = sqlx::query(
        "UPDATE clipping_jobs cj
         SET status = 'fallback_rendering',
             progress_percent = 85,
             current_step = 'fallback_delivery_rendering',
             completed_at = NULL,
             updated_at = NOW()
         FROM deliveries d
         WHERE d.id = cj.fallback_delivery_id
           AND cj.fallback_strategy = 'generated_summary_delivery'
           AND cj.status = 'completed'
           AND d.status IN ('pending', 'running')
           AND d.output_r2_url IS NULL",
    )
    .execute(&app_state.db_pool)
    .await
    .map_err(|e| format!("Failed to reconcile running fallback deliveries: {e}"))?;

    if still_rendering.rows_affected() > 0 {
        tracing::warn!(
            "Reconciled {} falsely-completed clipping jobs whose fallback deliveries are still rendering",
            still_rendering.rows_affected()
        );
    }

    let completed = sqlx::query(
        "UPDATE clipping_jobs cj
         SET status = 'completed',
             progress_percent = 100,
             current_step = 'fallback_delivery_completed',
             completed_at = COALESCE(cj.completed_at, NOW()),
             updated_at = NOW()
         FROM deliveries d
         WHERE d.id = cj.fallback_delivery_id
           AND cj.fallback_strategy = 'generated_summary_delivery'
           AND cj.status IN ('fallback_rendering', 'pending')
           AND d.status = 'completed'
           AND d.output_r2_url IS NOT NULL",
    )
    .execute(&app_state.db_pool)
    .await
    .map_err(|e| format!("Failed to reconcile completed fallback deliveries: {e}"))?;

    if completed.rows_affected() > 0 {
        tracing::warn!(
            "Reconciled {} fallback delivery jobs with generated media",
            completed.rows_affected()
        );
    }

    Ok(())
}

async fn recover_completed_fallback_deliveries_to_youtube(
    app_state: &Arc<AppState>,
) -> Result<(), String> {
    let Some(youtube_client) = app_state.youtube_client.as_ref() else {
        return Ok(());
    };
    let Some(oauth_client_id) = app_state.google_oauth_client_id.clone() else {
        return Ok(());
    };
    let Some(oauth_client_secret) = app_state.google_oauth_client_secret.clone() else {
        return Ok(());
    };

    let fallback_jobs: Vec<(
        i32,
        Uuid,
        i32,
        i32,
        String,
        String,
        String,
        String,
        Option<String>,
        Option<String>,
    )> = sqlx::query_as(
        "SELECT cj.id,
                d.id,
                ycl.user_id,
                ycl.destination_channel_id,
                COALESCE(cj.source_video_title, 'Generated fallback summary')::TEXT,
                d.title,
                d.prompt,
                d.output_r2_url,
                d.output_filename,
                d.source_url
         FROM clipping_jobs cj
         JOIN youtube_channel_linkages ycl ON ycl.id = cj.linkage_id
         JOIN connected_youtube_channels cyc ON cyc.id = ycl.destination_channel_id
         JOIN deliveries d ON d.id = cj.fallback_delivery_id
         WHERE cj.status = 'completed'
           AND cj.fallback_delivery_id IS NOT NULL
           AND cj.fallback_strategy = 'generated_summary_delivery'
           AND d.status = 'completed'
           AND d.output_r2_url IS NOT NULL
           AND d.youtube_video_id IS NULL
           AND COALESCE(cyc.requires_reauth, false) = false
           AND (
                d.youtube_upload_attempted_at IS NULL
                OR d.youtube_upload_attempted_at < NOW() - INTERVAL '30 minutes'
           )
         ORDER BY cj.updated_at ASC
         LIMIT 3",
    )
    .fetch_all(&app_state.db_pool)
    .await
    .map_err(|e| format!("Failed to fetch fallback delivery upload recovery jobs: {e}"))?;

    if fallback_jobs.is_empty() {
        return Ok(());
    }

    tracing::warn!(
        "Found {} completed fallback summary deliveries that still need YouTube upload",
        fallback_jobs.len()
    );

    let uploader = ClipUploader::new(
        Arc::new(youtube_client.clone()),
        app_state.db_pool.clone(),
        oauth_client_id,
        oauth_client_secret,
    );

    for (
        job_id,
        delivery_id,
        user_id,
        destination_channel_id,
        source_video_title,
        delivery_title,
        delivery_prompt,
        output_url,
        output_filename,
        source_url,
    ) in fallback_jobs
    {
        if let Err(error) = sqlx::query(
            "UPDATE deliveries
             SET youtube_upload_attempted_at = NOW(),
                 youtube_upload_error = NULL
             WHERE id = $1
               AND youtube_video_id IS NULL",
        )
        .bind(delivery_id)
        .execute(&app_state.db_pool)
        .await
        {
            tracing::warn!(
                "Failed to mark fallback delivery {} upload attempt: {}",
                delivery_id,
                error
            );
            continue;
        }

        let upload_result = async {
            let destination_channel =
                fetch_destination_channel(destination_channel_id, &app_state.db_pool).await?;
            let local_path =
                download_fallback_delivery_output(
                    app_state,
                    delivery_id,
                    &output_url,
                    output_filename.as_deref(),
                )
                .await?;
            let title = fallback_delivery_youtube_title(&source_video_title, &delivery_title);
            let description =
                fallback_delivery_youtube_description(&source_video_title, &delivery_prompt, source_url.as_deref());
            let tags = vec![
                "VideoSync".to_string(),
                "summary".to_string(),
                "animated summary".to_string(),
                "AI video".to_string(),
            ];

            let result = uploader
                .upload_longform_video(&local_path, &title, &description, &tags, &destination_channel)
                .await?;

            let _ = tokio::fs::remove_file(&local_path).await;

            Ok::<_, String>((result.video_id, result.url, title, description, local_path))
        }
        .await;

        match upload_result {
            Ok((youtube_video_id, youtube_url, title, description, local_path)) => {
                sqlx::query(
                    "UPDATE deliveries
                     SET youtube_video_id = $1,
                         youtube_url = $2,
                         youtube_uploaded_at = NOW(),
                         youtube_upload_error = NULL
                     WHERE id = $3",
                )
                .bind(&youtube_video_id)
                .bind(&youtube_url)
                .bind(delivery_id)
                .execute(&app_state.db_pool)
                .await
                .map_err(|e| format!("Failed to mark fallback delivery {delivery_id} uploaded: {e}"))?;

                sqlx::query(
                    "UPDATE clipping_jobs
                     SET current_step = 'fallback_posted_to_youtube',
                         updated_at = NOW()
                     WHERE id = $1",
                )
                .bind(job_id)
                .execute(&app_state.db_pool)
                .await
                .map_err(|e| format!("Failed to mark fallback job {job_id} posted: {e}"))?;

                sqlx::query(
                    "INSERT INTO youtube_uploads (
                         user_id, channel_id, local_video_path, youtube_video_id,
                         video_title, video_description, video_category, privacy_status,
                         upload_status, upload_progress, youtube_url, published_at,
                         created_at, updated_at
                     )
                     VALUES ($1, $2, $3, $4, $5, $6, '27', 'public',
                             'completed', 100, $7, NOW(), NOW(), NOW())
                     ON CONFLICT (youtube_video_id) DO NOTHING",
                )
                .bind(user_id)
                .bind(destination_channel_id)
                .bind(&local_path)
                .bind(&youtube_video_id)
                .bind(&title)
                .bind(&description)
                .bind(&youtube_url)
                .execute(&app_state.db_pool)
                .await
                .map_err(|e| format!("Failed to record fallback YouTube upload {youtube_video_id}: {e}"))?;

                tracing::warn!(
                    "Posted fallback summary delivery {} for clipping job {} to YouTube: {}",
                    delivery_id,
                    job_id,
                    youtube_url
                );
            }
            Err(error) => {
                mark_delivery_youtube_upload_error(&app_state.db_pool, delivery_id, &error).await;
                tracing::warn!(
                    "Fallback summary delivery {} upload recovery failed for job {}: {}",
                    delivery_id,
                    job_id,
                    error
                );
            }
        }
    }

    Ok(())
}

async fn download_fallback_delivery_output(
    app_state: &Arc<AppState>,
    delivery_id: Uuid,
    output_url: &str,
    output_filename: Option<&str>,
) -> Result<String, String> {
    let ext = output_filename
        .and_then(|filename| filename.rsplit('.').next())
        .filter(|ext| ext.len() <= 5 && ext.chars().all(|c| c.is_ascii_alphanumeric()))
        .unwrap_or("mp4");
    let local_path = format!("/tmp/videosync-fallback-deliveries/{delivery_id}.{ext}");

    tokio::fs::create_dir_all("/tmp/videosync-fallback-deliveries")
        .await
        .map_err(|e| format!("Failed to create fallback delivery temp directory: {e}"))?;

    if let Some((bucket, key)) = r2_bucket_and_key_from_url(output_url) {
        if let Some(r2_client) = app_state.r2_client.as_ref() {
            if bucket == r2_client.bucket {
                match r2_client.download(&key, &local_path).await {
                    Ok(()) => return Ok(local_path),
                    Err(error) => {
                        tracing::warn!(
                            "R2 download failed for fallback delivery {} key {}: {}",
                            delivery_id,
                            key,
                            error
                        );
                    }
                }
            }
        }
    }

    let response = reqwest::get(output_url)
        .await
        .map_err(|e| format!("Failed to download fallback delivery output: {e}"))?;
    if !response.status().is_success() {
        return Err(format!(
            "Fallback delivery output download returned HTTP {}",
            response.status()
        ));
    }

    let bytes = response
        .bytes()
        .await
        .map_err(|e| format!("Failed to read fallback delivery output bytes: {e}"))?;
    tokio::fs::write(&local_path, bytes)
        .await
        .map_err(|e| format!("Failed to save fallback delivery output: {e}"))?;

    Ok(local_path)
}

fn r2_bucket_and_key_from_url(output_url: &str) -> Option<(String, String)> {
    let url = url::Url::parse(output_url).ok()?;
    let host = url.host_str().unwrap_or_default();
    if !host.contains(".r2.cloudflarestorage.com") {
        return None;
    }

    let mut parts = url
        .path_segments()?
        .filter(|part| !part.is_empty());
    let bucket = parts.next()?.to_string();
    let key = parts.collect::<Vec<_>>().join("/");
    if key.is_empty() {
        None
    } else {
        Some((bucket, key))
    }
}

fn fallback_delivery_youtube_title(source_video_title: &str, delivery_title: &str) -> String {
    let base_title = if delivery_title.trim().is_empty() {
        source_video_title
    } else {
        delivery_title
    };
    let title = format!("{} | Animated Summary", base_title.trim());
    if title.chars().count() > 95 {
        let mut truncated: String = title.chars().take(92).collect();
        truncated.push_str("...");
        truncated
    } else {
        title
    }
}

fn fallback_delivery_youtube_description(
    source_video_title: &str,
    delivery_prompt: &str,
    source_url: Option<&str>,
) -> String {
    let mut description = format!(
        "Animated AI summary generated when the original source could not be downloaded.\n\nSource: {}\n\n{}",
        source_video_title.trim(),
        delivery_prompt.trim()
    );

    if let Some(url) = source_url.filter(|url| !url.trim().is_empty()) {
        description.push_str("\n\nOriginal reference: ");
        description.push_str(url.trim());
    }

    description.push_str("\n\nGenerated with VideoSync.");
    description
}

async fn mark_delivery_youtube_upload_error(pool: &sqlx::PgPool, delivery_id: Uuid, error: &str) {
    let truncated_error: String = error.chars().take(1000).collect();
    let _ = sqlx::query(
        "UPDATE deliveries
         SET youtube_upload_attempted_at = NOW(),
             youtube_upload_error = $1
         WHERE id = $2
           AND youtube_video_id IS NULL",
    )
    .bind(truncated_error)
    .bind(delivery_id)
    .execute(pool)
    .await;
}

// ============================================================================
// Manual clipping job processor
// ============================================================================

/// Poll for one pending manual clipping job and execute it.
async fn process_manual_jobs(app_state: &Arc<AppState>) -> Result<(), String> {
    let job_id: Option<uuid::Uuid> = sqlx::query_scalar(
        "SELECT id FROM manual_clipping_jobs WHERE status = 'pending' ORDER BY created_at ASC LIMIT 1"
    )
    .fetch_optional(&app_state.db_pool)
    .await
    .map_err(|e| format!("Failed to poll manual jobs: {}", e))?;

    if let Some(id) = job_id {
        tracing::info!("▶️  Executing manual clipping job {}", id);
        let state = Arc::clone(app_state);
        tokio::spawn(async move {
            match crate::jobs::manual_clipping_job::execute_manual_clipping_job(id, state.clone())
                .await
            {
                Ok(msg) => tracing::info!("✅ Manual job {} completed: {}", id, msg),
                Err(e) => {
                    tracing::error!("❌ Manual job {} failed: {}", id, e);
                    if let Err(db_err) = sqlx::query(
                        "UPDATE manual_clipping_jobs SET status = 'failed', error_message = $1, updated_at = NOW() WHERE id = $2"
                    )
                    .bind(&e)
                    .bind(id)
                    .execute(&state.db_pool)
                    .await
                    {
                        tracing::error!("Failed to mark manual clipping job {} as failed: {} — job may be stuck", id, db_err);
                    }
                    if let Ok(Some(Some(workflow_id))) = sqlx::query_scalar::<_, Option<uuid::Uuid>>(
                        "SELECT workflow_id FROM manual_clipping_jobs WHERE id = $1",
                    )
                    .bind(id)
                    .fetch_optional(&state.db_pool)
                    .await
                    {
                        let workflow_runtime =
                            crate::services::WorkflowRuntime::new(state.db_pool.clone());
                        let _ = workflow_runtime
                            .mark_failed(
                                workflow_id,
                                Some("manual_clipping_worker"),
                                &e,
                                None,
                            )
                            .await;
                    }
                }
            }
        });
    }

    Ok(())
}
