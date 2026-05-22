// Custom assertions for integration tests

use sqlx::{PgPool, Row};

/// Assert job is in the expected final status
pub async fn assert_job_status_sequence(
    pool: &PgPool,
    job_id: i32,
    expected_statuses: &[&str],
) -> Result<(), Box<dyn std::error::Error>> {
    let result = sqlx::query("SELECT status FROM clipping_jobs WHERE id = $1")
        .bind(job_id)
        .fetch_one(pool)
        .await?;

    let status: String = result.get("status");
    let last_expected = expected_statuses.last().unwrap();

    assert_eq!(
        &status, last_expected,
        "Job should be in '{}' status, got '{}'",
        last_expected, status
    );

    Ok(())
}

/// Assert job has error details when failed
pub async fn assert_job_has_error(
    pool: &PgPool,
    job_id: i32,
) -> Result<String, Box<dyn std::error::Error>> {
    let result = sqlx::query("SELECT status, error_message FROM clipping_jobs WHERE id = $1")
        .bind(job_id)
        .fetch_one(pool)
        .await?;

    let status: String = result.get("status");
    let error: Option<String> = result.try_get("error_message").ok().flatten();

    assert_eq!(status, "failed", "Job should be in failed status");
    assert!(error.is_some(), "Job should have error_message details");

    Ok(error.unwrap())
}

/// Assert job is claimed by a specific worker
pub async fn assert_job_claimed(
    pool: &PgPool,
    job_id: i32,
    expected_worker_id: &str,
) -> Result<(), Box<dyn std::error::Error>> {
    let result = sqlx::query("SELECT claimed_by, claimed_at FROM clipping_jobs WHERE id = $1")
        .bind(job_id)
        .fetch_one(pool)
        .await?;

    let claimed_by: Option<String> = result.try_get("claimed_by").ok().flatten();
    let claimed_at: Option<chrono::DateTime<chrono::Utc>> =
        result.try_get("claimed_at").ok().flatten();

    assert!(claimed_by.is_some(), "Job should be claimed");
    assert_eq!(
        claimed_by.unwrap(),
        expected_worker_id,
        "Job should be claimed by expected worker"
    );
    assert!(claimed_at.is_some(), "Job should have claimed_at timestamp");

    Ok(())
}

/// Assert job is NOT claimed
pub async fn assert_job_not_claimed(
    pool: &PgPool,
    job_id: i32,
) -> Result<(), Box<dyn std::error::Error>> {
    let result = sqlx::query("SELECT claimed_by FROM clipping_jobs WHERE id = $1")
        .bind(job_id)
        .fetch_one(pool)
        .await?;

    let claimed_by: Option<String> = result.try_get("claimed_by").ok().flatten();

    assert!(claimed_by.is_none(), "Job should NOT be claimed");

    Ok(())
}

/// Assert extracted clips exist in DB (column is clipping_job_id, not job_id)
pub async fn assert_clips_extracted(
    pool: &PgPool,
    job_id: i32,
    min_clips: usize,
) -> Result<usize, Box<dyn std::error::Error>> {
    let result =
        sqlx::query("SELECT COUNT(*) as count FROM extracted_clips WHERE clipping_job_id = $1")
            .bind(job_id)
            .fetch_one(pool)
            .await?;

    let count: i64 = result.get("count");

    assert!(
        count >= min_clips as i64,
        "Expected at least {} clips, got {}",
        min_clips,
        count
    );

    Ok(count as usize)
}

/// Assert retry count incremented
pub async fn assert_retry_incremented(
    pool: &PgPool,
    job_id: i32,
) -> Result<i32, Box<dyn std::error::Error>> {
    let result = sqlx::query("SELECT retry_count FROM clipping_jobs WHERE id = $1")
        .bind(job_id)
        .fetch_one(pool)
        .await?;

    let retry_count: i32 = result.get("retry_count");

    assert!(retry_count > 0, "Retry count should be incremented");

    Ok(retry_count)
}

/// Assert job completed within time limit (max_duration_secs from created_at to completed_at)
pub async fn assert_completed_within(
    pool: &PgPool,
    job_id: i32,
    max_duration_secs: i64,
) -> Result<(), Box<dyn std::error::Error>> {
    let result = sqlx::query("SELECT created_at, completed_at FROM clipping_jobs WHERE id = $1")
        .bind(job_id)
        .fetch_one(pool)
        .await?;

    let created_at: chrono::DateTime<chrono::Utc> = result.get("created_at");
    let completed_at: Option<chrono::DateTime<chrono::Utc>> =
        result.try_get("completed_at").ok().flatten();

    assert!(
        completed_at.is_some(),
        "Job should have a completed_at timestamp"
    );

    let duration = (completed_at.unwrap() - created_at).num_seconds();

    assert!(
        duration <= max_duration_secs,
        "Job took {} seconds, expected <= {}",
        duration,
        max_duration_secs
    );

    Ok(())
}

/// Assert the job reached the Phase A (Gemini analysis) stage.
/// Verifies progress_percent > 0 and status is not still 'pending'.
pub async fn assert_analysis_phase_reached(
    pool: &PgPool,
    job_id: i32,
) -> Result<(), Box<dyn std::error::Error>> {
    let result = sqlx::query("SELECT status, progress_percent FROM clipping_jobs WHERE id = $1")
        .bind(job_id)
        .fetch_one(pool)
        .await?;

    let status: String = result.get("status");
    let progress: i32 = result.try_get("progress_percent").unwrap_or(0);

    assert_ne!(status, "pending", "Job should have moved past pending");
    assert!(
        progress > 0,
        "Job should have nonzero progress (status: {})",
        status
    );

    Ok(())
}

/// Assert the YTDLP microservice URL is configured
pub fn assert_ytdlp_api_configured() {
    let url = std::env::var("YTDLP_API_URL").unwrap_or_default();
    assert!(
        !url.is_empty(),
        "YTDLP_API_URL must be set for download strategy tests"
    );
}
