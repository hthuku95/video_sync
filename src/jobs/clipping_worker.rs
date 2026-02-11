// Background worker that polls for pending clipping jobs and executes them

use crate::jobs::clipping_job::execute_clipping_job;
use crate::AppState;
use std::sync::Arc;
use tokio::time::{sleep, Duration};

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

    /// Process all pending clipping jobs
    async fn process_pending_jobs(&self) -> Result<(), String> {
        // Step 1: Detect and reset stuck jobs in intermediate states
        self.detect_stuck_jobs().await?;

        // Step 2: Auto-retry failed jobs that are eligible for retry
        self.auto_retry_failed_jobs().await?;

        // Step 3: Fetch pending AND failed jobs (limit to avoid overload)
        // Note: Failed jobs are also processed directly (in addition to auto-retry)
        let pending_jobs: Vec<i32> = sqlx::query_scalar(
            "SELECT id FROM clipping_jobs
             WHERE status IN ('pending', 'failed')
             ORDER BY
               CASE WHEN status = 'pending' THEN 0 ELSE 1 END,
               created_at ASC
             LIMIT 5",
        )
        .fetch_all(&self.app_state.db_pool)
        .await
        .map_err(|e| format!("Failed to fetch jobs: {}", e))?;

        if pending_jobs.is_empty() {
            tracing::debug!("No pending or failed clipping jobs");
            return Ok(());
        }

        tracing::info!("📋 Found {} jobs to process (pending + failed)", pending_jobs.len());

        // Process each job (both pending and failed)
        for job_id in pending_jobs {
            // Check current status before processing
            let current_status: Option<String> = sqlx::query_scalar(
                "SELECT status FROM clipping_jobs WHERE id = $1"
            )
            .bind(job_id)
            .fetch_optional(&self.app_state.db_pool)
            .await
            .ok()
            .flatten();

            if let Some(status) = current_status {
                if status == "failed" {
                    tracing::info!("🔄 Retrying failed job {}", job_id);
                    // Reset to pending before executing
                    let _ = sqlx::query(
                        "UPDATE clipping_jobs
                         SET status = 'pending',
                             error_message = NULL,
                             progress_percent = 0,
                             current_step = 'queued',
                             updated_at = NOW()
                         WHERE id = $1"
                    )
                    .bind(job_id)
                    .execute(&self.app_state.db_pool)
                    .await;
                }
            }

            tracing::info!("▶️  Executing clipping job {}", job_id);

            match execute_clipping_job(job_id, self.app_state.clone()).await {
                Ok(msg) => {
                    tracing::info!("✅ Job {} completed: {}", job_id, msg);
                }
                Err(e) => {
                    tracing::error!("❌ Job {} failed: {}", job_id, e);

                    // Mark job as failed
                    let _ = sqlx::query(
                        "UPDATE clipping_jobs
                         SET status = 'failed', error_message = $1, completed_at = NOW()
                         WHERE id = $2",
                    )
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
    async fn auto_retry_failed_jobs(&self) -> Result<(), String> {
        // Find failed jobs eligible for retry
        let retry_job_ids: Vec<i32> = sqlx::query_scalar(
            "SELECT id FROM clipping_jobs
             WHERE status = 'failed'
             AND completed_at > NOW() - INTERVAL '6 hours'
             AND completed_at < NOW() - INTERVAL '5 minutes'
             ORDER BY completed_at ASC
             LIMIT 10",
        )
        .fetch_all(&self.app_state.db_pool)
        .await
        .map_err(|e| format!("Failed to fetch failed jobs for retry: {}", e))?;

        if retry_job_ids.is_empty() {
            return Ok(());
        }

        tracing::info!("🔄 Found {} failed jobs eligible for automatic retry", retry_job_ids.len());

        // Reset them to pending status
        for job_id in retry_job_ids {
            match sqlx::query(
                "UPDATE clipping_jobs
                 SET status = 'pending',
                     error_message = NULL,
                     progress_percent = 0,
                     current_step = 'queued',
                     started_at = NULL,
                     completed_at = NULL,
                     updated_at = NOW()
                 WHERE id = $1
                 AND status = 'failed'",
            )
            .bind(job_id)
            .execute(&self.app_state.db_pool)
            .await
            {
                Ok(_) => {
                    tracing::info!("✅ Job {} automatically reset to pending for retry", job_id);
                }
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
        // Find jobs stuck in each intermediate state
        let stuck_jobs: Vec<(i32, String, String)> = sqlx::query_as(
            "SELECT id, status, updated_at::text
             FROM clipping_jobs
             WHERE (
                 (status = 'downloading' AND updated_at < NOW() - INTERVAL '10 minutes') OR
                 (status = 'analyzing' AND updated_at < NOW() - INTERVAL '60 minutes') OR
                 (status = 'extracting_clips' AND updated_at < NOW() - INTERVAL '15 minutes') OR
                 (status = 'posting' AND updated_at < NOW() - INTERVAL '20 minutes')
             )"
        )
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

            match sqlx::query(
                "UPDATE clipping_jobs
                 SET status = 'failed',
                     error_message = $1,
                     completed_at = NOW(),
                     updated_at = NOW()
                 WHERE id = $2"
            )
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
