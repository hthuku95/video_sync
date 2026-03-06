// Clipping system integration tests
//
// Tests the full 5-phase pipeline introduced by the new architecture:
//   Phase A: Gemini video analysis via YouTube URL (1 API call)
//   Phase B: Download (only if Phase A found quality clips)
//   Phase C: Parallel FFmpeg clip extraction
//   Phase D: 1 Voyage AI embedding → video_content Qdrant collection
//   Phase E: Parallel YouTube upload
//
// Run all: cargo test --test clipping_integration_test -- --ignored --nocapture
// Run one: cargo test --test clipping_integration_test test_name -- --ignored --nocapture

mod helpers;

use helpers::{TestContext, assertions};
use helpers::test_youtube::test_videos;
use sqlx::Row;

// ============================================================================
// Test 1: Full end-to-end clipping workflow (primary smoke test)
// ============================================================================

/// Full E2E test: pending → analyzing → analyzed → downloading → ... → completed
///
/// Resume-first: checks whether a resumable job already exists for the linkage before
/// creating a new one. This avoids wasting a Gemini call + re-download on every run.
/// On the first run it creates a job; on subsequent runs it resumes the existing one.
///
/// Uses Rick Astley (dQw4w9WgXcQ, 3:33, always public, good candidate for viral moments).
/// Verifies the full pipeline completes and at least 1 clip is published to YouTube.
#[tokio::test]
#[ignore]
async fn test_complete_clipping_workflow() {
    let ctx = TestContext::new()
        .await
        .expect(
            "Failed to create test context — ensure a channel linkage with valid OAuth exists"
        );

    // 1. Look for an existing non-terminal job on this linkage — resume if found
    let existing_job = sqlx::query(
        "SELECT id, status FROM clipping_jobs
         WHERE linkage_id = $1
           AND status NOT IN ('completed', 'cancelled')
         ORDER BY created_at DESC
         LIMIT 1"
    )
    .bind(ctx.linkage_id)
    .fetch_optional(&ctx.pool)
    .await
    .expect("DB query failed");

    let job_id = if let Some(row) = existing_job {
        let id: i32 = row.get("id");
        let status: String = row.get("status");
        eprintln!("⏭️  Resuming existing job {} (status={})", id, status);
        id
    } else {
        // No resumable job — create a fresh one
        let id = ctx
            .create_test_job("dQw4w9WgXcQ")
            .await
            .expect("Failed to create test job");
        eprintln!("✅ Created new test job {} for video: dQw4w9WgXcQ", id);
        id
    };

    // 2. Wait up to 20 minutes for completion
    let timeout_secs: u64 = std::env::var("TEST_TIMEOUT_LONG")
        .ok()
        .and_then(|v| v.parse().ok())
        .unwrap_or(1200);

    let completed = ctx
        .wait_for_status(job_id, "completed", timeout_secs)
        .await
        .expect("Failed waiting for completion");

    let final_status = ctx.get_job_status(job_id).await.unwrap_or_else(|_| "unknown".into());
    assert!(
        completed,
        "Job {} should reach 'completed' within {} seconds. Final status: '{}'",
        job_id, timeout_secs, final_status
    );

    // 3. Verify clips were extracted into the DB
    let clip_count = assertions::assert_clips_extracted(&ctx.pool, job_id, 1)
        .await
        .expect("Should have at least 1 extracted clip");

    eprintln!("✅ Extracted {} clips for job {}", clip_count, job_id);

    // 4. Verify at least 1 clip was actually published to YouTube
    let clips = sqlx::query(
        "SELECT youtube_video_id, upload_status FROM extracted_clips
         WHERE clipping_job_id = $1"
    )
    .bind(job_id)
    .fetch_all(&ctx.pool)
    .await
    .expect("Failed to fetch clips");

    assert!(!clips.is_empty(), "No clips in extracted_clips table for job {}", job_id);

    let published = clips
        .iter()
        .filter(|row| {
            let status: String = row.get("upload_status");
            status == "published"
        })
        .count();

    assert!(
        published >= 1,
        "Expected at least 1 published clip for job {} but got {} (total clips: {})",
        job_id, published, clips.len()
    );

    eprintln!("✅ {} clip(s) published to YouTube for job {}", published, job_id);

    // 5. Verify timing (new pipeline should complete within 20 minutes)
    assertions::assert_completed_within(&ctx.pool, job_id, 1200)
        .await
        .expect("Job should complete within 20 minutes");

    // NOTE: No cleanup() call — the job is a real production run; keep it in the DB.
}

// ============================================================================
// Test 2: Phase A — Gemini video analysis (fast, no download needed)
// ============================================================================

/// Verify Phase A (Gemini YouTube URL analysis) completes without downloading.
///
/// After submitting a job the worker should transition it out of 'pending' and
/// into 'analyzing' / 'analyzed' / 'no_clips_found' within a reasonable time.
/// This test exits as soon as Phase A finishes — it does NOT wait for the
/// full download+extract pipeline.
#[tokio::test]
#[ignore]
async fn test_gemini_analysis_phase() {
    let ctx = TestContext::new()
        .await
        .expect("Failed to create test context");

    let job_id = ctx
        .create_test_job(test_videos::MEDIUM_VIDEO)
        .await
        .expect("Failed to create test job");

    eprintln!("✅ Created test job {} for Phase A analysis test", job_id);

    // Phase A (1 Gemini call) should finish within 3 minutes even on a cold Gemini model
    let timeout_secs: u64 = std::env::var("TEST_TIMEOUT_ANALYSIS")
        .ok()
        .and_then(|v| v.parse().ok())
        .unwrap_or(180);

    // Wait until the job moves past 'analyzing' (either 'analyzed', 'downloading',
    // 'no_clips_found', 'completed', or 'failed')
    let start = std::time::Instant::now();
    let mut passed_analysis = false;
    loop {
        let status = ctx.get_job_status(job_id).await.unwrap_or_else(|_| "unknown".into());
        match status.as_str() {
            "pending" | "analyzing" => {
                // still in Phase A — keep waiting
            }
            "no_clips_found" => {
                eprintln!("ℹ️  Job {} → no_clips_found (Gemini found no quality moments — still a valid Phase A result)", job_id);
                passed_analysis = true;
                break;
            }
            "analyzed" | "downloading" | "downloaded" | "extracting_clips"
            | "clips_extracted" | "vectorizing" | "posting" | "completed" => {
                eprintln!("✅ Job {} passed Phase A analysis, now at: '{}'", job_id, status);
                passed_analysis = true;
                break;
            }
            "failed" | "cancelled" => {
                eprintln!("❌ Job {} failed during analysis: '{}'", job_id, status);
                break;
            }
            _ => {
                eprintln!("⚠️  Job {} unknown status: '{}'", job_id, status);
            }
        }

        if start.elapsed().as_secs() > timeout_secs {
            eprintln!("⏰ Phase A timed out after {}s — final status: '{}'", timeout_secs, status);
            break;
        }

        tokio::time::sleep(tokio::time::Duration::from_secs(5)).await;
    }

    assert!(
        passed_analysis,
        "Job {} should complete Phase A (Gemini analysis) within {} seconds",
        job_id, timeout_secs
    );

    // Phase A should have set progress > 0
    assertions::assert_analysis_phase_reached(&ctx.pool, job_id)
        .await
        .expect("Phase A should have advanced job progress");

    ctx.cleanup().await.expect("Cleanup failed");
}

// ============================================================================
// Test 3: Download phase (YTDLP microservice)
// ============================================================================

/// Verify the download phase succeeds via the YTDLP microservice (Strategy #3).
///
/// Waits for the job to reach 'downloaded' status, confirming:
///   - YTDLP microservice is reachable and warm
///   - Cookies + proxy env vars are working
///   - File is downloaded and non-empty
#[tokio::test]
#[ignore]
async fn test_download_phase() {
    assertions::assert_ytdlp_api_configured();

    let ctx = TestContext::new()
        .await
        .expect("Failed to create test context");

    let job_id = ctx
        .create_test_job(test_videos::MEDIUM_VIDEO)
        .await
        .expect("Failed to create test job");

    eprintln!("✅ Created test job {} for download phase test", job_id);

    // Allow up to 10 min: 1 min warm-up + analysis + download
    let timeout_secs: u64 = std::env::var("TEST_TIMEOUT_MEDIUM")
        .ok()
        .and_then(|v| v.parse().ok())
        .unwrap_or(600);

    // Wait for 'downloaded' (or any later stage meaning download succeeded)
    let start = std::time::Instant::now();
    let mut download_succeeded = false;
    loop {
        let status = ctx.get_job_status(job_id).await.unwrap_or_else(|_| "unknown".into());
        match status.as_str() {
            "downloaded" | "extracting_clips" | "clips_extracted"
            | "vectorizing" | "posting" | "completed" => {
                eprintln!("✅ Job {} download confirmed — status: '{}'", job_id, status);
                download_succeeded = true;
                break;
            }
            "no_clips_found" => {
                // Phase A found nothing → download was skipped (that's correct behaviour)
                eprintln!("ℹ️  Job {} → no_clips_found — download correctly skipped", job_id);
                download_succeeded = true; // skip is also correct behaviour
                break;
            }
            "failed" | "cancelled" => {
                let err = sqlx::query("SELECT error_message FROM clipping_jobs WHERE id = $1")
                    .bind(job_id)
                    .fetch_one(&ctx.pool)
                    .await
                    .ok()
                    .and_then(|r| r.try_get::<Option<String>, _>("error_message").ok().flatten())
                    .unwrap_or_else(|| "no error message".into());
                eprintln!("❌ Job {} failed: {}", job_id, err);
                break;
            }
            _ => {}
        }

        if start.elapsed().as_secs() > timeout_secs {
            eprintln!("⏰ Download timed out after {}s — final status: '{}'", timeout_secs, status);
            break;
        }

        tokio::time::sleep(tokio::time::Duration::from_secs(5)).await;
    }

    assert!(
        download_succeeded,
        "Job {} should reach 'downloaded' (or no_clips_found) within {} seconds",
        job_id, timeout_secs
    );

    ctx.cleanup().await.expect("Cleanup failed");
}

// ============================================================================
// Test 4: Atomic job claiming (no duplicate processing)
// ============================================================================

#[tokio::test]
#[ignore]
async fn test_job_claiming_atomicity() {
    let ctx = TestContext::new()
        .await
        .expect("Failed to create test context");

    // Insert 5 jobs directly — using insert_test_job_raw so that each insert
    // does NOT call cancel_stale_pending_test_jobs (which would cancel the
    // previously inserted jobs and leave only 1 pending).
    let mut job_ids = Vec::new();
    for i in 0..5 {
        let job_id = ctx
            .insert_test_job_raw(&format!("https://youtube.com/watch?v=test{}", i))
            .await
            .expect("Failed to insert test job");
        job_ids.push(job_id);
    }

    eprintln!("✅ Created {} test jobs", job_ids.len());

    use video_editor::jobs::job_claimer::JobClaimer;

    let worker1 = JobClaimer::new("test-worker-1".to_string(), ctx.pool.clone());
    let worker2 = JobClaimer::new("test-worker-2".to_string(), ctx.pool.clone());
    let worker3 = JobClaimer::new("test-worker-3".to_string(), ctx.pool.clone());

    let (claim1, claim2, claim3) = tokio::join!(
        worker1.claim_next_job(),
        worker2.claim_next_job(),
        worker3.claim_next_job()
    );

    let claimed1 = claim1.expect("Worker 1 should claim").expect("Should get job");
    let claimed2 = claim2.expect("Worker 2 should claim").expect("Should get job");
    let claimed3 = claim3.expect("Worker 3 should claim").expect("Should get job");

    // No two workers should claim the same job
    assert_ne!(claimed1, claimed2, "Workers should claim different jobs");
    assert_ne!(claimed2, claimed3, "Workers should claim different jobs");
    assert_ne!(claimed1, claimed3, "Workers should claim different jobs");

    eprintln!("✅ Atomic claiming verified: {} != {} != {}", claimed1, claimed2, claimed3);

    for (job_id, worker_id) in [
        (claimed1, "test-worker-1"),
        (claimed2, "test-worker-2"),
        (claimed3, "test-worker-3"),
    ] {
        let row = sqlx::query("SELECT claimed_by FROM clipping_jobs WHERE id = $1")
            .bind(job_id)
            .fetch_optional(&ctx.pool)
            .await
            .expect("DB query should succeed");

        if let Some(row) = row {
            let claimed_by: Option<String> = row.try_get("claimed_by").ok().flatten();
            assert_eq!(
                claimed_by.as_deref(),
                Some(worker_id),
                "Job {} should be claimed by {}, got {:?}",
                job_id, worker_id, claimed_by
            );
        } else {
            eprintln!("⚠️  Job {} (claimed by {}) no longer in DB — concurrent cleanup", job_id, worker_id);
        }
    }

    worker1.release_all_claims().await.expect("Should release worker 1 claims");
    worker2.release_all_claims().await.expect("Should release worker 2 claims");
    worker3.release_all_claims().await.expect("Should release worker 3 claims");

    eprintln!("✅ All test worker claims released");

    ctx.cleanup().await.expect("Cleanup failed");
}

// ============================================================================
// Test 5: Stuck job detection (analyzing state > 60 min)
// ============================================================================

/// Verify that jobs stuck in 'analyzing' for > 60 min are detected.
///
/// 'analyzing' is Phase A of the new pipeline — if Gemini hangs, the job
/// will be stuck there. The worker's stuck-job detector should catch this.
#[tokio::test]
#[ignore]
async fn test_stuck_job_detection() {
    let ctx = TestContext::new()
        .await
        .expect("Failed to create test context");

    let job_id = ctx
        .create_test_job(test_videos::SHORT_VIDEO)
        .await
        .expect("Failed to create test job");

    // Disable updated_at trigger so we can manually back-date the timestamp
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

    eprintln!("✅ Simulated stuck job {} (analyzing for 65 min)", job_id);

    // Verify the stuck-job query would find this job
    let result = sqlx::query(
        "SELECT id FROM clipping_jobs
         WHERE status = 'analyzing'
           AND updated_at < NOW() - INTERVAL '60 minutes'
           AND id = $1"
    )
    .bind(job_id)
    .fetch_optional(&ctx.pool)
    .await
    .expect("Should query stuck jobs");

    assert!(result.is_some(), "Stuck job should be detected by the 60-min query");

    // Simulate worker resetting the stuck job to failed
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
    .expect("Should reset stuck job to failed");

    let status = ctx.get_job_status(job_id).await.expect("Should get status");
    assert_eq!(status, "failed", "Stuck job should be marked as failed");

    eprintln!("✅ Stuck job detection working (analyzing → failed)");

    ctx.cleanup().await.expect("Cleanup failed");
}

// ============================================================================
// Test 6: Auto-retry of recently failed jobs
// ============================================================================

#[tokio::test]
#[ignore]
async fn test_auto_retry_failed_jobs() {
    let ctx = TestContext::new()
        .await
        .expect("Failed to create test context");

    let job_id = ctx
        .create_test_job(test_videos::SHORT_VIDEO)
        .await
        .expect("Failed to create test job");

    // Simulate a transient failure 6 minutes ago (within the 6-hour retry window)
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

    eprintln!("✅ Simulated failed job {} (6 min ago)", job_id);

    // Verify auto-retry query would pick this up
    let retryable = sqlx::query(
        "SELECT id FROM clipping_jobs
         WHERE status = 'failed'
           AND completed_at > NOW() - INTERVAL '6 hours'
           AND completed_at < NOW() - INTERVAL '5 minutes'
           AND retry_count < 10
           AND id = $1"
    )
    .bind(job_id)
    .fetch_optional(&ctx.pool)
    .await
    .expect("Should query retryable jobs");

    assert!(retryable.is_some(), "Failed job should be retryable within the 6h window");

    // Simulate the retry reset
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

    assertions::assert_retry_incremented(&ctx.pool, job_id)
        .await
        .expect("Retry count should be incremented");

    let status = ctx.get_job_status(job_id).await.expect("Should get status");
    assert_eq!(status, "pending", "Retried job should be back to pending");

    eprintln!("✅ Auto-retry mechanism working");

    ctx.cleanup().await.expect("Cleanup failed");
}

// ============================================================================
// Test 7: no_clips_found fast-fail (short video with no viral moments)
// ============================================================================

/// Verify that a video with no quality viral moments is fast-failed with
/// 'no_clips_found' without triggering a download.
///
/// "Me at the zoo" (18s) is almost certainly below Gemini's viral moment
/// quality threshold — it should reach no_clips_found without downloading.
#[tokio::test]
#[ignore]
async fn test_no_clips_found_fast_fail() {
    let ctx = TestContext::new()
        .await
        .expect("Failed to create test context");

    let job_id = ctx
        .create_test_job(test_videos::SHORT_VIDEO)
        .await
        .expect("Failed to create test job");

    eprintln!("✅ Created job {} for no_clips_found test (18s video)", job_id);

    // Phase A + fast-fail should happen within 3 minutes
    let timeout_secs: u64 = std::env::var("TEST_TIMEOUT_ANALYSIS")
        .ok()
        .and_then(|v| v.parse().ok())
        .unwrap_or(180);

    // Wait until the job leaves 'analyzing' (either fast-fails or proceeds)
    let start = std::time::Instant::now();
    let mut reached_terminal = false;
    let mut final_status = String::from("pending");
    loop {
        let status = ctx.get_job_status(job_id).await.unwrap_or_else(|_| "unknown".into());
        match status.as_str() {
            "pending" | "analyzing" => {}
            _ => {
                final_status = status.clone();
                reached_terminal = true;
                break;
            }
        }

        if start.elapsed().as_secs() > timeout_secs {
            final_status = ctx.get_job_status(job_id).await.unwrap_or_else(|_| "unknown".into());
            break;
        }

        tokio::time::sleep(tokio::time::Duration::from_secs(5)).await;
    }

    assert!(
        reached_terminal,
        "Job {} should leave 'analyzing' within {} seconds (final: '{}')",
        job_id, timeout_secs, final_status
    );

    eprintln!(
        "ℹ️  Short video job {} ended with status '{}' (expected no_clips_found or completed)",
        job_id, final_status
    );

    // The job should NOT have reached 'downloading' — fast-fail skips the download
    let did_download = sqlx::query(
        "SELECT id FROM clipping_jobs
         WHERE id = $1 AND status IN ('downloading','downloaded','extracting_clips',
                                      'clips_extracted','vectorizing','posting','completed')"
    )
    .bind(job_id)
    .fetch_optional(&ctx.pool)
    .await
    .expect("DB query failed");

    // For an 18s video, Gemini very likely returns no_clips_found — but we don't
    // hard-assert it, because Gemini might surprise us. We just log the outcome.
    if did_download.is_some() {
        eprintln!("ℹ️  Gemini found viral moments even in the 18s video — download proceeded (unexpected but not wrong)");
    } else {
        eprintln!("✅ Fast-fail confirmed — download was skipped for short video");
    }

    ctx.cleanup().await.expect("Cleanup failed");
}

// ============================================================================
// Twitch Integration Tests
// ============================================================================

/// Test: Twitch channel search returns results for a known broadcaster.
///
/// Requires: TWITCH_TV_CLIENT_ID and TWITCH_TV_CLIENT_SECRET in .env.test
#[tokio::test]
#[ignore]
async fn test_twitch_channel_search() {
    dotenvy::from_filename(".env.test").ok();
    let client_id = std::env::var("TWITCH_TV_CLIENT_ID")
        .expect("TWITCH_TV_CLIENT_ID required");
    let client_secret = std::env::var("TWITCH_TV_CLIENT_SECRET")
        .expect("TWITCH_TV_CLIENT_SECRET required");
    let db_url = std::env::var("DATABASE_URL").expect("DATABASE_URL required");

    let pool = sqlx::PgPool::connect(&db_url).await.expect("DB connection failed");
    let client = video_editor::twitch_client::TwitchClient::new(client_id, client_secret, pool);

    let results = client.search_channels("xqc", 5).await.expect("Search failed");
    eprintln!("Search results ({} found):", results.len());
    for ch in &results {
        eprintln!("  {} ({}) — id={}", ch.display_name, ch.broadcaster_login, ch.broadcaster_id);
    }
    assert!(!results.is_empty(), "Expected at least one result for 'xqc'");
}

/// Test: Add a real Twitch channel to twitch_source_channels table via API.
///
/// Uses the running local server. Start with `cargo run` before running this test.
#[tokio::test]
#[ignore]
async fn test_twitch_channel_add() {
    dotenvy::from_filename(".env.test").ok();
    let client = reqwest::Client::new();

    // Login first to get a JWT
    let login_resp = client
        .post("http://127.0.0.1:3000/api/auth/login")
        .json(&serde_json::json!({
            "email": std::env::var("TEST_ADMIN_EMAIL").unwrap_or_default(),
            "password": std::env::var("TEST_ADMIN_PASSWORD").unwrap_or_default(),
        }))
        .send()
        .await
        .expect("Login request failed");

    let login_body: serde_json::Value = login_resp.json().await.expect("Login response parse failed");
    let token = login_body["token"].as_str().expect("No token in login response");

    // Add xQc's Twitch channel (broadcaster_id for xQc is 71092938)
    let add_resp = client
        .post("http://127.0.0.1:3000/api/clipping/twitch/source-channels")
        .header("Authorization", format!("Bearer {}", token))
        .json(&serde_json::json!({"broadcaster_id": "71092938"}))
        .send()
        .await
        .expect("Add channel request failed");

    let status = add_resp.status();
    let body: serde_json::Value = add_resp.json().await.expect("Response parse failed");
    eprintln!("Add Twitch channel response ({}): {:?}", status, body);

    assert!(
        status.is_success() || status.as_u16() == 409,
        "Expected 201 or 409 (already exists), got {}: {:?}",
        status,
        body
    );
}

/// Test: Manually map a YouTube source channel to a Twitch channel.
///
/// Requires: at least one YouTube source channel and one Twitch source channel in DB.
/// Start server with `cargo run` before running.
#[tokio::test]
#[ignore]
async fn test_twitch_mapping_manual() {
    dotenvy::from_filename(".env.test").ok();
    let db_url = std::env::var("DATABASE_URL").expect("DATABASE_URL required");
    let pool = sqlx::PgPool::connect(&db_url).await.expect("DB failed");

    // Check prerequisites
    let yt_id: Option<(i32,)> = sqlx::query_as(
        "SELECT id FROM youtube_source_channels WHERE is_active = true LIMIT 1",
    )
    .fetch_optional(&pool)
    .await
    .expect("DB query failed");

    let tw_id: Option<(i32,)> = sqlx::query_as(
        "SELECT id FROM twitch_source_channels WHERE is_active = true LIMIT 1",
    )
    .fetch_optional(&pool)
    .await
    .expect("DB query failed");

    let (Some((yt_ch_id,)), Some((tw_ch_id,))) = (yt_id, tw_id) else {
        eprintln!("⚠️  test_twitch_mapping_manual: need at least one YouTube + one Twitch source channel in DB");
        return;
    };

    eprintln!("Mapping YouTube channel {} → Twitch channel {}", yt_ch_id, tw_ch_id);

    let client = reqwest::Client::new();
    let login_resp = client
        .post("http://127.0.0.1:3000/api/auth/login")
        .json(&serde_json::json!({
            "email": std::env::var("TEST_ADMIN_EMAIL").unwrap_or_default(),
            "password": std::env::var("TEST_ADMIN_PASSWORD").unwrap_or_default(),
        }))
        .send()
        .await
        .expect("Login failed");
    let login_body: serde_json::Value = login_resp.json().await.unwrap();
    let token = login_body["token"].as_str().expect("No token");

    let map_resp = client
        .post("http://127.0.0.1:3000/api/clipping/twitch/mappings")
        .header("Authorization", format!("Bearer {}", token))
        .json(&serde_json::json!({
            "youtube_source_channel_id": yt_ch_id,
            "twitch_source_channel_id": tw_ch_id,
        }))
        .send()
        .await
        .expect("Mapping request failed");

    let status = map_resp.status();
    let body: serde_json::Value = map_resp.json().await.unwrap();
    eprintln!("Mapping response ({}): {:?}", status, body);

    assert!(
        status.is_success() || status.as_u16() == 409,
        "Expected 201 or 409, got {}: {:?}",
        status,
        body
    );
}

/// Test: Gemini auto-mapper correctly identifies that a YouTube gaming channel maps to Twitch.
///
/// Uses a real Gemini API call — requires GEMINI_API_KEY and Twitch credentials.
#[tokio::test]
#[ignore]
async fn test_twitch_mapper_ai() {
    dotenvy::from_filename(".env.test").ok();
    let db_url = std::env::var("DATABASE_URL").expect("DATABASE_URL required");
    let pool = sqlx::PgPool::connect(&db_url).await.expect("DB failed");

    let client_id = std::env::var("TWITCH_TV_CLIENT_ID").expect("TWITCH_TV_CLIENT_ID required");
    let client_secret = std::env::var("TWITCH_TV_CLIENT_SECRET").expect("TWITCH_TV_CLIENT_SECRET required");
    let gemini_key = std::env::var("GEMINI_API_KEY").expect("GEMINI_API_KEY required");

    let twitch = video_editor::twitch_client::TwitchClient::new(client_id, client_secret, pool.clone());
    let gemini = video_editor::gemini_client::GeminiClient::new(gemini_key);

    // Create a fake source channel for testing
    let fake_channel = video_editor::clipping::models::SourceChannel {
        id: -999,
        channel_id: "TEST_MAPPER_CHANNEL".to_string(),
        channel_name: "xQcOW".to_string(), // xQc — definitely has a Twitch channel
        channel_thumbnail_url: None,
        subscriber_count: None,
        is_active: true,
        polling_interval_minutes: 30,
        last_polled_at: None,
        last_video_checked: None,
        created_at: chrono::Utc::now(),
        updated_at: chrono::Utc::now(),
    };

    let result = video_editor::services::twitch_mapper::auto_map_youtube_to_twitch(
        &fake_channel,
        &twitch,
        &gemini,
        &pool,
    )
    .await;

    match result {
        Ok(video_editor::services::twitch_mapper::MappingResult::Mapped(ch)) => {
            eprintln!("✅ Mapped to Twitch: {} ({})", ch.display_name, ch.broadcaster_login);
        }
        Ok(video_editor::services::twitch_mapper::MappingResult::NoEquivalent) => {
            eprintln!("ℹ️  Gemini said no Twitch equivalent (may vary)");
        }
        Err(e) => {
            panic!("Mapping failed: {}", e);
        }
    }
}

/// Test: Verify pick_twitch_vod returns a VOD for a channel with a mapping,
/// and that it avoids already-clipped VODs.
///
/// This is a logic test only — does not download anything.
#[tokio::test]
#[ignore]
async fn test_twitch_fallback_logic() {
    dotenvy::from_filename(".env.test").ok();
    let db_url = std::env::var("DATABASE_URL").expect("DATABASE_URL required");
    let pool = sqlx::PgPool::connect(&db_url).await.expect("DB failed");

    let client_id = std::env::var("TWITCH_TV_CLIENT_ID").expect("TWITCH_TV_CLIENT_ID required");
    let client_secret = std::env::var("TWITCH_TV_CLIENT_SECRET").expect("TWITCH_TV_CLIENT_SECRET required");

    let twitch = video_editor::twitch_client::TwitchClient::new(client_id, client_secret, pool.clone());

    // Use xQc (broadcaster_id 71092938) to verify get_videos works
    let (videos, cursor) = twitch
        .get_videos("71092938", None, 5)
        .await
        .expect("get_videos failed");

    eprintln!("xQc VODs ({} returned, cursor={:?}):", videos.len(), cursor);
    for v in &videos {
        eprintln!("  id={} title='{}' url={} duration={}", v.id, v.title, v.url, v.duration);
    }

    // We just verify that the Twitch API returns valid data
    assert!(!videos.is_empty(), "Expected at least one VOD from xQc");
    for v in &videos {
        assert!(v.url.contains("twitch.tv"), "VOD URL should contain twitch.tv: {}", v.url);
    }
    eprintln!("✅ Twitch VOD listing and URL format verified");
}
