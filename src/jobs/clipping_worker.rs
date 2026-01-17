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
        // Fetch pending jobs (limit to avoid overload)
        let pending_jobs: Vec<i32> = sqlx::query_scalar(
            "SELECT id FROM clipping_jobs
             WHERE status = 'pending'
             ORDER BY created_at ASC
             LIMIT 5",
        )
        .fetch_all(&self.app_state.db_pool)
        .await
        .map_err(|e| format!("Failed to fetch pending jobs: {}", e))?;

        if pending_jobs.is_empty() {
            tracing::debug!("No pending clipping jobs");
            return Ok(());
        }

        tracing::info!("📋 Found {} pending clipping jobs", pending_jobs.len());

        // Process each job
        for job_id in pending_jobs {
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
}
