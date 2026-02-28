// Background job for clipping workflow orchestration — NEW 5-PHASE ARCHITECTURE
//
// Phase A: Single Gemini video analysis via YouTube URL (replaces frame-by-frame pipeline)
// Phase B: Download video (only if Phase A found quality clips)
// Phase C: Parallel FFmpeg clip extraction
// Phase D: Lightweight vectorization (1 embedding into video_content collection)
// Phase E: Parallel YouTube upload + store_extracted_clip per clip

use crate::clipping::{
    ai_clipper::{AiClipper, ExtractedClipData},
    models::{ChannelLinkage, ClippingConfig, ClippingJob},
    uploader::ClipUploader,
    apify_client::ApifyClient,
};
use crate::models::youtube::ConnectedYouTubeChannel;
use crate::services::VideoVectorizationService;
use crate::AppState;
use chrono::Utc;
use sqlx::PgPool;
use std::sync::Arc;

/// Execute clipping job workflow
pub async fn execute_clipping_job(
    job_id: i32,
    app_state: Arc<AppState>,
) -> Result<String, String> {
    tracing::info!("🎬 Starting clipping job {} (new 5-phase architecture)", job_id);

    // Fetch job details
    let job = fetch_job_details(job_id, &app_state.db_pool).await?;
    let linkage = fetch_linkage(job.linkage_id, &app_state.db_pool).await?;

    let config = ClippingConfig {
        clips_per_video: linkage.clips_per_video,
        min_clip_duration_seconds: linkage.min_clip_duration_seconds,
        max_clip_duration_seconds: linkage.max_clip_duration_seconds,
    };

    let video_url = format!("https://youtube.com/watch?v={}", job.source_video_id);

    // =========================================================================
    // Phase A: Single Gemini video analysis via YouTube URL
    // =========================================================================
    update_job_status(job_id, "analyzing", 10, None, &app_state.db_pool).await?;
    tracing::info!("🔍 Phase A: Analyzing via YouTube URL (1 Gemini call)");

    let gemini_client = app_state.gemini_client.as_ref()
        .ok_or("Gemini client not configured — required for YouTube URL analysis")?;

    let analysis = tokio::time::timeout(
        tokio::time::Duration::from_secs(180), // 3 min for analysis
        gemini_client.analyze_video_from_url(
            &video_url,
            config.clips_per_video as usize,
            config.min_clip_duration_seconds as f64,
            config.max_clip_duration_seconds as f64,
        ),
    )
    .await
    .map_err(|_| "Gemini video analysis timed out after 180 seconds".to_string())?
    .map_err(|e| format!("Gemini video analysis failed: {}", e))?;

    // Fast-fail: if no moments meet quality threshold, skip download entirely
    let qualified_moments = analysis.qualified_moments(0.6);
    if qualified_moments.is_empty() {
        let status_msg = format!(
            "No viral moments found with quality >= 0.6 (overall quality: {:.2}). Video may not be suitable for clips.",
            analysis.overall_quality
        );
        update_job_status(job_id, "no_clips_found", 100, Some(&status_msg), &app_state.db_pool).await?;
        mark_job_completed(job_id, &app_state.db_pool).await?;
        return Ok(format!("No qualifying clips found (overall_quality={:.2})", analysis.overall_quality));
    }

    // Take top N moments up to clips_per_video
    let moments: Vec<_> = analysis.top_moments(config.clips_per_video as usize)
        .into_iter()
        .cloned()
        .collect();

    tracing::info!(
        "✅ Phase A complete: {} viral moments identified ({} qualify with score ≥ 0.6)",
        analysis.viral_moments.len(),
        qualified_moments.len()
    );

    update_job_status(job_id, "analyzed", 20, None, &app_state.db_pool).await?;

    // =========================================================================
    // Phase B: Download video (only now that we know clips exist)
    // =========================================================================
    update_job_status(job_id, "downloading", 25, None, &app_state.db_pool).await?;
    tracing::info!("⬇️  Phase B: Downloading video");

    let video_path = format!("downloads/clipping_{}_{}.mp4", job_id, job.source_video_id);

    let apify_token = std::env::var("APIFY_TOKEN")
        .map_err(|_| "APIFY_TOKEN not configured")?;
    let apify_actor = std::env::var("APIFY_YOUTUBE_CLIENT_ACTOR")
        .map_err(|_| "APIFY_YOUTUBE_CLIENT_ACTOR not configured")?;

    let apify_client = ApifyClient::new(apify_token, apify_actor);

    tracing::info!("Downloading video: {}", video_url);
    let _download_result = apify_client.download_video(&video_url, &video_path).await
        .map_err(|e| format!("Download failed: {}", e))?;

    // Validate downloaded file
    if !std::path::Path::new(&video_path).exists() {
        return Err(format!("Downloaded file not found: {}", video_path));
    }

    match crate::core::validate_video_file(&video_path) {
        Ok(true) => tracing::info!("✅ Downloaded video validated"),
        Ok(false) | Err(_) => {
            let _ = tokio::fs::remove_file(&video_path).await;
            return Err(format!("Downloaded video is corrupted: {}", video_path));
        }
    }

    update_job_status(job_id, "downloaded", 40, None, &app_state.db_pool).await?;
    update_job_video_path(job_id, &video_path, &app_state.db_pool).await?;

    // =========================================================================
    // Phase C: Parallel clip extraction
    // =========================================================================
    update_job_status(job_id, "extracting_clips", 50, None, &app_state.db_pool).await?;
    tracing::info!("✂️  Phase C: Parallel clip extraction ({} clips)", moments.len());

    // Create output directory
    tokio::fs::create_dir_all("outputs").await
        .map_err(|e| format!("Failed to create outputs directory: {}", e))?;

    let clipper = AiClipper::new(app_state.clone());
    let clips = clipper
        .extract_clips_from_moments(job_id, &video_path, &moments)
        .await?;

    if clips.is_empty() {
        return Err("All clip extractions failed".to_string());
    }

    tracing::info!("✅ Phase C complete: {}/{} clips extracted", clips.len(), moments.len());
    update_job_status(job_id, "clips_extracted", 60, None, &app_state.db_pool).await?;

    // =========================================================================
    // Phase D: Lightweight vectorization (1 embedding into video_content)
    // =========================================================================
    update_job_status(job_id, "vectorizing", 65, None, &app_state.db_pool).await?;
    tracing::info!("🔢 Phase D: Storing video analysis (1 embedding)");

    match VideoVectorizationService::store_video_analysis_from_gemini(
        &job.source_video_id,
        &video_url,
        Some(linkage.user_id),
        None, // channel_id set from linkage if needed
        &analysis,
        &app_state,
    ).await {
        Ok(()) => tracing::info!("✅ Phase D complete: video_content stored"),
        Err(e) => tracing::warn!("Phase D vectorization failed (non-fatal): {}", e),
    }

    // =========================================================================
    // Phase D continued: Save clips to database
    // =========================================================================
    let clip_db_ids = save_clips_to_database(job_id, &clips, &linkage, &app_state.db_pool).await?;

    // =========================================================================
    // Phase E: Parallel YouTube upload
    // =========================================================================
    update_job_status(job_id, "posting", 70, None, &app_state.db_pool).await?;
    tracing::info!("📤 Phase E: Uploading {} clips to YouTube", clips.len());

    let destination_channel = fetch_destination_channel(linkage.destination_channel_id, &app_state.db_pool).await?;

    let youtube_client = app_state
        .youtube_client
        .as_ref()
        .ok_or("YouTube client not available")?;

    let oauth_client_id = app_state
        .google_oauth_client_id
        .as_ref()
        .ok_or("Google OAuth client ID not configured")?;

    let oauth_client_secret = app_state
        .google_oauth_client_secret
        .as_ref()
        .ok_or("Google OAuth client secret not configured")?;

    let uploader = ClipUploader::new(
        Arc::new(youtube_client.clone()),
        app_state.db_pool.clone(),
        oauth_client_id.clone(),
        oauth_client_secret.clone(),
    );

    let mut uploaded_count = 0;
    for (clip, clip_id) in clips.iter().zip(clip_db_ids.iter()) {
        // Enforce daily 4-clip limit at upload time (not just at job-creation time).
        // This prevents race conditions where two concurrent jobs each see clips_today=0
        // and both upload, resulting in more than 4 clips posted in a day.
        let clips_today = count_clips_posted_today(destination_channel.id, &app_state.db_pool)
            .await
            .unwrap_or(0);
        if clips_today >= 4 {
            tracing::info!(
                "Daily upload limit (4 clips) reached for channel '{}' — stopping uploads for job {}",
                destination_channel.channel_name, job_id
            );
            break;
        }

        match uploader.upload_clip(clip, *clip_id, &destination_channel).await {
            Ok(_) => {
                uploaded_count += 1;
                let progress = 70 + (uploaded_count * 30 / clips.len() as i32);
                update_job_status(job_id, "posting", progress, None, &app_state.db_pool).await?;

                // Store clip in extracted_clips Qdrant collection (best-effort)
                store_clip_in_qdrant(*clip_id, job_id, &job.source_video_id, clip, &app_state).await;
            }
            Err(e) => {
                tracing::error!("Failed to upload clip {}: {}", clip.clip_number, e);
                let _ = uploader.mark_upload_failed(*clip_id, &e).await;
            }
        }
    }

    // If ALL uploads failed, mark job as failed so the auto-retry worker can retry it.
    // Do NOT trigger the 24-hour cooldown or mark_job_completed — that would permanently
    // block re-processing the source video and waste the cooldown window on a failed job.
    if uploaded_count == 0 {
        update_job_status(
            job_id,
            "failed",
            0,
            Some("All clip uploads failed. Job will retry when OAuth tokens are valid."),
            &app_state.db_pool,
        ).await?;
        update_linkage_stats(linkage.id, clips.len() as i32, 0, &app_state.db_pool).await?;
        let _ = tokio::fs::remove_file(&video_path).await;
        tracing::error!(
            "❌ Clipping job {} marked failed: 0/{} clips uploaded",
            job_id, clips.len()
        );
        return Err("All clip uploads failed — check OAuth token validity".to_string());
    }

    // At least 1 clip was uploaded successfully — mark job completed and set cooldown.
    update_job_status(job_id, "completed", 100, None, &app_state.db_pool).await?;
    mark_job_completed(job_id, &app_state.db_pool).await?;
    update_linkage_session_timestamp(linkage.id, &app_state.db_pool).await?;
    update_linkage_stats(linkage.id, clips.len() as i32, uploaded_count, &app_state.db_pool).await?;

    // Cleanup downloaded video
    let _ = tokio::fs::remove_file(&video_path).await;

    tracing::info!(
        "✅ Clipping job {} completed: {}/{} clips posted",
        job_id, uploaded_count, clips.len()
    );

    Ok(format!("Successfully posted {}/{} clips", uploaded_count, clips.len()))
}

/// Store an extracted clip in the Qdrant extracted_clips collection (best-effort, non-fatal).
async fn store_clip_in_qdrant(
    clip_id: i32,
    job_id: i32,
    source_video_id: &str,
    clip: &ExtractedClipData,
    app_state: &Arc<AppState>,
) {
    let qdrant_client = match &app_state.qdrant_client {
        Some(c) => c,
        None => return,
    };

    let text_to_embed = format!("{} {}", clip.ai_title, clip.ai_description);

    let embedding = if let Some(ref voyage) = app_state.voyage_embeddings {
        voyage.generate_single_embedding(text_to_embed).await.ok()
    } else if let Some(ref gemini) = app_state.gemini_client {
        gemini.embed_content(&format!("{} {}", clip.ai_title, clip.ai_description)).await.ok()
    } else {
        None
    };

    let embedding = match embedding {
        Some(e) => e,
        None => return,
    };

    let _ = qdrant_client.ensure_extracted_clips_collection().await;

    let payload = serde_json::json!({
        "clip_id": clip_id,
        "clipping_job_id": job_id,
        "source_video_id": source_video_id,
        "title": clip.ai_title,
        "hook": clip.ai_description,
        "start_sec": clip.start_time_seconds,
        "end_sec": clip.end_time_seconds,
        "duration_sec": clip.duration_seconds,
        "quality_score": clip.ai_confidence_score,
        "viral_factors": clip.viral_factors,
        "upload_status": "uploaded",
        "uploaded_at": chrono::Utc::now().to_rfc3339(),
    });

    if let Err(e) = qdrant_client.store_extracted_clip(clip_id, payload, embedding).await {
        tracing::debug!("Failed to store clip {} in Qdrant (non-fatal): {}", clip_id, e);
    }
}

// Helper functions

async fn fetch_job_details(job_id: i32, pool: &PgPool) -> Result<ClippingJob, String> {
    sqlx::query_as::<_, ClippingJob>("SELECT * FROM clipping_jobs WHERE id = $1")
        .bind(job_id)
        .fetch_one(pool)
        .await
        .map_err(|e| format!("Failed to fetch job: {}", e))
}

async fn fetch_linkage(linkage_id: i32, pool: &PgPool) -> Result<ChannelLinkage, String> {
    sqlx::query_as::<_, ChannelLinkage>("SELECT * FROM youtube_channel_linkages WHERE id = $1")
        .bind(linkage_id)
        .fetch_one(pool)
        .await
        .map_err(|e| format!("Failed to fetch linkage: {}", e))
}

async fn fetch_destination_channel(
    channel_id: i32,
    pool: &PgPool,
) -> Result<ConnectedYouTubeChannel, String> {
    sqlx::query_as::<_, ConnectedYouTubeChannel>(
        "SELECT * FROM connected_youtube_channels WHERE id = $1",
    )
    .bind(channel_id)
    .fetch_one(pool)
    .await
    .map_err(|e| format!("Failed to fetch destination channel: {}", e))
}

async fn update_job_status(
    job_id: i32,
    status: &str,
    progress: i32,
    error: Option<&str>,
    pool: &PgPool,
) -> Result<(), String> {
    sqlx::query(
        "UPDATE clipping_jobs
         SET status = $1, progress_percent = $2, current_step = $1, error_message = $3, updated_at = NOW()
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

async fn update_job_video_path(
    job_id: i32,
    video_path: &str,
    pool: &PgPool,
) -> Result<(), String> {
    sqlx::query("UPDATE clipping_jobs SET local_video_path = $1, started_at = $2 WHERE id = $3")
        .bind(video_path)
        .bind(Utc::now())
        .bind(job_id)
        .execute(pool)
        .await
        .map_err(|e| format!("Failed to update video path: {}", e))?;

    Ok(())
}

async fn mark_job_completed(job_id: i32, pool: &PgPool) -> Result<(), String> {
    sqlx::query("UPDATE clipping_jobs SET completed_at = $1 WHERE id = $2")
        .bind(Utc::now())
        .bind(job_id)
        .execute(pool)
        .await
        .map_err(|e| format!("Failed to mark job completed: {}", e))?;

    Ok(())
}

async fn update_linkage_session_timestamp(linkage_id: i32, pool: &PgPool) -> Result<(), String> {
    sqlx::query(
        "UPDATE youtube_channel_linkages SET last_clipping_session_at = NOW() WHERE id = $1",
    )
    .bind(linkage_id)
    .execute(pool)
    .await
    .map_err(|e| format!("Failed to update session timestamp: {}", e))?;

    Ok(())
}

async fn save_clips_to_database(
    job_id: i32,
    clips: &[ExtractedClipData],
    linkage: &ChannelLinkage,
    pool: &PgPool,
) -> Result<Vec<i32>, String> {
    // Delete any stale clips from a previous failed attempt for this job.
    // Without this, retried jobs accumulate duplicate extracted_clips rows
    // because the INSERT has no ON CONFLICT clause.
    sqlx::query("DELETE FROM extracted_clips WHERE clipping_job_id = $1")
        .bind(job_id)
        .execute(pool)
        .await
        .map_err(|e| format!("Failed to clean stale clips before save: {}", e))?;

    let mut clip_ids = Vec::new();

    for clip in clips {
        let clip_id: i32 = sqlx::query_scalar(
            "INSERT INTO extracted_clips
             (clipping_job_id, clip_number, local_clip_path,
              start_time_seconds, end_time_seconds, duration_seconds,
              ai_title, ai_description, ai_tags, ai_confidence_score, viral_factors,
              destination_channel_id, custom_thumbnail_path, thumbnail_generation_method)
             VALUES ($1, $2, $3, $4, $5, $6, $7, $8, $9, $10, $11, $12, $13, $14)
             RETURNING id",
        )
        .bind(job_id)
        .bind(clip.clip_number)
        .bind(&clip.local_clip_path)
        .bind(clip.start_time_seconds)
        .bind(clip.end_time_seconds)
        .bind(clip.duration_seconds)
        .bind(&clip.ai_title)
        .bind(&clip.ai_description)
        .bind(&clip.ai_tags)
        .bind(clip.ai_confidence_score)
        .bind(&clip.viral_factors)
        .bind(linkage.destination_channel_id)
        .bind(&clip.custom_thumbnail_path)
        .bind(clip.custom_thumbnail_path.as_ref().map(|_| "ffmpeg_timestamp"))
        .fetch_one(pool)
        .await
        .map_err(|e| format!("Failed to save clip: {}", e))?;

        clip_ids.push(clip_id);
    }

    Ok(clip_ids)
}

/// Count clips successfully published for a destination channel in the last 24 hours.
/// Used to enforce the 4-clip/day limit at upload time.
async fn count_clips_posted_today(destination_channel_id: i32, pool: &PgPool) -> Result<i64, String> {
    sqlx::query_scalar::<_, i64>(
        "SELECT COUNT(*) FROM extracted_clips
         WHERE destination_channel_id = $1
           AND upload_status = 'published'
           AND published_at >= NOW() - INTERVAL '24 hours'",
    )
    .bind(destination_channel_id)
    .fetch_one(pool)
    .await
    .map_err(|e| format!("Failed to count daily clips: {}", e))
}

async fn update_linkage_stats(
    linkage_id: i32,
    clips_generated: i32,
    clips_posted: i32,
    pool: &PgPool,
) -> Result<(), String> {
    sqlx::query(
        "UPDATE youtube_channel_linkages
         SET total_clips_generated = total_clips_generated + $1,
             total_clips_posted = total_clips_posted + $2,
             last_clip_generated_at = NOW()
         WHERE id = $3",
    )
    .bind(clips_generated)
    .bind(clips_posted)
    .bind(linkage_id)
    .execute(pool)
    .await
    .map_err(|e| format!("Failed to update linkage stats: {}", e))?;

    Ok(())
}
