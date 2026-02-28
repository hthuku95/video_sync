// Test database helper for integration tests
//
// Design: Uses REAL production channel linkages rather than fake TEST_SRC_*/TEST_DEST_*
// channels. This means:
//  - Tests operate on actual connected channels (the ones real users set up)
//  - No orphan TEST_SRC_*/TEST_DEST_* rows accumulate in the DB
//  - Cleanup only cancels test-created jobs — never deletes channels or linkages
//
// Prerequisite: At least one active youtube_channel_linkages row must exist with:
//  - is_active = true
//  - destination channel: is_active = true, requires_reauth = false, token_expiry > NOW() + 5 min
//
// If no such linkage exists, TestContext::new() panics with a helpful message.

use sqlx::{PgPool, Row, postgres::PgPoolOptions};
use std::env;

/// Test context that wraps a real production channel linkage.
/// Tests create clipping_jobs tied to this linkage and cancel them on cleanup.
/// Production channels and linkages are never modified or deleted by tests.
pub struct TestContext {
    pub pool: PgPool,
    /// ID of the youtube_channel_linkages row used for this test session.
    pub linkage_id: i32,
    /// ID of the youtube_source_channels row for the source channel.
    pub source_channel_id: i32,
    /// ID of the connected_youtube_channels row for the destination channel.
    pub destination_channel_id: i32,
}

impl TestContext {
    /// Create a new test context backed by a real active channel linkage.
    ///
    /// Panics with a clear message if no suitable linkage exists — the user
    /// must create one via the app UI before running integration tests.
    pub async fn new() -> Result<Self, Box<dyn std::error::Error>> {
        let database_url = env::var("DATABASE_URL")
            .expect("DATABASE_URL must be set for integration tests");

        let pool = PgPoolOptions::new()
            .max_connections(5)
            .acquire_timeout(std::time::Duration::from_secs(60))
            .connect(&database_url)
            .await?;

        // Find any active linkage whose destination channel has a non-expired OAuth token.
        // This is the same channel that production jobs will upload to.
        let row = sqlx::query(
            "SELECT l.id, l.source_channel_id, l.destination_channel_id
             FROM youtube_channel_linkages l
             JOIN connected_youtube_channels d ON d.id = l.destination_channel_id
             WHERE l.is_active = true
               AND d.is_active = true
               AND d.requires_reauth = false
               AND d.token_expiry > NOW() + INTERVAL '5 minutes'
             ORDER BY l.id
             LIMIT 1"
        )
        .fetch_one(&pool)
        .await
        .map_err(|_| {
            "No active channel linkage with valid OAuth found. \
             Connect a YouTube channel via the app UI and create a linkage, then re-run tests."
        })?;

        Ok(Self {
            pool,
            linkage_id: row.get("id"),
            source_channel_id: row.get("source_channel_id"),
            destination_channel_id: row.get("destination_channel_id"),
        })
    }

    /// Cancel all non-terminal jobs for this linkage that were created during the current
    /// test session. Called before creating a new test job to prevent queue buildup.
    ///
    /// Does NOT delete channels or linkages — those are production data.
    pub async fn cancel_stale_pending_test_jobs(&self) -> Result<u64, Box<dyn std::error::Error>> {
        let result = sqlx::query(
            "UPDATE clipping_jobs
             SET status = 'cancelled',
                 error_message = 'Cancelled by test harness (stale from previous run)',
                 claimed_by = NULL,
                 updated_at = NOW()
             WHERE linkage_id = $1
               AND status NOT IN ('completed', 'cancelled')"
        )
        .bind(self.linkage_id)
        .execute(&self.pool)
        .await?;

        let cancelled = result.rows_affected();
        if cancelled > 0 {
            eprintln!(
                "🧹 Cancelled {} stale test jobs for linkage {} before starting new test",
                cancelled, self.linkage_id
            );
        }
        Ok(cancelled)
    }

    /// Insert a job directly without cancelling stale jobs first.
    /// Use this when multiple jobs need to coexist (e.g., atomicity tests).
    pub async fn insert_test_job_raw(
        &self,
        video_id: &str,
    ) -> Result<i32, Box<dyn std::error::Error>> {
        let youtube_video_id = extract_video_id(video_id);

        let result = sqlx::query(
            "INSERT INTO clipping_jobs
             (linkage_id, source_video_id, status, created_at, updated_at)
             VALUES ($1, $2, 'pending', NOW(), NOW())
             RETURNING id"
        )
        .bind(self.linkage_id)
        .bind(youtube_video_id)
        .fetch_one(&self.pool)
        .await?;

        Ok(result.get("id"))
    }

    /// Create a clipping job for the test linkage, cancelling any stale jobs first.
    pub async fn create_test_job(
        &self,
        video_id: &str,
    ) -> Result<i32, Box<dyn std::error::Error>> {
        self.cancel_stale_pending_test_jobs().await?;

        let youtube_video_id = extract_video_id(video_id);

        let result = sqlx::query(
            "INSERT INTO clipping_jobs
             (linkage_id, source_video_id, status, created_at, updated_at)
             VALUES ($1, $2, 'pending', NOW(), NOW())
             RETURNING id"
        )
        .bind(self.linkage_id)
        .bind(youtube_video_id)
        .fetch_one(&self.pool)
        .await?;

        Ok(result.get("id"))
    }

    /// Get the current status of a clipping job.
    pub async fn get_job_status(&self, job_id: i32) -> Result<String, Box<dyn std::error::Error>> {
        let result = sqlx::query("SELECT status FROM clipping_jobs WHERE id = $1")
            .bind(job_id)
            .fetch_one(&self.pool)
            .await?;

        Ok(result.get("status"))
    }

    /// Wait for a job to reach a specific status (with timeout).
    /// Handles transient DB errors by retrying up to 10 consecutive times.
    pub async fn wait_for_status(
        &self,
        job_id: i32,
        target_status: &str,
        timeout_secs: u64,
    ) -> Result<bool, Box<dyn std::error::Error>> {
        let start = std::time::Instant::now();
        let mut consecutive_errors: u32 = 0;

        loop {
            match self.get_job_status(job_id).await {
                Ok(status) => {
                    consecutive_errors = 0;
                    if status == target_status {
                        return Ok(true);
                    }
                    // Early exit on all terminal states
                    let terminal_states = ["failed", "cancelled", "no_clips_found"];
                    if terminal_states.contains(&status.as_str()) {
                        eprintln!(
                            "Job {} reached terminal state '{}', expected '{}'",
                            job_id, status, target_status
                        );
                        return Ok(false);
                    }
                    eprintln!("Job {} status: {} (waiting for {})", job_id, status, target_status);
                }
                Err(e) => {
                    consecutive_errors += 1;
                    eprintln!(
                        "Warning: get_job_status transient error #{} (retrying): {}",
                        consecutive_errors, e
                    );
                    if consecutive_errors >= 10 {
                        return Err(e);
                    }
                    tokio::time::sleep(tokio::time::Duration::from_secs(10)).await;
                    continue;
                }
            }

            if start.elapsed().as_secs() > timeout_secs {
                return Ok(false);
            }

            tokio::time::sleep(tokio::time::Duration::from_secs(5)).await;
        }
    }

    /// Cancel any non-terminal jobs created for this linkage by tests.
    /// Does NOT delete channels, linkages, or completed jobs — those are production data.
    pub async fn cleanup(&self) -> Result<(), Box<dyn std::error::Error>> {
        sqlx::query(
            "UPDATE clipping_jobs
             SET status = 'cancelled',
                 error_message = 'Cancelled by test harness cleanup',
                 claimed_by = NULL,
                 updated_at = NOW()
             WHERE linkage_id = $1
               AND status NOT IN ('completed', 'cancelled')"
        )
        .bind(self.linkage_id)
        .execute(&self.pool)
        .await?;

        Ok(())
    }

    /// Cancel specific jobs by ID (for tests that track which jobs they created).
    pub async fn cleanup_test_jobs(&self, job_ids: &[i32]) -> Result<(), Box<dyn std::error::Error>> {
        if job_ids.is_empty() {
            return Ok(());
        }
        sqlx::query(
            "UPDATE clipping_jobs
             SET status = 'cancelled', updated_at = NOW()
             WHERE id = ANY($1) AND status NOT IN ('completed', 'cancelled')"
        )
        .bind(job_ids)
        .execute(&self.pool)
        .await?;

        Ok(())
    }
}

impl Drop for TestContext {
    fn drop(&mut self) {
        // Best-effort async cleanup: cancel any non-terminal test jobs.
        // Does NOT delete channels or linkages (production data).
        let pool = self.pool.clone();
        let linkage_id = self.linkage_id;

        tokio::spawn(async move {
            let _ = sqlx::query(
                "UPDATE clipping_jobs
                 SET status = 'cancelled',
                     error_message = 'Cancelled by test harness (Drop)',
                     claimed_by = NULL,
                     updated_at = NOW()
                 WHERE linkage_id = $1
                   AND status NOT IN ('completed', 'cancelled')"
            )
            .bind(linkage_id)
            .execute(&pool)
            .await;
        });
    }
}

/// Extract YouTube video ID from a URL or pass through if already an ID.
fn extract_video_id(video_id: &str) -> &str {
    if video_id.contains("youtube.com") || video_id.contains("youtu.be") {
        video_id
            .split("v=")
            .nth(1)
            .and_then(|s| s.split('&').next())
            .or_else(|| video_id.split('/').last())
            .unwrap_or(video_id)
    } else {
        video_id
    }
}

/// Helper to run migrations before tests (creates its own short-lived pool).
pub async fn ensure_migrations() -> Result<(), Box<dyn std::error::Error>> {
    let database_url = env::var("DATABASE_URL")?;
    let pool = PgPoolOptions::new()
        .max_connections(2)
        .connect(&database_url)
        .await?;

    sqlx::migrate!("./migrations")
        .run(&pool)
        .await?;

    Ok(())
}
