// Background job for clipping workflow orchestration

use crate::clipping::{
    ai_clipper::{AiClipper, ExtractedClipData},
    models::{ChannelLinkage, ClippingConfig, ClippingJob},
    uploader::ClipUploader,
    ytdlp_client::YtDlpClient,
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
    tracing::info!("🎬 Starting clipping job {}", job_id);

    // Fetch job details
    let job = fetch_job_details(job_id, &app_state.db_pool).await?;
    let linkage = fetch_linkage(job.linkage_id, &app_state.db_pool).await?;

    // Update job status
    update_job_status(job_id, "downloading", 10, None, &app_state.db_pool).await?;

    // Step 1: Download video using yt-dlp
    let video_url = format!("https://youtube.com/watch?v={}", job.source_video_id);
    let video_path = format!("downloads/clipping_{}_{}.mp4", job_id, job.source_video_id);

    tracing::info!("Downloading video: {}", video_url);
    let download_result = YtDlpClient::download_video(&video_url, &video_path).await?;

    update_job_status(job_id, "downloaded", 20, None, &app_state.db_pool).await?;
    update_job_video_path(job_id, &video_path, &app_state.db_pool).await?;

    // Step 2: Vectorize the full video
    update_job_status(job_id, "analyzing", 30, None, &app_state.db_pool).await?;

    tracing::info!("Vectorizing video for AI analysis");
    VideoVectorizationService::process_video_for_vectorization(
        &video_path,
        &job.source_video_id,
        &format!("clipping_job_{}", job_id),
        Some(linkage.user_id),
        &app_state,
    )
    .await
    .map_err(|e| format!("Vectorization failed: {}", e))?;

    update_job_status(job_id, "vectorized", 40, None, &app_state.db_pool).await?;

    // Step 3: Extract viral clips using AI
    update_job_status(job_id, "extracting_clips", 50, None, &app_state.db_pool).await?;

    let clipper = AiClipper::new(app_state.clone());
    let config = ClippingConfig {
        clips_per_video: linkage.clips_per_video,
        min_clip_duration_seconds: linkage.min_clip_duration_seconds,
        max_clip_duration_seconds: linkage.max_clip_duration_seconds,
    };

    let clips = clipper
        .extract_viral_clips(job_id, &video_path, &config)
        .await?;

    update_job_status(job_id, "clips_extracted", 60, None, &app_state.db_pool).await?;

    // Step 4: Save clips to database
    let clip_db_ids = save_clips_to_database(job_id, &clips, &app_state.db_pool).await?;

    // Step 5: Upload clips to YouTube
    update_job_status(job_id, "posting", 70, None, &app_state.db_pool).await?;

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
        youtube_client.clone(),
        app_state.db_pool.clone(),
        oauth_client_id.clone(),
        oauth_client_secret.clone(),
    );

    let mut uploaded_count = 0;
    for (clip, clip_id) in clips.iter().zip(clip_db_ids.iter()) {
        match uploader.upload_clip(clip, *clip_id, &destination_channel).await {
            Ok(_) => {
                uploaded_count += 1;
                let progress = 70 + (uploaded_count * 30 / clips.len() as i32);
                update_job_status(job_id, "posting", progress, None, &app_state.db_pool).await?;
            }
            Err(e) => {
                tracing::error!("Failed to upload clip {}: {}", clip.clip_number, e);
                let _ = uploader.mark_upload_failed(*clip_id, &e).await;
            }
        }
    }

    // Step 6: Mark job as completed
    update_job_status(job_id, "completed", 100, None, &app_state.db_pool).await?;
    mark_job_completed(job_id, &app_state.db_pool).await?;

    // Update linkage statistics
    update_linkage_stats(linkage.id, clips.len() as i32, uploaded_count, &app_state.db_pool).await?;

    // Cleanup: Delete downloaded video (optional, configurable)
    let _ = tokio::fs::remove_file(&video_path).await;

    tracing::info!(
        "✅ Clipping job {} completed: {}/{} clips posted",
        job_id,
        uploaded_count,
        clips.len()
    );

    Ok(format!(
        "Successfully posted {}/{} clips",
        uploaded_count,
        clips.len()
    ))
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
         SET status = $1, progress_percent = $2, current_step = $1, error_message = $3
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

async fn save_clips_to_database(
    job_id: i32,
    clips: &[ExtractedClipData],
    pool: &PgPool,
) -> Result<Vec<i32>, String> {
    let mut clip_ids = Vec::new();

    for clip in clips {
        let clip_id: i32 = sqlx::query_scalar(
            "INSERT INTO extracted_clips
             (clipping_job_id, clip_number, local_clip_path,
              start_time_seconds, end_time_seconds, duration_seconds,
              ai_title, ai_description, ai_tags, ai_confidence_score, viral_factors)
             VALUES ($1, $2, $3, $4, $5, $6, $7, $8, $9, $10, $11)
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
        .fetch_one(pool)
        .await
        .map_err(|e| format!("Failed to save clip: {}", e))?;

        clip_ids.push(clip_id);
    }

    Ok(clip_ids)
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
