// Background worker that polls for pending clipping jobs and executes them

use crate::agent::clipping_agent::GeminiClippingAgent;
use crate::jobs::clipping_job::execute_clipping_job;
use crate::jobs::error_classifier::{classify, ErrorClass};
use crate::jobs::job_claimer::JobClaimer;
use crate::jobs::worker_config::WorkerConfig;
use crate::AppState;
use std::sync::Arc;
use tokio::task::JoinSet;
use tokio::time::{sleep, Duration};

/// Process clipping jobs once (function-based to avoid lifetime issues)
pub async fn process_clipping_jobs_once(app_state: &Arc<AppState>) -> Result<(), String> {
    // Create a temporary worker instance
    let worker = ClippingWorker::new(app_state.clone(), 60);
    worker.process_pending_jobs().await
}

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
        tracing::info!("⏳ Worker startup delay: {}s (set via WORKER_STARTUP_DELAY_SECS)", startup_delay_secs);
        sleep(Duration::from_secs(startup_delay_secs)).await;
        tracing::info!("✅ Startup delay complete, worker is now active");
    }

    let mut interval = tokio::time::interval(
        tokio::time::Duration::from_secs(config.poll_interval_secs)
    );

    loop {
        interval.tick().await;
        update_worker_heartbeat(&app_state, &config.worker_id, None).await;

        match process_clipping_jobs_parallel(&app_state, &config).await {
            Ok(_) => {}
            Err(e) => {
                tracing::error!("❌ Clipping worker error: {}", e);
            }
        }
    }
}

pub struct ClippingWorker {
    app_state: Arc<AppState>,
    poll_interval_seconds: u64,
}

impl ClippingWorker {
    pub fn new(app_state: Arc<AppState>, poll_interval_seconds: u64) -> Self {
        Self {
            app_state,
            poll_interval_seconds,
        }
    }

    /// Start the worker loop (runs forever)
    pub async fn run(self) {
        tracing::info!(
            "🔧 Clipping worker started (polling every {}s)",
            self.poll_interval_seconds
        );

        loop {
            if let Err(e) = self.process_pending_jobs().await {
                tracing::error!("Worker error: {}", e);
            }

            sleep(Duration::from_secs(self.poll_interval_seconds)).await;
        }
    }

    /// Process all pending clipping jobs (public for external use)
    pub async fn process_pending_jobs_once(&self) -> Result<(), String> {
        self.process_pending_jobs().await
    }

    /// Process all pending clipping jobs (backward compatibility)
    async fn process_pending_jobs(&self) -> Result<(), String> {
        self.detect_stuck_jobs().await?;
        self.auto_retry_failed_jobs().await?;

        let query = String::from(
            "SELECT cj.id FROM clipping_jobs cj \
             JOIN youtube_channel_linkages l ON l.id = cj.linkage_id \
             WHERE cj.status = 'pending' \
             ORDER BY \
               CASE WHEN l.user_id = -1 THEN 0 ELSE 1 END, \
               cj.created_at ASC \
             LIMIT 1"
        );
        let job_id: Option<i32> = sqlx::query_scalar(&query)
            .fetch_optional(&self.app_state.db_pool)
            .await
            .map_err(|e| format!("Failed to fetch job: {}", e))?;

        if let Some(job_id) = job_id {
            tracing::info!("▶️  Executing clipping job {} (sequential mode)", job_id);

            match execute_clipping_job(job_id, self.app_state.clone()).await {
                Ok(msg) => {
                    tracing::info!("✅ Job {} completed: {}", job_id, msg);
                }
                Err(e) => {
                    tracing::error!("❌ Job {} failed: {}", job_id, e);

                    let new_status = match classify(&e) {
                        ErrorClass::Permanent => "cancelled",
                        _ => "failed",
                    };

                    let fail_query = format!(
                        "UPDATE clipping_jobs \
                         SET status = '{}', error_message = $1, completed_at = NOW() \
                         WHERE id = $2",
                        new_status
                    );
                    let _ = sqlx::query(&fail_query)
                        .bind(&e)
                        .bind(job_id)
                        .execute(&self.app_state.db_pool)
                        .await;
                }
            }
        }

        Ok(())
    }

    /// Automatically retry failed jobs — delegates to standalone function
    async fn auto_retry_failed_jobs(&self) -> Result<(), String> {
        auto_retry_failed_jobs(&self.app_state).await
    }

    /// Detect stuck jobs — delegates to standalone function
    async fn detect_stuck_jobs(&self) -> Result<(), String> {
        detect_stuck_jobs(&self.app_state).await
    }
}

// ============================================================================
// True parallel job processor — JoinSet-based
// ============================================================================

/// Process clipping jobs in parallel using JoinSet.
/// Fill JoinSet to concurrency limit, drain one slot before claiming next job.
async fn process_clipping_jobs_parallel(
    app_state: &Arc<AppState>,
    config: &WorkerConfig,
) -> Result<(), String> {
    detect_stuck_jobs(app_state).await?;
    auto_retry_failed_jobs(app_state).await?;
    check_pending_too_long(app_state).await;

    // Compile-time assertion: GeminiClippingAgent must be Send + 'static for JoinSet::spawn
    #[allow(dead_code)]
    fn _assert_agent_send() {
        fn is_send<T: Send + 'static>() {}
        is_send::<GeminiClippingAgent>();
    }

    let pending_count: i64 = sqlx::query_scalar(
        "SELECT COUNT(*) FROM clipping_jobs WHERE status = 'pending' AND claimed_by IS NULL"
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
                    join_set.spawn(async move {
                        execute_claimed_job(state, job_id).await
                    });
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
                if e.contains("RESOURCE_EXHAUSTED") || e.contains("Resource has been exhausted") || e.contains("quota") {
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
        tracing::warn!("⏸️  Pausing 120s for Gemini quota recovery");
        sleep(Duration::from_secs(120)).await;
    }

    if total_completed > 0 || total_failed > 0 {
        tracing::info!("📊 Cycle: {} completed, {} failed", total_completed, total_failed);
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

    let execution_result = if app_state.gemini_client.is_some() {
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
                    tracing::warn!("🚫 Job {} permanently failed ({}), setting status=cancelled", job_id, e);
                    "cancelled"
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
            let _ = sqlx::query(&fail_query)
                .bind(&e)
                .bind(job_id)
                .execute(&app_state.db_pool)
                .await;

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
            ).await {
                Ok(Ok(_)) => {}
                Ok(Err(e)) => {
                    tracing::error!("❌ Failed to reset timed-out job {}: {} — stuck-detection will handle it", job_id, e);
                }
                Err(_) => {
                    tracing::error!("❌ Cleanup for timed-out job {} timed out after 10s — stuck-detection will handle it", job_id);
                }
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
    let _ = sqlx::query(
        "INSERT INTO worker_heartbeats (worker_id, last_seen_at, updated_at, current_job_id)
         VALUES ($1, NOW(), NOW(), $2)
         ON CONFLICT (worker_id) DO UPDATE
           SET last_seen_at = NOW(), updated_at = NOW(), current_job_id = EXCLUDED.current_job_id"
    )
    .bind(worker_id)
    .bind(current_job_id)
    .execute(&app_state.db_pool)
    .await;
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
         AND claimed_at < NOW() - INTERVAL '5 minutes'"
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
    let stuck_query = String::from(
        "SELECT id, status, COALESCE(worker_heartbeat_at, updated_at)::text AS last_seen \
         FROM clipping_jobs \
         WHERE status IN ('downloading', 'analyzing', 'extracting_clips', 'posting') \
           AND ( \
               (worker_heartbeat_at IS NOT NULL \
                AND worker_heartbeat_at < NOW() - INTERVAL '3 minutes') \
               OR \
               (worker_heartbeat_at IS NULL AND ( \
                   (status = 'downloading'      AND updated_at < NOW() - INTERVAL '25 minutes') OR \
                   (status = 'analyzing'        AND updated_at < NOW() - INTERVAL '10 minutes') OR \
                   (status = 'extracting_clips' AND updated_at < NOW() - INTERVAL '15 minutes') OR \
                   (status = 'posting'          AND updated_at < NOW() - INTERVAL '30 minutes') \
               )) \
           )"
    );
    let stuck_jobs: Vec<(i32, String, String)> = sqlx::query_as(&stuck_query)
        .fetch_all(&app_state.db_pool)
        .await
        .map_err(|e| format!("Failed to fetch stuck jobs: {}", e))?;

    if stuck_jobs.is_empty() {
        return Ok(());
    }

    tracing::warn!("🔄 Found {} stuck jobs, resetting to failed", stuck_jobs.len());

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
             WHERE id = $2"
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
                    job_id, status, last_seen
                );
            }
            Err(e) => {
                tracing::error!("Failed to reset stuck job {}: {}", job_id, e);
            }
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
           AND created_at < NOW() - INTERVAL '15 minutes'"
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
                   AND created_at < NOW() - INTERVAL '60 minutes'"
            )
            .fetch_one(&app_state.db_pool)
            .await
            .unwrap_or(false);

            if is_critical {
                tracing::error!(
                    "🚨 WORKER ALERT: {} jobs pending >15 min unclaimed (oldest: {}). \
                     Worker may be down or severely overloaded!",
                    count, oldest
                );
            } else {
                tracing::warn!(
                    "⚠️  {} jobs pending >15 min unclaimed (oldest: {}). \
                     Worker may be slow or restarting.",
                    count, oldest
                );
            }
        }
    }
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
    // First: discard exhausted jobs (>= 10 retries) — move to dead-letter
    let _ = sqlx::query(
        "UPDATE clipping_jobs \
         SET status = 'discarded', \
             error_message = 'Exhausted all 10 retries. Use admin API to retry manually.', \
             updated_at = NOW() \
         WHERE status = 'failed' \
           AND COALESCE(retry_count, 0) >= 10 \
           AND updated_at > NOW() - INTERVAL '1 hour' \
         RETURNING id"
    )
    .execute(&app_state.db_pool)
    .await;

    // Warn about jobs approaching discard threshold
    let exhausted: Vec<(i32, Option<String>)> = sqlx::query_as(
        "SELECT id, error_message FROM clipping_jobs \
         WHERE status = 'discarded' \
         AND updated_at > NOW() - INTERVAL '1 hour'"
    )
    .fetch_all(&app_state.db_pool)
    .await
    .unwrap_or_default();

    for (job_id, err) in exhausted {
        tracing::error!(
            "🚨 CLIPPING JOB {} DISCARDED (exhausted 10 retries) — admin review required. \
             Use POST /api/admin/clipping/jobs/{}/retry to reset. Last error: {:?}",
            job_id, job_id, err
        );
    }

    // Fetch failed jobs eligible for retry.
    // Exponential backoff: cooldown = 2^retry_count minutes (capped at 256 min = ~4h).
    // Quota errors always get an extra 30-minute floor via OR clause.
    let retry_jobs: Vec<(i32, Option<String>)> = sqlx::query_as(
        "SELECT id, current_step FROM clipping_jobs \
         WHERE status = 'failed' \
         AND completed_at > NOW() - INTERVAL '7 days' \
         AND COALESCE(retry_count, 0) < 10 \
         AND ( \
             (error_message NOT LIKE '%RESOURCE_EXHAUSTED%' \
              AND completed_at < NOW() - (INTERVAL '1 minute' * POWER(2, LEAST(COALESCE(retry_count, 0), 8)))) \
             OR \
             (error_message LIKE '%RESOURCE_EXHAUSTED%' \
              AND completed_at < NOW() - INTERVAL '30 minutes') \
         ) \
         ORDER BY completed_at ASC \
         LIMIT 10"
    )
    .fetch_all(&app_state.db_pool)
    .await
    .map_err(|e| format!("Failed to fetch failed jobs for retry: {}", e))?;

    if retry_jobs.is_empty() {
        return Ok(());
    }

    tracing::info!("🔄 Found {} failed jobs eligible for automatic retry", retry_jobs.len());

    for (job_id, current_step) in retry_jobs {
        let resume_from: Option<&str> = match current_step.as_deref().unwrap_or("") {
            s if s.contains("posting") || s.contains("upload") => Some("clips_extracted"),
            s if s.contains("vectoriz")                         => Some("clips_extracted"),
            s if s.contains("extracting") || s == "clips_extracted" => Some("downloaded"),
            s if s.contains("download")                         => Some("analyzed"),
            _                                                   => None,
        };

        if let Some(phase) = resume_from {
            tracing::info!(
                "✅ Job {} reset to pending, will resume from '{}' (was at: {:?})",
                job_id, phase, current_step
            );
        } else {
            tracing::info!(
                "✅ Job {} reset to pending, will restart from Phase A (current_step: {:?})",
                job_id, current_step
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
                 updated_at = NOW(), \
                 retry_count = COALESCE(retry_count, 0) + 1, \
                 last_retry_at = NOW() \
             WHERE id = $2 \
             AND status = 'failed'"
        );
        match sqlx::query(&reset_query)
            .bind(resume_from)
            .bind(job_id)
            .execute(&app_state.db_pool)
            .await
        {
            Ok(_) => {}
            Err(e) => {
                tracing::error!("Failed to reset job {} for retry: {}", job_id, e);
            }
        }
    }

    Ok(())
}
