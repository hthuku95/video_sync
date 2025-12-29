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
use sqlx::PgPool;
use std::sync::Arc;

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
        // Extracted clips
        .route("/api/clipping/clips", get(list_clips))
        .route("/api/clipping/clips/:id", get(get_clip_details))
        .route("/api/clipping/clips/:id/repost", post(repost_clip))
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

async fn add_source_channel(
    Extension(state): Extension<Arc<AppState>>,
    Json(payload): Json<AddSourceChannelRequest>,
) -> Result<Json<Value>, StatusCode> {
    // Fetch channel info from YouTube API
    let youtube_client = state
        .youtube_client
        .as_ref()
        .ok_or(StatusCode::SERVICE_UNAVAILABLE)?;

    // Search for the channel
    let channel_info = youtube_client
        .search_channels(None, &payload.channel_id, 1, None)
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
    let linkages = sqlx::query_as::<_, ChannelLinkage>(
        "SELECT * FROM youtube_channel_linkages WHERE user_id = $1 ORDER BY created_at DESC",
    )
    .bind(claims.sub.parse::<i32>().unwrap_or(0))
    .fetch_all(&state.db_pool)
    .await
    .map_err(|_| StatusCode::INTERNAL_SERVER_ERROR)?;

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
    Path(id): Path<i32>,
) -> Result<Json<Value>, StatusCode> {
    let linkage = sqlx::query_as::<_, ChannelLinkage>("SELECT * FROM youtube_channel_linkages WHERE id = $1")
        .bind(id)
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
    Path(id): Path<i32>,
    Json(payload): Json<UpdateLinkageRequest>,
) -> Result<Json<Value>, StatusCode> {
    if let Some(active) = payload.is_active {
        sqlx::query("UPDATE youtube_channel_linkages SET is_active = $1 WHERE id = $2")
            .bind(active)
            .bind(id)
            .execute(&state.db_pool)
            .await
            .map_err(|_| StatusCode::INTERNAL_SERVER_ERROR)?;
    }

    if let Some(clips_per_video) = payload.clips_per_video {
        sqlx::query("UPDATE youtube_channel_linkages SET clips_per_video = $1 WHERE id = $2")
            .bind(clips_per_video)
            .bind(id)
            .execute(&state.db_pool)
            .await
            .map_err(|_| StatusCode::INTERNAL_SERVER_ERROR)?;
    }

    if let Some(min_duration) = payload.min_clip_duration_seconds {
        sqlx::query("UPDATE youtube_channel_linkages SET min_clip_duration_seconds = $1 WHERE id = $2")
            .bind(min_duration)
            .bind(id)
            .execute(&state.db_pool)
            .await
            .map_err(|_| StatusCode::INTERNAL_SERVER_ERROR)?;
    }

    if let Some(max_duration) = payload.max_clip_duration_seconds {
        sqlx::query("UPDATE youtube_channel_linkages SET max_clip_duration_seconds = $1 WHERE id = $2")
            .bind(max_duration)
            .bind(id)
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
    Path(id): Path<i32>,
) -> Result<Json<Value>, StatusCode> {
    sqlx::query("DELETE FROM youtube_channel_linkages WHERE id = $1")
        .bind(id)
        .execute(&state.db_pool)
        .await
        .map_err(|_| StatusCode::INTERNAL_SERVER_ERROR)?;

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
    Path(id): Path<i32>,
) -> Result<Json<Value>, StatusCode> {
    let job = sqlx::query_as::<_, ClippingJob>("SELECT * FROM clipping_jobs WHERE id = $1")
        .bind(id)
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
    Path(id): Path<i32>,
) -> Result<Json<Value>, StatusCode> {
    sqlx::query("UPDATE clipping_jobs SET status = 'cancelled' WHERE id = $1 AND status NOT IN ('completed', 'failed')")
        .bind(id)
        .execute(&state.db_pool)
        .await
        .map_err(|_| StatusCode::INTERNAL_SERVER_ERROR)?;

    Ok(Json(json!({
        "success": true,
        "message": "Job cancelled"
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
    Path(id): Path<i32>,
) -> Result<Json<Value>, StatusCode> {
    let clip = sqlx::query_as::<_, ExtractedClip>("SELECT * FROM extracted_clips WHERE id = $1")
        .bind(id)
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
    Path(id): Path<i32>,
) -> Result<Json<Value>, StatusCode> {
    // Reset upload status to retry
    sqlx::query(
        "UPDATE extracted_clips
         SET upload_status = 'pending', upload_error = NULL
         WHERE id = $1",
    )
    .bind(id)
    .execute(&state.db_pool)
    .await
    .map_err(|_| StatusCode::INTERNAL_SERVER_ERROR)?;

    Ok(Json(json!({
        "success": true,
        "message": "Clip queued for reposting"
    })))
}
