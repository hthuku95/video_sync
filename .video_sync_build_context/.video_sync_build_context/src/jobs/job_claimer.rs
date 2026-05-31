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

        // Atomically claim job using claimed_by column (status stays 'pending').
        // The ordering keeps test jobs fast, then favors users with fewer completed
        // jobs so new monetization accounts are not starved behind old queues.
        let claimed_job_id: Option<i32> = sqlx::query_scalar(
            "UPDATE clipping_jobs
             SET claimed_by = $1,
                 claimed_at = NOW(),
                 updated_at = NOW()
             WHERE id = (
                 SELECT cj.id FROM clipping_jobs cj
                 JOIN youtube_channel_linkages l ON l.id = cj.linkage_id
                 LEFT JOIN source_channel_health sch ON sch.source_channel_id = l.source_channel_id
                 LEFT JOIN LATERAL (
                     SELECT COUNT(*) AS completed_jobs
                     FROM clipping_jobs completed
                     JOIN youtube_channel_linkages completed_l
                       ON completed_l.id = completed.linkage_id
                     WHERE completed_l.user_id = l.user_id
                       AND completed.status = 'completed'
                 ) user_stats ON true
                 WHERE cj.status = 'pending' AND cj.claimed_by IS NULL
                   AND NOT EXISTS (
                       SELECT 1
                       FROM clipping_jobs active
                       WHERE active.source_video_id = cj.source_video_id
                         AND active.id <> cj.id
                         AND (
                             active.claimed_by IS NOT NULL
                             OR active.status IN ('downloading', 'analyzing', 'extracting_clips', 'posting')
                         )
                   )
                 ORDER BY
                     CASE WHEN l.user_id = -1 THEN 0 ELSE 1 END,
                     COALESCE(user_stats.completed_jobs, 0) ASC,
                     COALESCE(sch.health_score, 1.0) DESC,
                     cj.created_at ASC
                 LIMIT 1
                 FOR UPDATE OF cj SKIP LOCKED
             )
             RETURNING id",
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
