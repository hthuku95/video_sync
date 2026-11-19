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
/// HEARTBEAT DESIGN (owner directive Nov 14 2026): runs are NEVER killed for
/// taking long. Liveness is detected via heartbeats:
///   • the agentic workflow heartbeats every ~20s (pipeline_worker lease) and
///     its supervisor reclaims crashed runs from checkpoints automatically;
///   • the JOB heartbeats in its wait loop (updated_at touch) so a dead
///     watcher is detectable and re-adopted on worker restart.
/// Bounded timeouts exist ONLY around short finite steps (brief LLM call,
/// download, upload). The only wall-clock backstop is for a run whose
/// heartbeat has been stale for hours — which the pipeline supervisor is
/// supposed to reap first (AGENTIC_MAX_RUN_HOURS).
const BRIEF_TIMEOUT_SECS: u64 = 120;
const UPLOAD_TIMEOUT_SECS: u64 = 20 * 60;
/// Workflow heartbeat considered stale after this (supervisor reclaims first).
const WF_STALE_SECS: i64 = 30 * 60;
/// A non-terminal workflow stale this long fails the job (last-resort reap).
const WF_REAP_SECS: i64 = 6 * 60 * 60;
/// A 'generating' job with a job-heartbeat stale this long is re-adopted.
const JOB_ADOPT_STALE_SECS: i64 = 10 * 60;
/// Poll interval while waiting for the education pipeline to finish.
const PIPELINE_POLL_SECS: u64 = 30;
/// Max concurrent generations (droplet CPU protection — owner directive).
const MAX_CONCURRENT_GENERATIONS: usize = 2;
/// Max jobs created per scheduler cycle.
const MAX_JOBS_PER_CYCLE: usize = 4;

pub async fn run_education_worker_loop(state: Arc<AppState>) {
    // Spawn the background-lane claimers — this module OWNS the background
    // lane. The claimers are spawned here (not by main.rs) so the lane
    // selection is determined by the code that owns the work, not by env vars.
    crate::services::pipeline_worker::start_pipeline_infrastructure(state.clone(), "background");

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
    // DAILY UPLOAD BUDGET (owner design Nov 15-16 2026): the YouTube Data API
    // quota caps uploads project-wide (default 10,000 units/day; each upload
    // costs 1,600 → ~6 uploads/day until a quota increase is approved). The
    // scheduler posts to only a ROTATING SUBSET of channels each day, chosen
    // oldest-posted-first, so the distribution stays FAIR across all channels:
    // a channel skipped today is first in line tomorrow. Effective per-channel
    // rate = budget ÷ active channels.
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

    // Quota budget: max uploads in the rolling 24h window (all statuses).
    let budget: i64 = sqlx::query_scalar::<_, String>(
        "SELECT setting_value FROM system_settings WHERE setting_key = 'education_daily_upload_budget'",
    )
    .fetch_optional(&state.db_pool)
    .await
    .ok()
    .flatten()
    .and_then(|v| v.parse::<i64>().ok())
    .unwrap_or(6);

    let uploads_last_24h: i64 = sqlx::query_scalar(
        "SELECT COUNT(*) FROM education_post_jobs WHERE created_at > NOW() - INTERVAL '24 hours'",
    )
    .fetch_one(&state.db_pool)
    .await
    .unwrap_or(0);
    let remaining_today = (budget - uploads_last_24h).max(0);

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
        if remaining_today == 0 {
            tracing::info!(
                "🎓 daily upload budget exhausted ({budget}/24h) — channels rotate fairly, resume next window"
            );
            break;
        }
        if created >= MAX_JOBS_PER_CYCLE.min(remaining_today as usize) {
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
    // ── PROCESSING BUDGET ───────────────────────────────────────────────────
    // `education_daily_upload_budget` gated job *creation*, but a pre-existing
    // backlog (47 queued jobs, Nov 2026) was still drained in one pass, burning
    // NIM quota and B2 egress. Gate *starts* on the same rolling-24h budget so
    // the owner's "1 education video per day" is enforced end to end.
    let budget: i64 = sqlx::query_scalar::<_, String>(
        "SELECT setting_value FROM system_settings WHERE setting_key = 'education_daily_upload_budget'",
    )
    .fetch_optional(&state.db_pool)
    .await
    .ok()
    .flatten()
    .and_then(|v| v.parse::<i64>().ok())
    .unwrap_or(1)
    .max(0);

    let started_last_24h: i64 = sqlx::query_scalar(
        "SELECT COUNT(*) FROM education_post_jobs WHERE started_at > NOW() - INTERVAL '24 hours'",
    )
    .fetch_one(&state.db_pool)
    .await
    .unwrap_or(0);

    let mut remaining = (budget - started_last_24h).max(0) as usize;
    if remaining == 0 {
        return Ok(());
    }

    // ── RE-ADOPTION (checkpoint of the JOB state): 'generating' jobs whose
    // job-heartbeat is stale belong to a dead watcher — re-enter them with
    // resume=true (skip generation; wait on the existing workflow + upload).
    let stale: Vec<(uuid::Uuid, i32, i32, String)> = sqlx::query_as(
        r#"
        SELECT id, linkage_id, connected_channel_id, topic FROM education_post_jobs
        WHERE status = 'generating'
          AND updated_at < NOW() - make_interval(secs => $1)
        LIMIT $2
        "#,
    )
    .bind(JOB_ADOPT_STALE_SECS as f64)
    .bind(MAX_CONCURRENT_GENERATIONS as i64)
    .fetch_all(&state.db_pool)
    .await
    .map_err(|e| e.to_string())?;
    for (job_id, linkage_id, channel_id, topic) in stale.into_iter().take(remaining) {
        tracing::warn!(job = %job_id, "🔄 re-adopting education job (watcher heartbeat stale)");
        execute_job(state.clone(), job_id, linkage_id, channel_id, topic).await;
        remaining -= 1;
        if remaining == 0 {
            return Ok(());
        }
    }

    while remaining > 0 {
        let job = claim_next_job(state).await?;
        let Some((job_id, linkage_id, channel_id, topic)) = job else {
            break;
        };
        execute_job(state.clone(), job_id, linkage_id, channel_id, topic).await;
        remaining -= 1;
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
    match execute_job_inner(state.clone(), job_id, channel_id, &topic, false).await {
        Ok(()) => {}
        Err(e) => {
            let _ = sqlx::query(
                "UPDATE education_post_jobs SET status='failed', error_message=$1, updated_at=NOW() WHERE id=$2",
            )
            .bind(&e)
            .bind(job_id)
            .execute(&state.db_pool)
            .await;
            tracing::error!(job = %job_id, "🎓 education post failed: {e}");
        }
    }
}

async fn execute_job_inner(
    state: Arc<AppState>,
    job_id: uuid::Uuid,
    channel_id: i32,
    topic: &str,
    resume: bool,
) -> Result<(), String> {
    // RESUME (checkpoint of the JOB state): a job re-entered after a watcher
    // death skips straight to the wait+upload phase — the generation workflow
    // keeps running independently on the durable queue (heartbeat + lease +
    // supervisor reclaim + checkpoint resume at the pipeline level).
    let existing_workflow_id: Option<Uuid> = if resume {
        sqlx::query_scalar("SELECT workflow_id FROM education_post_jobs WHERE id = $1")
            .bind(job_id)
            .fetch_one(&state.db_pool)
            .await
            .map_err(|e| format!("resume lookup failed: {e}"))?
    } else {
        None
    };

    // ── 1. Agent writes the brief: fresh subtopic, no repeats. ──
    let posted: Vec<String> = if resume && existing_workflow_id.is_some() {
        Vec::new() // brief already written in the previous attempt
    } else {
        sqlx::query_scalar(
                "SELECT topic FROM education_posted_topics WHERE connected_channel_id = $1 ORDER BY posted_at DESC LIMIT 40",
            )
            .bind(channel_id)
            .fetch_all(&state.db_pool)
            .await
            .map_err(|e| e.to_string())?
    };

    let avoid = if posted.is_empty() {
        "(none yet — first video on this channel)".to_string()
    } else {
        posted.join("; ")
    };
    let brief = if resume && existing_workflow_id.is_some() {
        // resumed run: brief was persisted in the previous attempt
        sqlx::query_scalar::<_, Option<String>>(
            "SELECT brief FROM education_post_jobs WHERE id = $1",
        )
        .bind(job_id)
        .fetch_one(&state.db_pool)
        .await
        .map_err(|e| format!("resume brief fetch failed: {e}"))?
        .unwrap_or_else(|| format!("An educational lesson about {topic}."))
    } else {
        let brief_prompt = format!(
            "You are planning the next video for an educational YouTube channel whose topic is \"{topic}\".\n\
             Topics already covered (pick something NOT in this list): {avoid}\n\n\
             Write the video brief: pick ONE fresh subtopic and describe in 2-4 sentences what the \
             lesson teaches. Facts must be real and well-established. Reply with ONLY the brief text.",
            topic = topic,
            avoid = avoid,
        );
        tokio::time::timeout(
            std::time::Duration::from_secs(BRIEF_TIMEOUT_SECS),
            crate::llm_utils::generate_text_fast_with_nim(
                None,
                None,
                None,
                education_nim().as_ref(),
                &brief_prompt,
            ),
        )
        .await
        .map_err(|_| format!("brief generation timed out ({BRIEF_TIMEOUT_SECS}s)"))?
        .map_err(|e| format!("brief generation failed: {e}"))?
        .trim()
        .to_string()
    };
    if !resume {
        let _ =
            sqlx::query("UPDATE education_post_jobs SET brief=$1, updated_at=NOW() WHERE id=$2")
                .bind(&brief)
                .bind(job_id)
                .execute(&state.db_pool)
                .await;
    }

    // ── 2. Channel + user (for the delivery row + upload token). ──
    let (uid, channel_yt_id, channel_name): (i32, String, String) = sqlx::query_as(
        "SELECT user_id, channel_id, channel_name FROM connected_youtube_channels WHERE id = $1",
    )
    .bind(channel_id)
    .fetch_one(&state.db_pool)
    .await
    .map_err(|e| format!("channel lookup failed: {e}"))?;

    // ── 3. Education generation via the hardened pipeline. ──
    // (creation skipped on resume — the workflow already lives on the
    // durable queue; the pipeline worker drives it with heartbeat + lease +
    // checkpoint resume)
    let (delivery_id, workflow_id, title) = if resume {
        if let Some(wid) = existing_workflow_id {
            // job's delivery (and its persisted title) is recoverable from the
            // linked workflow
            let (did, title): (Uuid, String) = sqlx::query_as(
                "SELECT w.source_record_id, d.title FROM app_workflows w \
                 JOIN deliveries d ON d.id = w.source_record_id WHERE w.id = $1",
            )
            .bind(wid)
            .fetch_one(&state.db_pool)
            .await
            .map_err(|e| format!("resume delivery lookup failed: {e}"))?;
            (did, wid, title)
        } else {
            return Err("resume requested but the job has no workflow".into());
        }
    } else {
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
                lane: "background".to_string(),
            },
        )
        .await
        .map_err(|e| format!("pipeline enqueue failed: {e}"))?;

        let _ = sqlx::query(
            "UPDATE education_post_jobs SET workflow_id=$1, updated_at=NOW() WHERE id=$2",
        )
        .bind(workflow_id)
        .bind(job_id)
        .execute(&state.db_pool)
        .await;
        (delivery_id, workflow_id, title)
    };

    // ── 4. Wait for the pipeline to finish — HEARTBEAT LIVENESS, no wall clock. ──
    // The workflow heartbeats (lease) and the pipeline supervisor reclaims
    // crashed runs from checkpoints; this loop only reacts to terminal states
    // and reaps a run whose heartbeat has been dead for hours (last resort —
    // the supervisor's AGENTIC_MAX_RUN_HOURS should reap it first).
    let (status, output_url): (String, Option<String>) = loop {
        tokio::time::sleep(std::time::Duration::from_secs(PIPELINE_POLL_SECS)).await;

        // JOB heartbeat: proof this watcher is alive (drives re-adoption).
        let _ = sqlx::query("UPDATE education_post_jobs SET updated_at=NOW() WHERE id=$1")
            .bind(job_id)
            .execute(&state.db_pool)
            .await;

        let row = sqlx::query_as::<
            _,
            (
                Option<String>,
                Option<String>,
                Option<chrono::DateTime<chrono::Utc>>,
            ),
        >(
            "SELECT d.status, d.output_r2_url, w.updated_at
             FROM deliveries d
             LEFT JOIN app_workflows w ON w.id = d.workflow_id
             WHERE d.id = $1",
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

        // Liveness: reap a non-terminal run whose heartbeat is hours dead.
        if let Some(beat) = row.2 {
            let stale_secs = (chrono::Utc::now() - beat).num_seconds();
            if stale_secs > WF_REAP_SECS {
                let _ = sqlx::query(
                    "UPDATE app_workflows SET status='cancelled', current_step='heartbeat_dead_reaped', updated_at=NOW() WHERE id=$1 AND status IN ('queued','running')",
                )
                .bind(workflow_id)
                .execute(&state.db_pool)
                .await;
                return Err(format!(
                    "workflow heartbeat dead for {:.1}h — reaped (the pipeline supervisor should have reclaimed it first)",
                    stale_secs as f64 / 3600.0
                ));
            }
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
    let video_path = tokio::time::timeout(
        std::time::Duration::from_secs(10 * 60),
        crate::services::av_sync_qa::resolve_video_for_craft(&output_url),
    )
    .await
    .map_err(|_| "output download timed out (10 min)".to_string())?
    .map_err(|e| format!("output download failed: {e}"))?;

    // UPLOAD-BOUNDARY BACKSTOP (Nov 18 2026): never send an audio-only file
    // to YouTube. 20 education renders were published as narration-only
    // audio through publish_output (ext-gated validation hole). Defense in
    // depth: verify the resolved artifact actually has a video stream.
    {
        let meta = crate::core::analyze_video(video_path.to_str().unwrap_or("")).ok();
        if let Some(m) = &meta {
            if !m.has_video {
                let _ = tokio::fs::remove_file(&video_path).await;
                return Err(format!(
                    "REFUSING YouTube upload: resolved deliverable has NO video stream (duration {:.1}s). The generation pipeline published audio-only — regeneration required.",
                    m.duration_seconds
                ));
            }
        }
    }

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
    let upload = tokio::time::timeout(
        std::time::Duration::from_secs(UPLOAD_TIMEOUT_SECS),
        yt.upload_video(
            &access_token,
            video_path.to_str().unwrap_or(""),
            &yt_title,
            &description,
            "public",
            Some("27"), // Education category
            Some(vec![topic.to_string(), "education".to_string()]),
        ),
    )
    .await
    .map_err(|_| "YouTube upload timed out".to_string())?
    .map_err(|e| format!("YouTube upload failed: {e}"))?;
    let yt_video_id = upload.id.clone();

    // CLEANUP (owner directive Nov 15 2026): delete the downloaded video
    // bytes immediately — success or failure — so completed uploads never
    // bloat the worker's disk.
    let _ = tokio::fs::remove_file(&video_path).await;

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
