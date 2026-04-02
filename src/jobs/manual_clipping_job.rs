// Manual clipping job — same pipeline as auto-clipping but:
//   - No linkage_id required (user_id only)
//   - Skips Phase E (no YouTube upload)
//   - Uploads clips to R2 and returns presigned download URLs

use crate::clipping::{
    ai_clipper::AiClipper,
    apify_client::ApifyClient,
    gemini_video_analyzer::VideoAnalysis,
};
use crate::AppState;
use sqlx::Row;
use std::sync::Arc;
use uuid::Uuid;

/// Detect whether a URL is YouTube or Twitch.
fn detect_platform(url: &str) -> &'static str {
    if url.contains("twitch.tv") || url.contains("twitch.com") {
        "twitch"
    } else {
        "youtube"
    }
}

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

    let gemini = app_state
        .gemini_client
        .as_ref()
        .ok_or("Gemini client not configured")?;

    let analysis: VideoAnalysis = if video_platform == "youtube" {
        // YouTube: Gemini can analyze the URL directly
        tokio::time::timeout(
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
        .map_err(|_| "Gemini analysis timed out (240s)".to_string())?
        .map_err(|e| format!("Gemini analysis failed: {}", e))?
    } else {
        // Twitch: must download first, then analyze from local file
        update_status(job_id, "downloading", 15, None, &app_state.db_pool).await?;
        tracing::info!("⬇️  Downloading Twitch VOD before analysis");

        let dl_path = format!("downloads/manual_{}.mp4", job_id);
        tokio::fs::create_dir_all("downloads")
            .await
            .map_err(|e| format!("Failed to create downloads dir: {}", e))?;

        let apify_token = std::env::var("APIFY_TOKEN")
            .map_err(|_| "APIFY_TOKEN not configured")?;
        let apify_actor = std::env::var("APIFY_YOUTUBE_CLIENT_ACTOR")
            .map_err(|_| "APIFY_YOUTUBE_CLIENT_ACTOR not configured")?;
        let client = ApifyClient::new(apify_token, apify_actor);

        client
            .download_video(&video_url, &dl_path)
            .await
            .map_err(|e| format!("Twitch download failed: {}", e))?;

        tokio::time::timeout(
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
        .map_err(|_| "Gemini local analysis timed out (300s)".to_string())?
        .map_err(|e| format!("Gemini local analysis failed: {}", e))?
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
    .bind(analysis.video_summary.lines().next().unwrap_or("").to_string())
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

        let apify_token = std::env::var("APIFY_TOKEN")
            .map_err(|_| "APIFY_TOKEN not configured")?;
        let apify_actor = std::env::var("APIFY_YOUTUBE_CLIENT_ACTOR")
            .map_err(|_| "APIFY_YOUTUBE_CLIENT_ACTOR not configured")?;
        let client = ApifyClient::new(apify_token, apify_actor);

        client
            .download_video(&video_url, &dl_path)
            .await
            .map_err(|e| format!("Download failed: {}", e))?;

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
    let clips = clipper
        .extract_clips_from_moments(pseudo_job_id, &video_path, &moments, &analysis.content_type)
        .await
        .map_err(|e| format!("Clip extraction failed: {}", e))?;

    // =========================================================================
    // Phase D: AI Agent enhancement — runs the full 320-tool Gemini agent on
    // each clip so it can intelligently apply stabilization, color correction,
    // audio normalization, etc. before the clips are uploaded.
    // Best-effort: failures are logged but do not abort the job.
    // =========================================================================
    if let Some(gemini) = app_state.gemini_client.as_ref() {
        update_status(job_id, "enhancing", 65, None, &app_state.db_pool).await?;
        tracing::info!("🤖 Phase D: AI agent enhancing {} clips with 320 tools", clips.len());

        let agent = crate::agent::simple_gemini_agent::SimpleGeminiAgent::new(
            std::sync::Arc::new(gemini.clone()),
        );

        for (i, clip) in clips.iter().enumerate() {
            let clip_path = &clip.local_clip_path;
            if !std::path::Path::new(clip_path).exists() {
                continue;
            }

            let prompt = format!(
                "You are a professional video editor working on a clip for a Fiverr/PPH client.\n\
                 Clip file: {path}\n\
                 Title: {title}\n\
                 Duration: {dur:.0}s | Content type: {ct}\n\
                 Niche: {niche}\n\n\
                 Intelligently enhance this clip for social media / YouTube Shorts delivery:\n\
                 1. Analyze the clip quality (resolution, stability, audio levels, colour)\n\
                 2. Apply appropriate FFmpeg tools: stabilize if shaky, normalize audio, \
                    adjust brightness/contrast if needed, sharpen if soft\n\
                 3. Output the enhanced file back to the SAME path: {path}\n\
                 4. Keep under 90 seconds total. Do not re-encode unnecessarily.\n\
                 Use only tools that will genuinely improve this specific clip.",
                path = clip_path,
                title = clip.ai_title,
                dur = clip.duration_seconds,
                ct = analysis.content_type,
                niche = &video_url,
            );

            let session_id = format!("manual_clip_{}_{}", job_id, i + 1);
            match tokio::time::timeout(
                tokio::time::Duration::from_secs(120),
                agent.execute(&prompt, &session_id, Some(user_id), app_state.clone(), None),
            )
            .await
            {
                Ok(Ok(result)) => tracing::info!(
                    "✅ Clip {} enhanced: {}",
                    i + 1,
                    result.chars().take(100).collect::<String>()
                ),
                Ok(Err(e)) => tracing::warn!("⚠️  Clip {} enhancement failed (non-fatal): {}", i + 1, e),
                Err(_) => tracing::warn!("⚠️  Clip {} enhancement timed out (non-fatal)", i + 1),
            }
        }
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
                let url = r2.presign_get(&clip_key, seven_days_secs).await.unwrap_or_default();
                let expires = chrono::Utc::now() + chrono::Duration::seconds(seven_days_secs as i64);
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
                        let url = r2.presign_get(&thumb_key, seven_days_secs).await.unwrap_or_default();
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
              r2_clip_url_expires_at, thumbnail_r2_key, thumbnail_r2_url)
             VALUES ($1, $2, $3, $4, $5, $6, $7, $8, $9, $10, $11, $12, $13, $14)",
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
        .execute(&app_state.db_pool)
        .await
        .map_err(|e| format!("Failed to insert clip row: {}", e))?;
    }

    // Mark job complete
    sqlx::query(
        "UPDATE manual_clipping_jobs
         SET status = 'completed', progress_percent = 100, clips_count = $1,
             completed_at = NOW(), updated_at = NOW()
         WHERE id = $2",
    )
    .bind(clips.len() as i32)
    .bind(job_id)
    .execute(&app_state.db_pool)
    .await
    .ok();

    // Cleanup local temp files (best-effort)
    let _ = tokio::fs::remove_file(&video_path).await;
    for clip in &clips {
        let _ = tokio::fs::remove_file(&clip.local_clip_path).await;
        if let Some(ref tp) = clip.custom_thumbnail_path {
            let _ = tokio::fs::remove_file(tp).await;
        }
    }

    tracing::info!("✅ Manual clipping job {} completed: {} clips", job_id, clips.len());
    Ok(format!("{} clips generated", clips.len()))
}
