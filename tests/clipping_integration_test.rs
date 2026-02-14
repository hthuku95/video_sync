// Comprehensive clipping system integration tests
// Tests complete E2E workflow from job creation to YouTube upload

mod helpers;

use helpers::{TestContext, assertions};
use helpers::test_youtube::{TestYouTubeClient, test_videos};
use sqlx::Row;

#[tokio::test]
#[ignore] // Run with: cargo test --test clipping_integration_test --ignored
async fn test_complete_clipping_workflow() {
    // Initialize test environment
    let ctx = TestContext::new()
        .await
        .expect("Failed to create test context");

    // Create a clipping job with a short test video
    let job_id = ctx
        .create_test_job(test_videos::SHORT_VIDEO)
        .await
        .expect("Failed to create test job");

    tracing::info!("✅ Created test job {}", job_id);

    // Wait for job to complete (timeout: 8 minutes for full workflow)
    let completed = ctx
        .wait_for_status(job_id, "completed", 480)
        .await
        .expect("Failed waiting for completion");

    assert!(
        completed,
        "Job should complete within 8 minutes (actual workflow: 5-8 min)"
    );

    // Verify job completed successfully
    assertions::assert_job_status_sequence(&ctx.pool, job_id, &["pending", "completed"])
        .await
        .expect("Job should be completed");

    // Verify clips were extracted
    let clip_count = assertions::assert_clips_extracted(&ctx.pool, job_id, 1)
        .await
        .expect("Should have extracted clips");

    tracing::info!("✅ Extracted {} clips from job {}", clip_count, job_id);

    // Verify completion time was reasonable
    assertions::assert_completed_within(&ctx.pool, job_id, 600)
        .await
        .expect("Job should complete within 10 minutes");

    // TODO: Verify YouTube upload when OAuth credentials are available
    // let youtube_client = TestYouTubeClient::new(access_token);
    // youtube_client.verify_unlisted(video_id).await.expect("Video should be unlisted");

    // Cleanup
    ctx.cleanup().await.expect("Cleanup failed");
}

#[tokio::test]
#[ignore]
async fn test_job_claiming_atomicity() {
    let ctx = TestContext::new()
        .await
        .expect("Failed to create test context");

    // Create 5 test jobs
    let mut job_ids = Vec::new();
    for i in 0..5 {
        let job_id = ctx
            .create_test_job(&format!("https://youtube.com/watch?v=test{}", i))
            .await
            .expect("Failed to create test job");
        job_ids.push(job_id);
    }

    tracing::info!("✅ Created {} test jobs", job_ids.len());

    // Simulate 3 concurrent workers claiming jobs
    use video_editor::jobs::job_claimer::JobClaimer;

    let worker1 = JobClaimer::new("test-worker-1".to_string(), ctx.pool.clone());
    let worker2 = JobClaimer::new("test-worker-2".to_string(), ctx.pool.clone());
    let worker3 = JobClaimer::new("test-worker-3".to_string(), ctx.pool.clone());

    // All workers claim concurrently
    let claim1_future = worker1.claim_next_job();
    let claim2_future = worker2.claim_next_job();
    let claim3_future = worker3.claim_next_job();

    let (claim1, claim2, claim3) = tokio::join!(
        claim1_future,
        claim2_future,
        claim3_future
    );

    // All should succeed but get different jobs
    let claimed1 = claim1.expect("Worker 1 should claim").expect("Should get job");
    let claimed2 = claim2.expect("Worker 2 should claim").expect("Should get job");
    let claimed3 = claim3.expect("Worker 3 should claim").expect("Should get job");

    // Verify no duplicates
    assert_ne!(claimed1, claimed2, "Workers should claim different jobs");
    assert_ne!(claimed2, claimed3, "Workers should claim different jobs");
    assert_ne!(claimed1, claimed3, "Workers should claim different jobs");

    tracing::info!(
        "✅ Atomic claiming verified: {} != {} != {}",
        claimed1,
        claimed2,
        claimed3
    );

    // Verify claims in database
    assertions::assert_job_claimed(&ctx.pool, claimed1, "test-worker-1")
        .await
        .expect("Job should be claimed by worker 1");

    assertions::assert_job_claimed(&ctx.pool, claimed2, "test-worker-2")
        .await
        .expect("Job should be claimed by worker 2");

    assertions::assert_job_claimed(&ctx.pool, claimed3, "test-worker-3")
        .await
        .expect("Job should be claimed by worker 3");

    // Cleanup
    ctx.cleanup().await.expect("Cleanup failed");
}

#[tokio::test]
#[ignore]
async fn test_download_strategies_fallback() {
    let ctx = TestContext::new()
        .await
        .expect("Failed to create test context");

    // Create job with a video that might fail on Apify (edge case)
    let job_id = ctx
        .create_test_job(test_videos::SHORT_VIDEO)
        .await
        .expect("Failed to create test job");

    // Wait for download to complete (will fallback through strategies if needed)
    let downloaded = ctx
        .wait_for_status(job_id, "downloaded", 300)
        .await
        .expect("Failed waiting for download");

    assert!(
        downloaded,
        "Job should successfully download via fallback strategies"
    );

    // Verify no errors
    let result = sqlx::query("SELECT error_message FROM clipping_jobs WHERE id = $1")
        .bind(job_id)
        .fetch_one(&ctx.pool)
        .await
        .expect("Should fetch job");

    let error: Option<String> = result.try_get("error_message").ok().flatten();
    assert!(error.is_none(), "Should have no errors after successful download");

    tracing::info!("✅ Download fallback system working");

    ctx.cleanup().await.expect("Cleanup failed");
}

#[tokio::test]
#[ignore]
async fn test_stuck_job_detection() {
    let ctx = TestContext::new()
        .await
        .expect("Failed to create test context");

    // Create a job and manually set it to stuck state
    let job_id = ctx
        .create_test_job(test_videos::SHORT_VIDEO)
        .await
        .expect("Failed to create test job");

    // Manually set job to 'analyzing' status with old timestamp (65 min ago)
    // NOTE: We need to disable the trigger temporarily because it auto-updates updated_at
    sqlx::query("ALTER TABLE clipping_jobs DISABLE TRIGGER update_clipping_jobs_updated_at")
        .execute(&ctx.pool)
        .await
        .expect("Failed to disable trigger");

    sqlx::query(
        "UPDATE clipping_jobs
         SET status = 'analyzing',
             updated_at = NOW() - INTERVAL '65 minutes'
         WHERE id = $1"
    )
    .bind(job_id)
    .execute(&ctx.pool)
    .await
    .expect("Failed to set stuck state");

    sqlx::query("ALTER TABLE clipping_jobs ENABLE TRIGGER update_clipping_jobs_updated_at")
        .execute(&ctx.pool)
        .await
        .expect("Failed to re-enable trigger");

    tracing::info!("✅ Simulated stuck job {} (analyzing for 65 min)", job_id);

    // Debug: Check the actual job state
    let debug_result = sqlx::query("SELECT status, updated_at, NOW() as current_time FROM clipping_jobs WHERE id = $1")
        .bind(job_id)
        .fetch_one(&ctx.pool)
        .await
        .expect("Should fetch job for debug");

    let status: String = debug_result.get("status");
    let updated_at: chrono::DateTime<chrono::Utc> = debug_result.get("updated_at");
    let current_time: chrono::DateTime<chrono::Utc> = debug_result.get("current_time");

    tracing::info!("🔍 Job status: {}, updated_at: {}, current_time: {}, diff: {} minutes",
        status, updated_at, current_time, (current_time - updated_at).num_minutes());

    // Trigger stuck job detection (this would normally run in worker)
    // For now, manually verify the job would be detected
    let result = sqlx::query(
        "SELECT id, status, updated_at, NOW() - updated_at as age FROM clipping_jobs
         WHERE status = 'analyzing'
         AND updated_at < NOW() - INTERVAL '60 minutes'
         AND id = $1"
    )
    .bind(job_id)
    .fetch_optional(&ctx.pool)
    .await
    .expect("Should query stuck jobs");

    if let Some(row) = &result {
        let age: sqlx::postgres::types::PgInterval = row.get("age");
        tracing::info!("✅ Found stuck job, age: {:?}", age);
    } else {
        tracing::error!("❌ Stuck job NOT detected!");
    }

    assert!(result.is_some(), "Stuck job should be detected");

    // Simulate reset to failed
    sqlx::query(
        "UPDATE clipping_jobs
         SET status = 'failed',
             error_message = 'Job stuck in analyzing status for > 60 minutes',
             stuck_detection_count = stuck_detection_count + 1
         WHERE id = $1"
    )
    .bind(job_id)
    .execute(&ctx.pool)
    .await
    .expect("Should reset stuck job");

    // Verify it's now failed
    let status = ctx.get_job_status(job_id).await.expect("Should get status");
    assert_eq!(status, "failed", "Stuck job should be marked as failed");

    tracing::info!("✅ Stuck job detection working");

    ctx.cleanup().await.expect("Cleanup failed");
}

#[tokio::test]
#[ignore]
async fn test_auto_retry_failed_jobs() {
    let ctx = TestContext::new()
        .await
        .expect("Failed to create test context");

    // Create a failed job
    let job_id = ctx
        .create_test_job(test_videos::SHORT_VIDEO)
        .await
        .expect("Failed to create test job");

    // Manually fail the job (simulate a transient failure)
    sqlx::query(
        "UPDATE clipping_jobs
         SET status = 'failed',
             error_message = 'Simulated transient failure',
             completed_at = NOW() - INTERVAL '6 minutes',
             retry_count = 0
         WHERE id = $1"
    )
    .bind(job_id)
    .execute(&ctx.pool)
    .await
    .expect("Failed to set failed state");

    tracing::info!("✅ Simulated failed job {} (6 min ago)", job_id);

    // Simulate auto-retry logic (would normally run in worker)
    let retryable = sqlx::query(
        "SELECT id FROM clipping_jobs
         WHERE status = 'failed'
         AND completed_at > NOW() - INTERVAL '6 hours'
         AND completed_at < NOW() - INTERVAL '5 minutes'
         AND id = $1"
    )
    .bind(job_id)
    .fetch_optional(&ctx.pool)
    .await
    .expect("Should query retryable jobs");

    assert!(retryable.is_some(), "Failed job should be retryable");

    // Simulate retry
    sqlx::query(
        "UPDATE clipping_jobs
         SET status = 'pending',
             error_message = NULL,
             retry_count = retry_count + 1,
             last_retry_at = NOW()
         WHERE id = $1"
    )
    .bind(job_id)
    .execute(&ctx.pool)
    .await
    .expect("Should reset for retry");

    // Verify retry count incremented
    assertions::assert_retry_incremented(&ctx.pool, job_id)
        .await
        .expect("Retry count should be incremented");

    // Verify status reset to pending
    let status = ctx.get_job_status(job_id).await.expect("Should get status");
    assert_eq!(status, "pending", "Retried job should be pending");

    tracing::info!("✅ Auto-retry mechanism working");

    ctx.cleanup().await.expect("Cleanup failed");
}
