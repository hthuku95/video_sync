// Test database helper for integration tests
// Provides utilities for setting up test data and cleanup

use sqlx::{PgPool, Row, postgres::PgPoolOptions};
use std::env;
use std::sync::atomic::{AtomicU64, Ordering};

// Monotonically increasing counter ensures each TestContext in a test run gets unique IDs
static TEST_CONTEXT_COUNTER: AtomicU64 = AtomicU64::new(0);

/// Test context that manages test user and cleanup
pub struct TestContext {
    pub pool: PgPool,
    pub test_user_id: i32,
    pub test_linkage_id: i32,
    pub test_source_channel_id: i32,
    pub test_destination_channel_id: i32,
}

impl TestContext {
    /// Create a new test context with isolated test data
    pub async fn new() -> Result<Self, Box<dyn std::error::Error>> {
        // Get database URL from environment
        let database_url = env::var("DATABASE_URL")
            .expect("DATABASE_URL must be set for integration tests");

        // Create connection pool
        let pool = PgPoolOptions::new()
            .max_connections(5)
            .acquire_timeout(std::time::Duration::from_secs(60))
            .connect(&database_url)
            .await?;

        // Create test user (negative ID to avoid conflicts)
        let test_user_id = -1;
        // Use a dummy bcrypt hash for test user (password: "test_password")
        let test_password_hash = "$2b$12$LQv3c1yqBWVHxkd0LHAkCOYz6TtxMQJqhN8/LewY5GyYKKxZKKxZK";

        // Make test user an admin to simulate production conditions
        // This ensures tests validate real-world scenarios (whitelisted/admin access)
        sqlx::query(
            "INSERT INTO users (id, email, username, password_hash, is_staff, is_superuser, created_at, updated_at)
             VALUES ($1, $2, $3, $4, $5, $6, NOW(), NOW())
             ON CONFLICT (id) DO NOTHING"
        )
        .bind(test_user_id)
        .bind("test@videosync.test")
        .bind("test_admin")
        .bind(test_password_hash)
        .bind(true)  // is_staff = true
        .bind(true)  // is_superuser = true
        .execute(&pool)
        .await?;

        // Create test source channel (YouTube channel to monitor)
        // Combine timestamp + monotonic counter + process ID for uniqueness across parallel tests
        let ctx_num = TEST_CONTEXT_COUNTER.fetch_add(1, Ordering::SeqCst);
        let test_source_channel_yt_id = format!(
            "TEST_SRC_{}_{}_{}", chrono::Utc::now().timestamp(), ctx_num, std::process::id()
        );
        let source_result = sqlx::query(
            "INSERT INTO youtube_source_channels
             (channel_id, channel_name, is_active, created_at, updated_at)
             VALUES ($1, $2, $3, NOW(), NOW())
             ON CONFLICT (channel_id) DO UPDATE SET updated_at = NOW()
             RETURNING id"
        )
        .bind(&test_source_channel_yt_id)
        .bind("Test Source Channel")
        .bind(true)
        .fetch_one(&pool)
        .await?;

        let test_source_channel_id: i32 = source_result.get("id");

        // Create test destination channel (connected YouTube channel for uploads)
        let test_dest_channel_yt_id = format!(
            "TEST_DEST_{}_{}_{}", chrono::Utc::now().timestamp(), ctx_num, std::process::id()
        );
        let dest_result = sqlx::query(
            "INSERT INTO connected_youtube_channels
             (user_id, channel_id, channel_name, access_token, refresh_token, token_expiry, granted_scopes, created_at, updated_at)
             VALUES ($1, $2, $3, $4, $5, NOW() + INTERVAL '1 hour', $6, NOW(), NOW())
             ON CONFLICT (user_id, channel_id) DO UPDATE SET updated_at = NOW()
             RETURNING id"
        )
        .bind(test_user_id)
        .bind(&test_dest_channel_yt_id)
        .bind("Test Destination Channel")
        .bind("test_access_token")
        .bind("test_refresh_token")
        .bind("https://www.googleapis.com/auth/youtube.upload")
        .fetch_one(&pool)
        .await?;

        let test_destination_channel_id: i32 = dest_result.get("id");

        // Create test channel linkage (source → destination mapping)
        let linkage_result = sqlx::query(
            "INSERT INTO youtube_channel_linkages
             (user_id, source_channel_id, destination_channel_id, is_active, clips_per_video, created_at, updated_at)
             VALUES ($1, $2, $3, $4, $5, NOW(), NOW())
             RETURNING id"
        )
        .bind(test_user_id)
        .bind(test_source_channel_id)
        .bind(test_destination_channel_id)
        .bind(true)
        .bind(2)
        .fetch_one(&pool)
        .await?;

        let test_linkage_id: i32 = linkage_result.get("id");

        Ok(Self {
            pool,
            test_user_id,
            test_linkage_id,
            test_source_channel_id,
            test_destination_channel_id,
        })
    }

    /// Cancel all non-terminal AND recently-failed jobs for THIS test's linkage only.
    ///
    /// Scoped to `self.test_linkage_id` (not all test-user jobs) so parallel test runs
    /// don't cancel each other's active jobs. Each TestContext gets a unique linkage_id
    /// via a monotonic counter, so cancelling by linkage_id is safe.
    ///
    /// This prevents two interference patterns:
    ///   1. Non-terminal jobs (pending/in-progress) from a previous run of THIS test
    ///      block the new job in the queue.
    ///   2. Failed jobs from previous runs of THIS test get picked up by the auto-retry
    ///      mechanism (which retries jobs failed within the last 6 hours), causing them
    ///      to jump the queue ahead of the freshly created job.
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
        .bind(self.test_linkage_id)
        .execute(&self.pool)
        .await?;

        let cancelled = result.rows_affected();
        if cancelled > 0 {
            eprintln!("🧹 Cancelled {} stale test jobs (non-terminal + failed) before starting new test", cancelled);
        }
        Ok(cancelled)
    }

    /// Insert a job directly without triggering stale-job cleanup.
    /// Use this when you need multiple jobs to coexist (e.g. atomicity tests).
    pub async fn insert_test_job_raw(
        &self,
        video_id: &str,
    ) -> Result<i32, Box<dyn std::error::Error>> {
        let youtube_video_id = if video_id.contains("youtube.com") || video_id.contains("youtu.be") {
            video_id
                .split("v=")
                .nth(1)
                .and_then(|s| s.split('&').next())
                .or_else(|| video_id.split('/').last())
                .unwrap_or(video_id)
        } else {
            video_id
        };

        let result = sqlx::query(
            "INSERT INTO clipping_jobs
             (linkage_id, source_video_id, status, created_at, updated_at)
             VALUES ($1, $2, 'pending', NOW(), NOW())
             RETURNING id"
        )
        .bind(self.test_linkage_id)
        .bind(youtube_video_id)
        .fetch_one(&self.pool)
        .await?;

        Ok(result.get("id"))
    }

    /// Create a test clipping job
    pub async fn create_test_job(
        &self,
        video_id: &str,
    ) -> Result<i32, Box<dyn std::error::Error>> {
        // Cancel any stale pending test jobs first so the new job isn't stuck in queue
        self.cancel_stale_pending_test_jobs().await?;

        // Extract video ID from URL if full URL is provided
        let youtube_video_id = if video_id.contains("youtube.com") || video_id.contains("youtu.be") {
            video_id
                .split("v=")
                .nth(1)
                .and_then(|s| s.split('&').next())
                .or_else(|| video_id.split('/').last())
                .unwrap_or(video_id)
        } else {
            video_id
        };

        let result = sqlx::query(
            "INSERT INTO clipping_jobs
             (linkage_id, source_video_id, status, created_at, updated_at)
             VALUES ($1, $2, 'pending', NOW(), NOW())
             RETURNING id"
        )
        .bind(self.test_linkage_id)
        .bind(youtube_video_id)
        .fetch_one(&self.pool)
        .await?;

        Ok(result.get("id"))
    }

    /// Get job status
    pub async fn get_job_status(&self, job_id: i32) -> Result<String, Box<dyn std::error::Error>> {
        let result = sqlx::query("SELECT status FROM clipping_jobs WHERE id = $1")
            .bind(job_id)
            .fetch_one(&self.pool)
            .await?;

        Ok(result.get("status"))
    }

    /// Wait for job to reach a specific status (with timeout)
    /// Handles transient DB errors (PoolTimedOut, connection reset) by retrying
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
                    // no_clips_found: Phase A fast-fail (Gemini found no quality viral moments)
                    let terminal_states = ["failed", "cancelled", "no_clips_found"];
                    if terminal_states.contains(&status.as_str()) {
                        eprintln!("Job {} reached terminal state '{}', expected '{}'", job_id, status, target_status);
                        return Ok(false);
                    }
                    eprintln!("Job {} status: {} (waiting for {})", job_id, status, target_status);
                }
                Err(e) => {
                    consecutive_errors += 1;
                    eprintln!("Warning: get_job_status transient error #{} (retrying): {}", consecutive_errors, e);
                    if consecutive_errors >= 10 {
                        return Err(e);
                    }
                    // Back off before retrying after an error
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

    /// Cleanup all test data
    pub async fn cleanup(&self) -> Result<(), Box<dyn std::error::Error>> {
        // Delete in reverse foreign key order
        sqlx::query("DELETE FROM extracted_clips WHERE clipping_job_id IN (SELECT id FROM clipping_jobs WHERE linkage_id = $1)")
            .bind(self.test_linkage_id)
            .execute(&self.pool)
            .await?;

        sqlx::query("DELETE FROM clipping_jobs WHERE linkage_id = $1")
            .bind(self.test_linkage_id)
            .execute(&self.pool)
            .await?;

        sqlx::query("DELETE FROM pending_unclipped_videos WHERE linkage_id = $1")
            .bind(self.test_linkage_id)
            .execute(&self.pool)
            .await?;

        sqlx::query("DELETE FROM youtube_channel_linkages WHERE id = $1")
            .bind(self.test_linkage_id)
            .execute(&self.pool)
            .await?;

        sqlx::query("DELETE FROM connected_youtube_channels WHERE id = $1")
            .bind(self.test_destination_channel_id)
            .execute(&self.pool)
            .await?;

        sqlx::query("DELETE FROM clipped_source_videos WHERE source_channel_id = $1")
            .bind(self.test_source_channel_id)
            .execute(&self.pool)
            .await?;

        sqlx::query("DELETE FROM youtube_source_channels WHERE id = $1")
            .bind(self.test_source_channel_id)
            .execute(&self.pool)
            .await?;

        // NOTE: We intentionally do NOT delete the test user (id = -1).
        // Deleting it cascades via ON DELETE CASCADE through connected_youtube_channels
        // → youtube_channel_linkages → clipping_jobs, wiping other parallel tests' data.
        // The user row is reused across test runs via ON CONFLICT DO NOTHING.

        Ok(())
    }
}

impl Drop for TestContext {
    fn drop(&mut self) {
        // Schedule cleanup (runs in background)
        let pool = self.pool.clone();
        let linkage_id = self.test_linkage_id;
        let source_channel_id = self.test_source_channel_id;
        let dest_channel_id = self.test_destination_channel_id;
        let user_id = self.test_user_id;

        tokio::spawn(async move {
            let _ = sqlx::query("DELETE FROM extracted_clips WHERE clipping_job_id IN (SELECT id FROM clipping_jobs WHERE linkage_id = $1)")
                .bind(linkage_id)
                .execute(&pool)
                .await;

            let _ = sqlx::query("DELETE FROM clipping_jobs WHERE linkage_id = $1")
                .bind(linkage_id)
                .execute(&pool)
                .await;

            let _ = sqlx::query("DELETE FROM pending_unclipped_videos WHERE linkage_id = $1")
                .bind(linkage_id)
                .execute(&pool)
                .await;

            let _ = sqlx::query("DELETE FROM youtube_channel_linkages WHERE id = $1")
                .bind(linkage_id)
                .execute(&pool)
                .await;

            let _ = sqlx::query("DELETE FROM connected_youtube_channels WHERE id = $1")
                .bind(dest_channel_id)
                .execute(&pool)
                .await;

            let _ = sqlx::query("DELETE FROM clipped_source_videos WHERE source_channel_id = $1")
                .bind(source_channel_id)
                .execute(&pool)
                .await;

            let _ = sqlx::query("DELETE FROM youtube_source_channels WHERE id = $1")
                .bind(source_channel_id)
                .execute(&pool)
                .await;

            // NOTE: Do NOT delete the test user — see cleanup() for explanation.
            let _ = user_id; // suppress unused variable warning
        });
    }
}

/// Helper to run migrations before tests
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
