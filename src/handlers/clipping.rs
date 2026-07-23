// HTTP handlers for YouTube Clipping API endpoints

use crate::clipping::models::*;
use crate::clipping::uploader::ClipUploader;
use crate::middleware::{
    auth::auth_middleware, clipping_access::clipping_access_middleware,
    rate_limit::strict_rate_limit_middleware,
};
use crate::models::auth::Claims;
use crate::AppState;
use axum::{
    extract::ws::{Message, WebSocket, WebSocketUpgrade},
    extract::{Extension, Path, Query},
    http::StatusCode,
    response::{IntoResponse, Json},
    routing::{delete, get, post},
    Router,
};
use chrono::{DateTime, Utc};
use futures::{sink::SinkExt, stream::StreamExt};
use serde::Deserialize;
use serde_json::{json, Value};
use sqlx::Row;
use std::sync::Arc;
use url::Url;

pub fn clipping_routes() -> Router {
    // Mutation routes — strict rate limiting (10/min per IP)
    let mutation_routes = Router::new()
        .route("/api/clipping/jobs/:id/cancel", post(cancel_job))
        .route("/api/clipping/jobs/:id/retry", post(retry_job))
        .route("/api/clipping/jobs/:id/reset", post(reset_job))
        .route("/api/clipping/clips/:id/repost", post(repost_clip))
        .layer(axum::middleware::from_fn(strict_rate_limit_middleware))
        .layer(axum::middleware::from_fn(clipping_access_middleware))
        .layer(axum::middleware::from_fn(auth_middleware));

    // Protected API routes
    let api_routes = Router::new()
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
        // Extracted clips
        .route("/api/clipping/clips", get(list_clips))
        .route("/api/clipping/clips/:id", get(get_clip_details))
        // Clip review system
        .route(
            "/api/clipping/clips/pending-review",
            get(list_pending_review_clips),
        )
        .route(
            "/api/clipping/clips/:id/approve",
            axum::routing::put(approve_clip),
        )
        .route(
            "/api/clipping/clips/:id/reject",
            axum::routing::put(reject_clip),
        )
        .route(
            "/api/clipping/clips/:id/propose-edit",
            axum::routing::put(propose_edit_clip),
        )
        // Access check endpoint
        .route("/api/clipping/access-check", get(check_access))
        // Twitch source channels
        .route(
            "/api/clipping/twitch/source-channels",
            get(list_twitch_source_channels).post(add_twitch_source_channel),
        )
        .route(
            "/api/clipping/twitch/source-channels/search",
            post(search_twitch_channels),
        )
        .route(
            "/api/clipping/twitch/source-channels/:id",
            delete(remove_twitch_source_channel),
        )
        // Twitch ↔ YouTube mappings
        .route(
            "/api/clipping/twitch/mappings",
            get(list_twitch_mappings).post(create_twitch_mapping),
        )
        .route(
            "/api/clipping/twitch/mappings/:id",
            delete(delete_twitch_mapping),
        )
        // WebSocket route — protected by auth + clipping access
        .route("/ws/clipping-jobs/:job_id", get(ws_clipping_job_progress))
        // All API routes protected
        .layer(axum::middleware::from_fn(clipping_access_middleware))
        .layer(axum::middleware::from_fn(auth_middleware));

    mutation_routes.merge(api_routes)
}

/// WebSocket handler — subscribe to real-time progress for a specific clipping job.
///
/// Connects to JobManager using job_id.to_string() as the routing key.
/// The agent sends ProgressUpdate messages via job_manager.send_progress(job_id.to_string(), …).
/// This handler registers a receiver and forwards updates to the connected client as JSON.
///
/// Usage: `wss://host/ws/clipping-jobs/123?token=<jwt>`
async fn ws_clipping_job_progress(
    ws: WebSocketUpgrade,
    Path(job_id): Path<i32>,
    Extension(state): Extension<Arc<AppState>>,
    Extension(claims): Extension<Claims>,
) -> Result<impl IntoResponse, StatusCode> {
    let user_id: i32 = claims.sub.parse().map_err(|_| StatusCode::UNAUTHORIZED)?;

    // Verify the job belongs to this user
    let owns_job: bool = sqlx::query_scalar(
        "SELECT EXISTS(
            SELECT 1 FROM clipping_jobs cj
            JOIN youtube_channel_linkages l ON l.id = cj.linkage_id
            WHERE cj.id = $1 AND l.user_id = $2
        )",
    )
    .bind(job_id)
    .bind(user_id)
    .fetch_one(&state.db_pool)
    .await
    .unwrap_or(false);

    if !owns_job {
        return Err(StatusCode::FORBIDDEN);
    }

    Ok(ws.on_upgrade(move |socket| clipping_job_ws(socket, state, job_id)))
}

async fn clipping_job_ws(stream: WebSocket, state: Arc<AppState>, job_id: i32) {
    let (mut sender, mut receiver) = stream.split();
    let session_key = job_id.to_string();

    tracing::info!("🔌 WebSocket connected for clipping job {}", job_id);

    // Subscribe to progress updates — combines in-memory + Redis paths.
    let (progress_tx, mut progress_rx) = tokio::sync::mpsc::unbounded_channel::<String>();

    // Path 1: In-memory bridge (same-instance)
    {
        let (pg_tx, mut pg_rx) = tokio::sync::mpsc::unbounded_channel::<crate::jobs::ProgressUpdate>();
        state
            .job_manager
            .register_progress_sender(session_key.clone(), pg_tx)
            .await;
        let prog_tx = progress_tx.clone();
        tokio::spawn(async move {
            while let Some(update) = pg_rx.recv().await {
                if let Ok(json) = serde_json::to_string(&update) {
                    if prog_tx.send(json).is_err() {
                        break;
                    }
                }
            }
        });
    }

    // Path 2: Redis pub/sub (cross-instance)
    if let Some(ref bus) = state.pubsub_bus {
        if let Ok(redis_rx) = bus.subscribe(&format!("progress:{}", session_key)).await {
            tracing::info!("📡 Subscribed to Redis progress for clipping job {}", job_id);
            let prog_tx = progress_tx.clone();
            tokio::spawn(async move {
                while let Some(msg) = redis_rx.recv().await {
                    if prog_tx.send(msg).is_err() {
                        break;
                    }
                }
            });
        } else {
            tracing::warn!("Failed to subscribe to Redis progress");
        }
    }

    // Forward progress updates to the WebSocket client
    let send_loop = async {
        loop {
            tokio::select! {
                // Progress from agent (parsed from JSON string)
                Some(msg) = progress_rx.recv() => {
                    if sender.send(Message::Text(msg)).await.is_err() {
                        break; // Client disconnected
                    }
                }
                // Handle incoming client messages (ping/pong or close)
                msg = receiver.next() => {
                    match msg {
                        Some(Ok(Message::Close(_))) | None => break,
                        Some(Ok(Message::Ping(data))) => {
                            let _ = sender.send(Message::Pong(data)).await;
                        }
                        _ => {} // Ignore other client messages
                    }
                }
            }
        }
    };

    send_loop.await;

    tracing::info!("🔌 WebSocket disconnected for clipping job {}", job_id);
}

// Source Channel Handlers

async fn list_source_channels(
    Extension(state): Extension<Arc<AppState>>,
) -> Result<Json<Value>, StatusCode> {
    let channels = sqlx::query_as::<_, SourceChannel>(
        "SELECT * FROM youtube_source_channels ORDER BY created_at DESC",
    )
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
            return path[handle_pos + 1..]
                .split('/')
                .next()
                .unwrap_or("")
                .to_string();
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
    let input = payload
        .channel_url
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

    let channel = channel_info.items.first().ok_or(StatusCode::NOT_FOUND)?;

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

    // 🆕 THREE-WAY MAPPING REQUIREMENT:
    // Every source channel must have Twitch + Kick.com equivalents.
    // If either is missing, the channel is rejected.

    // 1. Try Twitch mapping
    let twitch_ok = if let (Some(twitch_client), Some(gemini)) = (
        state.twitch_client.as_ref(),
        state.video_gemini_client.as_ref().or(state.gemini_client.as_ref()),
    ) {
        match crate::services::twitch_mapper::auto_map_youtube_to_twitch(
            &source_channel,
            twitch_client,
            gemini,
            &state.db_pool,
        )
        .await
        {
            Ok(crate::services::twitch_mapper::MappingResult::Mapped(_)) => true,
            _ => false,
        }
    } else {
        false
    };

    // 2. Try Kick mapping
    let kick_ok = if state.kick_client.is_some() {
        crate::services::kick_mapper::auto_map_kick_channel(
            &state,
            &source_channel.channel_name,
            "youtube",
            source_channel.id,
        )
        .await
        .is_ok()
    } else {
        false
    };

    // 3. Reject if not all three platforms exist
    if !twitch_ok || !kick_ok {
        // Rollback: delete the source channel and its schedule
        let _ = sqlx::query("DELETE FROM clipping_poll_schedule WHERE source_channel_id = $1")
            .bind(source_channel.id)
            .execute(&state.db_pool)
            .await;
        let _ = sqlx::query("DELETE FROM youtube_source_channels WHERE id = $1")
            .bind(source_channel.id)
            .execute(&state.db_pool)
            .await;

        let mut missing = Vec::new();
        if !twitch_ok {
            missing.push("Twitch");
        }
        if !kick_ok {
            missing.push("Kick.com");
        }

        return Ok(Json(json!({
            "success": false,
            "reason": "three_way_mapping_required",
            "message": format!(
                "This creator does not have {} account(s). A YouTube channel must have both a Twitch and a Kick.com account to be a valid source channel.",
                missing.join(" and ")
            ),
        })));
    }

    Ok(Json(json!({
        "success": true,
        "channel": source_channel
    })))
}

async fn get_source_channel(
    Extension(state): Extension<Arc<AppState>>,
    Path(id): Path<i32>,
) -> Result<Json<Value>, StatusCode> {
    let channel =
        sqlx::query_as::<_, SourceChannel>("SELECT * FROM youtube_source_channels WHERE id = $1")
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
        sqlx::query(
            "UPDATE youtube_source_channels SET polling_interval_minutes = $1 WHERE id = $2",
        )
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
         ORDER BY l.created_at DESC",
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
    .bind(payload.clips_per_video.unwrap_or(3))
    .bind(payload.min_clip_duration_seconds.unwrap_or(15))
    .bind(payload.max_clip_duration_seconds.unwrap_or(60))
    .fetch_one(&state.db_pool)
    .await
    .map_err(|e| {
        if let sqlx::Error::Database(db_err) = &e {
            if db_err.code().as_deref() == Some("23505") {
                return StatusCode::CONFLICT;
            }
        }
        StatusCode::INTERNAL_SERVER_ERROR
    })?;

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
        "SELECT * FROM youtube_channel_linkages WHERE id = $1 AND user_id = $2",
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
        "SELECT EXISTS(SELECT 1 FROM youtube_channel_linkages WHERE id = $1 AND user_id = $2)",
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
        sqlx::query(
            "UPDATE youtube_channel_linkages SET is_active = $1 WHERE id = $2 AND user_id = $3",
        )
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
    #[allow(dead_code)]
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

    let mut enriched_jobs = Vec::with_capacity(jobs.len());
    for job in jobs {
        let mut job_value =
            serde_json::to_value(&job).map_err(|_| StatusCode::INTERNAL_SERVER_ERROR)?;

        let fallback_delivery = if let Some(delivery_id) = job.fallback_delivery_id {
            sqlx::query(
                "SELECT id, title, status, output_r2_url, output_filename, error_message,
                        created_at, completed_at
                 FROM deliveries
                 WHERE id = $1",
            )
            .bind(delivery_id)
            .fetch_optional(&state.db_pool)
            .await
            .map_err(|_| StatusCode::INTERNAL_SERVER_ERROR)?
            .map(|row| {
                let delivery_uuid: uuid::Uuid = row.get("id");
                json!({
                    "id": delivery_uuid.to_string(),
                    "delivery_page_url": format!("/delivery/{}", delivery_uuid),
                    "title": row.get::<String, _>("title"),
                    "status": row.get::<String, _>("status"),
                    "output_r2_url": row.try_get::<Option<String>, _>("output_r2_url").ok().flatten(),
                    "output_filename": row.try_get::<Option<String>, _>("output_filename").ok().flatten(),
                    "error_message": row.try_get::<Option<String>, _>("error_message").ok().flatten(),
                    "created_at": row.get::<chrono::DateTime<chrono::Utc>, _>("created_at").to_rfc3339(),
                    "completed_at": row.try_get::<Option<chrono::DateTime<chrono::Utc>>, _>("completed_at").ok().flatten().map(|d| d.to_rfc3339()),
                })
            })
        } else {
            None
        };

        if let Some(object) = job_value.as_object_mut() {
            object.insert("fallback_delivery".to_string(), json!(fallback_delivery));
        }
        enriched_jobs.push(job_value);
    }

    Ok(Json(json!({
        "success": true,
        "jobs": enriched_jobs
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
         WHERE cj.id = $1 AND ycl.user_id = $2",
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

    let fallback_delivery = if let Some(delivery_id) = job.fallback_delivery_id {
        sqlx::query(
            "SELECT id, title, status, output_r2_url, output_filename, error_message, source_url,
                    extra_args,
                    created_at, completed_at
             FROM deliveries
             WHERE id = $1",
        )
        .bind(delivery_id)
        .fetch_optional(&state.db_pool)
        .await
        .map_err(|_| StatusCode::INTERNAL_SERVER_ERROR)?
        .map(|row| {
            let delivery_uuid: uuid::Uuid = row.get("id");
            let extra_args = row
                .try_get::<Option<serde_json::Value>, _>("extra_args")
                .ok()
                .flatten()
                .unwrap_or_else(|| json!({}));
            json!({
                "id": delivery_uuid.to_string(),
                "delivery_page_url": format!("/delivery/{}", delivery_uuid),
                "title": row.get::<String, _>("title"),
                "status": row.get::<String, _>("status"),
                "output_r2_url": row.try_get::<Option<String>, _>("output_r2_url").ok().flatten(),
                "output_filename": row.try_get::<Option<String>, _>("output_filename").ok().flatten(),
                "error_message": row.try_get::<Option<String>, _>("error_message").ok().flatten(),
                "source_url": row.try_get::<Option<String>, _>("source_url").ok().flatten(),
                "youtube_video_id": extra_args.get("youtube_video_id").and_then(|value| value.as_str()),
                "youtube_url": extra_args.get("youtube_url").and_then(|value| value.as_str()),
                "published_at": extra_args.get("published_at").and_then(|value| value.as_str()),
                "created_at": row.get::<chrono::DateTime<chrono::Utc>, _>("created_at").to_rfc3339(),
                "completed_at": row.try_get::<Option<chrono::DateTime<chrono::Utc>>, _>("completed_at").ok().flatten().map(|d| d.to_rfc3339()),
                "fallback_strategy": job.fallback_strategy,
                "fallback_activated_at": job.fallback_activated_at.map(|d| d.to_rfc3339()),
            })
        })
    } else {
        None
    };

    Ok(Json(json!({
        "success": true,
        "job": job,
        "clips": clips,
        "fallback_delivery": fallback_delivery
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
         AND cj.status NOT IN ('completed', 'failed')",
    )
    .bind(id)
    .bind(user_id)
    .execute(&state.db_pool)
    .await
    .map_err(|_| StatusCode::INTERNAL_SERVER_ERROR)?;

    if result.rows_affected() == 0 {
        return Err(StatusCode::NOT_FOUND);
    }

    if let Ok(Some(Some(workflow_id))) = sqlx::query_scalar::<_, Option<uuid::Uuid>>(
        "SELECT workflow_id FROM clipping_jobs WHERE id = $1",
    )
    .bind(id)
    .fetch_optional(&state.db_pool)
    .await
    {
        let workflow_runtime = crate::services::WorkflowRuntime::new(state.db_pool.clone());
        let _ = workflow_runtime
            .mark_cancelled(
                workflow_id,
                Some("cancelled"),
                "Auto clipping workflow was cancelled by the user.",
            )
            .await;
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
         AND cj.status = 'failed'",
    )
    .bind(id)
    .bind(user_id)
    .execute(&state.db_pool)
    .await
    .map_err(|_| StatusCode::INTERNAL_SERVER_ERROR)?;

    if result.rows_affected() == 0 {
        return Err(StatusCode::NOT_FOUND);
    }

    tracing::info!(
        "🔄 Job {} reset to pending for retry by user {}",
        id,
        user_id
    );

    if let Ok(Some(Some(workflow_id))) = sqlx::query_scalar::<_, Option<uuid::Uuid>>(
        "SELECT workflow_id FROM clipping_jobs WHERE id = $1",
    )
    .bind(id)
    .fetch_optional(&state.db_pool)
    .await
    {
        let workflow_runtime = crate::services::WorkflowRuntime::new(state.db_pool.clone());
        let _ = workflow_runtime
            .mark_retrying(
                workflow_id,
                Some("queued"),
                1,
                "Auto clipping workflow was reset to pending for retry by the user.",
            )
            .await;
    }

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
         AND cj.status IN ('downloading', 'analyzing', 'extracting_clips', 'posting', 'failed')",
    )
    .bind(id)
    .bind(user_id)
    .execute(&state.db_pool)
    .await
    .map_err(|_| StatusCode::INTERNAL_SERVER_ERROR)?;

    if result.rows_affected() == 0 {
        return Err(StatusCode::NOT_FOUND);
    }

    tracing::info!(
        "🔄 Job {} manually reset to pending by user {} (unstuck operation)",
        id,
        user_id
    );

    if let Ok(Some(Some(workflow_id))) = sqlx::query_scalar::<_, Option<uuid::Uuid>>(
        "SELECT workflow_id FROM clipping_jobs WHERE id = $1",
    )
    .bind(id)
    .fetch_optional(&state.db_pool)
    .await
    {
        let workflow_runtime = crate::services::WorkflowRuntime::new(state.db_pool.clone());
        let _ = workflow_runtime
            .mark_retrying(
                workflow_id,
                Some("queued"),
                1,
                "Auto clipping workflow was manually reset to pending after getting stuck.",
            )
            .await;
    }

    Ok(Json(json!({
        "success": true,
        "message": "Job reset to pending and queued for processing"
    })))
}

// Extracted Clip Handlers

#[derive(Deserialize)]
struct ClipQueryParams {
    #[allow(dead_code)]
    job_id: Option<i32>,
    #[allow(dead_code)]
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
         WHERE ec.id = $1 AND ycl.user_id = $2",
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
         AND ycl.user_id = $2",
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

// ─────────────────────────── Twitch handlers ─────────────────────────────────

/// POST /api/clipping/twitch/source-channels/search
/// Search Twitch for channels matching a query. Does NOT write to DB.
async fn search_twitch_channels(
    Extension(state): Extension<Arc<AppState>>,
    Json(body): Json<crate::clipping::models::SearchTwitchChannelsRequest>,
) -> impl IntoResponse {
    let twitch = match &state.twitch_client {
        Some(c) => c,
        None => {
            return (
                StatusCode::SERVICE_UNAVAILABLE,
                Json(json!({"error": "Twitch client not configured"})),
            )
                .into_response();
        }
    };

    match twitch.search_channels(&body.query, 10).await {
        Ok(channels) => (StatusCode::OK, Json(json!({"channels": channels}))).into_response(),
        Err(e) => (
            StatusCode::BAD_GATEWAY,
            Json(json!({"error": format!("Twitch search failed: {}", e)})),
        )
            .into_response(),
    }
}

/// POST /api/clipping/twitch/source-channels
/// Add a Twitch channel to twitch_source_channels table.
async fn add_twitch_source_channel(
    Extension(state): Extension<Arc<AppState>>,
    Json(body): Json<crate::clipping::models::AddTwitchSourceChannelRequest>,
) -> impl IntoResponse {
    let twitch = match &state.twitch_client {
        Some(c) => c,
        None => {
            return (
                StatusCode::SERVICE_UNAVAILABLE,
                Json(json!({"error": "Twitch client not configured"})),
            )
                .into_response();
        }
    };

    // Fetch full channel metadata from Twitch by broadcaster_id via user lookup
    // (broadcaster_id is numeric; look up by querying /users?id= instead of ?login=)
    let user_resp = state
        .twitch_client
        .as_ref()
        .unwrap()
        .get_user_by_login(&body.broadcaster_id)
        .await;

    // Actually we need to look up by broadcaster ID. The TwitchClient only has get_user_by_login.
    // We'll search channels instead to resolve the metadata.
    let channel = match twitch.search_channels(&body.broadcaster_id, 1).await {
        Ok(results) if !results.is_empty() => {
            // Match exact broadcaster_id
            results
                .into_iter()
                .find(|c| c.broadcaster_id == body.broadcaster_id)
                .or_else(|| {
                    // fallback: just use the first result
                    None
                })
        }
        _ => None,
    };

    let channel = match channel {
        Some(c) => c,
        None => {
            // Try by login name as fallback
            match user_resp {
                Ok(Some(u)) => u,
                _ => {
                    return (
                        StatusCode::NOT_FOUND,
                        Json(json!({"error": "Twitch channel not found"})),
                    )
                        .into_response();
                }
            }
        }
    };

    match sqlx::query_as::<_, crate::clipping::models::TwitchSourceChannel>(
        "INSERT INTO twitch_source_channels
             (broadcaster_id, broadcaster_login, display_name, profile_image_url)
         VALUES ($1, $2, $3, $4)
         ON CONFLICT (broadcaster_id) DO UPDATE
           SET broadcaster_login = EXCLUDED.broadcaster_login,
               display_name      = EXCLUDED.display_name,
               profile_image_url = EXCLUDED.profile_image_url,
               updated_at        = NOW()
         RETURNING *",
    )
    .bind(&channel.broadcaster_id)
    .bind(&channel.broadcaster_login)
    .bind(&channel.display_name)
    .bind(&channel.profile_image_url)
    .fetch_one(&state.db_pool)
    .await
    {
        Ok(row) => (StatusCode::CREATED, Json(json!(row))).into_response(),
        Err(e) => (
            StatusCode::INTERNAL_SERVER_ERROR,
            Json(json!({"error": format!("DB insert failed: {}", e)})),
        )
            .into_response(),
    }
}

/// GET /api/clipping/twitch/source-channels
/// List all Twitch source channels, annotated with their mapped YouTube channel (if any).
async fn list_twitch_source_channels(
    Extension(state): Extension<Arc<AppState>>,
) -> impl IntoResponse {
    let rows = sqlx::query(
        "SELECT tsc.*,
                ysc.channel_name AS mapped_youtube_channel_name
         FROM twitch_source_channels tsc
         LEFT JOIN youtube_twitch_channel_mappings ytm ON ytm.twitch_source_channel_id = tsc.id
         LEFT JOIN youtube_source_channels ysc ON ysc.id = ytm.youtube_source_channel_id
         ORDER BY tsc.created_at DESC",
    )
    .fetch_all(&state.db_pool)
    .await;

    match rows {
        Ok(rows) => {
            let channels: Vec<Value> = rows
                .iter()
                .map(|r| {
                    json!({
                        "id": r.try_get::<i32, _>("id").unwrap_or(0),
                        "broadcaster_id": r.try_get::<String, _>("broadcaster_id").unwrap_or_default(),
                        "broadcaster_login": r.try_get::<String, _>("broadcaster_login").unwrap_or_default(),
                        "display_name": r.try_get::<String, _>("display_name").unwrap_or_default(),
                        "profile_image_url": r.try_get::<Option<String>, _>("profile_image_url").ok().flatten(),
                        "is_active": r.try_get::<bool, _>("is_active").unwrap_or(true),
                        "mapped_youtube_channel_name": r.try_get::<Option<String>, _>("mapped_youtube_channel_name").ok().flatten(),
                        "created_at": r.try_get::<DateTime<Utc>, _>("created_at").ok(),
                    })
                })
                .collect();
            Json(json!({"channels": channels})).into_response()
        }
        Err(e) => (
            StatusCode::INTERNAL_SERVER_ERROR,
            Json(json!({"error": format!("DB query failed: {}", e)})),
        )
            .into_response(),
    }
}

/// DELETE /api/clipping/twitch/source-channels/:id
async fn remove_twitch_source_channel(
    Path(id): Path<i32>,
    Extension(state): Extension<Arc<AppState>>,
) -> impl IntoResponse {
    match sqlx::query("DELETE FROM twitch_source_channels WHERE id = $1")
        .bind(id)
        .execute(&state.db_pool)
        .await
    {
        Ok(r) if r.rows_affected() > 0 => {
            Json(json!({"message": "Twitch channel removed"})).into_response()
        }
        Ok(_) => (StatusCode::NOT_FOUND, Json(json!({"error": "Not found"}))).into_response(),
        Err(e) => (
            StatusCode::INTERNAL_SERVER_ERROR,
            Json(json!({"error": format!("DB error: {}", e)})),
        )
            .into_response(),
    }
}

/// POST /api/clipping/twitch/mappings
/// Manually create a YouTube ↔ Twitch mapping.
async fn create_twitch_mapping(
    Extension(state): Extension<Arc<AppState>>,
    Json(body): Json<crate::clipping::models::CreateTwitchMappingRequest>,
) -> impl IntoResponse {
    // Verify both IDs exist
    let yt_exists: Option<(i32,)> =
        sqlx::query_as::<_, (i32,)>("SELECT id FROM youtube_source_channels WHERE id = $1")
            .bind(body.youtube_source_channel_id)
            .fetch_optional(&state.db_pool)
            .await
            .unwrap_or(None);
    if yt_exists.is_none() {
        return (
            StatusCode::NOT_FOUND,
            Json(json!({"error": "YouTube source channel not found"})),
        )
            .into_response();
    }

    let tw_exists: Option<(i32,)> =
        sqlx::query_as::<_, (i32,)>("SELECT id FROM twitch_source_channels WHERE id = $1")
            .bind(body.twitch_source_channel_id)
            .fetch_optional(&state.db_pool)
            .await
            .unwrap_or(None);
    if tw_exists.is_none() {
        return (
            StatusCode::NOT_FOUND,
            Json(json!({"error": "Twitch source channel not found"})),
        )
            .into_response();
    }

    match sqlx::query(
        "INSERT INTO youtube_twitch_channel_mappings
             (youtube_source_channel_id, twitch_source_channel_id)
         VALUES ($1, $2)",
    )
    .bind(body.youtube_source_channel_id)
    .bind(body.twitch_source_channel_id)
    .execute(&state.db_pool)
    .await
    {
        Ok(_) => {
            // Update twitch_mapping_status on the YouTube channel
            sqlx::query(
                "UPDATE youtube_source_channels SET twitch_mapping_status = 'mapped' WHERE id = $1",
            )
            .bind(body.youtube_source_channel_id)
            .execute(&state.db_pool)
            .await
            .ok();

            (
                StatusCode::CREATED,
                Json(json!({"message": "Mapping created"})),
            )
                .into_response()
        }
        Err(e) if e.to_string().contains("duplicate") || e.to_string().contains("unique") => (
            StatusCode::CONFLICT,
            Json(json!({"error": "Mapping already exists for this YouTube channel"})),
        )
            .into_response(),
        Err(e) => (
            StatusCode::INTERNAL_SERVER_ERROR,
            Json(json!({"error": format!("DB error: {}", e)})),
        )
            .into_response(),
    }
}

/// GET /api/clipping/twitch/mappings
async fn list_twitch_mappings(Extension(state): Extension<Arc<AppState>>) -> impl IntoResponse {
    let rows = sqlx::query(
        "SELECT ytm.id,
                ytm.youtube_source_channel_id,
                ytm.twitch_source_channel_id,
                ytm.created_at,
                ysc.channel_name AS youtube_channel_name,
                tsc.broadcaster_login,
                tsc.display_name AS twitch_display_name
         FROM youtube_twitch_channel_mappings ytm
         JOIN youtube_source_channels ysc ON ysc.id = ytm.youtube_source_channel_id
         JOIN twitch_source_channels tsc ON tsc.id = ytm.twitch_source_channel_id
         ORDER BY ytm.created_at DESC",
    )
    .fetch_all(&state.db_pool)
    .await;

    match rows {
        Ok(rows) => {
            let mappings: Vec<Value> = rows
                .iter()
                .map(|r| {
                    json!({
                        "id": r.try_get::<i32, _>("id").unwrap_or(0),
                        "youtube_source_channel_id": r.try_get::<i32, _>("youtube_source_channel_id").unwrap_or(0),
                        "twitch_source_channel_id": r.try_get::<i32, _>("twitch_source_channel_id").unwrap_or(0),
                        "youtube_channel_name": r.try_get::<String, _>("youtube_channel_name").unwrap_or_default(),
                        "broadcaster_login": r.try_get::<String, _>("broadcaster_login").unwrap_or_default(),
                        "twitch_display_name": r.try_get::<String, _>("twitch_display_name").unwrap_or_default(),
                        "created_at": r.try_get::<DateTime<Utc>, _>("created_at").ok(),
                    })
                })
                .collect();
            Json(json!({"mappings": mappings})).into_response()
        }
        Err(e) => (
            StatusCode::INTERNAL_SERVER_ERROR,
            Json(json!({"error": format!("DB error: {}", e)})),
        )
            .into_response(),
    }
}

// ─────────────────────────── Clip Review Handlers ────────────────────────────

/// GET /api/clipping/clips/pending-review
/// List all clips with review_status = 'pending_review' owned by the current user.
async fn list_pending_review_clips(
    Extension(state): Extension<Arc<AppState>>,
    Extension(claims): Extension<Claims>,
) -> Result<Json<Value>, StatusCode> {
    let user_id = claims.sub.parse::<i32>().unwrap_or(0);

    let rows = sqlx::query(
        "SELECT ec.*, cj.source_video_title
         FROM extracted_clips ec
         JOIN clipping_jobs cj ON ec.clipping_job_id = cj.id
         JOIN youtube_channel_linkages ycl ON cj.linkage_id = ycl.id
         WHERE ycl.user_id = $1 AND ec.review_status = 'pending_review'
         ORDER BY ec.created_at DESC",
    )
    .bind(user_id)
    .fetch_all(&state.db_pool)
    .await
    .map_err(|_| StatusCode::INTERNAL_SERVER_ERROR)?;

    let clips: Vec<Value> = rows
        .iter()
        .map(|r| {
            json!({
                "id": r.try_get::<i32, _>("id").unwrap_or(0),
                "clipping_job_id": r.try_get::<i32, _>("clipping_job_id").unwrap_or(0),
                "clip_number": r.try_get::<i32, _>("clip_number").unwrap_or(0),
                "local_clip_path": r.try_get::<String, _>("local_clip_path").unwrap_or_default(),
                "duration_seconds": r.try_get::<f64, _>("duration_seconds").unwrap_or(0.0),
                "ai_title": r.try_get::<Option<String>, _>("ai_title").ok().flatten(),
                "proposed_title": r.try_get::<Option<String>, _>("proposed_title").ok().flatten(),
                "proposed_description": r.try_get::<Option<String>, _>("proposed_description").ok().flatten(),
                "review_status": r.try_get::<String, _>("review_status").unwrap_or_default(),
                "qa_status": r.try_get::<String, _>("qa_status").unwrap_or_else(|_| "not_reviewed".to_string()),
                "qa_score": r.try_get::<Option<i32>, _>("qa_score").ok().flatten(),
                "qa_feedback": r.try_get::<Option<String>, _>("qa_feedback").ok().flatten(),
                "qa_retry_hint": r.try_get::<Option<String>, _>("qa_retry_hint").ok().flatten(),
                "source_video_title": r.try_get::<Option<String>, _>("source_video_title").ok().flatten(),
                "created_at": r.try_get::<DateTime<Utc>, _>("created_at").ok(),
                "download_url": r.try_get::<Option<String>, _>("r2_clip_url").ok().flatten(),
            })
        })
        .collect();

    Ok(Json(json!({
        "success": true,
        "clips": clips,
        "count": clips.len()
    })))
}

#[derive(Deserialize)]
struct RejectClipRequest {
    reason: Option<String>,
}

#[derive(Deserialize)]
struct ProposeEditRequest {
    proposed_title: Option<String>,
    proposed_description: Option<String>,
}

/// PUT /api/clipping/clips/:id/approve
/// Approve a pending-review clip: trigger actual YouTube upload now.
async fn approve_clip(
    Extension(state): Extension<Arc<AppState>>,
    Extension(claims): Extension<Claims>,
    Path(id): Path<i32>,
) -> Result<Json<Value>, StatusCode> {
    let user_id = claims.sub.parse::<i32>().unwrap_or(0);

    // Verify ownership and fetch clip + linkage info in one query
    let row = sqlx::query(
        "SELECT ec.id, ec.local_clip_path, ec.proposed_title, ec.proposed_description,
                ec.review_status, ec.ai_tags, ec.custom_thumbnail_path,
                ec.clip_number,
                ycl.destination_channel_id, ycl.id AS linkage_id
         FROM extracted_clips ec
         JOIN clipping_jobs cj ON ec.clipping_job_id = cj.id
         JOIN youtube_channel_linkages ycl ON cj.linkage_id = ycl.id
         WHERE ec.id = $1 AND ycl.user_id = $2",
    )
    .bind(id)
    .bind(user_id)
    .fetch_optional(&state.db_pool)
    .await
    .map_err(|_| StatusCode::INTERNAL_SERVER_ERROR)?
    .ok_or(StatusCode::NOT_FOUND)?;

    let review_status: String = row.try_get("review_status").unwrap_or_default();
    if review_status != "pending_review" {
        return Err(StatusCode::CONFLICT);
    }

    let local_clip_path: String = row.try_get("local_clip_path").unwrap_or_default();
    let proposed_title: Option<String> = row.try_get("proposed_title").unwrap_or(None);
    let proposed_description: Option<String> = row.try_get("proposed_description").unwrap_or(None);
    let ai_tags: Option<serde_json::Value> = row.try_get("ai_tags").unwrap_or(None);
    let custom_thumbnail_path: Option<String> =
        row.try_get("custom_thumbnail_path").unwrap_or(None);
    let clip_number: i32 = row.try_get("clip_number").unwrap_or(0);
    let destination_channel_id: i32 = row.try_get("destination_channel_id").unwrap_or(0);

    // Fetch destination channel with tokens
    let dest_channel = sqlx::query_as::<_, crate::models::youtube::ConnectedYouTubeChannel>(
        "SELECT * FROM connected_youtube_channels WHERE id = $1",
    )
    .bind(destination_channel_id)
    .fetch_optional(&state.db_pool)
    .await
    .map_err(|_| StatusCode::INTERNAL_SERVER_ERROR)?
    .ok_or(StatusCode::NOT_FOUND)?;

    let youtube_client = state
        .youtube_client
        .as_ref()
        .ok_or(StatusCode::SERVICE_UNAVAILABLE)?;

    let oauth_client_id = state
        .google_oauth_client_id
        .as_ref()
        .ok_or(StatusCode::SERVICE_UNAVAILABLE)?;

    let oauth_client_secret = state
        .google_oauth_client_secret
        .as_ref()
        .ok_or(StatusCode::SERVICE_UNAVAILABLE)?;

    let uploader = ClipUploader::new(
        Arc::new(youtube_client.clone()),
        state.db_pool.clone(),
        oauth_client_id.clone(),
        oauth_client_secret.clone(),
    );

    // Build an ExtractedClipData from the DB row for the uploader
    let title = proposed_title.unwrap_or_else(|| format!("Clip {}", clip_number));
    let description = proposed_description.unwrap_or_default();
    let tags: Vec<String> = ai_tags
        .as_ref()
        .and_then(|v| serde_json::from_value(v.clone()).ok())
        .unwrap_or_default();

    let clip_data = crate::clipping::ai_clipper::ExtractedClipData {
        clip_number,
        local_clip_path: local_clip_path.clone(),
        start_time_seconds: 0.0,
        end_time_seconds: 0.0,
        duration_seconds: 0.0,
        ai_title: title,
        ai_description: description,
        ai_tags: tags,
        ai_confidence_score: 1.0,
        viral_factors: vec![],
        custom_thumbnail_path: custom_thumbnail_path.clone(),
        thumbnail_generation_method: custom_thumbnail_path.as_ref().map(|_| "manual".to_string()),
        enhancement_applied: false,
        enhancement_tools: Vec::new(),
        enhancement_reasoning: None,
        r2_clip_key: None,
        r2_thumb_key: None,
        r2_clip_url: None,
        qa_status: None,
        qa_score: None,
        qa_feedback: None,
        qa_retry_hint: None,
    };

    // Upload — requires_human_approval=false so it goes through immediately
    match uploader
        .upload_clip(&clip_data, id, &dest_channel, false)
        .await
    {
        Ok(result) => {
            // Mark as approved in DB
            sqlx::query(
                "UPDATE extracted_clips
                 SET review_status = 'approved',
                     reviewed_by = $1,
                     reviewed_at = NOW(),
                     updated_at = NOW()
                 WHERE id = $2",
            )
            .bind(user_id)
            .bind(id)
            .execute(&state.db_pool)
            .await
            .map_err(|_| StatusCode::INTERNAL_SERVER_ERROR)?;

            Ok(Json(json!({
                "success": true,
                "youtube_video_id": result.video_id,
                "youtube_url": result.url
            })))
        }
        Err(e) => {
            tracing::error!("approve_clip: upload failed for clip {}: {}", id, e);
            Err(StatusCode::INTERNAL_SERVER_ERROR)
        }
    }
}

/// PUT /api/clipping/clips/:id/reject
/// Reject a pending-review clip with an optional reason.
async fn reject_clip(
    Extension(state): Extension<Arc<AppState>>,
    Extension(claims): Extension<Claims>,
    Path(id): Path<i32>,
    payload: Option<Json<RejectClipRequest>>,
) -> Result<Json<Value>, StatusCode> {
    let user_id = claims.sub.parse::<i32>().unwrap_or(0);

    let reason = payload.and_then(|p| p.reason.clone());

    let result = sqlx::query(
        "UPDATE extracted_clips ec
         SET review_status = 'rejected',
             review_notes = $1,
             reviewed_by = $2,
             reviewed_at = NOW(),
             upload_status = 'rejected',
             updated_at = NOW()
         FROM clipping_jobs cj
         JOIN youtube_channel_linkages ycl ON cj.linkage_id = ycl.id
         WHERE ec.id = $3
           AND ec.clipping_job_id = cj.id
           AND ycl.user_id = $2",
    )
    .bind(&reason)
    .bind(user_id)
    .bind(id)
    .execute(&state.db_pool)
    .await
    .map_err(|_| StatusCode::INTERNAL_SERVER_ERROR)?;

    if result.rows_affected() == 0 {
        return Err(StatusCode::NOT_FOUND);
    }

    Ok(Json(json!({"success": true, "message": "Clip rejected"})))
}

/// PUT /api/clipping/clips/:id/propose-edit
/// Update the proposed title/description before approving.
async fn propose_edit_clip(
    Extension(state): Extension<Arc<AppState>>,
    Extension(claims): Extension<Claims>,
    Path(id): Path<i32>,
    Json(payload): Json<ProposeEditRequest>,
) -> Result<Json<Value>, StatusCode> {
    let user_id = claims.sub.parse::<i32>().unwrap_or(0);

    // Verify ownership
    let exists = sqlx::query_scalar::<_, bool>(
        "SELECT EXISTS(
             SELECT 1 FROM extracted_clips ec
             JOIN clipping_jobs cj ON ec.clipping_job_id = cj.id
             JOIN youtube_channel_linkages ycl ON cj.linkage_id = ycl.id
             WHERE ec.id = $1 AND ycl.user_id = $2
         )",
    )
    .bind(id)
    .bind(user_id)
    .fetch_one(&state.db_pool)
    .await
    .map_err(|_| StatusCode::INTERNAL_SERVER_ERROR)?;

    if !exists {
        return Err(StatusCode::NOT_FOUND);
    }

    if let Some(ref title) = payload.proposed_title {
        sqlx::query(
            "UPDATE extracted_clips SET proposed_title = $1, updated_at = NOW() WHERE id = $2",
        )
        .bind(title)
        .bind(id)
        .execute(&state.db_pool)
        .await
        .map_err(|_| StatusCode::INTERNAL_SERVER_ERROR)?;
    }

    if let Some(ref desc) = payload.proposed_description {
        sqlx::query(
            "UPDATE extracted_clips SET proposed_description = $1, updated_at = NOW() WHERE id = $2",
        )
        .bind(desc)
        .bind(id)
        .execute(&state.db_pool)
        .await
        .map_err(|_| StatusCode::INTERNAL_SERVER_ERROR)?;
    }

    Ok(Json(
        json!({"success": true, "message": "Proposed edit saved"}),
    ))
}

/// DELETE /api/clipping/twitch/mappings/:id
async fn delete_twitch_mapping(
    Path(id): Path<i32>,
    Extension(state): Extension<Arc<AppState>>,
) -> impl IntoResponse {
    // Get youtube_source_channel_id before deleting so we can update mapping status
    let yt_id: Option<(i32,)> = sqlx::query_as::<_, (i32,)>(
        "SELECT youtube_source_channel_id FROM youtube_twitch_channel_mappings WHERE id = $1",
    )
    .bind(id)
    .fetch_optional(&state.db_pool)
    .await
    .unwrap_or(None);

    match sqlx::query("DELETE FROM youtube_twitch_channel_mappings WHERE id = $1")
        .bind(id)
        .execute(&state.db_pool)
        .await
    {
        Ok(r) if r.rows_affected() > 0 => {
            if let Some((yt_ch_id,)) = yt_id {
                sqlx::query(
                    "UPDATE youtube_source_channels SET twitch_mapping_status = 'unmapped' WHERE id = $1",
                )
                .bind(yt_ch_id)
                .execute(&state.db_pool)
                .await
                .ok();
            }
            Json(json!({"message": "Mapping deleted"})).into_response()
        }
        Ok(_) => (StatusCode::NOT_FOUND, Json(json!({"error": "Not found"}))).into_response(),
        Err(e) => (
            StatusCode::INTERNAL_SERVER_ERROR,
            Json(json!({"error": format!("DB error: {}", e)})),
        )
            .into_response(),
    }
}
