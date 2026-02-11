// HTTP handlers for YouTube Clipping API endpoints

use axum::{
    extract::{Extension, Path, Query},
    http::StatusCode,
    response::Json,
    routing::{delete, get, patch, post},
    Router,
};
use crate::clipping::models::*;
use crate::middleware::{auth::auth_middleware, clipping_access::clipping_access_middleware};
use crate::models::auth::Claims;
use crate::AppState;
use serde::{Deserialize, Serialize};
use serde_json::{json, Value};
use sqlx::{PgPool, Row};
use std::sync::Arc;
use url::Url;
use chrono::{DateTime, Utc};

pub fn clipping_routes() -> Router {
    Router::new()
        // Source channel management
        .route(
            "/api/clipping/source-channels",
            get(list_source_channels).post(add_source_channel),
        )
        .route(
            "/api/clipping/source-channels/:id",
            get(get_source_channel)
                .patch(update_source_channel)
                .delete(remove_source_channel),
        )
        // Channel linkage management
        .route(
            "/api/clipping/linkages",
            get(list_linkages).post(create_linkage),
        )
        .route(
            "/api/clipping/linkages/:id",
            get(get_linkage)
                .patch(update_linkage)
                .delete(delete_linkage),
        )
        // Clipping job monitoring
        .route("/api/clipping/jobs", get(list_jobs))
        .route("/api/clipping/jobs/:id", get(get_job_status))
        .route("/api/clipping/jobs/:id/cancel", post(cancel_job))
        .route("/api/clipping/jobs/:id/retry", post(retry_job))
        .route("/api/clipping/jobs/:id/reset", post(reset_job))
        // Extracted clips
        .route("/api/clipping/clips", get(list_clips))
        .route("/api/clipping/clips/:id", get(get_clip_details))
        .route("/api/clipping/clips/:id/repost", post(repost_clip))
        // Access check endpoint
        .route("/api/clipping/access-check", get(check_access))
        // All routes protected by clipping access middleware
        .layer(axum::middleware::from_fn(clipping_access_middleware))
        .layer(axum::middleware::from_fn(auth_middleware))
}

// Source Channel Handlers

async fn list_source_channels(
    Extension(state): Extension<Arc<AppState>>,
) -> Result<Json<Value>, StatusCode> {
    let channels = sqlx::query_as::<_, SourceChannel>("SELECT * FROM youtube_source_channels ORDER BY created_at DESC")
        .fetch_all(&state.db_pool)
        .await
        .map_err(|_| StatusCode::INTERNAL_SERVER_ERROR)?;

    Ok(Json(json!({
        "success": true,
        "channels": channels
    })))
}

/// Extract channel identifier from various YouTube URL formats
/// Supports:
/// - https://www.youtube.com/@handle -> @handle
/// - https://www.youtube.com/channel/UC... -> UC...
/// - https://www.youtube.com/c/CustomName -> CustomName
/// - Direct handle: @handle -> @handle
/// - Direct ID: UC... -> UC...
fn extract_channel_identifier(input: &str) -> String {
    let trimmed = input.trim();

    // If it's already a handle or channel ID (no URL), return as-is
    if !trimmed.starts_with("http://") && !trimmed.starts_with("https://") {
        return trimmed.to_string();
    }

    // Parse URL and extract path
    if let Ok(url) = Url::parse(trimmed) {
        let path = url.path();

        // Handle @username format: //@username or /@username
        if let Some(handle_pos) = path.find("/@") {
            return path[handle_pos + 1..].split('/').next().unwrap_or("").to_string();
        }

        // Handle /channel/UCxxx format
        if path.starts_with("/channel/") {
            return path[9..].split('/').next().unwrap_or("").to_string();
        }

        // Handle /c/CustomName format
        if path.starts_with("/c/") {
            return path[3..].split('/').next().unwrap_or("").to_string();
        }

        // Handle /user/Username format
        if path.starts_with("/user/") {
            return path[6..].split('/').next().unwrap_or("").to_string();
        }
    }

    // Fallback: return original input
    trimmed.to_string()
}

async fn add_source_channel(
    Extension(state): Extension<Arc<AppState>>,
    Json(payload): Json<AddSourceChannelRequest>,
) -> Result<Json<Value>, StatusCode> {
    // Accept either channel_url (from content_machine) or channel_id (from embedded UI)
    let input = payload.channel_url
        .or(payload.channel_id)
        .ok_or(StatusCode::BAD_REQUEST)?;

    // Extract channel identifier from URL or use directly if it's already an ID/handle
    let channel_id = extract_channel_identifier(&input);

    if channel_id.is_empty() {
        return Err(StatusCode::BAD_REQUEST);
    }

    // Fetch channel info from YouTube API
    let youtube_client = state
        .youtube_client
        .as_ref()
        .ok_or(StatusCode::SERVICE_UNAVAILABLE)?;

    // Search for the channel
    let channel_info = youtube_client
        .search_channels(None, &channel_id, 1, None)
        .await
        .map_err(|_| StatusCode::BAD_REQUEST)?;

    let channel = channel_info
        .items
        .first()
        .ok_or(StatusCode::NOT_FOUND)?;

    // Get thumbnail URL safely from JSON value
    let thumbnail_url = channel
        .snippet
        .thumbnails
        .get("default")
        .and_then(|t| t.get("url"))
        .and_then(|u| u.as_str())
        .map(|s| s.to_string());

    // Insert into database
    let source_channel = sqlx::query_as::<_, SourceChannel>(
        "INSERT INTO youtube_source_channels
         (channel_id, channel_name, channel_thumbnail_url, subscriber_count, polling_interval_minutes)
         VALUES ($1, $2, $3, $4, $5)
         RETURNING *",
    )
    .bind(&channel.id.channel_id)
    .bind(&channel.snippet.title)
    .bind(thumbnail_url)
    .bind(0i64) // Subscriber count can be fetched separately
    .bind(payload.polling_interval_minutes.unwrap_or(30))
    .fetch_one(&state.db_pool)
    .await
    .map_err(|_| StatusCode::INTERNAL_SERVER_ERROR)?;

    // Initialize poll schedule
    sqlx::query(
        "INSERT INTO clipping_poll_schedule (source_channel_id, next_poll_at)
         VALUES ($1, NOW())",
    )
    .bind(source_channel.id)
    .execute(&state.db_pool)
    .await
    .map_err(|_| StatusCode::INTERNAL_SERVER_ERROR)?;

    Ok(Json(json!({
        "success": true,
        "channel": source_channel
    })))
}

async fn get_source_channel(
    Extension(state): Extension<Arc<AppState>>,
    Path(id): Path<i32>,
) -> Result<Json<Value>, StatusCode> {
    let channel = sqlx::query_as::<_, SourceChannel>("SELECT * FROM youtube_source_channels WHERE id = $1")
        .bind(id)
        .fetch_optional(&state.db_pool)
        .await
        .map_err(|_| StatusCode::INTERNAL_SERVER_ERROR)?
        .ok_or(StatusCode::NOT_FOUND)?;

    Ok(Json(json!({
        "success": true,
        "channel": channel
    })))
}

async fn update_source_channel(
    Extension(state): Extension<Arc<AppState>>,
    Path(id): Path<i32>,
    Json(payload): Json<Value>,
) -> Result<Json<Value>, StatusCode> {
    let is_active = payload["is_active"].as_bool();
    let polling_interval = payload["polling_interval_minutes"].as_i64();

    if let Some(active) = is_active {
        sqlx::query("UPDATE youtube_source_channels SET is_active = $1 WHERE id = $2")
            .bind(active)
            .bind(id)
            .execute(&state.db_pool)
            .await
            .map_err(|_| StatusCode::INTERNAL_SERVER_ERROR)?;
    }

    if let Some(interval) = polling_interval {
        sqlx::query("UPDATE youtube_source_channels SET polling_interval_minutes = $1 WHERE id = $2")
            .bind(interval as i32)
            .bind(id)
            .execute(&state.db_pool)
            .await
            .map_err(|_| StatusCode::INTERNAL_SERVER_ERROR)?;
    }

    Ok(Json(json!({
        "success": true,
        "message": "Source channel updated"
    })))
}

async fn remove_source_channel(
    Extension(state): Extension<Arc<AppState>>,
    Path(id): Path<i32>,
) -> Result<Json<Value>, StatusCode> {
    sqlx::query("DELETE FROM youtube_source_channels WHERE id = $1")
        .bind(id)
        .execute(&state.db_pool)
        .await
        .map_err(|_| StatusCode::INTERNAL_SERVER_ERROR)?;

    Ok(Json(json!({
        "success": true,
        "message": "Source channel removed"
    })))
}

// Channel Linkage Handlers

async fn list_linkages(
    Extension(state): Extension<Arc<AppState>>,
    Extension(claims): Extension<Claims>,
) -> Result<Json<Value>, StatusCode> {
    let user_id = claims.sub.parse::<i32>().unwrap_or(0);

    // Fetch linkages with channel names via JOINs
    let rows = sqlx::query(
        "SELECT
            l.*,
            sc.channel_name as source_channel_name,
            cc.channel_name as destination_channel_name
         FROM youtube_channel_linkages l
         LEFT JOIN youtube_source_channels sc ON l.source_channel_id = sc.id
         LEFT JOIN connected_youtube_channels cc ON l.destination_channel_id = cc.id
         WHERE l.user_id = $1
         ORDER BY l.created_at DESC"
    )
    .bind(user_id)
    .fetch_all(&state.db_pool)
    .await
    .map_err(|_| StatusCode::INTERNAL_SERVER_ERROR)?;

    // Manually construct the enriched response
    let mut linkages = Vec::new();
    for row in rows {
        let mut linkage_json = json!({
            "id": row.get::<i32, _>("id"),
            "user_id": row.get::<i32, _>("user_id"),
            "source_channel_id": row.get::<i32, _>("source_channel_id"),
            "destination_channel_id": row.get::<i32, _>("destination_channel_id"),
            "is_active": row.get::<bool, _>("is_active"),
            "clips_per_video": row.get::<i32, _>("clips_per_video"),
            "min_clip_duration_seconds": row.get::<i32, _>("min_clip_duration_seconds"),
            "max_clip_duration_seconds": row.get::<i32, _>("max_clip_duration_seconds"),
            "total_clips_generated": row.get::<i32, _>("total_clips_generated"),
            "total_clips_posted": row.get::<i32, _>("total_clips_posted"),
            "clipping_cooldown_hours": row.get::<i32, _>("clipping_cooldown_hours"),
            "created_at": row.get::<DateTime<Utc>, _>("created_at"),
            "updated_at": row.get::<DateTime<Utc>, _>("updated_at"),
        });

        // Add enriched channel data
        if let Ok(source_name) = row.try_get::<String, _>("source_channel_name") {
            linkage_json["source_channel_name"] = json!(source_name);
        }
        if let Ok(dest_name) = row.try_get::<String, _>("destination_channel_name") {
            linkage_json["destination_channel_name"] = json!(dest_name);
        }

        linkages.push(linkage_json);
    }

    Ok(Json(json!({
        "success": true,
        "linkages": linkages
    })))
}

async fn create_linkage(
    Extension(state): Extension<Arc<AppState>>,
    Extension(claims): Extension<Claims>,
    Json(payload): Json<CreateLinkageRequest>,
) -> Result<Json<Value>, StatusCode> {
    let user_id = claims.sub.parse::<i32>().unwrap_or(0);

    let linkage = sqlx::query_as::<_, ChannelLinkage>(
        "INSERT INTO youtube_channel_linkages
         (user_id, source_channel_id, destination_channel_id, clips_per_video,
          min_clip_duration_seconds, max_clip_duration_seconds)
         VALUES ($1, $2, $3, $4, $5, $6)
         RETURNING *",
    )
    .bind(user_id)
    .bind(payload.source_channel_id)
    .bind(payload.destination_channel_id)
    .bind(payload.clips_per_video.unwrap_or(2))
    .bind(payload.min_clip_duration_seconds.unwrap_or(60))
    .bind(payload.max_clip_duration_seconds.unwrap_or(120))
    .fetch_one(&state.db_pool)
    .await
    .map_err(|_| StatusCode::INTERNAL_SERVER_ERROR)?;

    Ok(Json(json!({
        "success": true,
        "linkage": linkage
    })))
}

async fn get_linkage(
    Extension(state): Extension<Arc<AppState>>,
    Extension(claims): Extension<Claims>,
    Path(id): Path<i32>,
) -> Result<Json<Value>, StatusCode> {
    let user_id = claims.sub.parse::<i32>().unwrap_or(0);

    let linkage = sqlx::query_as::<_, ChannelLinkage>(
        "SELECT * FROM youtube_channel_linkages WHERE id = $1 AND user_id = $2"
    )
        .bind(id)
        .bind(user_id)
        .fetch_optional(&state.db_pool)
        .await
        .map_err(|_| StatusCode::INTERNAL_SERVER_ERROR)?
        .ok_or(StatusCode::NOT_FOUND)?;

    Ok(Json(json!({
        "success": true,
        "linkage": linkage
    })))
}

async fn update_linkage(
    Extension(state): Extension<Arc<AppState>>,
    Extension(claims): Extension<Claims>,
    Path(id): Path<i32>,
    Json(payload): Json<UpdateLinkageRequest>,
) -> Result<Json<Value>, StatusCode> {
    let user_id = claims.sub.parse::<i32>().unwrap_or(0);

    // Verify ownership before allowing updates
    let exists = sqlx::query_scalar::<_, bool>(
        "SELECT EXISTS(SELECT 1 FROM youtube_channel_linkages WHERE id = $1 AND user_id = $2)"
    )
    .bind(id)
    .bind(user_id)
    .fetch_one(&state.db_pool)
    .await
    .map_err(|_| StatusCode::INTERNAL_SERVER_ERROR)?;

    if !exists {
        return Err(StatusCode::NOT_FOUND);
    }

    if let Some(active) = payload.is_active {
        sqlx::query("UPDATE youtube_channel_linkages SET is_active = $1 WHERE id = $2 AND user_id = $3")
            .bind(active)
            .bind(id)
            .bind(user_id)
            .execute(&state.db_pool)
            .await
            .map_err(|_| StatusCode::INTERNAL_SERVER_ERROR)?;
    }

    if let Some(clips_per_video) = payload.clips_per_video {
        sqlx::query("UPDATE youtube_channel_linkages SET clips_per_video = $1 WHERE id = $2 AND user_id = $3")
            .bind(clips_per_video)
            .bind(id)
            .bind(user_id)
            .execute(&state.db_pool)
            .await
            .map_err(|_| StatusCode::INTERNAL_SERVER_ERROR)?;
    }

    if let Some(min_duration) = payload.min_clip_duration_seconds {
        sqlx::query("UPDATE youtube_channel_linkages SET min_clip_duration_seconds = $1 WHERE id = $2 AND user_id = $3")
            .bind(min_duration)
            .bind(id)
            .bind(user_id)
            .execute(&state.db_pool)
            .await
            .map_err(|_| StatusCode::INTERNAL_SERVER_ERROR)?;
    }

    if let Some(max_duration) = payload.max_clip_duration_seconds {
        sqlx::query("UPDATE youtube_channel_linkages SET max_clip_duration_seconds = $1 WHERE id = $2 AND user_id = $3")
            .bind(max_duration)
            .bind(id)
            .bind(user_id)
            .execute(&state.db_pool)
            .await
            .map_err(|_| StatusCode::INTERNAL_SERVER_ERROR)?;
    }

    Ok(Json(json!({
        "success": true,
        "message": "Linkage updated"
    })))
}

async fn delete_linkage(
    Extension(state): Extension<Arc<AppState>>,
    Extension(claims): Extension<Claims>,
    Path(id): Path<i32>,
) -> Result<Json<Value>, StatusCode> {
    let user_id = claims.sub.parse::<i32>().unwrap_or(0);

    let result = sqlx::query("DELETE FROM youtube_channel_linkages WHERE id = $1 AND user_id = $2")
        .bind(id)
        .bind(user_id)
        .execute(&state.db_pool)
        .await
        .map_err(|_| StatusCode::INTERNAL_SERVER_ERROR)?;

    if result.rows_affected() == 0 {
        return Err(StatusCode::NOT_FOUND);
    }

    Ok(Json(json!({
        "success": true,
        "message": "Linkage deleted"
    })))
}

// Clipping Job Handlers

#[derive(Deserialize)]
struct JobQueryParams {
    status: Option<String>,
    linkage_id: Option<i32>,
    limit: Option<i64>,
    offset: Option<i64>,
}

async fn list_jobs(
    Extension(state): Extension<Arc<AppState>>,
    Extension(claims): Extension<Claims>,
    Query(params): Query<JobQueryParams>,
) -> Result<Json<Value>, StatusCode> {
    let user_id = claims.sub.parse::<i32>().unwrap_or(0);
    let limit = params.limit.unwrap_or(50);
    let offset = params.offset.unwrap_or(0);

    let jobs = if let Some(status) = params.status {
        sqlx::query_as::<_, ClippingJob>(
            "SELECT cj.* FROM clipping_jobs cj
             JOIN youtube_channel_linkages ycl ON cj.linkage_id = ycl.id
             WHERE ycl.user_id = $1 AND cj.status = $2
             ORDER BY cj.created_at DESC
             LIMIT $3 OFFSET $4",
        )
        .bind(user_id)
        .bind(status)
        .bind(limit)
        .bind(offset)
        .fetch_all(&state.db_pool)
        .await
    } else {
        sqlx::query_as::<_, ClippingJob>(
            "SELECT cj.* FROM clipping_jobs cj
             JOIN youtube_channel_linkages ycl ON cj.linkage_id = ycl.id
             WHERE ycl.user_id = $1
             ORDER BY cj.created_at DESC
             LIMIT $2 OFFSET $3",
        )
        .bind(user_id)
        .bind(limit)
        .bind(offset)
        .fetch_all(&state.db_pool)
        .await
    }
    .map_err(|_| StatusCode::INTERNAL_SERVER_ERROR)?;

    Ok(Json(json!({
        "success": true,
        "jobs": jobs
    })))
}

async fn get_job_status(
    Extension(state): Extension<Arc<AppState>>,
    Extension(claims): Extension<Claims>,
    Path(id): Path<i32>,
) -> Result<Json<Value>, StatusCode> {
    let user_id = claims.sub.parse::<i32>().unwrap_or(0);

    // Verify user owns this job through the linkage
    let job = sqlx::query_as::<_, ClippingJob>(
        "SELECT cj.* FROM clipping_jobs cj
         JOIN youtube_channel_linkages ycl ON cj.linkage_id = ycl.id
         WHERE cj.id = $1 AND ycl.user_id = $2"
    )
        .bind(id)
        .bind(user_id)
        .fetch_optional(&state.db_pool)
        .await
        .map_err(|_| StatusCode::INTERNAL_SERVER_ERROR)?
        .ok_or(StatusCode::NOT_FOUND)?;

    // Get extracted clips for this job
    let clips = sqlx::query_as::<_, ExtractedClip>(
        "SELECT * FROM extracted_clips WHERE clipping_job_id = $1 ORDER BY clip_number",
    )
    .bind(id)
    .fetch_all(&state.db_pool)
    .await
    .map_err(|_| StatusCode::INTERNAL_SERVER_ERROR)?;

    Ok(Json(json!({
        "success": true,
        "job": job,
        "clips": clips
    })))
}

async fn cancel_job(
    Extension(state): Extension<Arc<AppState>>,
    Extension(claims): Extension<Claims>,
    Path(id): Path<i32>,
) -> Result<Json<Value>, StatusCode> {
    let user_id = claims.sub.parse::<i32>().unwrap_or(0);

    // Verify ownership and cancel in one query
    let result = sqlx::query(
        "UPDATE clipping_jobs cj
         SET status = 'cancelled'
         FROM youtube_channel_linkages ycl
         WHERE cj.id = $1
         AND cj.linkage_id = ycl.id
         AND ycl.user_id = $2
         AND cj.status NOT IN ('completed', 'failed')"
    )
        .bind(id)
        .bind(user_id)
        .execute(&state.db_pool)
        .await
        .map_err(|_| StatusCode::INTERNAL_SERVER_ERROR)?;

    if result.rows_affected() == 0 {
        return Err(StatusCode::NOT_FOUND);
    }

    Ok(Json(json!({
        "success": true,
        "message": "Job cancelled"
    })))
}

/// Retry a failed clipping job
///
/// POST /api/clipping/jobs/:id/retry
async fn retry_job(
    Extension(state): Extension<Arc<AppState>>,
    Extension(claims): Extension<Claims>,
    Path(id): Path<i32>,
) -> Result<Json<Value>, StatusCode> {
    let user_id = claims.sub.parse::<i32>().unwrap_or(0);

    // Verify ownership and retry in one query (track retry count)
    let result = sqlx::query(
        "UPDATE clipping_jobs cj
         SET status = 'pending',
             error_message = NULL,
             progress_percent = 0,
             current_step = 'queued',
             started_at = NULL,
             completed_at = NULL,
             updated_at = NOW(),
             retry_count = COALESCE(cj.retry_count, 0) + 1,
             last_retry_at = NOW()
         FROM youtube_channel_linkages ycl
         WHERE cj.id = $1
         AND cj.linkage_id = ycl.id
         AND ycl.user_id = $2
         AND cj.status = 'failed'"
    )
        .bind(id)
        .bind(user_id)
        .execute(&state.db_pool)
        .await
        .map_err(|_| StatusCode::INTERNAL_SERVER_ERROR)?;

    if result.rows_affected() == 0 {
        return Err(StatusCode::NOT_FOUND);
    }

    tracing::info!("🔄 Job {} reset to pending for retry by user {}", id, user_id);

    Ok(Json(json!({
        "success": true,
        "message": "Job queued for retry"
    })))
}

/// Reset a stuck clipping job from any intermediate state
///
/// POST /api/clipping/jobs/:id/reset
///
/// This endpoint allows users to manually reset jobs that are stuck in intermediate states
/// (downloading, analyzing, extracting_clips, posting) back to 'pending' so they can be retried.
/// This is useful when jobs hang due to external failures or timeouts.
async fn reset_job(
    Extension(state): Extension<Arc<AppState>>,
    Extension(claims): Extension<Claims>,
    Path(id): Path<i32>,
) -> Result<Json<Value>, StatusCode> {
    let user_id = claims.sub.parse::<i32>().unwrap_or(0);

    // Verify ownership and reset in one query (track retry for manual resets)
    // Allow resetting from any intermediate state OR failed state
    let result = sqlx::query(
        "UPDATE clipping_jobs cj
         SET status = 'pending',
             error_message = NULL,
             progress_percent = 0,
             current_step = 'queued',
             updated_at = NOW(),
             retry_count = COALESCE(cj.retry_count, 0) + 1,
             last_retry_at = NOW()
         FROM youtube_channel_linkages ycl
         WHERE cj.id = $1
         AND cj.linkage_id = ycl.id
         AND ycl.user_id = $2
         AND cj.status IN ('downloading', 'analyzing', 'extracting_clips', 'posting', 'failed')"
    )
        .bind(id)
        .bind(user_id)
        .execute(&state.db_pool)
        .await
        .map_err(|_| StatusCode::INTERNAL_SERVER_ERROR)?;

    if result.rows_affected() == 0 {
        return Err(StatusCode::NOT_FOUND);
    }

    tracing::info!("🔄 Job {} manually reset to pending by user {} (unstuck operation)", id, user_id);

    Ok(Json(json!({
        "success": true,
        "message": "Job reset to pending and queued for processing"
    })))
}

// Extracted Clip Handlers

#[derive(Deserialize)]
struct ClipQueryParams {
    job_id: Option<i32>,
    upload_status: Option<String>,
    limit: Option<i64>,
    offset: Option<i64>,
}

async fn list_clips(
    Extension(state): Extension<Arc<AppState>>,
    Extension(claims): Extension<Claims>,
    Query(params): Query<ClipQueryParams>,
) -> Result<Json<Value>, StatusCode> {
    let user_id = claims.sub.parse::<i32>().unwrap_or(0);
    let limit = params.limit.unwrap_or(50);
    let offset = params.offset.unwrap_or(0);

    let clips = sqlx::query_as::<_, ExtractedClip>(
        "SELECT ec.* FROM extracted_clips ec
         JOIN clipping_jobs cj ON ec.clipping_job_id = cj.id
         JOIN youtube_channel_linkages ycl ON cj.linkage_id = ycl.id
         WHERE ycl.user_id = $1
         ORDER BY ec.created_at DESC
         LIMIT $2 OFFSET $3",
    )
    .bind(user_id)
    .bind(limit)
    .bind(offset)
    .fetch_all(&state.db_pool)
    .await
    .map_err(|_| StatusCode::INTERNAL_SERVER_ERROR)?;

    Ok(Json(json!({
        "success": true,
        "clips": clips
    })))
}

async fn get_clip_details(
    Extension(state): Extension<Arc<AppState>>,
    Extension(claims): Extension<Claims>,
    Path(id): Path<i32>,
) -> Result<Json<Value>, StatusCode> {
    let user_id = claims.sub.parse::<i32>().unwrap_or(0);

    // Verify user owns this clip through job -> linkage chain
    let clip = sqlx::query_as::<_, ExtractedClip>(
        "SELECT ec.* FROM extracted_clips ec
         JOIN clipping_jobs cj ON ec.clipping_job_id = cj.id
         JOIN youtube_channel_linkages ycl ON cj.linkage_id = ycl.id
         WHERE ec.id = $1 AND ycl.user_id = $2"
    )
        .bind(id)
        .bind(user_id)
        .fetch_optional(&state.db_pool)
        .await
        .map_err(|_| StatusCode::INTERNAL_SERVER_ERROR)?
        .ok_or(StatusCode::NOT_FOUND)?;

    Ok(Json(json!({
        "success": true,
        "clip": clip
    })))
}

async fn repost_clip(
    Extension(state): Extension<Arc<AppState>>,
    Extension(claims): Extension<Claims>,
    Path(id): Path<i32>,
) -> Result<Json<Value>, StatusCode> {
    let user_id = claims.sub.parse::<i32>().unwrap_or(0);

    // Verify ownership and reset upload status in one query
    let result = sqlx::query(
        "UPDATE extracted_clips ec
         SET upload_status = 'pending', upload_error = NULL
         FROM clipping_jobs cj
         JOIN youtube_channel_linkages ycl ON cj.linkage_id = ycl.id
         WHERE ec.id = $1
         AND ec.clipping_job_id = cj.id
         AND ycl.user_id = $2"
    )
    .bind(id)
    .bind(user_id)
    .execute(&state.db_pool)
    .await
    .map_err(|_| StatusCode::INTERNAL_SERVER_ERROR)?;

    if result.rows_affected() == 0 {
        return Err(StatusCode::NOT_FOUND);
    }

    Ok(Json(json!({
        "success": true,
        "message": "Clip queued for reposting"
    })))
}

/// Lightweight endpoint to check if user has clipping access
/// Returns 200 OK if access granted, 403 if denied
/// Used by frontend to conditionally show/hide clipping card
async fn check_access() -> Json<Value> {
    // If middleware allows request through, user has access
    Json(json!({
        "has_access": true
    }))
}
