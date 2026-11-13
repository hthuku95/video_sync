//! Education-posting worker (owner directive Nov 13 2026).
//!
//! Converts the legacy YouTube-API system from clip-posting to EDUCATION
//! posting: channels connected by whitelisted users/admins receive scheduled
//! generated educational videos (default topic: History) instead of clips.
//!
//! Flow per job:
//!   1. Scheduler: every 30 min, for each ACTIVE education linkage, check
//!      whether the channel is due (cadence from system_settings
//!      `education_videos_per_channel_per_day`, default 0.5 = 1 video every
//!      2 days). Create `education_post_jobs` rows for due channels
//!      (staggered round-robin, bounded batch per cycle).
//!   2. Executor (bounded concurrency): for each job —
//!      a. Agent writes the video brief from the linkage topic, picking a
//!         FRESH subtopic (education_posted_topics ledger prevents repeats),
//!         using the dedicated education NIM key.
//!      b. Education generation via AgenticServicePipeline (nim_profile=
//!         "education" — the full hardened stack: research step, audio-first,
//!         feature budget, dual QA).
//!      c. Wait for the workflow to finish; fetch the output video.
//!      d. Upload to the connected channel via YouTube Data API (resumable-
//!         backed upload_video; tokens refreshed when expired).
//!      e. Record youtube_video_id, mark completed, append the topic ledger.
//!
//! Runs ONLY when the EDUCATION_WORKER env is set (dedicated Render worker
//! service `vs-edu-worker`) — the API instance never runs this loop.

use serde_json::json;
use std::sync::Arc;
use uuid::Uuid;

use crate::AppState;

const CYCLE_SECS: u64 = 30 * 60;
/// Wall-clock budget for one education generation + upload attempt.
const JOB_TIMEOUT_SECS: u64 = 45 * 60;
/// Poll interval while waiting for the education pipeline to finish.
const PIPELINE_POLL_SECS: u64 = 30;
/// Max concurrent generations (droplet CPU protection — owner directive).
const MAX_CONCURRENT_GENERATIONS: usize = 2;
/// Max jobs created per scheduler cycle.
const MAX_JOBS_PER_CYCLE: usize = 4;

pub async fn run_education_worker_loop(state: Arc<AppState>) {
    tracing::info!(
        "🎓 Education-posting worker started (cycle: {}s, concurrency: {})",
        CYCLE_SECS,
        MAX_CONCURRENT_GENERATIONS
    );
    let mut ticker = tokio::time::interval(std::time::Duration::from_secs(CYCLE_SECS));
    ticker.set_missed_tick_behavior(tokio::time::MissedTickBehavior::Delay);
    loop {
        ticker.tick().await;
        if let Err(e) = schedule_due_jobs(&state).await {
            tracing::error!("education scheduler error: {e}");
        }
        if let Err(e) = run_due_jobs(&state).await {
            tracing::error!("education executor error: {e}");
        }
    }
}

// ── Scheduler ───────────────────────────────────────────────────────────────

async fn schedule_due_jobs(state: &Arc<AppState>) -> Result<(), String> {
    let rate: f64 = sqlx::query_scalar::<_, String>(
        "SELECT setting_value FROM system_settings WHERE setting_key = 'education_videos_per_channel_per_day'",
    )
    .fetch_optional(&state.db_pool)
    .await
    .ok()
    .flatten()
    .and_then(|v| v.parse::<f64>().ok())
    .unwrap_or(0.5);
    let cadence_hours = (24.0 / rate.max(0.05)).max(6.0);

    // Active linkages, oldest-posted first (staggered round-robin).
    let rows = sqlx::query_as::<_, (i32, String, Option<chrono::DateTime<chrono::Utc>>)>(
        r#"
        SELECT el.id, el.topic,
               (SELECT MAX(created_at) FROM education_post_jobs j
                 WHERE j.connected_channel_id = el.connected_channel_id) AS last_job
        FROM education_linkages el
        JOIN connected_youtube_channels c ON c.id = el.connected_channel_id
        WHERE el.active = TRUE
          AND NOT EXISTS (
              SELECT 1 FROM education_post_jobs j2
              WHERE j2.linkage_id = el.id AND j2.status IN ('queued','generating','uploading'))
        ORDER BY last_job NULLS FIRST
        "#,
    )
    .fetch_all(&state.db_pool)
    .await
    .map_err(|e| e.to_string())?;

    let now = chrono::Utc::now();
    let mut created = 0usize;
    for (linkage_id, topic, last_job) in rows {
        if created >= MAX_JOBS_PER_CYCLE {
            break;
        }
        let due = match last_job {
            None => true,
            Some(t) => now - t >= chrono::Duration::hours(cadence_hours as i64),
        };
        if !due {
            continue;
        }
        let channel_id: i32 =
            sqlx::query_scalar("SELECT connected_channel_id FROM education_linkages WHERE id = $1")
                .bind(linkage_id)
                .fetch_one(&state.db_pool)
                .await
                .map_err(|e| e.to_string())?;

        let _ = sqlx::query(
            "INSERT INTO education_post_jobs (linkage_id, connected_channel_id, topic, status, scheduled_for)
             VALUES ($1, $2, $3, 'queued', NOW())",
        )
        .bind(linkage_id)
        .bind(channel_id)
        .bind(&topic)
        .execute(&state.db_pool)
        .await;
        created += 1;
        tracing::info!(linkage_id, channel_id, %topic, "🎓 education post queued");
    }
    Ok(())
}

// ── Executor ────────────────────────────────────────────────────────────────

async fn run_due_jobs(state: &Arc<AppState>) -> Result<(), String> {
    for _ in 0..MAX_CONCURRENT_GENERATIONS {
        let job = claim_next_job(state).await?;
        let Some((job_id, linkage_id, channel_id, topic)) = job else {
            break;
        };
        execute_job(state.clone(), job_id, linkage_id, channel_id, topic).await;
    }
    Ok(())
}

type ClaimedJob = (uuid::Uuid, i32, i32, String);

/// Atomic claim: queued → generating (SKIP LOCKED so two cycles never double-run).
async fn claim_next_job(state: &Arc<AppState>) -> Result<Option<ClaimedJob>, String> {
    let row = sqlx::query_as::<_, (uuid::Uuid, i32, i32, String)>(
        r#"
        WITH next_job AS (
            SELECT id FROM education_post_jobs
            WHERE status = 'queued'
            ORDER BY scheduled_for NULLS FIRST, created_at
            FOR UPDATE SKIP LOCKED LIMIT 1
        )
        UPDATE education_post_jobs j
        SET status = 'generating', started_at = NOW(), updated_at = NOW()
        FROM next_job nj
        WHERE j.id = nj.id
        RETURNING j.id, j.linkage_id, j.connected_channel_id, j.topic
        "#,
    )
    .fetch_optional(&state.db_pool)
    .await
    .map_err(|e| e.to_string())?;
    Ok(row)
}

async fn execute_job(
    state: Arc<AppState>,
    job_id: uuid::Uuid,
    _linkage_id: i32,
    channel_id: i32,
    topic: String,
) {
    let result = tokio::time::timeout(
        std::time::Duration::from_secs(JOB_TIMEOUT_SECS),
        execute_job_inner(state.clone(), job_id, channel_id, &topic),
    )
    .await;
    match result {
        Ok(Ok(())) => {}
        Ok(Err(e)) => {
            let _ = sqlx::query(
                "UPDATE education_post_jobs SET status='failed', error_message=$1, updated_at=NOW() WHERE id=$2",
            )
            .bind(&e)
            .bind(job_id)
            .execute(&state.db_pool)
            .await;
            tracing::error!(job = %job_id, "🎓 education post failed: {e}");
        }
        Err(_) => {
            let msg = format!("education post timed out after {JOB_TIMEOUT_SECS}s");
            let _ = sqlx::query(
                "UPDATE education_post_jobs SET status='failed', error_message=$1, updated_at=NOW() WHERE id=$2",
            )
            .bind(&msg)
            .bind(job_id)
            .execute(&state.db_pool)
            .await;
            tracing::error!(job = %job_id, "{msg}");
        }
    }
}

async fn execute_job_inner(
    state: Arc<AppState>,
    job_id: uuid::Uuid,
    channel_id: i32,
    topic: &str,
) -> Result<(), String> {
    // ── 1. Agent writes the brief: fresh subtopic, no repeats. ──
    let posted: Vec<String> = sqlx::query_scalar(
        "SELECT topic FROM education_posted_topics WHERE connected_channel_id = $1 ORDER BY posted_at DESC LIMIT 40",
    )
    .bind(channel_id)
    .fetch_all(&state.db_pool)
    .await
    .map_err(|e| e.to_string())?;

    let avoid = if posted.is_empty() {
        "(none yet — first video on this channel)".to_string()
    } else {
        posted.join("; ")
    };
    let brief_prompt = format!(
        "You are planning the next video for an educational YouTube channel whose topic is \"{topic}\".\n\
         Topics already covered (pick something NOT in this list): {avoid}\n\n\
         Write the video brief: pick ONE fresh subtopic and describe in 2-4 sentences what the \
         lesson teaches. Facts must be real and well-established. Reply with ONLY the brief text.",
        topic = topic,
        avoid = avoid,
    );
    let brief = crate::llm_utils::generate_text_fast_with_nim(
        None,
        None,
        None,
        education_nim().as_ref(),
        &brief_prompt,
    )
    .await
    .map_err(|e| format!("brief generation failed: {e}"))?
    .trim()
    .to_string();
    let _ = sqlx::query("UPDATE education_post_jobs SET brief=$1, updated_at=NOW() WHERE id=$2")
        .bind(&brief)
        .bind(job_id)
        .execute(&state.db_pool)
        .await;

    // ── 2. Channel + user (for the delivery row + upload token). ──
    let (uid, channel_yt_id, channel_name): (i32, String, String) = sqlx::query_as(
        "SELECT user_id, channel_id, channel_name FROM connected_youtube_channels WHERE id = $1",
    )
    .bind(channel_id)
    .fetch_one(&state.db_pool)
    .await
    .map_err(|e| format!("channel lookup failed: {e}"))?;

    // ── 3. Education generation via the hardened pipeline. ──
    let title = format!("{topic} — {}", brief.chars().take(60).collect::<String>());
    let prompt = format!(
        "Generate an animated educational lesson video explaining the subtopic chosen in this brief. \
         Topic area: {topic}. Target ~90s — can be longer if the topic requires it. Brief: {brief}"
    );
    let delivery_id: Uuid = sqlx::query_scalar(
        "INSERT INTO deliveries
         (client_ref, title, gig_type, prompt, style, duration, extra_args, status, user_id)
         VALUES ($1, $2, 'education', $3, 'educational', $4, $5, 'pending', $6)
         RETURNING id",
    )
    .bind(format!("education_post:{job_id}"))
    .bind(&title)
    .bind(&prompt)
    .bind(90.0_f64)
    .bind(json!({"education_post": true, "job_id": job_id.to_string(), "topic": topic}))
    .bind(uid)
    .fetch_one(&state.db_pool)
    .await
    .map_err(|e| format!("delivery insert failed: {e}"))?;

    let workflow_id = crate::services::AgenticServicePipeline::start(
        state.clone(),
        "education",
        crate::services::ServiceInput {
            title: title.clone(),
            brief: format!(
                "{prompt}\n\nProduce a clear, curriculum-friendly animated explainer. \
                 Ground every fact in authoritative sources (see the research step)."
            ),
            source_url: None,
            style: "educational".to_string(),
            duration_seconds: 90.0,
            delivery_id,
            prospect_id: None,
            session_uuid: None,
            user_id: Some(uid),
            source_table: Some("deliveries".to_string()),
            source_record_id: Some(delivery_id),
            idempotency_key: Some(format!("education-post:{job_id}")),
            reference_images: vec![],
            nim_profile: Some("education".to_string()),
        },
    )
    .await
    .map_err(|e| format!("pipeline enqueue failed: {e}"))?;

    let _ =
        sqlx::query("UPDATE education_post_jobs SET workflow_id=$1, updated_at=NOW() WHERE id=$2")
            .bind(workflow_id)
            .bind(job_id)
            .execute(&state.db_pool)
            .await;

    // ── 4. Wait for the pipeline to finish. ──
    let deadline = tokio::time::Instant::now() + std::time::Duration::from_secs(JOB_TIMEOUT_SECS);
    let (status, output_url): (String, Option<String>) = loop {
        tokio::time::sleep(std::time::Duration::from_secs(PIPELINE_POLL_SECS)).await;
        if tokio::time::Instant::now() > deadline {
            return Err("pipeline did not finish in time".into());
        }
        let row = sqlx::query_as::<_, (Option<String>, Option<String>)>(
            "SELECT d.status, d.output_r2_url FROM deliveries d WHERE d.id = $1",
        )
        .bind(delivery_id)
        .fetch_one(&state.db_pool)
        .await
        .map_err(|e| format!("delivery poll failed: {e}"))?;
        let st = row.0.clone().unwrap_or_default();
        if st.eq_ignore_ascii_case("completed") {
            break (st, row.1);
        }
        if st.eq_ignore_ascii_case("failed") {
            return Err(format!(
                "education generation failed: {}",
                sqlx::query_scalar::<_, Option<String>>(
                    "SELECT error_message FROM deliveries WHERE id = $1"
                )
                .bind(delivery_id)
                .fetch_one(&state.db_pool)
                .await
                .ok()
                .flatten()
                .unwrap_or_else(|| "unknown".into())
            ));
        }
    };
    let output_url = output_url.ok_or("generation completed but no output URL")?;
    let _ = sqlx::query(
        "UPDATE education_post_jobs SET status='uploading', output_url=$1, progress_percent=80, updated_at=NOW() WHERE id=$2",
    )
    .bind(&output_url)
    .bind(job_id)
    .execute(&state.db_pool)
    .await;

    // ── 5. Download + upload to the connected channel. ──
    let video_path = crate::services::av_sync_qa::resolve_video_for_craft(&output_url)
        .await
        .map_err(|e| format!("output download failed: {e}"))?;

    let access_token = fresh_channel_token(&state, channel_id).await?;
    let yt_title = format!("{} | {}", topic, title);
    let yt_title = yt_title.chars().take(95).collect::<String>();
    let description = format!(
        "{brief}\n\nAn educational lesson from the VideoSync Studio education network. Topic: {topic}."
    );

    let yt = state
        .youtube_client
        .as_ref()
        .ok_or("YouTube client not configured")?;
    let upload = yt
        .upload_video(
            &access_token,
            video_path.to_str().unwrap_or(""),
            &yt_title,
            &description,
            "public",
            Some("27"), // Education category
            Some(vec![topic.to_string(), "education".to_string()]),
        )
        .await
        .map_err(|e| format!("YouTube upload failed: {e}"))?;
    let yt_video_id = upload.id.clone();

    let _ = sqlx::query(
        "UPDATE education_post_jobs SET status='completed', youtube_video_id=$1, \
         youtube_url=$2, youtube_title=$3, progress_percent=100, completed_at=NOW(), updated_at=NOW() WHERE id=$4",
    )
    .bind(&yt_video_id)
    .bind(format!("https://youtube.com/watch?v={yt_video_id}"))
    .bind(&yt_title)
    .bind(job_id)
    .execute(&state.db_pool)
    .await;

    // ── 6. Anti-repeat ledger. ──
    let _ = sqlx::query(
        "INSERT INTO education_posted_topics (connected_channel_id, topic, youtube_video_id) VALUES ($1, $2, $3)",
    )
    .bind(channel_id)
    .bind(topic)
    .bind(&yt_video_id)
    .execute(&state.db_pool)
    .await;

    tracing::info!(job = %job_id, channel = %channel_yt_id, %channel_name, video = %yt_video_id, "🎓 education video posted");
    Ok(())
}

fn education_nim() -> Option<crate::nvidia_nim_client::NvidiaNimClient> {
    std::env::var("NVIDIA_API_KEY_EDUCATION")
        .ok()
        .filter(|k| !k.trim().is_empty())
        .map(crate::nvidia_nim_client::NvidiaNimClient::new)
}

/// Channel OAuth token, refreshed when expired (mirrors the handler logic).
async fn fresh_channel_token(state: &Arc<AppState>, channel_id: i32) -> Result<String, String> {
    use sqlx::Row;
    let row = sqlx::query(
        "SELECT access_token, refresh_token, token_expiry FROM connected_youtube_channels WHERE id = $1",
    )
    .bind(channel_id)
    .fetch_one(&state.db_pool)
    .await
    .map_err(|e| format!("channel token lookup failed: {e}"))?;
    let access_token: String = row.try_get("access_token").map_err(|e| e.to_string())?;
    let refresh_token: String = row.try_get("refresh_token").map_err(|e| e.to_string())?;
    let expiry: chrono::DateTime<chrono::Utc> =
        row.try_get("token_expiry").map_err(|e| e.to_string())?;

    if chrono::Utc::now() + chrono::Duration::minutes(5) < expiry {
        return Ok(access_token);
    }

    let client_id =
        std::env::var("GOOGLE_CLIENT_ID").map_err(|_| "GOOGLE_CLIENT_ID unset".to_string())?;
    let client_secret = std::env::var("GOOGLE_CLIENT_SECRET")
        .map_err(|_| "GOOGLE_CLIENT_SECRET unset".to_string())?;
    let yt = state
        .youtube_client
        .as_ref()
        .ok_or("YouTube client not configured")?;
    let refreshed = yt
        .refresh_access_token(&refresh_token, &client_id, &client_secret)
        .await
        .map_err(|e| format!("token refresh failed: {e}"))?;
    let new_expiry = chrono::Utc::now() + chrono::Duration::seconds(refreshed.expires_in as i64);
    let _ = sqlx::query(
        "UPDATE connected_youtube_channels SET access_token=$1, token_expiry=$2, updated_at=NOW() WHERE id=$3",
    )
    .bind(&refreshed.access_token)
    .bind(new_expiry)
    .bind(channel_id)
    .execute(&state.db_pool)
    .await;
    Ok(refreshed.access_token)
}
