// Background worker that polls for pending clipping jobs and executes them

use crate::agent::clipping_agent::GeminiClippingAgent;
use crate::jobs::clipping_job::execute_clipping_job;
use crate::jobs::job_claimer::JobClaimer;
use crate::jobs::worker_config::WorkerConfig;
use crate::AppState;
use std::sync::Arc;
use tokio::time::{sleep, Duration};

/// Process clipping jobs once (function-based to avoid lifetime issues)
pub async fn process_clipping_jobs_once(app_state: &Arc<AppState>) -> Result<(), String> {
    // Create a temporary worker instance
    let worker = ClippingWorker::new(app_state.clone(), 60);
    worker.process_pending_jobs().await
}

/// Run the clipping worker in a background loop (spawnable)
/// Now supports parallel job processing with configurable concurrency
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
    // before the first worker cycle runs. Set WORKER_STARTUP_DELAY_SECS=120 for testing.
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

        // Process jobs in parallel using the new architecture
        match process_clipping_jobs_parallel(&app_state, &config).await {
            Ok(_) => {
                // Success is logged by the worker itself
            }
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
        // Legacy method - redirects to sequential processing for backward compatibility
        // This ensures existing code that calls this method continues to work
        self.detect_stuck_jobs().await?;
        self.auto_retry_failed_jobs().await?;

        // Fetch a single job and process it (sequential mode)
        // Test-user jobs (user_id = -1) are prioritized so integration tests don't queue behind production jobs
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

                    let fail_query = String::from(
                        "UPDATE clipping_jobs \
                         SET status = 'failed', error_message = $1, completed_at = NOW() \
                         WHERE id = $2"
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

    /// Automatically retry failed jobs that meet retry criteria
    ///
    /// Retry criteria:
    /// - Job status is 'failed'
    /// - Job failed within the last 6 hours (to avoid retrying old failures)
    /// - Job has been failed for at least 5 minutes (to avoid immediate retry loops)
    ///
    /// Phase-aware: sets resume_from based on current_step so retries skip completed phases.
    async fn auto_retry_failed_jobs(&self) -> Result<(), String> {
        // Fetch id + current_step to determine the appropriate resume point
        let retry_jobs: Vec<(i32, Option<String>)> = sqlx::query_as(
            "SELECT id, current_step FROM clipping_jobs \
             WHERE status = 'failed' \
             AND completed_at > NOW() - INTERVAL '6 hours' \
             AND completed_at < NOW() - INTERVAL '5 minutes' \
             AND COALESCE(retry_count, 0) < 5 \
             ORDER BY completed_at ASC \
             LIMIT 10"
        )
        .fetch_all(&self.app_state.db_pool)
        .await
        .map_err(|e| format!("Failed to fetch failed jobs for retry: {}", e))?;

        // Escalate jobs that have exhausted all retries — admin must manually retry via admin API
        let exhausted: Vec<(i32, Option<String>)> = sqlx::query_as(
            "SELECT id, error_message FROM clipping_jobs \
             WHERE status = 'failed' \
             AND COALESCE(retry_count, 0) >= 5 \
             AND updated_at > NOW() - INTERVAL '1 hour'"
        )
        .fetch_all(&self.app_state.db_pool)
        .await
        .unwrap_or_default();

        for (job_id, err) in exhausted {
            tracing::warn!(
                "🚨 CLIPPING JOB {} EXHAUSTED ALL 5 RETRIES — admin review required. \
                 Use POST /api/admin/clipping/jobs/{}/retry to reset. Last error: {:?}",
                job_id, job_id, err
            );
        }

        if retry_jobs.is_empty() {
            return Ok(());
        }

        tracing::info!("🔄 Found {} failed jobs eligible for automatic retry", retry_jobs.len());

        for (job_id, current_step) in retry_jobs {
            // Determine the resume point based on where the job was when it failed.
            // current_step mirrors the last status set by update_job_status().
            let resume_from: Option<&str> = match current_step.as_deref().unwrap_or("") {
                s if s.contains("posting") || s.contains("upload") => Some("clips_extracted"),
                s if s.contains("vectoriz")                         => Some("clips_extracted"),
                s if s.contains("extracting") || s == "clips_extracted" => Some("downloaded"),
                s if s.contains("download")                         => Some("analyzed"),
                _                                                   => None,
            };

            if let Some(phase) = resume_from {
                tracing::info!(
                    "✅ Job {} will retry from '{}' (was stuck at: {:?})",
                    job_id, phase, current_step
                );
            } else {
                tracing::info!(
                    "✅ Job {} will retry from Phase A (current_step: {:?})",
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
                     updated_at = NOW(), \
                     retry_count = COALESCE(retry_count, 0) + 1, \
                     last_retry_at = NOW() \
                 WHERE id = $2 \
                 AND status = 'failed'"
            );
            match sqlx::query(&reset_query)
                .bind(resume_from)
                .bind(job_id)
                .execute(&self.app_state.db_pool)
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

    /// Detect jobs stuck in intermediate states and reset them to 'failed'
    ///
    /// Jobs stuck in intermediate states for longer than their timeout threshold are
    /// automatically marked as failed. This prevents jobs from hanging indefinitely
    /// due to process crashes, network issues, or other failures.
    ///
    /// State-specific timeout thresholds:
    /// - downloading: 10 minutes (video downloads should complete quickly)
    /// - analyzing: 60 minutes (vectorization can take long for big videos)
    /// - extracting_clips: 15 minutes (clip extraction + AI analysis)
    /// - posting: 20 minutes (YouTube API uploads can be slow)
    async fn detect_stuck_jobs(&self) -> Result<(), String> {
        // Release stale claims — pending jobs with claimed_by set from possibly dead workers
        let stale_claim_query = String::from(
            "UPDATE clipping_jobs \
             SET claimed_by = NULL, claimed_at = NULL, updated_at = NOW() \
             WHERE status = 'pending' \
             AND claimed_by IS NOT NULL \
             AND claimed_at < NOW() - INTERVAL '5 minutes'"
        );
        match sqlx::query(&stale_claim_query)
            .execute(&self.app_state.db_pool)
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

        // Find jobs stuck in each intermediate state
        let stuck_query = String::from(
            "SELECT id, status, updated_at::text \
             FROM clipping_jobs \
             WHERE ( \
                 (status = 'downloading' AND updated_at < NOW() - INTERVAL '10 minutes') OR \
                 (status = 'analyzing' AND updated_at < NOW() - INTERVAL '60 minutes') OR \
                 (status = 'extracting_clips' AND updated_at < NOW() - INTERVAL '10 minutes') OR \
                 (status = 'posting' AND updated_at < NOW() - INTERVAL '20 minutes') \
             )"
        );
        let stuck_jobs: Vec<(i32, String, String)> = sqlx::query_as(&stuck_query)
            .fetch_all(&self.app_state.db_pool)
            .await
            .map_err(|e| format!("Failed to fetch stuck jobs: {}", e))?;

        if stuck_jobs.is_empty() {
            return Ok(());
        }

        tracing::warn!("🔄 Found {} stuck jobs, resetting to failed", stuck_jobs.len());

        // Reset each stuck job to 'failed' with clear error message
        for (job_id, status, updated_at) in stuck_jobs {
            let error_message = format!(
                "Job stuck/timed out in '{}' state. Last updated: {}. Automatically reset by worker.",
                status, updated_at
            );

            let reset_stuck_query = String::from(
                "UPDATE clipping_jobs \
                 SET status = 'failed', \
                     error_message = $1, \
                     completed_at = NOW(), \
                     updated_at = NOW(), \
                     stuck_detection_count = COALESCE(stuck_detection_count, 0) + 1 \
                 WHERE id = $2"
            );
            match sqlx::query(&reset_stuck_query)
                .bind(&error_message)
                .bind(job_id)
                .execute(&self.app_state.db_pool)
                .await
            {
                Ok(_) => {
                    tracing::info!(
                        "✅ Job {} reset from '{}' to 'failed' (stuck for too long)",
                        job_id, status
                    );
                }
                Err(e) => {
                    tracing::error!("Failed to reset stuck job {}: {}", job_id, e);
                }
            }
        }

        Ok(())
    }
}

/// Process clipping jobs in parallel using JoinSet
/// This is the new high-performance parallel job processor
async fn process_clipping_jobs_parallel(
    app_state: &Arc<AppState>,
    config: &WorkerConfig,
) -> Result<(), String> {
    // Step 1: Pre-processing (stuck jobs, auto-retry) - keep sequential
    // These are quick operations that prepare jobs for parallel execution
    detect_stuck_jobs(app_state).await?;
    auto_retry_failed_jobs(app_state).await?;

    // Step 2: Check if there are any jobs to process
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

    // Step 3: Process jobs using atomic claiming (sequential for now)
    // TODO: Implement true parallel execution once Send trait lifetime issues are resolved
    // The current implementation uses atomic job claiming which prevents race conditions
    // but processes jobs sequentially. This is still an improvement over the old implementation.
    let worker_id = config.worker_id.clone();
    let mut total_completed = 0;
    let mut total_failed = 0;

    // Process up to concurrency * 5 jobs per cycle (burst processing)
    let max_jobs_per_cycle = config.concurrency * 5;

    for _ in 0..max_jobs_per_cycle {
        match process_single_job_with_claim(app_state.clone(), worker_id.clone()).await {
            Ok(job_id) => {
                total_completed += 1;
                tracing::info!("✅ Job {} completed successfully", job_id);
            }
            Err(e) => {
                if e.contains("No jobs available") {
                    // No more jobs, exit loop
                    break;
                } else {
                    total_failed += 1;
                    tracing::error!("❌ Job processing failed: {}", e);
                }
            }
        }
    }

    if total_completed > 0 || total_failed > 0 {
        tracing::info!(
            "📊 Processing cycle complete: {} completed, {} failed",
            total_completed,
            total_failed
        );
    }

    Ok(())
}

/// Process a single job with atomic claiming
async fn process_single_job_with_claim(
    app_state: Arc<AppState>,
    worker_id: String,
) -> Result<i32, String> {
    // Create job claimer for this task
    let claimer = JobClaimer::new(worker_id, app_state.db_pool.clone());

    // Step 1: Atomically claim next available job
    let job_id = match claimer.claim_next_job().await? {
        Some(id) => id,
        None => return Err("No jobs available".to_string()),
    };

    tracing::info!("🎬 Processing job {} (claimed)", job_id);

    // Step 2: Execute the clipping job with a per-job timeout.
    // Uses ClippingAgent (Claude-orchestrated) when claude_client is configured;
    // falls back to execute_clipping_job (hardcoded sequential) otherwise.
    let job_timeout_secs: u64 = std::env::var("JOB_EXECUTION_TIMEOUT_SECS")
        .ok()
        .and_then(|v| v.parse().ok())
        .unwrap_or(7200); // 2 hours — accommodates new pipeline

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

            // Mark job as failed and release claim
            let fail_query = String::from(
                "UPDATE clipping_jobs \
                 SET status = 'failed', \
                     error_message = $1, \
                     completed_at = NOW(), \
                     claimed_by = NULL, \
                     updated_at = NOW() \
                 WHERE id = $2"
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

            // Mark as failed and release claim.
            // Use the shared pool directly with a short total timeout. Creating a new pool
            // here (old approach) caused connect() to hang indefinitely — blocking the
            // entire worker thread. The shared pool (20 connections) is always available.
            // The independent stuck-detection task (every 60s) will handle the job if
            // this update somehow fails.
            let fail_query = "UPDATE clipping_jobs \
                 SET status = 'failed', \
                     error_message = $1, \
                     completed_at = NOW(), \
                     claimed_by = NULL, \
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
                    tracing::error!("❌ Failed to reset timed-out job {} (pool error): {} — stuck-detection will handle it", job_id, e);
                }
                Err(_) => {
                    tracing::error!("❌ Cleanup for timed-out job {} timed out after 10s — stuck-detection will handle it", job_id);
                }
            }

            Err(format!("Job {} timed out", job_id))
        }
    }
}

/// Public wrapper — called by the main tokio runtime as an independent background task.
/// Runs every 60s regardless of whether the worker thread is busy processing a job.
pub async fn run_stuck_job_detection(app_state: &Arc<AppState>) -> Result<(), String> {
    detect_stuck_jobs(app_state).await
}

/// Detect jobs stuck in intermediate states and reset them to 'failed' (standalone function)
async fn detect_stuck_jobs(app_state: &Arc<AppState>) -> Result<(), String> {
    // First: release stale claims — jobs that are 'pending' but have a claimed_by set from a
    // (possibly dead) worker process. This happens when the worker claims a job but then
    // crashes or loses DB connectivity before it can start executing.
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

    let stuck_query = String::from(
        "SELECT id, status, updated_at::text \
         FROM clipping_jobs \
         WHERE ( \
             (status = 'downloading' AND updated_at < NOW() - INTERVAL '10 minutes') OR \
             (status = 'analyzing' AND updated_at < NOW() - INTERVAL '60 minutes') OR \
             (status = 'extracting_clips' AND updated_at < NOW() - INTERVAL '10 minutes') OR \
             (status = 'posting' AND updated_at < NOW() - INTERVAL '20 minutes') \
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

    for (job_id, status, updated_at) in stuck_jobs {
        let error_message = format!(
            "Job stuck/timed out in '{}' state. Last updated: {}. Automatically reset by worker.",
            status, updated_at
        );

        let reset_stuck_query = String::from(
            "UPDATE clipping_jobs \
             SET status = 'failed', \
                 error_message = $1, \
                 completed_at = NOW(), \
                 updated_at = NOW(), \
                 claimed_by = NULL, \
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
                    "✅ Job {} reset from '{}' to 'failed' (stuck for too long)",
                    job_id, status
                );
            }
            Err(e) => {
                tracing::error!("Failed to reset stuck job {}: {}", job_id, e);
            }
        }
    }

    Ok(())
}

/// Automatically retry failed jobs that meet retry criteria (standalone function)
///
/// Phase-aware: sets resume_from so retries skip completed phases rather than re-running
/// from Phase A every time. A job that failed at "posting" resumes at "clips_extracted",
/// skipping Gemini re-analysis, re-download, and re-extraction.
async fn auto_retry_failed_jobs(app_state: &Arc<AppState>) -> Result<(), String> {
    // Fetch id + current_step to determine the appropriate resume point per job
    let retry_jobs: Vec<(i32, Option<String>)> = sqlx::query_as(
        "SELECT id, current_step FROM clipping_jobs \
         WHERE status = 'failed' \
         AND completed_at > NOW() - INTERVAL '6 hours' \
         AND completed_at < NOW() - INTERVAL '5 minutes' \
         AND COALESCE(retry_count, 0) < 5 \
         ORDER BY completed_at ASC \
         LIMIT 10"
    )
    .fetch_all(&app_state.db_pool)
    .await
    .map_err(|e| format!("Failed to fetch failed jobs for retry: {}", e))?;

    // Escalate jobs that have exhausted all retries — admin must manually retry via admin API
    let exhausted: Vec<(i32, Option<String>)> = sqlx::query_as(
        "SELECT id, error_message FROM clipping_jobs \
         WHERE status = 'failed' \
         AND COALESCE(retry_count, 0) >= 5 \
         AND updated_at > NOW() - INTERVAL '1 hour'"
    )
    .fetch_all(&app_state.db_pool)
    .await
    .unwrap_or_default();

    for (job_id, err) in exhausted {
        tracing::warn!(
            "🚨 CLIPPING JOB {} EXHAUSTED ALL 5 RETRIES — admin review required. \
             Use POST /api/admin/clipping/jobs/{}/retry to reset. Last error: {:?}",
            job_id, job_id, err
        );
    }

    if retry_jobs.is_empty() {
        return Ok(());
    }

    tracing::info!("🔄 Found {} failed jobs eligible for automatic retry", retry_jobs.len());

    for (job_id, current_step) in retry_jobs {
        // Map current_step → resume_from so execute_clipping_job() skips already-done phases.
        // current_step is set by update_job_status() and mirrors the last known status.
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
