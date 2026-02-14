// Atomic job claiming module for parallel worker coordination
// Uses PostgreSQL FOR UPDATE SKIP LOCKED to prevent race conditions

use sqlx::PgPool;

pub struct JobClaimer {
    worker_id: String,
    db_pool: PgPool,
}

impl JobClaimer {
    pub fn new(worker_id: String, db_pool: PgPool) -> Self {
        Self { worker_id, db_pool }
    }

    /// Atomically claim the next available pending job
    ///
    /// This uses PostgreSQL's FOR UPDATE SKIP LOCKED to ensure:
    /// - Only one worker can claim a job (atomicity)
    /// - Workers don't wait on locked rows (skip locked)
    /// - No duplicate processing (transaction safety)
    ///
    /// The job status remains 'pending' while claimed - coordination is via claimed_by column.
    /// This avoids violating the valid_status_values constraint which only allows 11 statuses.
    ///
    /// Returns:
    /// - Ok(Some(job_id)) if a job was successfully claimed
    /// - Ok(None) if no jobs are available
    /// - Err(String) if database error occurred
    pub async fn claim_next_job(&self) -> Result<Option<i32>, String> {
        // Clone worker_id to avoid lifetime issues with sqlx bind across await points
        let worker_id = self.worker_id.clone();

        // Atomically claim job using claimed_by column (status stays 'pending')
        let claimed_job_id: Option<i32> = sqlx::query_scalar(
            "UPDATE clipping_jobs
             SET claimed_by = $1,
                 claimed_at = NOW(),
                 updated_at = NOW()
             WHERE id = (
                 SELECT id FROM clipping_jobs
                 WHERE status = 'pending' AND claimed_by IS NULL
                 ORDER BY created_at ASC
                 LIMIT 1
                 FOR UPDATE SKIP LOCKED
             )
             RETURNING id"
        )
        .bind(worker_id.clone())
        .fetch_optional(&self.db_pool)
        .await
        .map_err(|e| format!("Failed to claim job: {}", e))?;

        if let Some(job_id) = claimed_job_id {
            tracing::info!("✅ Worker {} claimed job {}", worker_id, job_id);
            Ok(Some(job_id))
        } else {
            // No jobs available
            Ok(None)
        }
    }

    /// Release a claim on a job (used for error handling)
    async fn release_claim(&self, job_id: i32) -> Result<(), String> {
        let worker_id = self.worker_id.clone();

        sqlx::query(
            "UPDATE clipping_jobs
             SET claimed_by = NULL,
                 claimed_at = NULL,
                 updated_at = NOW()
             WHERE id = $1 AND claimed_by = $2 AND status = 'pending'"
        )
        .bind(job_id)
        .bind(worker_id.clone())
        .execute(&self.db_pool)
        .await
        .map_err(|e| format!("Failed to release claim: {}", e))?;

        tracing::info!("Released claim on job {} by {}", job_id, worker_id);
        Ok(())
    }

    /// Check if a specific job is claimed by this worker
    pub async fn is_claimed_by_me(&self, job_id: i32) -> Result<bool, String> {
        let worker_id = self.worker_id.clone();

        let result: Option<String> = sqlx::query_scalar(
            "SELECT claimed_by FROM clipping_jobs WHERE id = $1"
        )
        .bind(job_id)
        .fetch_optional(&self.db_pool)
        .await
        .map_err(|e| format!("Failed to check claim: {}", e))?;

        Ok(result.as_ref().map_or(false, |id| id == &worker_id))
    }

    /// Release all claims by this worker (cleanup on shutdown)
    pub async fn release_all_claims(&self) -> Result<usize, String> {
        let worker_id = self.worker_id.clone();

        let result = sqlx::query(
            "UPDATE clipping_jobs
             SET claimed_by = NULL,
                 claimed_at = NULL,
                 updated_at = NOW()
             WHERE claimed_by = $1 AND status = 'pending'"
        )
        .bind(worker_id.clone())
        .execute(&self.db_pool)
        .await
        .map_err(|e| format!("Failed to release all claims: {}", e))?;

        let released_count = result.rows_affected() as usize;
        if released_count > 0 {
            tracing::info!(
                "Released {} claims by worker {} during shutdown",
                released_count,
                worker_id
            );
        }

        Ok(released_count)
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    // Note: These tests require a test database setup
    // They are integration tests and should be run with --ignored flag

    #[tokio::test]
    #[ignore]
    async fn test_claim_job() {
        // This would require test database setup
        // See integration tests for full testing
    }

    #[test]
    fn test_worker_id_format() {
        let worker_id = format!(
            "{}-{}-{}",
            "test-host",
            std::process::id(),
            chrono::Utc::now().timestamp()
        );

        assert!(worker_id.contains("test-host"));
        assert!(worker_id.contains(&std::process::id().to_string()));
    }
}
