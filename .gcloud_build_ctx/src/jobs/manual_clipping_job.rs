// Manual clipping job — same pipeline as auto-clipping but:
//   - No linkage_id required (user_id only)
//   - Skips Phase E (no YouTube upload)
//   - Uploads clips to R2 and returns presigned download URLs

use crate::clipping::{
    ai_clipper::AiClipper, apify_client::ApifyClient, gemini_video_analyzer::VideoAnalysis,
};
use crate::AppState;
use sqlx::Row;
use std::sync::Arc;
use uuid::Uuid;

/// Update manual job status in DB.
async fn update_status(
    job_id: Uuid,
    status: &str,
    progress: i32,
    error: Option<&str>,
    pool: &sqlx::PgPool,
) -> Result<(), String> {
    sqlx::query(
        "UPDATE manual_clipping_jobs
         SET status = $1, progress_percent = $2, error_message = $3, updated_at = NOW()
         WHERE id = $4",
    )
    .bind(status)
    .bind(progress)
    .bind(error)
    .bind(job_id)
    .execute(pool)
    .await
    .map_err(|e| format!("Failed to update job status: {}", e))?;

    if let Some(workflow_id) = workflow_id_for_manual_job(job_id, pool).await? {
        let workflow_runtime = crate::services::WorkflowRuntime::new(pool.clone());
        let step_message = error.unwrap_or(status);
        match status {
            "cancelled" => {
                workflow_runtime
                    .mark_cancelled(workflow_id, Some(status), step_message)
                    .await?;
            }
            "failed" => {
                workflow_runtime
                    .mark_failed(workflow_id, Some(status), step_message, None)
                    .await?;
            }
            "completed" => {
                workflow_runtime
                    .heartbeat(
                        workflow_id,
                        crate::services::WorkflowStatus::Running,
                        Some(status),
                        step_message,
                        serde_json::json!({
                            "job_id": job_id,
                            "progress_percent": progress,
                        }),
                    )
                    .await?;
            }
            _ => {
                let workflow_status = if status == "pending" {
                    crate::services::WorkflowStatus::Queued
                } else {
                    crate::services::WorkflowStatus::Running
                };
                workflow_runtime
                    .heartbeat(
                        workflow_id,
                        workflow_status,
                        Some(status),
                        step_message,
                        serde_json::json!({
                            "job_id": job_id,
                            "progress_percent": progress,
                        }),
                    )
                    .await?;
            }
        }
    }
    Ok(())
}

async fn workflow_id_for_manual_job(
    job_id: Uuid,
    pool: &sqlx::PgPool,
) -> Result<Option<Uuid>, String> {
    sqlx::query_scalar::<_, Option<Uuid>>(
        "SELECT workflow_id FROM manual_clipping_jobs WHERE id = $1",
    )
    .bind(job_id)
    .fetch_optional(pool)
    .await
    .map(|row| row.flatten())
    .map_err(|e| format!("Failed to fetch manual clipping workflow id: {}", e))
}

async fn mark_manual_job_completed(
    job_id: Uuid,
    clip_count: i32,
    pool: &sqlx::PgPool,
) -> Result<(), String> {
    sqlx::query(
        "UPDATE manual_clipping_jobs
         SET status = 'completed', progress_percent = 100, clips_count = $1,
             completed_at = NOW(), updated_at = NOW()
         WHERE id = $2",
    )
    .bind(clip_count)
    .bind(job_id)
    .execute(pool)
    .await
    .map_err(|e| format!("Failed to mark manual clipping job completed: {}", e))?;

    let verified_clip_count = sqlx::query_scalar::<_, i64>(
        "SELECT COUNT(*) FROM manual_clipping_clips WHERE job_id = $1",
    )
    .bind(job_id)
    .fetch_one(pool)
    .await
    .map_err(|e| format!("Failed to verify manual clipping artifacts: {}", e))?;

    if verified_clip_count == 0 {
        if let Some(workflow_id) = workflow_id_for_manual_job(job_id, pool).await? {
            let workflow_runtime = crate::services::WorkflowRuntime::new(pool.clone());
            let _ = workflow_runtime
                .mark_failed(
                    workflow_id,
                    Some("artifact_verification"),
                    "Manual clipping workflow completed without any persisted clip records.",
                    None,
                )
                .await;
        }
        return Err("Manual clipping workflow completed without any persisted clip records.".to_string());
    }

    if let Some(workflow_id) = workflow_id_for_manual_job(job_id, pool).await? {
        let workflow_runtime = crate::services::WorkflowRuntime::new(pool.clone());
        workflow_runtime
            .mark_completed(
                workflow_id,
                Some("completed"),
                "Manual clipping workflow completed with persisted clip artifacts.",
                serde_json::json!({
                    "verified_clip_count": verified_clip_count,
                    "reported_clip_count": clip_count,
                }),
            )
            .await?;
    }

    Ok(())
}

/// Main executor for a manual clipping job.
pub async fn execute_manual_clipping_job(
    job_id: Uuid,
    app_state: Arc<AppState>,
) -> Result<String, String> {
    tracing::info!("🎬 Manual clipping job {} started", job_id);

    // Fetch job row
    let row = sqlx::query(
        "SELECT user_id, video_url, video_platform, clips_requested,
                min_clip_duration_seconds, max_clip_duration_seconds
         FROM manual_clipping_jobs WHERE id = $1",
    )
    .bind(job_id)
    .fetch_one(&app_state.db_pool)
    .await
    .map_err(|e| format!("Job not found: {}", e))?;

    let user_id: i32 = row.get("user_id");
    let video_url: String = row.get("video_url");
    let video_platform: String = row.get("video_platform");
    let clips_requested: i32 = row.get("clips_requested");
    let min_dur: i32 = row.get("min_clip_duration_seconds");
    let max_dur: i32 = row.get("max_clip_duration_seconds");

    // =========================================================================
    // Phase A: Gemini video analysis
    // =========================================================================
    update_status(job_id, "analyzing", 10, None, &app_state.db_pool).await?;
    tracing::info!("🔍 Phase A: Analyzing {}", video_url);

    // Prefer the dedicated manual-clipping key to avoid exhausting the shared quota.
    let gemini = app_state
        .manual_clipping_gemini_client
        .as_ref()
        .or(app_state.gemini_client.as_ref())
        .ok_or("Gemini client not configured")?;

    let analysis: VideoAnalysis = if video_platform == "youtube" {
        // YouTube: Gemini can analyze the URL directly
        let gemini_result = tokio::time::timeout(
            tokio::time::Duration::from_secs(240),
            gemini.analyze_video_from_url(
                &video_url,
                clips_requested as usize,
                min_dur as f64,
                max_dur as f64,
                &[],
            ),
        )
        .await
        .map_err(|_| "Gemini analysis timed out (240s)".to_string())?;

        match gemini_result {
            Ok(a) => a,
            Err(e)
                if e.to_string().contains("429")
                    || e.to_string().to_lowercase().contains("quota") =>
            {
                tracing::warn!("⚠️ Gemini 429 on manual clipping (YouTube URL), trying video_gemini_client: {}", e);
                // Tier 2: video_gemini_client (VIDEO_GEMINI_API_KEY — separate quota pool)
                let video_gemini_result = if let Some(vg) = app_state.video_gemini_client.as_ref() {
                    tokio::time::timeout(
                        tokio::time::Duration::from_secs(240),
                        vg.analyze_video_from_url(
                            &video_url,
                            clips_requested as usize,
                            min_dur as f64,
                            max_dur as f64,
                            &[],
                        ),
                    )
                    .await
                    .ok()
                    .and_then(|r| r.ok())
                } else {
                    None
                };
                if let Some(a) = video_gemini_result {
                    tracing::info!(
                        "✅ video_gemini_client fallback succeeded for YouTube analysis"
                    );
                    a
                } else {
                    // Tier 3: BlenderMCPServer (BLENDER_GEMINI_API_KEY — fully isolated quota)
                    tracing::warn!("⚠️ video_gemini_client also failed or unavailable, falling back to BlenderMCPServer");
                    if let Some(blender) = app_state.blender_mcp_client.as_ref() {
                        blender.analyze_video(&video_url, clips_requested as u32, min_dur as f64, max_dur as f64, &[])
                            .await
                            .map_err(|be| format!("All Gemini tiers exhausted (429); BlenderMCP fallback also failed: {}", be))?
                    } else {
                        return Err(format!(
                            "Gemini analysis failed (429 quota): {}. No fallback configured.",
                            e
                        ));
                    }
                }
            }
            Err(e) => return Err(format!("Gemini analysis failed: {}", e)),
        }
    } else {
        // Twitch: must download first, then analyze from local file
        update_status(job_id, "downloading", 15, None, &app_state.db_pool).await?;
        tracing::info!("⬇️  Downloading Twitch VOD before analysis");

        let dl_path = format!("downloads/manual_{}.mp4", job_id);
        tokio::fs::create_dir_all("downloads")
            .await
            .map_err(|e| format!("Failed to create downloads dir: {}", e))?;

        // Use empty strings as fallback so Strategies 1/3/4/5 still run without Apify creds
        let apify_token = std::env::var("APIFY_TOKEN").unwrap_or_default();
        let apify_actor = std::env::var("APIFY_YOUTUBE_CLIENT_ACTOR").unwrap_or_default();
        let client = ApifyClient::new(apify_token, apify_actor);

        client
            .download_video(&video_url, &dl_path)
            .await
            .map_err(|e| format!("All download strategies failed (Twitch VOD): {}", e))?;

        let local_analysis_result = tokio::time::timeout(
            tokio::time::Duration::from_secs(300),
            gemini.analyze_video_from_local_file(
                &dl_path,
                clips_requested as usize,
                min_dur as f64,
                max_dur as f64,
                &[],
            ),
        )
        .await
        .map_err(|_| "Gemini local analysis timed out (300s)".to_string())?;

        match local_analysis_result {
            Ok(a) => a,
            Err(e)
                if e.to_string().contains("429")
                    || e.to_string().to_lowercase().contains("quota") =>
            {
                tracing::warn!(
                    "⚠️ Gemini 429 on Twitch local analysis, trying video_gemini_client: {}",
                    e
                );
                // Tier 2: video_gemini_client (VIDEO_GEMINI_API_KEY — separate quota pool)
                let video_gemini_result = if let Some(vg) = app_state.video_gemini_client.as_ref() {
                    tokio::time::timeout(
                        tokio::time::Duration::from_secs(300),
                        vg.analyze_video_from_local_file(
                            &dl_path,
                            clips_requested as usize,
                            min_dur as f64,
                            max_dur as f64,
                            &[],
                        ),
                    )
                    .await
                    .ok()
                    .and_then(|r| r.ok())
                } else {
                    None
                };
                if let Some(a) = video_gemini_result {
                    tracing::info!(
                        "✅ video_gemini_client fallback succeeded for Twitch local analysis"
                    );
                    a
                } else {
                    // Tier 3: BlenderMCPServer via R2 presigned URL (BLENDER_GEMINI_API_KEY — fully isolated quota)
                    tracing::warn!("⚠️ video_gemini_client also failed or unavailable, uploading to R2 for BlenderMCP fallback");
                    if let (Some(blender), Some(r2)) = (
                        app_state.blender_mcp_client.as_ref(),
                        app_state.r2_client.as_ref(),
                    ) {
                        let r2_key = format!("temp/manual_clipping/{}.mp4", job_id);
                        r2.upload(&dl_path, &r2_key).await.map_err(|re| {
                            format!("R2 upload for BlenderMCP fallback failed: {}", re)
                        })?;
                        let presigned_url = r2.presign_get(&r2_key, 3600).await.map_err(|re| {
                            format!("R2 presign for BlenderMCP fallback failed: {}", re)
                        })?;
                        tracing::info!(
                            "📤 Uploaded Twitch VOD to R2 for BlenderMCP analysis: {}",
                            r2_key
                        );
                        blender.analyze_video(
                        &presigned_url,
                        clips_requested as u32,
                        min_dur as f64,
                        max_dur as f64,
                        &[],
                    )
                    .await
                    .map_err(|be| format!("Gemini 429 on local Twitch analysis; BlenderMCP fallback also failed: {}", be))?
                    } else {
                        return Err(format!("Gemini local analysis failed (429 quota): {}. No BlenderMCP/R2 fallback available.", e));
                    }
                }
            }
            Err(e) => return Err(format!("Gemini local analysis failed: {}", e)),
        }
    };

    let moments: Vec<crate::clipping::gemini_video_analyzer::ViralMoment> = analysis
        .qualified_moments(0.5)
        .into_iter()
        .cloned()
        .collect();
    if moments.is_empty() {
        let msg = "No viral moments found in video";
        update_status(job_id, "failed", 0, Some(msg), &app_state.db_pool).await?;
        return Err(msg.to_string());
    }

    // Persist viral moments JSON
    let moments_json = serde_json::to_value(&analysis).unwrap_or_default();
    sqlx::query(
        "UPDATE manual_clipping_jobs
         SET viral_moments_json = $1, video_title = $2, updated_at = NOW()
         WHERE id = $3",
    )
    .bind(&moments_json)
    .bind(
        analysis
            .video_summary
            .lines()
            .next()
            .unwrap_or("")
            .to_string(),
    )
    .bind(job_id)
    .execute(&app_state.db_pool)
    .await
    .ok();

    // =========================================================================
    // Phase B: Download video (skip if Twitch — already downloaded above)
    // =========================================================================
    let video_path = if video_platform == "youtube" {
        update_status(job_id, "downloading", 30, None, &app_state.db_pool).await?;
        tracing::info!("⬇️  Phase B: Downloading YouTube video");

        let dl_path = format!("downloads/manual_{}.mp4", job_id);
        tokio::fs::create_dir_all("downloads")
            .await
            .map_err(|e| format!("Failed to create downloads dir: {}", e))?;

        // Use empty strings as fallback so Strategies 1/3/4/5 still run without Apify creds
        let apify_token = std::env::var("APIFY_TOKEN").unwrap_or_default();
        let apify_actor = std::env::var("APIFY_YOUTUBE_CLIENT_ACTOR").unwrap_or_default();
        let client = ApifyClient::new(apify_token, apify_actor);

        client
            .download_video(&video_url, &dl_path)
            .await
            .map_err(|e| format!("All download strategies failed (YouTube): {}", e))?;

        if !std::path::Path::new(&dl_path).exists() {
            return Err(format!("Downloaded file not found: {}", dl_path));
        }

        dl_path
    } else {
        // Twitch: already downloaded in Phase A block above
        format!("downloads/manual_{}.mp4", job_id)
    };

    // =========================================================================
    // Phase C: Parallel FFmpeg clip extraction
    // =========================================================================
    update_status(job_id, "extracting", 50, None, &app_state.db_pool).await?;
    tracing::info!("✂️  Phase C: Extracting {} clips", moments.len());

    tokio::fs::create_dir_all("outputs")
        .await
        .map_err(|e| format!("Failed to create outputs dir: {}", e))?;

    // Use a job_id integer for the AiClipper (it expects i32 for file naming)
    // We use a stable hash of the UUID's last 8 hex chars as a pseudo-integer
    let pseudo_job_id: i32 = i32::from_str_radix(&job_id.to_string().replace('-', "")[..6], 16)
        .unwrap_or(999999)
        .abs();

    let clipper = AiClipper::new(app_state.clone());
    let mut clips = clipper
        .extract_clips_from_moments(pseudo_job_id, &video_path, &moments, &analysis.content_type)
        .await
        .map_err(|e| format!("Clip extraction failed: {}", e))?;

    let min_duration = min_dur as f64;
    let max_duration = max_dur as f64;
    let before_duration_filter = clips.len();
    clips.retain(|clip| {
        clip.duration_seconds >= min_duration && clip.duration_seconds <= max_duration
    });

    if clips.len() != before_duration_filter {
        tracing::warn!(
            "Manual clipping job {} dropped {} clip(s) outside configured duration bounds (min={}s, max={}s)",
            job_id,
            before_duration_filter.saturating_sub(clips.len()),
            min_dur,
            max_dur
        );
    }

    if clips.is_empty() {
        let msg = format!(
            "All extracted clips were outside configured duration bounds (min={}s, max={}s)",
            min_dur, max_dur
        );
        update_status(job_id, "failed", 0, Some(&msg), &app_state.db_pool).await?;
        return Err(msg);
    }

    // =========================================================================
    // Phase D: AI Agent enhancement — runs the full 320-tool Gemini agent on
    // each clip so it can intelligently apply stabilization, color correction,
    // audio normalization, etc. before the clips are uploaded.
    // Best-effort: failures are logged but do not abort the job.
    // =========================================================================
    if app_state
        .manual_clipping_gemini_client
        .as_ref()
        .or(app_state.gemini_client.as_ref())
        .is_some()
    {
        update_status(job_id, "enhancing", 65, None, &app_state.db_pool).await?;
        tracing::info!(
            "🤖 Phase D: AI agent enhancing {} clips with 320 tools",
            clips.len()
        );

        crate::jobs::clipping_job::enhance_clips_with_full_agent(
            i32::from_str_radix(&job_id.to_string().replace('-', "")[..6], 16)
                .unwrap_or(999999)
                .abs(),
            user_id,
            &video_url,
            &analysis.content_type,
            &mut clips,
            &app_state,
            "manual_clipping",
        )
        .await;
    } else {
        tracing::info!("ℹ️  Phase D skipped — Gemini not configured");
    }

    // =========================================================================
    // Phase R2: Upload clips + generate presigned download URLs
    // =========================================================================
    update_status(job_id, "uploading", 80, None, &app_state.db_pool).await?;
    tracing::info!("☁️  Phase R2: Uploading {} clips", clips.len());

    let r2 = app_state
        .r2_client
        .as_ref()
        .ok_or("R2 client not configured")?;

    let seven_days_secs = 7 * 24 * 3600u64;

    for (i, clip) in clips.iter().enumerate() {
        let clip_n = i + 1;
        let clip_key = format!("manual/{}/{}/clip_{}.mp4", user_id, job_id, clip_n);
        let thumb_key = format!("manual/{}/{}/thumb_{}.jpg", user_id, job_id, clip_n);

        // Upload clip
        let (clip_url, clip_expires) = match r2.upload(&clip.local_clip_path, &clip_key).await {
            Ok(()) => {
                let url = r2
                    .presign_get(&clip_key, seven_days_secs)
                    .await
                    .unwrap_or_default();
                let expires =
                    chrono::Utc::now() + chrono::Duration::seconds(seven_days_secs as i64);
                (Some(url), Some(expires))
            }
            Err(e) => {
                tracing::warn!("R2 clip upload failed for clip {}: {}", clip_n, e);
                (None, None)
            }
        };

        // Upload thumbnail (best-effort)
        let (thumb_url, thumb_key_stored) = match clip.custom_thumbnail_path.as_deref() {
            Some(tp) if std::path::Path::new(tp).exists() => {
                match r2.upload(tp, &thumb_key).await {
                    Ok(()) => {
                        let url = r2
                            .presign_get(&thumb_key, seven_days_secs)
                            .await
                            .unwrap_or_default();
                        (Some(url), Some(thumb_key.clone()))
                    }
                    Err(e) => {
                        tracing::warn!("R2 thumb upload failed: {}", e);
                        (None, None)
                    }
                }
            }
            _ => (None, None),
        };

        let viral_factors_json = serde_json::to_value(&clip.viral_factors).unwrap_or_default();

        sqlx::query(
            "INSERT INTO manual_clipping_clips
             (job_id, clip_number, title, description, start_time_seconds, end_time_seconds,
              duration_seconds, quality_score, viral_factors, r2_clip_key, r2_clip_url,
              r2_clip_url_expires_at, thumbnail_r2_key, thumbnail_r2_url,
              qa_status, qa_score, qa_feedback, qa_retry_hint)
             VALUES ($1, $2, $3, $4, $5, $6, $7, $8, $9, $10, $11, $12, $13, $14, $15, $16, $17, $18)",
        )
        .bind(job_id)
        .bind(clip_n as i32)
        .bind(&clip.ai_title)
        .bind(&clip.ai_description)
        .bind(clip.start_time_seconds)
        .bind(clip.end_time_seconds)
        .bind(clip.end_time_seconds - clip.start_time_seconds)
        .bind(clip.ai_confidence_score)
        .bind(&viral_factors_json)
        .bind(&clip_key)
        .bind(&clip_url)
        .bind(clip_expires)
        .bind(&thumb_key_stored)
        .bind(&thumb_url)
        .bind(clip.qa_status.as_deref().unwrap_or("not_reviewed"))
        .bind(clip.qa_score)
        .bind(&clip.qa_feedback)
        .bind(&clip.qa_retry_hint)
        .execute(&app_state.db_pool)
        .await
        .map_err(|e| format!("Failed to insert clip row: {}", e))?;

        let artifact = crate::services::media_review::MediaReviewArtifact {
            review_id: format!("manual-clip-{}-{}", job_id, clip_n),
            asset_kind: "manual_clip".to_string(),
            source_type: "manual_clipping".to_string(),
            service_slug: Some("clipper-enhancement-pack".to_string()),
            owner_user_id: Some(user_id),
            output_url: clip_url.clone(),
            source_url: Some(video_url.clone()),
            prompt: Some(format!("{} {}", clip.ai_title, clip.ai_description)),
            title: Some(clip.ai_title.clone()),
            company: None,
            review_status: clip
                .qa_status
                .clone()
                .unwrap_or_else(|| "completed".to_string()),
            qa_score: clip.qa_score.or(Some(clip.ai_confidence_score.round() as i32)),
            qa_feedback: clip.qa_feedback.clone(),
            narration_text: None,
            visual_direction: clip.enhancement_reasoning.clone(),
            transcript_excerpt: Some(clip.ai_description.clone()),
            tags: vec![
                "manual-clipping".to_string(),
                "clip".to_string(),
                video_platform.clone(),
            ],
        };

        if let Err(error) =
            crate::services::media_review::MediaReviewService::store_artifact(
                &app_state,
                artifact,
            )
            .await
        {
            tracing::warn!(
                "Failed to store media review artifact for manual clip {} / {}: {}",
                job_id,
                clip_n,
                error
            );
        }
    }

    // Mark job complete
    mark_manual_job_completed(job_id, clips.len() as i32, &app_state.db_pool).await?;

    // Cleanup local temp files (best-effort)
    let _ = tokio::fs::remove_file(&video_path).await;
    for clip in &clips {
        let _ = tokio::fs::remove_file(&clip.local_clip_path).await;
        if let Some(ref tp) = clip.custom_thumbnail_path {
            let _ = tokio::fs::remove_file(tp).await;
        }
    }

    tracing::info!(
        "✅ Manual clipping job {} completed: {} clips",
        job_id,
        clips.len()
    );
    Ok(format!("{} clips generated", clips.len()))
}
