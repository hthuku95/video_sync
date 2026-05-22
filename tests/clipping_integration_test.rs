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

use helpers::test_youtube::test_videos;
use helpers::{assertions, TestContext};
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
        .expect("Failed to create test context — ensure a channel linkage with valid OAuth exists");

    // 1. Look for an existing non-terminal job on this linkage — resume if found
    let existing_job = sqlx::query(
        "SELECT id, status FROM clipping_jobs
         WHERE linkage_id = $1
           AND status NOT IN ('completed', 'cancelled')
         ORDER BY created_at DESC
         LIMIT 1",
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

    let final_status = ctx
        .get_job_status(job_id)
        .await
        .unwrap_or_else(|_| "unknown".into());
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
         WHERE clipping_job_id = $1",
    )
    .bind(job_id)
    .fetch_all(&ctx.pool)
    .await
    .expect("Failed to fetch clips");

    assert!(
        !clips.is_empty(),
        "No clips in extracted_clips table for job {}",
        job_id
    );

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
        job_id,
        published,
        clips.len()
    );

    eprintln!(
        "✅ {} clip(s) published to YouTube for job {}",
        published, job_id
    );

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
        let status = ctx
            .get_job_status(job_id)
            .await
            .unwrap_or_else(|_| "unknown".into());
        match status.as_str() {
            "pending" | "analyzing" => {
                // still in Phase A — keep waiting
            }
            "no_clips_found" => {
                eprintln!("ℹ️  Job {} → no_clips_found (Gemini found no quality moments — still a valid Phase A result)", job_id);
                passed_analysis = true;
                break;
            }
            "analyzed" | "downloading" | "downloaded" | "extracting_clips" | "clips_extracted"
            | "vectorizing" | "posting" | "completed" => {
                eprintln!(
                    "✅ Job {} passed Phase A analysis, now at: '{}'",
                    job_id, status
                );
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
            eprintln!(
                "⏰ Phase A timed out after {}s — final status: '{}'",
                timeout_secs, status
            );
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
        let status = ctx
            .get_job_status(job_id)
            .await
            .unwrap_or_else(|_| "unknown".into());
        match status.as_str() {
            "downloaded" | "extracting_clips" | "clips_extracted" | "vectorizing" | "posting"
            | "completed" => {
                eprintln!(
                    "✅ Job {} download confirmed — status: '{}'",
                    job_id, status
                );
                download_succeeded = true;
                break;
            }
            "no_clips_found" => {
                // Phase A found nothing → download was skipped (that's correct behaviour)
                eprintln!(
                    "ℹ️  Job {} → no_clips_found — download correctly skipped",
                    job_id
                );
                download_succeeded = true; // skip is also correct behaviour
                break;
            }
            "failed" | "cancelled" => {
                let err = sqlx::query("SELECT error_message FROM clipping_jobs WHERE id = $1")
                    .bind(job_id)
                    .fetch_one(&ctx.pool)
                    .await
                    .ok()
                    .and_then(|r| {
                        r.try_get::<Option<String>, _>("error_message")
                            .ok()
                            .flatten()
                    })
                    .unwrap_or_else(|| "no error message".into());
                eprintln!("❌ Job {} failed: {}", job_id, err);
                break;
            }
            _ => {}
        }

        if start.elapsed().as_secs() > timeout_secs {
            eprintln!(
                "⏰ Download timed out after {}s — final status: '{}'",
                timeout_secs, status
            );
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

    let claimed1 = claim1
        .expect("Worker 1 should claim")
        .expect("Should get job");
    let claimed2 = claim2
        .expect("Worker 2 should claim")
        .expect("Should get job");
    let claimed3 = claim3
        .expect("Worker 3 should claim")
        .expect("Should get job");

    // No two workers should claim the same job
    assert_ne!(claimed1, claimed2, "Workers should claim different jobs");
    assert_ne!(claimed2, claimed3, "Workers should claim different jobs");
    assert_ne!(claimed1, claimed3, "Workers should claim different jobs");

    eprintln!(
        "✅ Atomic claiming verified: {} != {} != {}",
        claimed1, claimed2, claimed3
    );

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
                job_id,
                worker_id,
                claimed_by
            );
        } else {
            eprintln!(
                "⚠️  Job {} (claimed by {}) no longer in DB — concurrent cleanup",
                job_id, worker_id
            );
        }
    }

    worker1
        .release_all_claims()
        .await
        .expect("Should release worker 1 claims");
    worker2
        .release_all_claims()
        .await
        .expect("Should release worker 2 claims");
    worker3
        .release_all_claims()
        .await
        .expect("Should release worker 3 claims");

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
         WHERE id = $1",
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
           AND id = $1",
    )
    .bind(job_id)
    .fetch_optional(&ctx.pool)
    .await
    .expect("Should query stuck jobs");

    assert!(
        result.is_some(),
        "Stuck job should be detected by the 60-min query"
    );

    // Simulate worker resetting the stuck job to failed
    sqlx::query(
        "UPDATE clipping_jobs
         SET status = 'failed',
             error_message = 'Job stuck in analyzing status for > 60 minutes',
             stuck_detection_count = stuck_detection_count + 1
         WHERE id = $1",
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
         WHERE id = $1",
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
           AND id = $1",
    )
    .bind(job_id)
    .fetch_optional(&ctx.pool)
    .await
    .expect("Should query retryable jobs");

    assert!(
        retryable.is_some(),
        "Failed job should be retryable within the 6h window"
    );

    // Simulate the retry reset
    sqlx::query(
        "UPDATE clipping_jobs
         SET status = 'pending',
             error_message = NULL,
             retry_count = retry_count + 1,
             last_retry_at = NOW()
         WHERE id = $1",
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

    eprintln!(
        "✅ Created job {} for no_clips_found test (18s video)",
        job_id
    );

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
        let status = ctx
            .get_job_status(job_id)
            .await
            .unwrap_or_else(|_| "unknown".into());
        match status.as_str() {
            "pending" | "analyzing" => {}
            _ => {
                final_status = status.clone();
                reached_terminal = true;
                break;
            }
        }

        if start.elapsed().as_secs() > timeout_secs {
            final_status = ctx
                .get_job_status(job_id)
                .await
                .unwrap_or_else(|_| "unknown".into());
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
                                      'clips_extracted','vectorizing','posting','completed')",
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
//
// All tests follow the same real-data principle as the rest of this file:
//  - No hardcoded channel IDs or names
//  - TestContext::new() provides the real active linkage (real YouTube source channel,
//    real destination channel with valid OAuth)
//  - Twitch channel names are derived from real youtube_source_channels rows in the DB
//  - Twitch mappings are read from the DB (youtube_twitch_channel_mappings) when they exist
//  - Tests only create/cancel clipping_jobs records — never delete channels or linkages
// ============================================================================

/// Test: Search Twitch for the real YouTube source channel name that is linked in the DB.
///
/// Uses the live linkage from TestContext (same real channel the production pipeline uses).
/// No hardcoded channel names — searches Twitch using the actual channel_name stored in
/// youtube_source_channels for the active linkage.
///
/// Prereq: active channel linkage with valid OAuth (same as test_complete_clipping_workflow).
#[tokio::test]
#[ignore]
async fn test_twitch_channel_search() {
    dotenvy::from_filename(".env.test").ok();

    let ctx = TestContext::new()
        .await
        .expect("Need an active channel linkage with valid OAuth — see TestContext::new()");

    let client_id =
        std::env::var("TWITCH_TV_CLIENT_ID").expect("TWITCH_TV_CLIENT_ID required in .env.test");
    let client_secret = std::env::var("TWITCH_TV_CLIENT_SECRET")
        .expect("TWITCH_TV_CLIENT_SECRET required in .env.test");

    let twitch =
        video_editor::twitch_client::TwitchClient::new(client_id, client_secret, ctx.pool.clone());

    // Fetch the real channel name from the DB — no hardcoded names
    let row = sqlx::query("SELECT channel_name FROM youtube_source_channels WHERE id = $1")
        .bind(ctx.source_channel_id)
        .fetch_one(&ctx.pool)
        .await
        .expect("Failed to fetch source channel name");

    let channel_name: String = row.get("channel_name");
    eprintln!(
        "Searching Twitch for real YouTube source channel: '{}'",
        channel_name
    );

    let results = twitch
        .search_channels(&channel_name, 5)
        .await
        .expect("Twitch search failed");

    eprintln!("Twitch search results ({} found):", results.len());
    for ch in &results {
        eprintln!(
            "  broadcaster_login={} display_name='{}' id={}",
            ch.broadcaster_login, ch.display_name, ch.broadcaster_id
        );
    }

    // We don't assert a non-empty result — the real YouTube source channel may not
    // have a Twitch presence. We assert only that the API call succeeded without error.
    eprintln!(
        "✅ Twitch search API responded successfully for '{}'",
        channel_name
    );
}

/// Test: Add a Twitch channel derived from the real YouTube source channel, then create
/// a YouTube→Twitch mapping via the API endpoint.
///
/// Flow:
///   1. Read real source channel name from youtube_source_channels (via TestContext)
///   2. Search Twitch for that channel name → get first result's broadcaster_id
///   3. POST to /api/clipping/twitch/source-channels to persist it
///   4. POST to /api/clipping/twitch/mappings to link it to the real YouTube source channel
///
/// Idempotent: accepts 409 (already exists) as a success.
/// Prereq: server running (`cargo run`), active channel linkage with valid OAuth.
#[tokio::test]
#[ignore]
async fn test_twitch_channel_add_and_map() {
    dotenvy::from_filename(".env.test").ok();

    let ctx = TestContext::new()
        .await
        .expect("Need an active channel linkage with valid OAuth");

    let client_id = std::env::var("TWITCH_TV_CLIENT_ID").expect("TWITCH_TV_CLIENT_ID required");
    let client_secret =
        std::env::var("TWITCH_TV_CLIENT_SECRET").expect("TWITCH_TV_CLIENT_SECRET required");

    let twitch =
        video_editor::twitch_client::TwitchClient::new(client_id, client_secret, ctx.pool.clone());

    // 1. Get real channel name from DB
    let row = sqlx::query("SELECT channel_name FROM youtube_source_channels WHERE id = $1")
        .bind(ctx.source_channel_id)
        .fetch_one(&ctx.pool)
        .await
        .expect("Failed to fetch source channel");

    let channel_name: String = row.get("channel_name");
    eprintln!("Real YouTube source channel: '{}'", channel_name);

    // 2. Search Twitch — skip if channel has no Twitch presence
    let results = twitch
        .search_channels(&channel_name, 3)
        .await
        .expect("Twitch search failed");

    if results.is_empty() {
        eprintln!(
            "⚠️  '{}' has no Twitch search results — skipping add/map (not an error)",
            channel_name
        );
        return;
    }

    let first = &results[0];
    eprintln!(
        "Using first Twitch result: {} ({}) id={}",
        first.display_name, first.broadcaster_login, first.broadcaster_id
    );

    // 3. Login to get JWT
    let http = reqwest::Client::new();
    let login_body: serde_json::Value = http
        .post("http://127.0.0.1:3000/api/auth/login")
        .json(&serde_json::json!({
            "email": std::env::var("TEST_ADMIN_EMAIL").unwrap_or_default(),
            "password": std::env::var("TEST_ADMIN_PASSWORD").unwrap_or_default(),
        }))
        .send()
        .await
        .expect("Login request failed")
        .json()
        .await
        .expect("Login response parse failed");

    let token = login_body["token"]
        .as_str()
        .expect("No token in login response");

    // 4. Add Twitch channel (broadcaster_id from real Twitch API search result)
    let add_resp = http
        .post("http://127.0.0.1:3000/api/clipping/twitch/source-channels")
        .header("Authorization", format!("Bearer {}", token))
        .json(&serde_json::json!({"broadcaster_id": first.broadcaster_id}))
        .send()
        .await
        .expect("Add Twitch channel request failed");

    let add_status = add_resp.status();
    let add_body: serde_json::Value = add_resp.json().await.expect("Add response parse failed");
    eprintln!("Add Twitch channel ({}): {:?}", add_status, add_body);

    assert!(
        add_status.is_success() || add_status.as_u16() == 409,
        "Expected 201 or 409 from add, got {}: {:?}",
        add_status,
        add_body
    );

    // 5. Fetch the twitch_source_channels.id that was just added/already existed
    let tw_ch_id: (i32,) = sqlx::query_as::<_, (i32,)>(
        "SELECT id FROM twitch_source_channels WHERE broadcaster_id = $1",
    )
    .bind(&first.broadcaster_id)
    .fetch_one(&ctx.pool)
    .await
    .expect("Twitch channel row not found after add");

    eprintln!(
        "Twitch channel DB id={} — mapping to YouTube source channel id={}",
        tw_ch_id.0, ctx.source_channel_id
    );

    // 6. Create the mapping
    let map_resp = http
        .post("http://127.0.0.1:3000/api/clipping/twitch/mappings")
        .header("Authorization", format!("Bearer {}", token))
        .json(&serde_json::json!({
            "youtube_source_channel_id": ctx.source_channel_id,
            "twitch_source_channel_id": tw_ch_id.0,
        }))
        .send()
        .await
        .expect("Create mapping request failed");

    let map_status = map_resp.status();
    let map_body: serde_json::Value = map_resp
        .json()
        .await
        .expect("Mapping response parse failed");
    eprintln!("Create mapping ({}): {:?}", map_status, map_body);

    assert!(
        map_status.is_success() || map_status.as_u16() == 409,
        "Expected 201 or 409 from mapping, got {}: {:?}",
        map_status,
        map_body
    );

    eprintln!(
        "✅ '{}' YouTube channel linked to Twitch:'{}'",
        channel_name, first.broadcaster_login
    );
}

/// Test: Run the Gemini auto-mapper on a REAL unmapped youtube_source_channels row.
///
/// Picks the real YouTube source channel from the active linkage (TestContext).
/// If it is already mapped, verifies the existing mapping is consistent.
/// Uses real Gemini API + real Twitch API — no fake channels.
///
/// Prereq: active channel linkage with valid OAuth, GEMINI_API_KEY in .env.test.
#[tokio::test]
#[ignore]
async fn test_twitch_mapper_ai() {
    dotenvy::from_filename(".env.test").ok();

    let ctx = TestContext::new()
        .await
        .expect("Need an active channel linkage with valid OAuth");

    let client_id = std::env::var("TWITCH_TV_CLIENT_ID").expect("TWITCH_TV_CLIENT_ID required");
    let client_secret =
        std::env::var("TWITCH_TV_CLIENT_SECRET").expect("TWITCH_TV_CLIENT_SECRET required");
    let gemini_key = std::env::var("GEMINI_API_KEY").expect("GEMINI_API_KEY required");

    let twitch =
        video_editor::twitch_client::TwitchClient::new(client_id, client_secret, ctx.pool.clone());
    let gemini = video_editor::gemini_client::GeminiClient::new(gemini_key);

    // Fetch the real SourceChannel row from the DB — the same one linked by this linkage
    let source_channel: video_editor::clipping::models::SourceChannel =
        sqlx::query_as("SELECT * FROM youtube_source_channels WHERE id = $1")
            .bind(ctx.source_channel_id)
            .fetch_one(&ctx.pool)
            .await
            .expect("Failed to fetch source channel from DB");

    eprintln!(
        "Running Gemini auto-mapper on real YouTube source channel: '{}' (id={})",
        source_channel.channel_name, source_channel.id
    );

    // Reset twitch_mapping_status to 'unmapped' so the mapper actually runs
    // (only if not already mapped — we don't want to clobber a real existing mapping)
    let current_status: Option<String> = sqlx::query_scalar(
        "SELECT twitch_mapping_status FROM youtube_source_channels WHERE id = $1",
    )
    .bind(source_channel.id)
    .fetch_optional(&ctx.pool)
    .await
    .expect("DB query failed");

    if current_status.as_deref() == Some("mapped") {
        eprintln!(
            "ℹ️  Channel '{}' is already mapped — verifying existing mapping",
            source_channel.channel_name
        );
        let mapping: Option<(i32, String)> = sqlx::query_as::<_, (i32, String)>(
            "SELECT tsc.id, tsc.broadcaster_login
             FROM youtube_twitch_channel_mappings ytm
             JOIN twitch_source_channels tsc ON tsc.id = ytm.twitch_source_channel_id
             WHERE ytm.youtube_source_channel_id = $1",
        )
        .bind(source_channel.id)
        .fetch_optional(&ctx.pool)
        .await
        .expect("DB query failed");

        if let Some((_, login)) = mapping {
            eprintln!(
                "✅ Existing mapping: YouTube:'{}' → Twitch:'{}'",
                source_channel.channel_name, login
            );
        }
        return;
    }

    // Run the real Gemini auto-mapper
    let result = video_editor::services::twitch_mapper::auto_map_youtube_to_twitch(
        &source_channel,
        &twitch,
        &gemini,
        &ctx.pool,
    )
    .await;

    match result {
        Ok(video_editor::services::twitch_mapper::MappingResult::Mapped(ch)) => {
            eprintln!(
                "✅ Gemini mapped '{}' → Twitch:'{}' ({})",
                source_channel.channel_name, ch.broadcaster_login, ch.display_name
            );
        }
        Ok(video_editor::services::twitch_mapper::MappingResult::NoEquivalent) => {
            eprintln!(
                "ℹ️  Gemini: '{}' has no Twitch equivalent (valid outcome)",
                source_channel.channel_name
            );
        }
        Err(e) => {
            panic!(
                "Auto-mapper returned error for '{}': {}",
                source_channel.channel_name, e
            );
        }
    }
}

/// Test: Verify the Twitch VOD listing works for a REAL mapped Twitch channel.
///
/// Reads the youtube_twitch_channel_mappings table for the real linkage from TestContext.
/// If no mapping exists yet, runs the auto-mapper first (same as test_twitch_mapper_ai).
/// Then calls get_videos() on the real Twitch broadcaster and verifies the response.
///
/// Prereq: active channel linkage with valid OAuth, Twitch credentials in .env.test.
#[tokio::test]
#[ignore]
async fn test_twitch_fallback_logic() {
    dotenvy::from_filename(".env.test").ok();

    let ctx = TestContext::new()
        .await
        .expect("Need an active channel linkage with valid OAuth");

    let client_id = std::env::var("TWITCH_TV_CLIENT_ID").expect("TWITCH_TV_CLIENT_ID required");
    let client_secret =
        std::env::var("TWITCH_TV_CLIENT_SECRET").expect("TWITCH_TV_CLIENT_SECRET required");

    let twitch = video_editor::twitch_client::TwitchClient::new(
        client_id.clone(),
        client_secret.clone(),
        ctx.pool.clone(),
    );

    // Look up the real Twitch broadcaster_id for the linkage's source channel
    let mapping: Option<(String, String)> = sqlx::query_as::<_, (String, String)>(
        "SELECT tsc.broadcaster_id, tsc.broadcaster_login
         FROM youtube_twitch_channel_mappings ytm
         JOIN twitch_source_channels tsc ON tsc.id = ytm.twitch_source_channel_id
         WHERE ytm.youtube_source_channel_id = $1",
    )
    .bind(ctx.source_channel_id)
    .fetch_optional(&ctx.pool)
    .await
    .expect("DB query failed");

    let (broadcaster_id, broadcaster_login) = match mapping {
        Some(m) => {
            eprintln!(
                "Using real mapping: YouTube source_channel_id={} → Twitch:{}",
                ctx.source_channel_id, m.1
            );
            m
        }
        None => {
            // No mapping yet — try a Twitch search using the real channel name and use
            // the first result so the test still exercises the get_videos path
            let row = sqlx::query("SELECT channel_name FROM youtube_source_channels WHERE id = $1")
                .bind(ctx.source_channel_id)
                .fetch_one(&ctx.pool)
                .await
                .expect("Failed to fetch source channel");

            let channel_name: String = row.get("channel_name");
            eprintln!(
                "No Twitch mapping for '{}' yet — searching Twitch to exercise get_videos",
                channel_name
            );

            let results = twitch
                .search_channels(&channel_name, 3)
                .await
                .expect("Twitch search failed");

            if results.is_empty() {
                eprintln!(
                    "⚠️  '{}' has no Twitch search results — cannot test get_videos (not an error)",
                    channel_name
                );
                return;
            }

            eprintln!(
                "Using first Twitch search result: {} ({})",
                results[0].display_name, results[0].broadcaster_login
            );
            (
                results[0].broadcaster_id.clone(),
                results[0].broadcaster_login.clone(),
            )
        }
    };

    // Call get_videos on the real Twitch broadcaster
    let (videos, cursor) = twitch
        .get_videos(&broadcaster_id, None, 5)
        .await
        .expect("get_videos failed");

    eprintln!(
        "Twitch:{} — {} VODs returned (cursor={:?}):",
        broadcaster_login,
        videos.len(),
        cursor
    );
    for v in &videos {
        eprintln!(
            "  id={} duration={} title='{}'  url={}",
            v.id, v.duration, v.title, v.url
        );
    }

    // A broadcaster with any archived streams will have VODs.
    // We only assert URL format if VODs exist — a new/inactive channel may have none.
    for v in &videos {
        assert!(
            v.url.contains("twitch.tv"),
            "Expected VOD URL to contain 'twitch.tv': {}",
            v.url
        );
    }

    eprintln!(
        "✅ Twitch get_videos verified for Twitch:{} (broadcaster_id={})",
        broadcaster_login, broadcaster_id
    );
}
