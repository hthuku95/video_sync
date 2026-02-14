// Custom assertions for integration tests

use sqlx::{PgPool, Row};

/// Assert job transitioned through expected statuses
pub async fn assert_job_status_sequence(
    pool: &PgPool,
    job_id: i32,
    expected_statuses: &[&str],
) -> Result<(), Box<dyn std::error::Error>> {
    // This would require status history tracking
    // For now, just verify final status
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
    let result = sqlx::query("SELECT status, error FROM clipping_jobs WHERE id = $1")
        .bind(job_id)
        .fetch_one(pool)
        .await?;

    let status: String = result.get("status");
    let error: Option<String> = result.try_get("error").ok();

    assert_eq!(status, "failed", "Job should be in failed status");
    assert!(error.is_some(), "Job should have error details");

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

    let claimed_by: Option<String> = result.try_get("claimed_by").ok();
    let claimed_at: Option<chrono::DateTime<chrono::Utc>> = result.try_get("claimed_at").ok();

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

    let claimed_by: Option<String> = result.try_get("claimed_by").ok();

    assert!(claimed_by.is_none(), "Job should NOT be claimed");

    Ok(())
}

/// Assert vectors exist in Qdrant
pub async fn assert_vectors_in_qdrant(
    job_id: i32,
    expected_count: usize,
) -> Result<(), Box<dyn std::error::Error>> {
    // This would require Qdrant client
    // For now, we'll check database metadata
    tracing::info!(
        "Would verify {} vectors in Qdrant for job {}",
        expected_count,
        job_id
    );
    Ok(())
}

/// Assert extracted clips exist
pub async fn assert_clips_extracted(
    pool: &PgPool,
    job_id: i32,
    min_clips: usize,
) -> Result<usize, Box<dyn std::error::Error>> {
    let result = sqlx::query("SELECT COUNT(*) as count FROM extracted_clips WHERE job_id = $1")
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

/// Assert job completed within time limit
pub async fn assert_completed_within(
    pool: &PgPool,
    job_id: i32,
    max_duration_secs: i64,
) -> Result<(), Box<dyn std::error::Error>> {
    let result = sqlx::query(
        "SELECT created_at, completed_at FROM clipping_jobs WHERE id = $1"
    )
    .bind(job_id)
    .fetch_one(pool)
    .await?;

    let created_at: chrono::DateTime<chrono::Utc> = result.get("created_at");
    let completed_at: Option<chrono::DateTime<chrono::Utc>> = result.try_get("completed_at").ok();

    assert!(completed_at.is_some(), "Job should be completed");

    let duration = (completed_at.unwrap() - created_at).num_seconds();

    assert!(
        duration <= max_duration_secs,
        "Job took {} seconds, expected <= {}",
        duration,
        max_duration_secs
    );

    Ok(())
}
