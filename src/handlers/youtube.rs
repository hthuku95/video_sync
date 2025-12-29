// YouTube integration handlers
// Handles OAuth connection, channel management, and video uploads

use crate::models::youtube::*;
use crate::youtube_client;
use crate::middleware::auth::auth_middleware;
use crate::AppState;
use axum::{
    extract::{Extension, Path, Query},
    http::StatusCode,
    response::{Html, Json, Redirect},
    routing::{get, post, delete, patch, put},
    Router,
};
use base64::Engine;
use serde::{Deserialize, Serialize};
use serde_json::json;
use std::sync::Arc;

pub fn youtube_routes() -> Router {
    // Public routes (no auth required)
    let public_routes = Router::new()
        .route("/youtube/callback", get(youtube_oauth_callback))
        .route("/youtube/manage", get(youtube_management_page))
        .route("/youtube/coming-soon", get(youtube_coming_soon_page));

    // Protected routes (auth required)
    let protected_routes = Router::new()
        // OAuth connection (requires auth to know which user)
        .route("/youtube/connect", get(initiate_youtube_connection))

        // Channel management (protected)
        .route("/api/youtube/channels", get(list_connected_channels))
        .route("/api/youtube/channels/:id/disconnect", delete(disconnect_channel))
        .route("/api/youtube/channels/:id/refresh", post(refresh_channel_token))

        // Video upload (protected)
        .route("/api/youtube/upload", post(upload_video_to_youtube))
        .route("/api/youtube/uploads", get(list_upload_history))

        // Video management (NEW)
        .route("/api/youtube/videos/:video_id", delete(delete_video_from_youtube))
        .route("/api/youtube/videos/:video_id", patch(update_video_metadata))
        .route("/api/youtube/videos/:video_id/thumbnail", post(upload_custom_thumbnail))
        .route("/api/youtube/videos/:video_id/thumbnail/generate", post(generate_and_upload_thumbnail))
        .route("/api/youtube/videos/:video_id/schedule", post(schedule_video_publish))

        // Playlist management (NEW)
        .route("/api/youtube/playlists", get(list_playlists))
        .route("/api/youtube/playlists", post(create_playlist))
        .route("/api/youtube/playlists/:id", patch(update_playlist))
        .route("/api/youtube/playlists/:id", delete(delete_playlist))
        .route("/api/youtube/playlists/:id/videos", post(add_video_to_playlist))
        .route("/api/youtube/playlists/:playlist_id/videos/:video_id", delete(remove_video_from_playlist))

        // Analytics (NEW)
        .route("/api/youtube/videos/:video_id/analytics", get(get_video_analytics))
        .route("/api/youtube/videos/:video_id/analytics/realtime", get(get_realtime_stats))
        .route("/api/youtube/channels/:id/analytics", get(get_channel_analytics))

        // Search & Discovery (NEW)
        .route("/api/youtube/search", get(search_videos))
        .route("/api/youtube/trending", get(get_trending_videos))
        .route("/api/youtube/videos/:video_id/related", get(get_related_videos))

        // Comment moderation (NEW)
        .route("/api/youtube/videos/:video_id/comments", get(get_video_comments))
        .route("/api/youtube/comments/:comment_id/reply", post(reply_to_comment))
        .route("/api/youtube/comments/:comment_id", delete(delete_comment))

        // Captions (NEW)
        .route("/api/youtube/videos/:video_id/captions", get(list_captions))
        .route("/api/youtube/videos/:video_id/captions", post(upload_caption))
        .route("/api/youtube/captions/:caption_id", delete(delete_caption))

        // Resumable uploads (NEW)
        .route("/api/youtube/upload/resumable", post(initiate_resumable_upload))
        .route("/api/youtube/upload/resumable/:upload_id/chunk", put(upload_chunk))
        .layer(axum::middleware::from_fn(crate::middleware::youtube_access::youtube_access_middleware))
        .layer(axum::middleware::from_fn(auth_middleware));

    // Merge public and protected routes (proper order)
    Router::new()
        .merge(public_routes)
        .merge(protected_routes)
}

#[derive(Deserialize)]
pub struct YouTubeConnectQuery {
    pub redirect_to: Option<String>,
}

#[derive(Deserialize)]
pub struct YouTubeCallbackQuery {
    pub code: Option<String>,
    pub state: Option<String>,
    pub error: Option<String>,
}

// ============================================================================
// YouTube OAuth Connection Flow
// ============================================================================

/// Initiate YouTube channel connection (OAuth flow)
/// Returns OAuth URL as JSON for JavaScript to redirect to
pub async fn initiate_youtube_connection(
    Query(params): Query<YouTubeConnectQuery>,
    Extension(state): Extension<Arc<AppState>>,
    Extension(claims): Extension<crate::models::auth::Claims>,
) -> Result<Json<serde_json::Value>, (StatusCode, Json<serde_json::Value>)> {
    let user_id = claims.sub.parse::<i32>().unwrap_or(0);

    // Check if Google OAuth is configured
    let client_id = state.google_oauth_client_id.as_ref().ok_or_else(|| {
        (
            StatusCode::SERVICE_UNAVAILABLE,
            Json(json!({
                "success": false,
                "message": "Google OAuth not configured"
            }))
        )
    })?;

    // Generate state parameter with user ID and redirect URL
    let state_data = json!({
        "user_id": user_id,
        "redirect_to": params.redirect_to.unwrap_or("/youtube/manage".to_string()),
        "timestamp": chrono::Utc::now().timestamp()
    });
    let state_param = base64::prelude::BASE64_URL_SAFE_NO_PAD.encode(state_data.to_string());

    // Required YouTube scopes (expanded for extended features)
    let scopes = [
        "https://www.googleapis.com/auth/youtube.upload",
        "https://www.googleapis.com/auth/youtube.readonly",
        "https://www.googleapis.com/auth/youtube.force-ssl",  // NEW: For delete, update, thumbnails, comments, captions
        "https://www.googleapis.com/auth/yt-analytics.readonly",  // NEW: For analytics data
        "https://www.googleapis.com/auth/userinfo.email",
        "https://www.googleapis.com/auth/userinfo.profile",
    ];

    let redirect_uri = std::env::var("GOOGLE_OAUTH_REDIRECT_URI")
        .unwrap_or_else(|_| "http://localhost:3000/youtube/callback".to_string());

    let auth_url = youtube_client::build_google_oauth_url(
        client_id,
        &redirect_uri,
        &scopes,
        &state_param,
    );

    tracing::info!("🔐 Initiating YouTube OAuth for user {} (can connect ANY Google account)", user_id);

    Ok(Json(json!({
        "success": true,
        "auth_url": auth_url,
        "message": "Redirect to Google OAuth"
    })))
}

/// Handle YouTube OAuth callback
pub async fn youtube_oauth_callback(
    Query(params): Query<YouTubeCallbackQuery>,
    Extension(state): Extension<Arc<AppState>>,
) -> Result<Html<String>, (StatusCode, Html<String>)> {
    // Check for OAuth error
    if let Some(error) = params.error {
        tracing::error!("YouTube OAuth error: {}", error);
        return Ok(Html(format!(
            r#"<!DOCTYPE html><html><head><title>Connection Failed</title></head>
            <body><h1>❌ Connection Failed</h1><p>Error: {}</p>
            <a href="/youtube/manage">Try Again</a></body></html>"#,
            error
        )));
    }

    let code = params.code.ok_or_else(|| {
        (
            StatusCode::BAD_REQUEST,
            Html("<h1>Missing authorization code</h1>".to_string())
        )
    })?;

    // Decode state parameter
    let state_json = params.state.ok_or_else(|| {
        (
            StatusCode::BAD_REQUEST,
            Html("<h1>Missing state parameter</h1>".to_string())
        )
    })?;

    let state_bytes = base64::prelude::BASE64_URL_SAFE_NO_PAD.decode(&state_json)
        .map_err(|_| (StatusCode::BAD_REQUEST, Html("<h1>Invalid state</h1>".to_string())))?;

    let state_str = String::from_utf8(state_bytes)
        .map_err(|_| (StatusCode::BAD_REQUEST, Html("<h1>Invalid state</h1>".to_string())))?;

    let state_data: serde_json::Value = serde_json::from_str(&state_str)
        .map_err(|_| (StatusCode::BAD_REQUEST, Html("<h1>Invalid state</h1>".to_string())))?;

    let user_id = state_data["user_id"].as_i64().ok_or_else(|| {
        (StatusCode::BAD_REQUEST, Html("<h1>Invalid state</h1>".to_string()))
    })? as i32;

    let redirect_to = state_data["redirect_to"]
        .as_str()
        .unwrap_or("/youtube/manage")
        .to_string();

    // Exchange code for tokens
    let client_id = state.google_oauth_client_id.as_ref().unwrap();
    let client_secret = state.google_oauth_client_secret.as_ref().unwrap();
    let redirect_uri = std::env::var("GOOGLE_OAUTH_REDIRECT_URI")
        .unwrap_or_else(|_| "http://localhost:3000/youtube/callback".to_string());

    let client = reqwest::Client::new();
    let token_response = youtube_client::exchange_code_for_token(
        &client,
        &code,
        client_id,
        client_secret,
        &redirect_uri,
    )
    .await
    .map_err(|e| {
        tracing::error!("Failed to exchange code: {}", e);
        (StatusCode::INTERNAL_SERVER_ERROR, Html(format!("<h1>Failed to exchange code: {}</h1>", e)))
    })?;

    let access_token = &token_response.access_token;
    let refresh_token = token_response.refresh_token.ok_or_else(|| {
        (StatusCode::INTERNAL_SERVER_ERROR, Html("<h1>No refresh token received</h1>".to_string()))
    })?;

    // Get user's YouTube channels
    let youtube = state.youtube_client.as_ref().ok_or_else(|| {
        (StatusCode::SERVICE_UNAVAILABLE, Html("<h1>YouTube client not initialized</h1>".to_string()))
    })?;

    let channels = youtube.list_channels(access_token).await
        .map_err(|e| {
            tracing::error!("Failed to list channels: {}", e);
            (StatusCode::INTERNAL_SERVER_ERROR, Html(format!("<h1>Failed to list channels: {}</h1>", e)))
        })?;

    if channels.is_empty() {
        return Ok(Html(r#"
<!DOCTYPE html><html><head><title>No Channels Found</title></head>
<body><h1>⚠️ No YouTube Channels Found</h1>
<p>Your Google account doesn't have any YouTube channels.</p>
<p>Please create a YouTube channel first, then try connecting again.</p>
<a href="/youtube/manage">Back to Management</a></body></html>
        "#.to_string()));
    }

    // Calculate token expiry
    let token_expiry = chrono::Utc::now() + chrono::Duration::seconds(token_response.expires_in);

    // Save all channels to database
    let mut saved_count = 0;
    for channel in channels {
        let result = sqlx::query(
            r#"
            INSERT INTO connected_youtube_channels (
                user_id, channel_id, channel_name, channel_description,
                channel_thumbnail_url, subscriber_count, video_count,
                access_token, refresh_token, token_expiry, granted_scopes,
                is_active, created_at, updated_at
            ) VALUES ($1, $2, $3, $4, $5, $6, $7, $8, $9, $10, $11, true, NOW(), NOW())
            ON CONFLICT (user_id, channel_id)
            DO UPDATE SET
                access_token = $8,
                refresh_token = $9,
                token_expiry = $10,
                granted_scopes = $11,
                channel_name = $3,
                channel_description = $4,
                channel_thumbnail_url = $5,
                subscriber_count = $6,
                video_count = $7,
                is_active = true,
                updated_at = NOW()
            "#
        )
        .bind(user_id)
        .bind(&channel.id)
        .bind(&channel.title)
        .bind(&channel.description)
        .bind(&channel.thumbnail_url)
        .bind(channel.subscriber_count)
        .bind(channel.video_count)
        .bind(&access_token)
        .bind(&refresh_token)
        .bind(token_expiry)
        .bind(&token_response.scope)
        .execute(&state.db_pool)
        .await;

        match result {
            Ok(_) => {
                saved_count += 1;
                tracing::info!("✅ Connected YouTube channel: {} (ID: {})", channel.title, channel.id);
            }
            Err(e) => {
                tracing::error!("Failed to save channel {}: {}", channel.title, e);
            }
        }
    }

    // Return success page
    Ok(Html(format!(
        r#"<!DOCTYPE html><html><head><title>Channels Connected</title>
        <style>body {{ font-family: Arial; max-width: 600px; margin: 100px auto; text-align: center; }}</style>
        </head><body>
        <h1>✅ YouTube Channels Connected!</h1>
        <p>Successfully connected {} YouTube channel(s) to your account.</p>
        <p><a href="{}">Continue</a></p>
        <script>setTimeout(() => window.location.href = '{}', 2000);</script>
        </body></html>"#,
        saved_count, redirect_to, redirect_to
    )))
}

// ============================================================================
// Channel Management API
// ============================================================================

/// List user's connected YouTube channels
pub async fn list_connected_channels(
    Extension(state): Extension<Arc<AppState>>,
    Extension(claims): Extension<crate::models::auth::Claims>,
) -> Result<Json<serde_json::Value>, StatusCode> {
    let user_id = claims.sub.parse::<i32>().unwrap_or(0);

    let channels = sqlx::query_as::<_, ConnectedYouTubeChannel>(
        "SELECT * FROM connected_youtube_channels WHERE user_id = $1 AND is_active = true ORDER BY created_at DESC"
    )
    .bind(user_id)
    .fetch_all(&state.db_pool)
    .await
    .map_err(|_| StatusCode::INTERNAL_SERVER_ERROR)?;

    let responses: Vec<ConnectedChannelResponse> = channels.into_iter()
        .map(ConnectedChannelResponse::from)
        .collect();

    Ok(Json(json!({
        "success": true,
        "channels": responses
    })))
}

/// Disconnect a YouTube channel
pub async fn disconnect_channel(
    Path(channel_id): Path<i32>,
    Extension(state): Extension<Arc<AppState>>,
    Extension(claims): Extension<crate::models::auth::Claims>,
) -> Result<Json<serde_json::Value>, (StatusCode, Json<serde_json::Value>)> {
    let user_id = claims.sub.parse::<i32>().unwrap_or(0);

    let result = sqlx::query(
        "UPDATE connected_youtube_channels SET is_active = false, updated_at = NOW()
         WHERE id = $1 AND user_id = $2"
    )
    .bind(channel_id)
    .bind(user_id)
    .execute(&state.db_pool)
    .await
    .map_err(|_| {
        (
            StatusCode::INTERNAL_SERVER_ERROR,
            Json(json!({"success": false, "message": "Database error"}))
        )
    })?;

    if result.rows_affected() == 0 {
        return Err((
            StatusCode::NOT_FOUND,
            Json(json!({"success": false, "message": "Channel not found"}))
        ));
    }

    Ok(Json(json!({
        "success": true,
        "message": "Channel disconnected successfully"
    })))
}

/// Refresh channel access token
pub async fn refresh_channel_token(
    Path(channel_id): Path<i32>,
    Extension(state): Extension<Arc<AppState>>,
    Extension(claims): Extension<crate::models::auth::Claims>,
) -> Result<Json<serde_json::Value>, (StatusCode, Json<serde_json::Value>)> {
    let user_id = claims.sub.parse::<i32>().unwrap_or(0);

    // Get channel
    let channel = sqlx::query_as::<_, ConnectedYouTubeChannel>(
        "SELECT * FROM connected_youtube_channels WHERE id = $1 AND user_id = $2"
    )
    .bind(channel_id)
    .bind(user_id)
    .fetch_optional(&state.db_pool)
    .await
    .map_err(|_| (
        StatusCode::INTERNAL_SERVER_ERROR,
        Json(json!({"success": false, "message": "Database error"}))
    ))?
    .ok_or_else(|| (
        StatusCode::NOT_FOUND,
        Json(json!({"success": false, "message": "Channel not found"}))
    ))?;

    // Refresh token
    let youtube = state.youtube_client.as_ref().ok_or_else(|| {
        (StatusCode::SERVICE_UNAVAILABLE, Json(json!({"success": false, "message": "YouTube client not initialized"})))
    })?;

    let client_id = state.google_oauth_client_id.as_ref().unwrap();
    let client_secret = state.google_oauth_client_secret.as_ref().unwrap();

    let token_response = youtube.refresh_access_token(
        &channel.refresh_token,
        client_id,
        client_secret,
    )
    .await
    .map_err(|e| {
        tracing::error!("Failed to refresh token: {}", e);
        (StatusCode::INTERNAL_SERVER_ERROR, Json(json!({"success": false, "message": format!("Token refresh failed: {}", e)})))
    })?;

    // Update database
    let new_expiry = chrono::Utc::now() + chrono::Duration::seconds(token_response.expires_in);

    sqlx::query(
        "UPDATE connected_youtube_channels
         SET access_token = $1, token_expiry = $2, updated_at = NOW()
         WHERE id = $3"
    )
    .bind(&token_response.access_token)
    .bind(new_expiry)
    .bind(channel_id)
    .execute(&state.db_pool)
    .await
    .map_err(|_| (
        StatusCode::INTERNAL_SERVER_ERROR,
        Json(json!({"success": false, "message": "Failed to update token"}))
    ))?;

    Ok(Json(json!({
        "success": true,
        "message": "Token refreshed successfully"
    })))
}

// ============================================================================
// Video Upload API
// ============================================================================

/// Upload video to YouTube channel
pub async fn upload_video_to_youtube(
    Extension(state): Extension<Arc<AppState>>,
    Extension(claims): Extension<crate::models::auth::Claims>,
    Json(payload): Json<UploadToYouTubeRequest>,
) -> Result<Json<serde_json::Value>, (StatusCode, Json<serde_json::Value>)> {
    let user_id = claims.sub.parse::<i32>().unwrap_or(0);

    // Get channel and verify ownership
    let mut channel = sqlx::query_as::<_, ConnectedYouTubeChannel>(
        "SELECT * FROM connected_youtube_channels WHERE id = $1 AND user_id = $2 AND is_active = true"
    )
    .bind(payload.channel_id)
    .bind(user_id)
    .fetch_optional(&state.db_pool)
    .await
    .map_err(|_| (
        StatusCode::INTERNAL_SERVER_ERROR,
        Json(json!({"success": false, "message": "Database error"}))
    ))?
    .ok_or_else(|| (
        StatusCode::NOT_FOUND,
        Json(json!({"success": false, "message": "Channel not found or not connected"}))
    ))?;

    // Check if token needs refresh
    if channel.token_expiry < chrono::Utc::now() + chrono::Duration::minutes(5) {
        tracing::info!("🔄 Refreshing expired token for channel: {}", channel.channel_name);

        let youtube = state.youtube_client.as_ref().ok_or_else(|| {
            (StatusCode::SERVICE_UNAVAILABLE, Json(json!({"success": false, "message": "YouTube client not initialized"})))
        })?;

        let client_id = state.google_oauth_client_id.as_ref().unwrap();
        let client_secret = state.google_oauth_client_secret.as_ref().unwrap();

        let token_response = youtube.refresh_access_token(
            &channel.refresh_token,
            client_id,
            client_secret,
        )
        .await
        .map_err(|e| {
            tracing::error!("Failed to refresh token: {}", e);
            (StatusCode::UNAUTHORIZED, Json(json!({"success": false, "message": "Token expired. Please reconnect your channel."})))
        })?;

        // Update token in memory and database
        channel.access_token = token_response.access_token.clone();
        channel.token_expiry = chrono::Utc::now() + chrono::Duration::seconds(token_response.expires_in);

        sqlx::query(
            "UPDATE connected_youtube_channels
             SET access_token = $1, token_expiry = $2, updated_at = NOW()
             WHERE id = $3"
        )
        .bind(&channel.access_token)
        .bind(channel.token_expiry)
        .bind(payload.channel_id)
        .execute(&state.db_pool)
        .await
        .ok();
    }

    // Verify video file exists
    if !std::path::Path::new(&payload.video_path).exists() {
        return Err((
            StatusCode::NOT_FOUND,
            Json(json!({"success": false, "message": "Video file not found"}))
        ));
    }

    // Create upload record
    let upload_id: i32 = sqlx::query_scalar(
        "INSERT INTO youtube_uploads (
            user_id, channel_id, local_video_path, video_title, video_description,
            video_category, privacy_status, upload_status, created_at, updated_at
        ) VALUES ($1, $2, $3, $4, $5, $6, $7, 'uploading', NOW(), NOW())
        RETURNING id"
    )
    .bind(user_id)
    .bind(payload.channel_id)
    .bind(&payload.video_path)
    .bind(&payload.title)
    .bind(&payload.description)
    .bind(payload.category.as_deref().unwrap_or("22"))
    .bind(&payload.privacy_status)
    .fetch_one(&state.db_pool)
    .await
    .map_err(|_| (
        StatusCode::INTERNAL_SERVER_ERROR,
        Json(json!({"success": false, "message": "Failed to create upload record"}))
    ))?;

    // Upload to YouTube
    let youtube = state.youtube_client.as_ref().unwrap();

    tracing::info!("📤 Uploading video to YouTube: {} ({})", payload.title, channel.channel_name);

    let upload_result = youtube.upload_video(
        &channel.access_token,
        &payload.video_path,
        &payload.title,
        payload.description.as_deref().unwrap_or(""),
        &payload.privacy_status,
        payload.category.as_deref(),
        payload.tags,
    )
    .await;

    match upload_result {
        Ok(response) => {
            let youtube_url = format!("https://www.youtube.com/watch?v={}", response.id);

            // Update upload record
            sqlx::query(
                "UPDATE youtube_uploads
                 SET youtube_video_id = $1, youtube_url = $2, published_at = $3,
                     upload_status = 'completed', upload_progress = 100, updated_at = NOW()
                 WHERE id = $4"
            )
            .bind(&response.id)
            .bind(&youtube_url)
            .bind(&response.snippet.published_at)
            .bind(upload_id)
            .execute(&state.db_pool)
            .await
            .ok();

            tracing::info!("✅ Video uploaded successfully: {}", youtube_url);

            Ok(Json(json!({
                "success": true,
                "message": "Video uploaded to YouTube successfully",
                "upload": {
                    "id": upload_id,
                    "youtube_video_id": response.id,
                    "youtube_url": youtube_url,
                    "title": response.snippet.title,
                    "privacy_status": payload.privacy_status,
                    "published_at": response.snippet.published_at
                }
            })))
        }
        Err(e) => {
            tracing::error!("❌ Failed to upload video: {}", e);

            // Update upload record with error
            sqlx::query(
                "UPDATE youtube_uploads
                 SET upload_status = 'failed', error_message = $1, updated_at = NOW()
                 WHERE id = $2"
            )
            .bind(e.to_string())
            .bind(upload_id)
            .execute(&state.db_pool)
            .await
            .ok();

            Err((
                StatusCode::INTERNAL_SERVER_ERROR,
                Json(json!({"success": false, "message": format!("Upload failed: {}", e)}))
            ))
        }
    }
}

/// List upload history
pub async fn list_upload_history(
    Extension(state): Extension<Arc<AppState>>,
    Extension(claims): Extension<crate::models::auth::Claims>,
) -> Result<Json<serde_json::Value>, StatusCode> {
    let user_id = claims.sub.parse::<i32>().unwrap_or(0);

    let uploads = sqlx::query_as::<_, YouTubeUpload>(
        "SELECT * FROM youtube_uploads WHERE user_id = $1 ORDER BY created_at DESC LIMIT 50"
    )
    .bind(user_id)
    .fetch_all(&state.db_pool)
    .await
    .map_err(|_| StatusCode::INTERNAL_SERVER_ERROR)?;

    Ok(Json(json!({
        "success": true,
        "uploads": uploads
    })))
}

// ============================================================================
// YouTube Management Page
// ============================================================================

pub async fn youtube_management_page() -> Html<String> {
    let html = r###"
<!DOCTYPE html>
<html lang="en">
<head>
    <meta charset="UTF-8">
    <meta name="viewport" content="width=device-width, initial-scale=1.0">
    <title>YouTube Channels - VideoSync</title>
    <style>
        * { margin: 0; padding: 0; box-sizing: border-box; }
        body { font-family: -apple-system, BlinkMacSystemFont, 'Segui UI', Roboto, sans-serif; background: linear-gradient(135deg, #1a1a2e 0%, #16213e 50%, #0f1419 100%); min-height: 100vh; color: #e8e8e8; }
        .container { max-width: 1200px; margin: 0 auto; padding: 2rem; }
        .header { background: rgba(30, 30, 52, 0.8); backdrop-filter: blur(20px); border-bottom: 1px solid rgba(59, 130, 246, 0.3); padding: 1.5rem 2rem; margin-bottom: 2rem; border-radius: 15px; }
        .header h1 { font-size: 2rem; margin-bottom: 0.5rem; }
        .header p { color: #cbd5e1; }
        .btn { padding: 0.75rem 1.5rem; background: linear-gradient(135deg, #3b82f6, #1d4ed8); color: white; border: none; border-radius: 10px; font-weight: 600; cursor: pointer; transition: all 0.3s; text-decoration: none; display: inline-block; }
        .btn:hover { background: linear-gradient(135deg, #2563eb, #1e40af); transform: translateY(-2px); box-shadow: 0 4px 12px rgba(59, 130, 246, 0.4); }
        .btn-secondary { background: rgba(30, 30, 52, 0.8); border: 2px solid rgba(59, 130, 246, 0.3); }
        .btn-secondary:hover { background: rgba(59, 130, 246, 0.2); border-color: rgba(59, 130, 246, 0.6); }
        .btn-danger { background: linear-gradient(135deg, #dc3545, #c82333); }
        .btn-danger:hover { background: linear-gradient(135deg, #c82333, #bd2130); }
        .channels-grid { display: grid; grid-template-columns: repeat(auto-fill, minmax(350px, 1fr)); gap: 1.5rem; margin-top: 2rem; }
        .channel-card { background: rgba(30, 30, 52, 0.6); border: 1px solid rgba(59, 130, 246, 0.2); border-radius: 15px; padding: 1.5rem; transition: all 0.3s; }
        .channel-card:hover { transform: translateY(-5px); box-shadow: 0 15px 30px rgba(59, 130, 246, 0.2); border-color: rgba(59, 130, 246, 0.4); }
        .channel-header { display: flex; gap: 1rem; margin-bottom: 1rem; }
        .channel-thumbnail { width: 60px; height: 60px; border-radius: 50%; object-fit: cover; }
        .channel-info h3 { font-size: 1.2rem; margin-bottom: 0.25rem; }
        .channel-stats { color: #cbd5e1; font-size: 0.9rem; }
        .channel-actions { display: flex; gap: 0.5rem; margin-top: 1rem; }
        .empty-state { text-align: center; padding: 4rem 2rem; }
        .empty-state-icon { font-size: 4rem; margin-bottom: 1rem; }
        .loading { text-align: center; padding: 3rem; color: #cbd5e1; }
    </style>
</head>
<body>
    <div class="container">
        <div class="header">
            <div style="display: flex; justify-content: space-between; align-items: center;">
                <div>
                    <h1>📺 YouTube Channels</h1>
                    <p>Manage your connected YouTube channels for video uploads</p>
                </div>
                <div style="display: flex; gap: 1rem;">
                    <a href="/dashboard" class="btn btn-secondary">← Back to Dashboard</a>
                    <button onclick="connectNewChannel()" class="btn">+ Connect Channel</button>
                </div>
            </div>
        </div>

        <div id="channelsContainer" class="loading">
            Loading your channels...
        </div>
    </div>

    <script>
        const authToken = localStorage.getItem('authToken');
        if (!authToken) {
            window.location.href = '/login';
        }

        async function loadChannels() {
            try {
                const response = await fetch('/api/youtube/channels', {
                    headers: { 'Authorization': 'Bearer ' + authToken }
                });
                const data = await response.json();

                const container = document.getElementById('channelsContainer');

                if (data.success && data.channels.length > 0) {
                    container.className = 'channels-grid';
                    container.innerHTML = data.channels.map(channel => `
                        <div class="channel-card">
                            <div class="channel-header">
                                ${channel.channel_thumbnail_url ?
                                    `<img src="${channel.channel_thumbnail_url}" class="channel-thumbnail" alt="${channel.channel_name}">` :
                                    '<div class="channel-thumbnail" style="background: linear-gradient(135deg, #3b82f6, #1d4ed8); display: flex; align-items: center; justify-content: center; font-size: 1.5rem;">📺</div>'
                                }
                                <div class="channel-info">
                                    <h3>${channel.channel_name}</h3>
                                    <div class="channel-stats">
                                        ${channel.subscriber_count !== null ? channel.subscriber_count.toLocaleString() + ' subscribers' : 'N/A'} •
                                        ${channel.video_count !== null ? channel.video_count.toLocaleString() + ' videos' : 'N/A'}
                                    </div>
                                </div>
                            </div>
                            <div class="channel-actions">
                                <button onclick="disconnectChannel(${channel.id}, '${channel.channel_name}')" class="btn btn-danger" style="flex: 1; padding: 0.5rem;">Disconnect</button>
                            </div>
                        </div>
                    `).join('');
                } else {
                    container.className = 'empty-state';
                    container.innerHTML = `
                        <div class="empty-state-icon">📺</div>
                        <h2>No Channels Connected</h2>
                        <p style="color: #cbd5e1; margin: 1rem 0;">Connect your YouTube channel to start uploading videos directly from your edits</p>
                        <button onclick="connectNewChannel()" class="btn">Connect Your First Channel</button>
                    `;
                }
            } catch (error) {
                console.error('Error loading channels:', error);
                document.getElementById('channelsContainer').innerHTML = `
                    <div class="empty-state">
                        <h2>❌ Error Loading Channels</h2>
                        <p>${error.message}</p>
                    </div>
                `;
            }
        }

        async function connectNewChannel() {
            // Get auth token from localStorage
            const authToken = localStorage.getItem('authToken');

            if (!authToken) {
                // Not logged in - redirect to login first
                window.location.href = '/login?redirect=/youtube/manage';
                return;
            }

            // Make authenticated request to get OAuth URL
            try {
                const response = await fetch('/youtube/connect?redirect_to=' + encodeURIComponent('/youtube/manage'), {
                    headers: {
                        'Authorization': 'Bearer ' + authToken
                    }
                });

                if (response.ok) {
                    const data = await response.json();
                    if (data.success && data.auth_url) {
                        // Redirect to Google OAuth (works with ANY Google account!)
                        console.log('🔐 Redirecting to Google OAuth for YouTube connection...');
                        window.location.href = data.auth_url;
                    } else {
                        alert('Failed to get OAuth URL: ' + (data.message || 'Unknown error'));
                    }
                } else if (response.status === 401) {
                    // Token expired - redirect to login
                    alert('Session expired. Please log in again.');
                    window.location.href = '/login?redirect=/youtube/manage';
                } else {
                    // Handle other errors
                    const error = await response.json().catch(() => ({message: 'Unknown error'}));
                    alert('Failed to connect: ' + (error.message || 'Please try again'));
                }
            } catch (error) {
                console.error('Connection error:', error);
                alert('Failed to initiate YouTube connection. Please try again.');
            }
        }

        async function disconnectChannel(channelId, channelName) {
            if (!confirm("Are you sure you want to disconnect " + channelName + "?")) {
                return;
            }

            try {
                const response = await fetch("/api/youtube/channels/" + channelId + "/disconnect", {
                    method: 'DELETE',
                    headers: { 'Authorization': 'Bearer ' + authToken }
                });
                const data = await response.json();

                if (data.success) {
                    alert('✅ Channel disconnected successfully');
                    loadChannels();
                } else {
                    alert('❌ Error: ' + data.message);
                }
            } catch (error) {
                console.error('Error disconnecting channel:', error);
                alert('❌ Network error');
            }
        }

        loadChannels();
    </script>
</body>
</html>
    "###;

    Html(html.to_string())
}

// ============================================================================
// Video Management Handlers
// ============================================================================

/// Delete a video from YouTube
///
/// DELETE /api/youtube/videos/:video_id
pub async fn delete_video_from_youtube(
    Path(video_id): Path<String>,
    Extension(state): Extension<Arc<AppState>>,
    Extension(claims): Extension<crate::models::auth::Claims>,
) -> Result<Json<serde_json::Value>, (StatusCode, Json<serde_json::Value>)> {
    let user_id = claims.sub.parse::<i32>().unwrap_or(0);

    let youtube = state.youtube_client.as_ref().ok_or_else(|| {
        (
            StatusCode::SERVICE_UNAVAILABLE,
            Json(json!({
                "success": false,
                "message": "YouTube client not initialized"
            })),
        )
    })?;

    // Find the upload record to get channel info
    let upload = sqlx::query_as::<_, crate::models::youtube::YouTubeUpload>(
        "SELECT * FROM youtube_uploads WHERE youtube_video_id = $1 AND user_id = $2 AND deleted_at IS NULL"
    )
    .bind(&video_id)
    .bind(user_id)
    .fetch_optional(&state.db_pool)
    .await
    .map_err(|e| {
        tracing::error!("Database error: {}", e);
        (
            StatusCode::INTERNAL_SERVER_ERROR,
            Json(json!({"success": false, "message": "Database error"})),
        )
    })?
    .ok_or_else(|| {
        (
            StatusCode::NOT_FOUND,
            Json(json!({"success": false, "message": "Video not found or already deleted"})),
        )
    })?;

    // Get channel to obtain access token
    let channel = sqlx::query_as::<_, crate::models::youtube::ConnectedYouTubeChannel>(
        "SELECT * FROM connected_youtube_channels WHERE id = $1 AND user_id = $2 AND is_active = true"
    )
    .bind(upload.channel_id)
    .bind(user_id)
    .fetch_optional(&state.db_pool)
    .await
    .map_err(|e| {
        (
            StatusCode::INTERNAL_SERVER_ERROR,
            Json(json!({"success": false, "message": "Database error"})),
        )
    })?
    .ok_or_else(|| {
        (
            StatusCode::NOT_FOUND,
            Json(json!({"success": false, "message": "YouTube channel not connected"})),
        )
    })?;

    // Check if channel has required scope
    if !channel.granted_scopes.contains("youtube.force-ssl") {
        return Err((
            StatusCode::FORBIDDEN,
            Json(json!({
                "success": false,
                "message": "Additional permissions required to delete videos",
                "requires_reauth": true,
                "reconnect_url": "/youtube/connect?reauth=true"
            })),
        ));
    }

    // Delete from YouTube
    youtube.delete_video(&channel.access_token, &video_id)
        .await
        .map_err(|e| {
            tracing::error!("YouTube API error: {}", e);
            (
                StatusCode::BAD_GATEWAY,
                Json(json!({"success": false, "message": format!("YouTube API error: {}", e)})),
            )
        })?;

    // Soft delete in database
    sqlx::query(
        "UPDATE youtube_uploads SET deleted_at = NOW(), updated_at = NOW() WHERE id = $1"
    )
    .bind(upload.id)
    .execute(&state.db_pool)
    .await
    .ok();

    Ok(Json(json!({
        "success": true,
        "message": "Video deleted from YouTube successfully",
        "video_id": video_id
    })))
}

/// Update video metadata (title, description, privacy, tags)
///
/// PATCH /api/youtube/videos/:video_id
pub async fn update_video_metadata(
    Path(video_id): Path<String>,
    Extension(state): Extension<Arc<AppState>>,
    Extension(claims): Extension<crate::models::auth::Claims>,
    Json(payload): Json<crate::models::youtube::UpdateVideoRequest>,
) -> Result<Json<serde_json::Value>, (StatusCode, Json<serde_json::Value>)> {
    let user_id = claims.sub.parse::<i32>().unwrap_or(0);

    let youtube = state.youtube_client.as_ref().ok_or_else(|| {
        (
            StatusCode::SERVICE_UNAVAILABLE,
            Json(json!({"success": false, "message": "YouTube client not initialized"})),
        )
    })?;

    // Find the upload record
    let upload = sqlx::query_as::<_, crate::models::youtube::YouTubeUpload>(
        "SELECT * FROM youtube_uploads WHERE youtube_video_id = $1 AND user_id = $2 AND deleted_at IS NULL"
    )
    .bind(&video_id)
    .bind(user_id)
    .fetch_optional(&state.db_pool)
    .await
    .map_err(|_| {
        (
            StatusCode::INTERNAL_SERVER_ERROR,
            Json(json!({"success": false, "message": "Database error"})),
        )
    })?
    .ok_or_else(|| {
        (
            StatusCode::NOT_FOUND,
            Json(json!({"success": false, "message": "Video not found"})),
        )
    })?;

    // Get channel
    let channel = sqlx::query_as::<_, crate::models::youtube::ConnectedYouTubeChannel>(
        "SELECT * FROM connected_youtube_channels WHERE id = $1 AND user_id = $2 AND is_active = true"
    )
    .bind(upload.channel_id)
    .bind(user_id)
    .fetch_optional(&state.db_pool)
    .await
    .map_err(|_| {
        (
            StatusCode::INTERNAL_SERVER_ERROR,
            Json(json!({"success": false, "message": "Database error"})),
        )
    })?
    .ok_or_else(|| {
        (
            StatusCode::NOT_FOUND,
            Json(json!({"success": false, "message": "YouTube channel not connected"})),
        )
    })?;

    // Check scope
    if !channel.granted_scopes.contains("youtube.force-ssl") {
        return Err((
            StatusCode::FORBIDDEN,
            Json(json!({
                "success": false,
                "message": "Additional permissions required",
                "requires_reauth": true,
                "reconnect_url": "/youtube/connect?reauth=true"
            })),
        ));
    }

    // Update on YouTube
    let update_response = youtube.update_video(
        &channel.access_token,
        &video_id,
        payload.title.as_deref(),
        payload.description.as_deref(),
        payload.privacy_status.as_deref(),
        payload.category_id.as_deref(),
        payload.tags.clone(),
    )
    .await
    .map_err(|e| {
        tracing::error!("YouTube API error: {}", e);
        (
            StatusCode::BAD_GATEWAY,
            Json(json!({"success": false, "message": format!("YouTube API error: {}", e)})),
        )
    })?;

    // Update local database
    sqlx::query(
        "UPDATE youtube_uploads SET
         video_title = COALESCE($1, video_title),
         video_description = COALESCE($2, video_description),
         privacy_status = COALESCE($3, privacy_status),
         video_category = COALESCE($4, video_category),
         metadata_updated_at = NOW(),
         updated_at = NOW()
         WHERE id = $5"
    )
    .bind(payload.title.as_ref())
    .bind(payload.description.as_ref())
    .bind(payload.privacy_status.as_ref())
    .bind(payload.category_id.as_ref())
    .bind(upload.id)
    .execute(&state.db_pool)
    .await
    .ok();

    Ok(Json(json!({
        "success": true,
        "message": "Video metadata updated successfully",
        "video": {
            "id": update_response.id,
            "title": update_response.snippet.title,
            "description": update_response.snippet.description,
            "privacy_status": update_response.status.privacy_status
        }
    })))
}

/// Upload custom thumbnail (multipart file upload)
///
/// POST /api/youtube/videos/:video_id/thumbnail
pub async fn upload_custom_thumbnail(
    Path(video_id): Path<String>,
    Extension(state): Extension<Arc<AppState>>,
    Extension(claims): Extension<crate::models::auth::Claims>,
    mut multipart: axum::extract::Multipart,
) -> Result<Json<serde_json::Value>, (StatusCode, Json<serde_json::Value>)> {
    let user_id = claims.sub.parse::<i32>().unwrap_or(0);

    let youtube = state.youtube_client.as_ref().ok_or_else(|| {
        (
            StatusCode::SERVICE_UNAVAILABLE,
            Json(json!({"success": false, "message": "YouTube client not initialized"})),
        )
    })?;

    // Extract image file from multipart
    let mut image_data: Option<Vec<u8>> = None;
    let mut content_type = "image/jpeg".to_string();

    while let Some(field) = multipart.next_field().await.ok().flatten() {
        if field.name() == Some("thumbnail") {
            content_type = field.content_type()
                .unwrap_or("image/jpeg")
                .to_string();
            image_data = Some(field.bytes().await.unwrap_or_default().to_vec());
            break;
        }
    }

    let image_data = image_data.ok_or_else(|| {
        (
            StatusCode::BAD_REQUEST,
            Json(json!({"success": false, "message": "No thumbnail image provided"})),
        )
    })?;

    // Verify ownership
    let upload = sqlx::query_as::<_, crate::models::youtube::YouTubeUpload>(
        "SELECT * FROM youtube_uploads WHERE youtube_video_id = $1 AND user_id = $2 AND deleted_at IS NULL"
    )
    .bind(&video_id)
    .bind(user_id)
    .fetch_optional(&state.db_pool)
    .await
    .map_err(|_| {
        (
            StatusCode::INTERNAL_SERVER_ERROR,
            Json(json!({"success": false, "message": "Database error"})),
        )
    })?
    .ok_or_else(|| {
        (
            StatusCode::NOT_FOUND,
            Json(json!({"success": false, "message": "Video not found"})),
        )
    })?;

    // Get channel
    let channel = sqlx::query_as::<_, crate::models::youtube::ConnectedYouTubeChannel>(
        "SELECT * FROM connected_youtube_channels WHERE id = $1 AND user_id = $2 AND is_active = true"
    )
    .bind(upload.channel_id)
    .bind(user_id)
    .fetch_optional(&state.db_pool)
    .await
    .map_err(|_| {
        (
            StatusCode::INTERNAL_SERVER_ERROR,
            Json(json!({"success": false, "message": "Database error"})),
        )
    })?
    .ok_or_else(|| {
        (
            StatusCode::NOT_FOUND,
            Json(json!({"success": false, "message": "Channel not connected"})),
        )
    })?;

    // Check scope
    if !channel.granted_scopes.contains("youtube.force-ssl") {
        return Err((
            StatusCode::FORBIDDEN,
            Json(json!({
                "success": false,
                "message": "Additional permissions required",
                "requires_reauth": true,
                "reconnect_url": "/youtube/connect?reauth=true"
            })),
        ));
    }

    // Upload to YouTube
    let thumb_response = youtube.upload_thumbnail(
        &channel.access_token,
        &video_id,
        image_data,
        &content_type,
    )
    .await
    .map_err(|e| {
        (
            StatusCode::BAD_GATEWAY,
            Json(json!({"success": false, "message": format!("YouTube API error: {}", e)})),
        )
    })?;

    Ok(Json(json!({
        "success": true,
        "message": "Thumbnail uploaded successfully",
        "thumbnail": thumb_response.items.first()
    })))
}

/// Auto-generate and upload thumbnail from video
///
/// POST /api/youtube/videos/:video_id/thumbnail/generate
pub async fn generate_and_upload_thumbnail(
    Path(video_id): Path<String>,
    Extension(state): Extension<Arc<AppState>>,
    Extension(claims): Extension<crate::models::auth::Claims>,
    Json(payload): Json<crate::models::youtube::GenerateThumbnailRequest>,
) -> Result<Json<serde_json::Value>, (StatusCode, Json<serde_json::Value>)> {
    let user_id = claims.sub.parse::<i32>().unwrap_or(0);

    let youtube = state.youtube_client.as_ref().ok_or_else(|| {
        (
            StatusCode::SERVICE_UNAVAILABLE,
            Json(json!({"success": false, "message": "YouTube client not initialized"})),
        )
    })?;

    // Verify ownership and get video path
    let upload = sqlx::query_as::<_, crate::models::youtube::YouTubeUpload>(
        "SELECT * FROM youtube_uploads WHERE youtube_video_id = $1 AND user_id = $2 AND deleted_at IS NULL"
    )
    .bind(&video_id)
    .bind(user_id)
    .fetch_optional(&state.db_pool)
    .await
    .map_err(|_| {
        (
            StatusCode::INTERNAL_SERVER_ERROR,
            Json(json!({"success": false, "message": "Database error"})),
        )
    })?
    .ok_or_else(|| {
        (
            StatusCode::NOT_FOUND,
            Json(json!({"success": false, "message": "Video not found"})),
        )
    })?;

    // Get channel
    let channel = sqlx::query_as::<_, crate::models::youtube::ConnectedYouTubeChannel>(
        "SELECT * FROM connected_youtube_channels WHERE id = $1 AND user_id = $2"
    )
    .bind(upload.channel_id)
    .bind(user_id)
    .fetch_optional(&state.db_pool)
    .await
    .map_err(|_| {
        (
            StatusCode::INTERNAL_SERVER_ERROR,
            Json(json!({"success": false, "message": "Database error"})),
        )
    })?
    .ok_or_else(|| {
        (
            StatusCode::NOT_FOUND,
            Json(json!({"success": false, "message": "Channel not connected"})),
        )
    })?;

    // Check scope
    if !channel.granted_scopes.contains("youtube.force-ssl") {
        return Err((
            StatusCode::FORBIDDEN,
            Json(json!({
                "success": false,
                "message": "Additional permissions required",
                "requires_reauth": true,
                "reconnect_url": "/youtube/connect?reauth=true"
            })),
        ));
    }

    // Generate thumbnail from video
    let thumbnail_path = format!("outputs/thumbnails/youtube_{}.jpg", video_id);
    std::fs::create_dir_all("outputs/thumbnails").ok();

    let width = payload.width.unwrap_or(1280);
    let height = payload.height.unwrap_or(720);

    crate::transform::create_thumbnail_scaled(
        &upload.local_video_path,
        &thumbnail_path,
        payload.timestamp,
        width,
        height,
    )
    .map_err(|e| {
        tracing::error!("Thumbnail generation failed: {}", e);
        (
            StatusCode::INTERNAL_SERVER_ERROR,
            Json(json!({"success": false, "message": format!("Failed to generate thumbnail: {}", e)})),
        )
    })?;

    // Read generated thumbnail
    let image_data = tokio::fs::read(&thumbnail_path)
        .await
        .map_err(|e| {
            (
                StatusCode::INTERNAL_SERVER_ERROR,
                Json(json!({"success": false, "message": format!("Failed to read thumbnail: {}", e)})),
            )
        })?;

    // Upload to YouTube
    let thumb_response = youtube.upload_thumbnail(
        &channel.access_token,
        &video_id,
        image_data,
        "image/jpeg",
    )
    .await
    .map_err(|e| {
        (
            StatusCode::BAD_GATEWAY,
            Json(json!({"success": false, "message": format!("YouTube API error: {}", e)})),
        )
    })?;

    // Update database with thumbnail path
    sqlx::query(
        "UPDATE youtube_uploads SET custom_thumbnail_path = $1, updated_at = NOW() WHERE id = $2"
    )
    .bind(&thumbnail_path)
    .bind(upload.id)
    .execute(&state.db_pool)
    .await
    .ok();

    Ok(Json(json!({
        "success": true,
        "message": "Thumbnail generated and uploaded successfully",
        "thumbnail": thumb_response.items.first(),
        "generated_at_timestamp": payload.timestamp,
        "resolution": format!("{}x{}", width, height)
    })))
}

// ============================================================================
// Playlist Management Handlers
// ============================================================================

/// List user's playlists
///
/// GET /api/youtube/playlists
pub async fn list_playlists(
    Extension(state): Extension<Arc<AppState>>,
    Extension(claims): Extension<crate::models::auth::Claims>,
) -> Result<Json<serde_json::Value>, (StatusCode, Json<serde_json::Value>)> {
    let user_id = claims.sub.parse::<i32>().unwrap_or(0);

    let youtube = state.youtube_client.as_ref().ok_or_else(|| {
        (
            StatusCode::SERVICE_UNAVAILABLE,
            Json(json!({"success": false, "message": "YouTube client not initialized"})),
        )
    })?;

    // Get user's channels
    let channels = sqlx::query_as::<_, crate::models::youtube::ConnectedYouTubeChannel>(
        "SELECT * FROM connected_youtube_channels WHERE user_id = $1 AND is_active = true"
    )
    .bind(user_id)
    .fetch_all(&state.db_pool)
    .await
    .map_err(|_| {
        (
            StatusCode::INTERNAL_SERVER_ERROR,
            Json(json!({"success": false, "message": "Database error"})),
        )
    })?;

    let mut all_playlists = Vec::new();

    for channel in channels {
        // Fetch playlists from YouTube
        match youtube.list_playlists(&channel.access_token).await {
            Ok(response) => {
                for playlist in response.items {
                    all_playlists.push(json!({
                        "youtube_playlist_id": playlist.id,
                        "title": playlist.snippet.title,
                        "description": playlist.snippet.description,
                        "privacy_status": playlist.status.privacy_status,
                        "video_count": playlist.content_details.as_ref().map(|cd| cd.item_count).unwrap_or(0),
                        "channel_id": channel.id,
                        "channel_name": channel.channel_name,
                    }));
                }
            }
            Err(e) => {
                tracing::warn!("Failed to fetch playlists for channel {}: {}", channel.channel_name, e);
            }
        }
    }

    Ok(Json(json!({
        "success": true,
        "playlists": all_playlists
    })))
}

/// Create a new playlist
///
/// POST /api/youtube/playlists
pub async fn create_playlist(
    Extension(state): Extension<Arc<AppState>>,
    Extension(claims): Extension<crate::models::auth::Claims>,
    Json(payload): Json<crate::models::youtube::CreatePlaylistRequest>,
) -> Result<Json<serde_json::Value>, (StatusCode, Json<serde_json::Value>)> {
    let user_id = claims.sub.parse::<i32>().unwrap_or(0);

    let youtube = state.youtube_client.as_ref().ok_or_else(|| {
        (
            StatusCode::SERVICE_UNAVAILABLE,
            Json(json!({"success": false, "message": "YouTube client not initialized"})),
        )
    })?;

    // Get channel
    let channel = sqlx::query_as::<_, crate::models::youtube::ConnectedYouTubeChannel>(
        "SELECT * FROM connected_youtube_channels WHERE id = $1 AND user_id = $2 AND is_active = true"
    )
    .bind(payload.channel_id)
    .bind(user_id)
    .fetch_optional(&state.db_pool)
    .await
    .map_err(|_| {
        (
            StatusCode::INTERNAL_SERVER_ERROR,
            Json(json!({"success": false, "message": "Database error"})),
        )
    })?
    .ok_or_else(|| {
        (
            StatusCode::NOT_FOUND,
            Json(json!({"success": false, "message": "Channel not found"})),
        )
    })?;

    // Check scope
    if !channel.granted_scopes.contains("youtube.force-ssl") {
        return Err((
            StatusCode::FORBIDDEN,
            Json(json!({
                "success": false,
                "message": "Additional permissions required",
                "requires_reauth": true,
                "reconnect_url": "/youtube/connect?reauth=true"
            })),
        ));
    }

    // Create on YouTube
    let playlist_response = youtube.create_playlist(
        &channel.access_token,
        &payload.title,
        payload.description.as_deref(),
        &payload.privacy_status,
    )
    .await
    .map_err(|e| {
        (
            StatusCode::BAD_GATEWAY,
            Json(json!({"success": false, "message": format!("YouTube API error: {}", e)})),
        )
    })?;

    // Save to database
    sqlx::query(
        "INSERT INTO youtube_playlists (user_id, channel_id, youtube_playlist_id, title, description, privacy_status, video_count)
         VALUES ($1, $2, $3, $4, $5, $6, 0)"
    )
    .bind(user_id)
    .bind(payload.channel_id)
    .bind(&playlist_response.id)
    .bind(&playlist_response.snippet.title)
    .bind(&playlist_response.snippet.description)
    .bind(&playlist_response.status.privacy_status)
    .execute(&state.db_pool)
    .await
    .ok();

    Ok(Json(json!({
        "success": true,
        "message": "Playlist created successfully",
        "playlist": {
            "id": playlist_response.id,
            "title": playlist_response.snippet.title,
            "description": playlist_response.snippet.description,
            "privacy_status": playlist_response.status.privacy_status
        }
    })))
}

/// Update playlist metadata
///
/// PATCH /api/youtube/playlists/:id
pub async fn update_playlist(
    Path(playlist_id): Path<String>,
    Extension(state): Extension<Arc<AppState>>,
    Extension(claims): Extension<crate::models::auth::Claims>,
    Json(payload): Json<crate::models::youtube::UpdatePlaylistRequest>,
) -> Result<Json<serde_json::Value>, (StatusCode, Json<serde_json::Value>)> {
    let user_id = claims.sub.parse::<i32>().unwrap_or(0);

    let youtube = state.youtube_client.as_ref().ok_or_else(|| {
        (
            StatusCode::SERVICE_UNAVAILABLE,
            Json(json!({"success": false, "message": "YouTube client not initialized"})),
        )
    })?;

    // Verify ownership
    let playlist = sqlx::query_as::<_, crate::models::youtube::YouTubePlaylist>(
        "SELECT * FROM youtube_playlists WHERE youtube_playlist_id = $1 AND user_id = $2"
    )
    .bind(&playlist_id)
    .bind(user_id)
    .fetch_optional(&state.db_pool)
    .await
    .map_err(|_| {
        (
            StatusCode::INTERNAL_SERVER_ERROR,
            Json(json!({"success": false, "message": "Database error"})),
        )
    })?
    .ok_or_else(|| {
        (
            StatusCode::NOT_FOUND,
            Json(json!({"success": false, "message": "Playlist not found"})),
        )
    })?;

    // Get channel
    let channel = sqlx::query_as::<_, crate::models::youtube::ConnectedYouTubeChannel>(
        "SELECT * FROM connected_youtube_channels WHERE id = $1 AND user_id = $2"
    )
    .bind(playlist.channel_id)
    .bind(user_id)
    .fetch_optional(&state.db_pool)
    .await
    .map_err(|_| {
        (
            StatusCode::INTERNAL_SERVER_ERROR,
            Json(json!({"success": false, "message": "Database error"})),
        )
    })?
    .ok_or_else(|| {
        (
            StatusCode::NOT_FOUND,
            Json(json!({"success": false, "message": "Channel not connected"})),
        )
    })?;

    // Check scope
    if !channel.granted_scopes.contains("youtube.force-ssl") {
        return Err((
            StatusCode::FORBIDDEN,
            Json(json!({
                "success": false,
                "message": "Additional permissions required",
                "requires_reauth": true,
                "reconnect_url": "/youtube/connect?reauth=true"
            })),
        ));
    }

    // Update on YouTube
    let update_response = youtube.update_playlist(
        &channel.access_token,
        &playlist_id,
        payload.title.as_deref(),
        payload.description.as_deref(),
        payload.privacy_status.as_deref(),
    )
    .await
    .map_err(|e| {
        (
            StatusCode::BAD_GATEWAY,
            Json(json!({"success": false, "message": format!("YouTube API error: {}", e)})),
        )
    })?;

    // Update in database
    sqlx::query(
        "UPDATE youtube_playlists SET
         title = COALESCE($1, title),
         description = COALESCE($2, description),
         privacy_status = COALESCE($3, privacy_status),
         updated_at = NOW()
         WHERE id = $4"
    )
    .bind(payload.title.as_ref())
    .bind(payload.description.as_ref())
    .bind(payload.privacy_status.as_ref())
    .bind(playlist.id)
    .execute(&state.db_pool)
    .await
    .ok();

    Ok(Json(json!({
        "success": true,
        "message": "Playlist updated successfully",
        "playlist": {
            "id": update_response.id,
            "title": update_response.snippet.title
        }
    })))
}

/// Delete a playlist
///
/// DELETE /api/youtube/playlists/:id
pub async fn delete_playlist(
    Path(playlist_id): Path<String>,
    Extension(state): Extension<Arc<AppState>>,
    Extension(claims): Extension<crate::models::auth::Claims>,
) -> Result<Json<serde_json::Value>, (StatusCode, Json<serde_json::Value>)> {
    let user_id = claims.sub.parse::<i32>().unwrap_or(0);

    let youtube = state.youtube_client.as_ref().ok_or_else(|| {
        (
            StatusCode::SERVICE_UNAVAILABLE,
            Json(json!({"success": false, "message": "YouTube client not initialized"})),
        )
    })?;

    // Verify ownership
    let playlist = sqlx::query_as::<_, crate::models::youtube::YouTubePlaylist>(
        "SELECT * FROM youtube_playlists WHERE youtube_playlist_id = $1 AND user_id = $2"
    )
    .bind(&playlist_id)
    .bind(user_id)
    .fetch_optional(&state.db_pool)
    .await
    .map_err(|_| {
        (
            StatusCode::INTERNAL_SERVER_ERROR,
            Json(json!({"success": false, "message": "Database error"})),
        )
    })?
    .ok_or_else(|| {
        (
            StatusCode::NOT_FOUND,
            Json(json!({"success": false, "message": "Playlist not found"})),
        )
    })?;

    // Get channel
    let channel = sqlx::query_as::<_, crate::models::youtube::ConnectedYouTubeChannel>(
        "SELECT * FROM connected_youtube_channels WHERE id = $1"
    )
    .bind(playlist.channel_id)
    .fetch_optional(&state.db_pool)
    .await
    .map_err(|_| {
        (
            StatusCode::INTERNAL_SERVER_ERROR,
            Json(json!({"success": false, "message": "Database error"})),
        )
    })?
    .ok_or_else(|| {
        (
            StatusCode::NOT_FOUND,
            Json(json!({"success": false, "message": "Channel not found"})),
        )
    })?;

    // Check scope
    if !channel.granted_scopes.contains("youtube.force-ssl") {
        return Err((
            StatusCode::FORBIDDEN,
            Json(json!({
                "success": false,
                "message": "Additional permissions required",
                "requires_reauth": true,
                "reconnect_url": "/youtube/connect?reauth=true"
            })),
        ));
    }

    // Delete from YouTube
    youtube.delete_playlist(&channel.access_token, &playlist_id)
        .await
        .map_err(|e| {
            (
                StatusCode::BAD_GATEWAY,
                Json(json!({"success": false, "message": format!("YouTube API error: {}", e)})),
            )
        })?;

    // Delete from database (cascade will handle playlist items)
    sqlx::query("DELETE FROM youtube_playlists WHERE id = $1")
        .bind(playlist.id)
        .execute(&state.db_pool)
        .await
        .ok();

    Ok(Json(json!({
        "success": true,
        "message": "Playlist deleted successfully",
        "playlist_id": playlist_id
    })))
}

/// Add video to playlist
///
/// POST /api/youtube/playlists/:id/videos
pub async fn add_video_to_playlist(
    Path(playlist_id): Path<String>,
    Extension(state): Extension<Arc<AppState>>,
    Extension(claims): Extension<crate::models::auth::Claims>,
    Json(payload): Json<crate::models::youtube::AddVideoToPlaylistRequest>,
) -> Result<Json<serde_json::Value>, (StatusCode, Json<serde_json::Value>)> {
    let user_id = claims.sub.parse::<i32>().unwrap_or(0);

    let youtube = state.youtube_client.as_ref().ok_or_else(|| {
        (
            StatusCode::SERVICE_UNAVAILABLE,
            Json(json!({"success": false, "message": "YouTube client not initialized"})),
        )
    })?;

    // Verify ownership
    let playlist = sqlx::query_as::<_, crate::models::youtube::YouTubePlaylist>(
        "SELECT * FROM youtube_playlists WHERE youtube_playlist_id = $1 AND user_id = $2"
    )
    .bind(&playlist_id)
    .bind(user_id)
    .fetch_optional(&state.db_pool)
    .await
    .map_err(|_| {
        (
            StatusCode::INTERNAL_SERVER_ERROR,
            Json(json!({"success": false, "message": "Database error"})),
        )
    })?
    .ok_or_else(|| {
        (
            StatusCode::NOT_FOUND,
            Json(json!({"success": false, "message": "Playlist not found"})),
        )
    })?;

    // Get channel
    let channel = sqlx::query_as::<_, crate::models::youtube::ConnectedYouTubeChannel>(
        "SELECT * FROM connected_youtube_channels WHERE id = $1"
    )
    .bind(playlist.channel_id)
    .fetch_optional(&state.db_pool)
    .await
    .map_err(|_| {
        (
            StatusCode::INTERNAL_SERVER_ERROR,
            Json(json!({"success": false, "message": "Database error"})),
        )
    })?
    .ok_or_else(|| {
        (
            StatusCode::NOT_FOUND,
            Json(json!({"success": false, "message": "Channel not found"})),
        )
    })?;

    // Add to YouTube
    let item_response = youtube.add_video_to_playlist(
        &channel.access_token,
        &playlist_id,
        &payload.video_id,
        payload.position,
    )
    .await
    .map_err(|e| {
        (
            StatusCode::BAD_GATEWAY,
            Json(json!({"success": false, "message": format!("YouTube API error: {}", e)})),
        )
    })?;

    // Save to database
    sqlx::query(
        "INSERT INTO youtube_playlist_items (playlist_id, youtube_video_id, youtube_playlist_item_id, position)
         VALUES ($1, $2, $3, $4)
         ON CONFLICT (playlist_id, youtube_video_id) DO UPDATE SET position = $4"
    )
    .bind(playlist.id)
    .bind(&payload.video_id)
    .bind(&item_response.id)
    .bind(item_response.snippet.position)
    .execute(&state.db_pool)
    .await
    .ok();

    // Increment video count
    sqlx::query("UPDATE youtube_playlists SET video_count = video_count + 1 WHERE id = $1")
        .bind(playlist.id)
        .execute(&state.db_pool)
        .await
        .ok();

    Ok(Json(json!({
        "success": true,
        "message": "Video added to playlist successfully",
        "item_id": item_response.id
    })))
}

/// Remove video from playlist
///
/// DELETE /api/youtube/playlists/:playlist_id/videos/:video_id
pub async fn remove_video_from_playlist(
    Path((playlist_id, video_id)): Path<(String, String)>,
    Extension(state): Extension<Arc<AppState>>,
    Extension(claims): Extension<crate::models::auth::Claims>,
) -> Result<Json<serde_json::Value>, (StatusCode, Json<serde_json::Value>)> {
    let user_id = claims.sub.parse::<i32>().unwrap_or(0);

    let youtube = state.youtube_client.as_ref().ok_or_else(|| {
        (
            StatusCode::SERVICE_UNAVAILABLE,
            Json(json!({"success": false, "message": "YouTube client not initialized"})),
        )
    })?;

    // Verify ownership and get playlist item ID
    let playlist = sqlx::query_as::<_, crate::models::youtube::YouTubePlaylist>(
        "SELECT * FROM youtube_playlists WHERE youtube_playlist_id = $1 AND user_id = $2"
    )
    .bind(&playlist_id)
    .bind(user_id)
    .fetch_optional(&state.db_pool)
    .await
    .map_err(|_| {
        (
            StatusCode::INTERNAL_SERVER_ERROR,
            Json(json!({"success": false, "message": "Database error"})),
        )
    })?
    .ok_or_else(|| {
        (
            StatusCode::NOT_FOUND,
            Json(json!({"success": false, "message": "Playlist not found"})),
        )
    })?;

    let item = sqlx::query_as::<_, crate::models::youtube::YouTubePlaylistItem>(
        "SELECT * FROM youtube_playlist_items WHERE playlist_id = $1 AND youtube_video_id = $2"
    )
    .bind(playlist.id)
    .bind(&video_id)
    .fetch_optional(&state.db_pool)
    .await
    .map_err(|_| {
        (
            StatusCode::INTERNAL_SERVER_ERROR,
            Json(json!({"success": false, "message": "Database error"})),
        )
    })?
    .ok_or_else(|| {
        (
            StatusCode::NOT_FOUND,
            Json(json!({"success": false, "message": "Video not in playlist"})),
        )
    })?;

    let playlist_item_id = item.youtube_playlist_item_id.ok_or_else(|| {
        (
            StatusCode::BAD_REQUEST,
            Json(json!({"success": false, "message": "Invalid playlist item"})),
        )
    })?;

    // Get channel
    let channel = sqlx::query_as::<_, crate::models::youtube::ConnectedYouTubeChannel>(
        "SELECT * FROM connected_youtube_channels WHERE id = $1"
    )
    .bind(playlist.channel_id)
    .fetch_optional(&state.db_pool)
    .await
    .map_err(|_| {
        (
            StatusCode::INTERNAL_SERVER_ERROR,
            Json(json!({"success": false, "message": "Database error"})),
        )
    })?
    .ok_or_else(|| {
        (
            StatusCode::NOT_FOUND,
            Json(json!({"success": false, "message": "Channel not found"})),
        )
    })?;

    // Remove from YouTube
    youtube.remove_video_from_playlist(&channel.access_token, &playlist_item_id)
        .await
        .map_err(|e| {
            (
                StatusCode::BAD_GATEWAY,
                Json(json!({"success": false, "message": format!("YouTube API error: {}", e)})),
            )
        })?;

    // Remove from database
    sqlx::query("DELETE FROM youtube_playlist_items WHERE id = $1")
        .bind(item.id)
        .execute(&state.db_pool)
        .await
        .ok();

    // Decrement video count
    sqlx::query("UPDATE youtube_playlists SET video_count = GREATEST(0, video_count - 1) WHERE id = $1")
        .bind(playlist.id)
        .execute(&state.db_pool)
        .await
        .ok();

    Ok(Json(json!({
        "success": true,
        "message": "Video removed from playlist successfully"
    })))
}

// ============================================================================
// Analytics Handlers
// ============================================================================

/// Get video analytics for a date range
///
/// GET /api/youtube/videos/:video_id/analytics?startDate=YYYY-MM-DD&endDate=YYYY-MM-DD
pub async fn get_video_analytics(
    Path(video_id): Path<String>,
    Query(params): Query<crate::models::youtube::AnalyticsRequest>,
    Extension(state): Extension<Arc<AppState>>,
    Extension(claims): Extension<crate::models::auth::Claims>,
) -> Result<Json<serde_json::Value>, (StatusCode, Json<serde_json::Value>)> {
    let user_id = claims.sub.parse::<i32>().unwrap_or(0);

    let analytics_client = state.youtube_analytics_client.as_ref().ok_or_else(|| {
        (
            StatusCode::SERVICE_UNAVAILABLE,
            Json(json!({"success": false, "message": "YouTube Analytics client not initialized"})),
        )
    })?;

    // Verify ownership
    let upload = sqlx::query_as::<_, crate::models::youtube::YouTubeUpload>(
        "SELECT * FROM youtube_uploads WHERE youtube_video_id = $1 AND user_id = $2"
    )
    .bind(&video_id)
    .bind(user_id)
    .fetch_optional(&state.db_pool)
    .await
    .map_err(|_| {
        (
            StatusCode::INTERNAL_SERVER_ERROR,
            Json(json!({"success": false, "message": "Database error"})),
        )
    })?
    .ok_or_else(|| {
        (
            StatusCode::NOT_FOUND,
            Json(json!({"success": false, "message": "Video not found"})),
        )
    })?;

    // Get channel for access token
    let channel = sqlx::query_as::<_, crate::models::youtube::ConnectedYouTubeChannel>(
        "SELECT * FROM connected_youtube_channels WHERE id = $1"
    )
    .bind(upload.channel_id)
    .fetch_optional(&state.db_pool)
    .await
    .map_err(|_| {
        (
            StatusCode::INTERNAL_SERVER_ERROR,
            Json(json!({"success": false, "message": "Database error"})),
        )
    })?
    .ok_or_else(|| {
        (
            StatusCode::NOT_FOUND,
            Json(json!({"success": false, "message": "Channel not found"})),
        )
    })?;

    // Check scope
    if !channel.granted_scopes.contains("yt-analytics.readonly") {
        return Err((
            StatusCode::FORBIDDEN,
            Json(json!({
                "success": false,
                "message": "YouTube Analytics permission required",
                "requires_reauth": true,
                "reconnect_url": "/youtube/connect?reauth=true"
            })),
        ));
    }

    // Fetch from YouTube Analytics API
    let analytics_response = analytics_client.get_video_analytics(
        &channel.access_token,
        &video_id,
        &params.start_date,
        &params.end_date,
    )
    .await
    .map_err(|e| {
        (
            StatusCode::BAD_GATEWAY,
            Json(json!({"success": false, "message": format!("YouTube Analytics API error: {}", e)})),
        )
    })?;

    let metrics = analytics_response.to_metrics().ok_or_else(|| {
        (
            StatusCode::NOT_FOUND,
            Json(json!({"success": false, "message": "No analytics data available for this video"})),
        )
    })?;

    Ok(Json(json!({
        "success": true,
        "video_id": video_id,
        "date_range": {
            "start": params.start_date,
            "end": params.end_date
        },
        "metrics": {
            "views": metrics.views,
            "watch_time_minutes": metrics.watch_time_minutes,
            "average_view_duration": metrics.average_view_duration,
            "average_view_percentage": metrics.average_view_percentage,
            "likes": metrics.likes,
            "dislikes": metrics.dislikes,
            "comments": metrics.comments,
            "shares": metrics.shares,
            "subscribers_gained": metrics.subscribers_gained,
            "subscribers_lost": metrics.subscribers_lost
        }
    })))
}

/// Get real-time video statistics
///
/// GET /api/youtube/videos/:video_id/analytics/realtime
pub async fn get_realtime_stats(
    Path(video_id): Path<String>,
    Extension(state): Extension<Arc<AppState>>,
    Extension(claims): Extension<crate::models::auth::Claims>,
) -> Result<Json<serde_json::Value>, (StatusCode, Json<serde_json::Value>)> {
    let user_id = claims.sub.parse::<i32>().unwrap_or(0);

    let youtube = state.youtube_client.as_ref().ok_or_else(|| {
        (
            StatusCode::SERVICE_UNAVAILABLE,
            Json(json!({"success": false, "message": "YouTube client not initialized"})),
        )
    })?;

    // Verify ownership
    let upload = sqlx::query_as::<_, crate::models::youtube::YouTubeUpload>(
        "SELECT * FROM youtube_uploads WHERE youtube_video_id = $1 AND user_id = $2"
    )
    .bind(&video_id)
    .bind(user_id)
    .fetch_optional(&state.db_pool)
    .await
    .map_err(|_| {
        (
            StatusCode::INTERNAL_SERVER_ERROR,
            Json(json!({"success": false, "message": "Database error"})),
        )
    })?
    .ok_or_else(|| {
        (
            StatusCode::NOT_FOUND,
            Json(json!({"success": false, "message": "Video not found"})),
        )
    })?;

    // Get channel
    let channel = sqlx::query_as::<_, crate::models::youtube::ConnectedYouTubeChannel>(
        "SELECT * FROM connected_youtube_channels WHERE id = $1"
    )
    .bind(upload.channel_id)
    .fetch_optional(&state.db_pool)
    .await
    .map_err(|_| {
        (
            StatusCode::INTERNAL_SERVER_ERROR,
            Json(json!({"success": false, "message": "Database error"})),
        )
    })?
    .ok_or_else(|| {
        (
            StatusCode::NOT_FOUND,
            Json(json!({"success": false, "message": "Channel not found"})),
        )
    })?;

    // Get stats from YouTube Data API
    let stats_response = youtube.get_video_realtime_stats(&channel.access_token, &video_id)
        .await
        .map_err(|e| {
            (
                StatusCode::BAD_GATEWAY,
                Json(json!({"success": false, "message": format!("YouTube API error: {}", e)})),
            )
        })?;

    let video_item = stats_response.items.first().ok_or_else(|| {
        (
            StatusCode::NOT_FOUND,
            Json(json!({"success": false, "message": "Video not found on YouTube"})),
        )
    })?;

    Ok(Json(json!({
        "success": true,
        "video_id": video_id,
        "title": video_item.snippet.title,
        "stats": {
            "view_count": video_item.statistics.view_count,
            "like_count": video_item.statistics.like_count,
            "comment_count": video_item.statistics.comment_count
        }
    })))
}

/// Get channel analytics
///
/// GET /api/youtube/channels/:id/analytics?startDate=YYYY-MM-DD&endDate=YYYY-MM-DD
pub async fn get_channel_analytics(
    Path(channel_id): Path<i32>,
    Query(params): Query<crate::models::youtube::AnalyticsRequest>,
    Extension(state): Extension<Arc<AppState>>,
    Extension(claims): Extension<crate::models::auth::Claims>,
) -> Result<Json<serde_json::Value>, (StatusCode, Json<serde_json::Value>)> {
    let user_id = claims.sub.parse::<i32>().unwrap_or(0);

    let analytics_client = state.youtube_analytics_client.as_ref().ok_or_else(|| {
        (
            StatusCode::SERVICE_UNAVAILABLE,
            Json(json!({"success": false, "message": "YouTube Analytics client not initialized"})),
        )
    })?;

    // Verify ownership
    let channel = sqlx::query_as::<_, crate::models::youtube::ConnectedYouTubeChannel>(
        "SELECT * FROM connected_youtube_channels WHERE id = $1 AND user_id = $2"
    )
    .bind(channel_id)
    .bind(user_id)
    .fetch_optional(&state.db_pool)
    .await
    .map_err(|_| {
        (
            StatusCode::INTERNAL_SERVER_ERROR,
            Json(json!({"success": false, "message": "Database error"})),
        )
    })?
    .ok_or_else(|| {
        (
            StatusCode::NOT_FOUND,
            Json(json!({"success": false, "message": "Channel not found"})),
        )
    })?;

    // Check scope
    if !channel.granted_scopes.contains("yt-analytics.readonly") {
        return Err((
            StatusCode::FORBIDDEN,
            Json(json!({
                "success": false,
                "message": "YouTube Analytics permission required",
                "requires_reauth": true,
                "reconnect_url": "/youtube/connect?reauth=true"
            })),
        ));
    }

    // Fetch analytics
    let analytics_response = analytics_client.get_channel_analytics(
        &channel.access_token,
        &params.start_date,
        &params.end_date,
    )
    .await
    .map_err(|e| {
        (
            StatusCode::BAD_GATEWAY,
            Json(json!({"success": false, "message": format!("YouTube Analytics API error: {}", e)})),
        )
    })?;

    let metrics = analytics_response.to_metrics().ok_or_else(|| {
        (
            StatusCode::NOT_FOUND,
            Json(json!({"success": false, "message": "No analytics data available"})),
        )
    })?;

    Ok(Json(json!({
        "success": true,
        "channel_id": channel_id,
        "date_range": {
            "start": params.start_date,
            "end": params.end_date
        },
        "metrics": {
            "views": metrics.views,
            "watch_time_minutes": metrics.watch_time_minutes,
            "subscribers_gained": metrics.subscribers_gained,
            "subscribers_lost": metrics.subscribers_lost
        }
    })))
}

// ============================================================================
// Search & Discovery Handlers
// ============================================================================

/// Search YouTube videos
///
/// GET /api/youtube/search?q=query&maxResults=25&order=relevance
pub async fn search_videos(
    Query(params): Query<crate::models::youtube::SearchRequest>,
    Extension(state): Extension<Arc<AppState>>,
    Extension(claims): Extension<crate::models::auth::Claims>,
) -> Result<Json<serde_json::Value>, (StatusCode, Json<serde_json::Value>)> {
    let youtube = state.youtube_client.as_ref().ok_or_else(|| {
        (
            StatusCode::SERVICE_UNAVAILABLE,
            Json(json!({"success": false, "message": "YouTube client not initialized"})),
        )
    })?;

    // Get user's access token if available for personalized search
    let user_id = claims.sub.parse::<i32>().unwrap_or(0);
    let channel = sqlx::query_as::<_, crate::models::youtube::ConnectedYouTubeChannel>(
        "SELECT * FROM connected_youtube_channels WHERE user_id = $1 AND is_active = true LIMIT 1"
    )
    .bind(user_id)
    .fetch_optional(&state.db_pool)
    .await
    .ok()
    .flatten();

    let access_token = channel.as_ref().map(|c| c.access_token.as_str());
    let max_results = params.max_results.unwrap_or(25).min(50);

    let search_response = youtube.search_videos(
        access_token,
        &params.query,
        max_results,
        params.order.as_deref(),
    )
    .await
    .map_err(|e| {
        (
            StatusCode::BAD_GATEWAY,
            Json(json!({"success": false, "message": format!("YouTube API error: {}", e)})),
        )
    })?;

    let results: Vec<_> = search_response.items.iter().map(|item| {
        json!({
            "video_id": item.id.video_id,
            "title": item.snippet.title,
            "description": item.snippet.description,
            "channel_id": item.snippet.channel_id,
            "channel_title": item.snippet.channel_title,
            "thumbnail_url": item.snippet.thumbnails.get("default").and_then(|t| t.get("url")),
            "published_at": item.snippet.published_at
        })
    }).collect();

    Ok(Json(json!({
        "success": true,
        "query": params.query,
        "results": results,
        "total_results": search_response.page_info.total_results
    })))
}

/// Get trending videos
///
/// GET /api/youtube/trending?regionCode=US&categoryId=22&maxResults=25
pub async fn get_trending_videos(
    Query(params): Query<crate::models::youtube::TrendingRequest>,
    Extension(state): Extension<Arc<AppState>>,
) -> Result<Json<serde_json::Value>, (StatusCode, Json<serde_json::Value>)> {
    let youtube = state.youtube_client.as_ref().ok_or_else(|| {
        (
            StatusCode::SERVICE_UNAVAILABLE,
            Json(json!({"success": false, "message": "YouTube client not initialized"})),
        )
    })?;

    let max_results = params.max_results.unwrap_or(25).min(50);

    let trending_response = youtube.get_trending_videos(
        params.region_code.as_deref(),
        params.category_id.as_deref(),
        max_results,
    )
    .await
    .map_err(|e| {
        (
            StatusCode::BAD_GATEWAY,
            Json(json!({"success": false, "message": format!("YouTube API error: {}", e)})),
        )
    })?;

    let results: Vec<_> = trending_response.items.iter().map(|item| {
        json!({
            "video_id": item.id,
            "title": item.snippet.title,
            "channel_title": item.snippet.channel_title,
            "thumbnail_url": item.snippet.thumbnails.get("default").and_then(|t| t.get("url")),
            "view_count": item.statistics.view_count,
            "like_count": item.statistics.like_count,
            "comment_count": item.statistics.comment_count
        })
    }).collect();

    Ok(Json(json!({
        "success": true,
        "results": results
    })))
}

/// Get related videos
///
/// GET /api/youtube/videos/:video_id/related?maxResults=20
pub async fn get_related_videos(
    Path(video_id): Path<String>,
    Query(params): Query<std::collections::HashMap<String, String>>,
    Extension(state): Extension<Arc<AppState>>,
) -> Result<Json<serde_json::Value>, (StatusCode, Json<serde_json::Value>)> {
    let youtube = state.youtube_client.as_ref().ok_or_else(|| {
        (
            StatusCode::SERVICE_UNAVAILABLE,
            Json(json!({"success": false, "message": "YouTube client not initialized"})),
        )
    })?;

    let max_results = params.get("maxResults")
        .and_then(|v| v.parse::<i32>().ok())
        .unwrap_or(20)
        .min(50);

    let related_response = youtube.get_related_videos(&video_id, max_results)
        .await
        .map_err(|e| {
            (
                StatusCode::BAD_GATEWAY,
                Json(json!({"success": false, "message": format!("YouTube API error: {}", e)})),
            )
        })?;

    let results: Vec<_> = related_response.items.iter().map(|item| {
        json!({
            "video_id": item.id.video_id,
            "title": item.snippet.title,
            "channel_title": item.snippet.channel_title,
            "thumbnail_url": item.snippet.thumbnails.get("default").and_then(|t| t.get("url"))
        })
    }).collect();

    Ok(Json(json!({
        "success": true,
        "related_to": video_id,
        "results": results
    })))
}

// ============================================================================
// Comment Moderation Handlers
// ============================================================================

/// Get video comments
///
/// GET /api/youtube/videos/:video_id/comments?maxResults=100
pub async fn get_video_comments(
    Path(video_id): Path<String>,
    Query(params): Query<std::collections::HashMap<String, String>>,
    Extension(state): Extension<Arc<AppState>>,
    Extension(claims): Extension<crate::models::auth::Claims>,
) -> Result<Json<serde_json::Value>, (StatusCode, Json<serde_json::Value>)> {
    let user_id = claims.sub.parse::<i32>().unwrap_or(0);

    let youtube = state.youtube_client.as_ref().ok_or_else(|| {
        (
            StatusCode::SERVICE_UNAVAILABLE,
            Json(json!({"success": false, "message": "YouTube client not initialized"})),
        )
    })?;

    // Verify ownership
    let upload = sqlx::query_as::<_, crate::models::youtube::YouTubeUpload>(
        "SELECT * FROM youtube_uploads WHERE youtube_video_id = $1 AND user_id = $2"
    )
    .bind(&video_id)
    .bind(user_id)
    .fetch_optional(&state.db_pool)
    .await
    .map_err(|_| {
        (
            StatusCode::INTERNAL_SERVER_ERROR,
            Json(json!({"success": false, "message": "Database error"})),
        )
    })?
    .ok_or_else(|| {
        (
            StatusCode::NOT_FOUND,
            Json(json!({"success": false, "message": "Video not found"})),
        )
    })?;

    // Get channel
    let channel = sqlx::query_as::<_, crate::models::youtube::ConnectedYouTubeChannel>(
        "SELECT * FROM connected_youtube_channels WHERE id = $1"
    )
    .bind(upload.channel_id)
    .fetch_optional(&state.db_pool)
    .await
    .map_err(|_| {
        (
            StatusCode::INTERNAL_SERVER_ERROR,
            Json(json!({"success": false, "message": "Database error"})),
        )
    })?
    .ok_or_else(|| {
        (
            StatusCode::NOT_FOUND,
            Json(json!({"success": false, "message": "Channel not found"})),
        )
    })?;

    let max_results = params.get("maxResults")
        .and_then(|v| v.parse::<i32>().ok())
        .unwrap_or(100);

    let comments_response = youtube.get_video_comments(&channel.access_token, &video_id, max_results)
        .await
        .map_err(|e| {
            (
                StatusCode::BAD_GATEWAY,
                Json(json!({"success": false, "message": format!("YouTube API error: {}", e)})),
            )
        })?;

    let comments: Vec<_> = comments_response.items.iter().map(|thread| {
        let comment = &thread.snippet.top_level_comment;
        json!({
            "comment_id": comment.id,
            "author_name": comment.snippet.author_display_name,
            "text": comment.snippet.text_display,
            "like_count": comment.snippet.like_count,
            "published_at": comment.snippet.published_at,
            "reply_count": thread.snippet.total_reply_count,
            "can_reply": thread.snippet.can_reply
        })
    }).collect();

    Ok(Json(json!({
        "success": true,
        "video_id": video_id,
        "comments": comments,
        "total": comments_response.page_info.total_results
    })))
}

/// Reply to a comment
///
/// POST /api/youtube/comments/:comment_id/reply
pub async fn reply_to_comment(
    Path(comment_id): Path<String>,
    Extension(state): Extension<Arc<AppState>>,
    Extension(claims): Extension<crate::models::auth::Claims>,
    Json(payload): Json<crate::models::youtube::ReplyToCommentRequest>,
) -> Result<Json<serde_json::Value>, (StatusCode, Json<serde_json::Value>)> {
    let user_id = claims.sub.parse::<i32>().unwrap_or(0);

    let youtube = state.youtube_client.as_ref().ok_or_else(|| {
        (
            StatusCode::SERVICE_UNAVAILABLE,
            Json(json!({"success": false, "message": "YouTube client not initialized"})),
        )
    })?;

    // Get any active channel for the user
    let channel = sqlx::query_as::<_, crate::models::youtube::ConnectedYouTubeChannel>(
        "SELECT * FROM connected_youtube_channels WHERE user_id = $1 AND is_active = true LIMIT 1"
    )
    .bind(user_id)
    .fetch_optional(&state.db_pool)
    .await
    .map_err(|_| {
        (
            StatusCode::INTERNAL_SERVER_ERROR,
            Json(json!({"success": false, "message": "Database error"})),
        )
    })?
    .ok_or_else(|| {
        (
            StatusCode::NOT_FOUND,
            Json(json!({"success": false, "message": "No YouTube channel connected"})),
        )
    })?;

    // Check scope
    if !channel.granted_scopes.contains("youtube.force-ssl") {
        return Err((
            StatusCode::FORBIDDEN,
            Json(json!({
                "success": false,
                "message": "Additional permissions required",
                "requires_reauth": true,
                "reconnect_url": "/youtube/connect?reauth=true"
            })),
        ));
    }

    // Reply on YouTube
    let reply_response = youtube.reply_to_comment(&channel.access_token, &comment_id, &payload.text)
        .await
        .map_err(|e| {
            (
                StatusCode::BAD_GATEWAY,
                Json(json!({"success": false, "message": format!("YouTube API error: {}", e)})),
            )
        })?;

    Ok(Json(json!({
        "success": true,
        "message": "Reply posted successfully",
        "comment_id": reply_response.id
    })))
}

/// Delete a comment
///
/// DELETE /api/youtube/comments/:comment_id
pub async fn delete_comment(
    Path(comment_id): Path<String>,
    Extension(state): Extension<Arc<AppState>>,
    Extension(claims): Extension<crate::models::auth::Claims>,
) -> Result<Json<serde_json::Value>, (StatusCode, Json<serde_json::Value>)> {
    let user_id = claims.sub.parse::<i32>().unwrap_or(0);

    let youtube = state.youtube_client.as_ref().ok_or_else(|| {
        (
            StatusCode::SERVICE_UNAVAILABLE,
            Json(json!({"success": false, "message": "YouTube client not initialized"})),
        )
    })?;

    // Get channel
    let channel = sqlx::query_as::<_, crate::models::youtube::ConnectedYouTubeChannel>(
        "SELECT * FROM connected_youtube_channels WHERE user_id = $1 AND is_active = true LIMIT 1"
    )
    .bind(user_id)
    .fetch_optional(&state.db_pool)
    .await
    .map_err(|_| {
        (
            StatusCode::INTERNAL_SERVER_ERROR,
            Json(json!({"success": false, "message": "Database error"})),
        )
    })?
    .ok_or_else(|| {
        (
            StatusCode::NOT_FOUND,
            Json(json!({"success": false, "message": "No YouTube channel connected"})),
        )
    })?;

    // Check scope
    if !channel.granted_scopes.contains("youtube.force-ssl") {
        return Err((
            StatusCode::FORBIDDEN,
            Json(json!({
                "success": false,
                "message": "Additional permissions required",
                "requires_reauth": true,
                "reconnect_url": "/youtube/connect?reauth=true"
            })),
        ));
    }

    // Delete from YouTube
    youtube.delete_comment(&channel.access_token, &comment_id)
        .await
        .map_err(|e| {
            (
                StatusCode::BAD_GATEWAY,
                Json(json!({"success": false, "message": format!("YouTube API error: {}", e)})),
            )
        })?;

    Ok(Json(json!({
        "success": true,
        "message": "Comment deleted successfully",
        "comment_id": comment_id
    })))
}

// ============================================================================
// Caption Management Handlers
// ============================================================================

/// List caption tracks for a video
///
/// GET /api/youtube/videos/:video_id/captions
pub async fn list_captions(
    Path(video_id): Path<String>,
    Extension(state): Extension<Arc<AppState>>,
    Extension(claims): Extension<crate::models::auth::Claims>,
) -> Result<Json<serde_json::Value>, (StatusCode, Json<serde_json::Value>)> {
    let user_id = claims.sub.parse::<i32>().unwrap_or(0);

    let youtube = state.youtube_client.as_ref().ok_or_else(|| {
        (
            StatusCode::SERVICE_UNAVAILABLE,
            Json(json!({"success": false, "message": "YouTube client not initialized"})),
        )
    })?;

    // Verify ownership
    let upload = sqlx::query_as::<_, crate::models::youtube::YouTubeUpload>(
        "SELECT * FROM youtube_uploads WHERE youtube_video_id = $1 AND user_id = $2"
    )
    .bind(&video_id)
    .bind(user_id)
    .fetch_optional(&state.db_pool)
    .await
    .map_err(|_| {
        (
            StatusCode::INTERNAL_SERVER_ERROR,
            Json(json!({"success": false, "message": "Database error"})),
        )
    })?
    .ok_or_else(|| {
        (
            StatusCode::NOT_FOUND,
            Json(json!({"success": false, "message": "Video not found"})),
        )
    })?;

    // Get channel
    let channel = sqlx::query_as::<_, crate::models::youtube::ConnectedYouTubeChannel>(
        "SELECT * FROM connected_youtube_channels WHERE id = $1"
    )
    .bind(upload.channel_id)
    .fetch_optional(&state.db_pool)
    .await
    .map_err(|_| {
        (
            StatusCode::INTERNAL_SERVER_ERROR,
            Json(json!({"success": false, "message": "Database error"})),
        )
    })?
    .ok_or_else(|| {
        (
            StatusCode::NOT_FOUND,
            Json(json!({"success": false, "message": "Channel not found"})),
        )
    })?;

    let captions_response = youtube.list_captions(&channel.access_token, &video_id)
        .await
        .map_err(|e| {
            (
                StatusCode::BAD_GATEWAY,
                Json(json!({"success": false, "message": format!("YouTube API error: {}", e)})),
            )
        })?;

    let captions: Vec<_> = captions_response.items.iter().map(|track| {
        json!({
            "caption_id": track.id,
            "language": track.snippet.language,
            "name": track.snippet.name,
            "track_kind": track.snippet.track_kind,
            "is_draft": track.snippet.is_draft
        })
    }).collect();

    Ok(Json(json!({
        "success": true,
        "video_id": video_id,
        "captions": captions
    })))
}

/// Upload caption file
///
/// POST /api/youtube/videos/:video_id/captions
pub async fn upload_caption(
    Path(video_id): Path<String>,
    Extension(state): Extension<Arc<AppState>>,
    Extension(claims): Extension<crate::models::auth::Claims>,
    Json(payload): Json<crate::models::youtube::UploadCaptionRequest>,
) -> Result<Json<serde_json::Value>, (StatusCode, Json<serde_json::Value>)> {
    let user_id = claims.sub.parse::<i32>().unwrap_or(0);

    let youtube = state.youtube_client.as_ref().ok_or_else(|| {
        (
            StatusCode::SERVICE_UNAVAILABLE,
            Json(json!({"success": false, "message": "YouTube client not initialized"})),
        )
    })?;

    // Verify ownership
    let upload = sqlx::query_as::<_, crate::models::youtube::YouTubeUpload>(
        "SELECT * FROM youtube_uploads WHERE youtube_video_id = $1 AND user_id = $2"
    )
    .bind(&video_id)
    .bind(user_id)
    .fetch_optional(&state.db_pool)
    .await
    .map_err(|_| {
        (
            StatusCode::INTERNAL_SERVER_ERROR,
            Json(json!({"success": false, "message": "Database error"})),
        )
    })?
    .ok_or_else(|| {
        (
            StatusCode::NOT_FOUND,
            Json(json!({"success": false, "message": "Video not found"})),
        )
    })?;

    // Get channel
    let channel = sqlx::query_as::<_, crate::models::youtube::ConnectedYouTubeChannel>(
        "SELECT * FROM connected_youtube_channels WHERE id = $1"
    )
    .bind(upload.channel_id)
    .fetch_optional(&state.db_pool)
    .await
    .map_err(|_| {
        (
            StatusCode::INTERNAL_SERVER_ERROR,
            Json(json!({"success": false, "message": "Database error"})),
        )
    })?
    .ok_or_else(|| {
        (
            StatusCode::NOT_FOUND,
            Json(json!({"success": false, "message": "Channel not found"})),
        )
    })?;

    // Check scope
    if !channel.granted_scopes.contains("youtube.force-ssl") {
        return Err((
            StatusCode::FORBIDDEN,
            Json(json!({
                "success": false,
                "message": "Additional permissions required",
                "requires_reauth": true,
                "reconnect_url": "/youtube/connect?reauth=true"
            })),
        ));
    }

    // Read caption file
    let caption_data = tokio::fs::read(&payload.caption_file)
        .await
        .map_err(|e| {
            (
                StatusCode::BAD_REQUEST,
                Json(json!({"success": false, "message": format!("Failed to read caption file: {}", e)})),
            )
        })?;

    let name = payload.name.unwrap_or_else(|| format!("{} captions", payload.language));

    // Upload to YouTube
    let caption_response = youtube.upload_caption(
        &channel.access_token,
        &video_id,
        &payload.language,
        &name,
        caption_data,
    )
    .await
    .map_err(|e| {
        (
            StatusCode::BAD_GATEWAY,
            Json(json!({"success": false, "message": format!("YouTube API error: {}", e)})),
        )
    })?;

    // Save to database
    sqlx::query(
        "INSERT INTO youtube_captions (youtube_video_id, youtube_caption_id, language, name, track_kind, local_file_path)
         VALUES ($1, $2, $3, $4, $5, $6)"
    )
    .bind(&video_id)
    .bind(&caption_response.id)
    .bind(&payload.language)
    .bind(&name)
    .bind(&caption_response.snippet.track_kind)
    .bind(&payload.caption_file)
    .execute(&state.db_pool)
    .await
    .ok();

    Ok(Json(json!({
        "success": true,
        "message": "Caption uploaded successfully",
        "caption_id": caption_response.id,
        "language": payload.language
    })))
}

/// Delete a caption track
///
/// DELETE /api/youtube/captions/:caption_id
pub async fn delete_caption(
    Path(caption_id): Path<String>,
    Extension(state): Extension<Arc<AppState>>,
    Extension(claims): Extension<crate::models::auth::Claims>,
) -> Result<Json<serde_json::Value>, (StatusCode, Json<serde_json::Value>)> {
    let user_id = claims.sub.parse::<i32>().unwrap_or(0);

    let youtube = state.youtube_client.as_ref().ok_or_else(|| {
        (
            StatusCode::SERVICE_UNAVAILABLE,
            Json(json!({"success": false, "message": "YouTube client not initialized"})),
        )
    })?;

    // Get channel (any active channel for the user)
    let channel = sqlx::query_as::<_, crate::models::youtube::ConnectedYouTubeChannel>(
        "SELECT * FROM connected_youtube_channels WHERE user_id = $1 AND is_active = true LIMIT 1"
    )
    .bind(user_id)
    .fetch_optional(&state.db_pool)
    .await
    .map_err(|_| {
        (
            StatusCode::INTERNAL_SERVER_ERROR,
            Json(json!({"success": false, "message": "Database error"})),
        )
    })?
    .ok_or_else(|| {
        (
            StatusCode::NOT_FOUND,
            Json(json!({"success": false, "message": "No YouTube channel connected"})),
        )
    })?;

    // Check scope
    if !channel.granted_scopes.contains("youtube.force-ssl") {
        return Err((
            StatusCode::FORBIDDEN,
            Json(json!({
                "success": false,
                "message": "Additional permissions required",
                "requires_reauth": true,
                "reconnect_url": "/youtube/connect?reauth=true"
            })),
        ));
    }

    // Delete from YouTube
    youtube.delete_caption(&channel.access_token, &caption_id)
        .await
        .map_err(|e| {
            (
                StatusCode::BAD_GATEWAY,
                Json(json!({"success": false, "message": format!("YouTube API error: {}", e)})),
            )
        })?;

    // Delete from database
    sqlx::query("DELETE FROM youtube_captions WHERE youtube_caption_id = $1")
        .bind(&caption_id)
        .execute(&state.db_pool)
        .await
        .ok();

    Ok(Json(json!({
        "success": true,
        "message": "Caption deleted successfully",
        "caption_id": caption_id
    })))
}

// ============================================================================
// Scheduling Handler
// ============================================================================

/// Schedule video for future publication
///
/// POST /api/youtube/videos/:video_id/schedule
pub async fn schedule_video_publish(
    Path(video_id): Path<String>,
    Extension(state): Extension<Arc<AppState>>,
    Extension(claims): Extension<crate::models::auth::Claims>,
    Json(payload): Json<crate::models::youtube::ScheduleVideoRequest>,
) -> Result<Json<serde_json::Value>, (StatusCode, Json<serde_json::Value>)> {
    let user_id = claims.sub.parse::<i32>().unwrap_or(0);

    let youtube = state.youtube_client.as_ref().ok_or_else(|| {
        (
            StatusCode::SERVICE_UNAVAILABLE,
            Json(json!({"success": false, "message": "YouTube client not initialized"})),
        )
    })?;

    // Verify ownership
    let upload = sqlx::query_as::<_, crate::models::youtube::YouTubeUpload>(
        "SELECT * FROM youtube_uploads WHERE youtube_video_id = $1 AND user_id = $2"
    )
    .bind(&video_id)
    .bind(user_id)
    .fetch_optional(&state.db_pool)
    .await
    .map_err(|_| {
        (
            StatusCode::INTERNAL_SERVER_ERROR,
            Json(json!({"success": false, "message": "Database error"})),
        )
    })?
    .ok_or_else(|| {
        (
            StatusCode::NOT_FOUND,
            Json(json!({"success": false, "message": "Video not found"})),
        )
    })?;

    // Get channel
    let channel = sqlx::query_as::<_, crate::models::youtube::ConnectedYouTubeChannel>(
        "SELECT * FROM connected_youtube_channels WHERE id = $1"
    )
    .bind(upload.channel_id)
    .fetch_optional(&state.db_pool)
    .await
    .map_err(|_| {
        (
            StatusCode::INTERNAL_SERVER_ERROR,
            Json(json!({"success": false, "message": "Database error"})),
        )
    })?
    .ok_or_else(|| {
        (
            StatusCode::NOT_FOUND,
            Json(json!({"success": false, "message": "Channel not found"})),
        )
    })?;

    // Note: YouTube requires video to be private before scheduling
    // Update video with publishAt timestamp
    let update_body = json!({
        "id": video_id,
        "status": {
            "privacyStatus": "private",
            "publishAt": payload.publish_at
        }
    });

    // Make direct API call for scheduling
    let response = reqwest::Client::new()
        .put("https://www.googleapis.com/youtube/v3/videos")
        .query(&[("part", "status")])
        .header("Authorization", format!("Bearer {}", channel.access_token))
        .header("Content-Type", "application/json")
        .json(&update_body)
        .send()
        .await
        .map_err(|e| {
            (
                StatusCode::BAD_GATEWAY,
                Json(json!({"success": false, "message": format!("Failed to schedule video: {}", e)})),
            )
        })?;

    if !response.status().is_success() {
        let error_text = response.text().await.unwrap_or_default();
        return Err((
            StatusCode::BAD_GATEWAY,
            Json(json!({"success": false, "message": format!("YouTube API error: {}", error_text)})),
        ));
    }

    // Update database
    sqlx::query(
        "UPDATE youtube_uploads SET scheduled_publish_at = $1, is_scheduled = true, privacy_status = 'private', updated_at = NOW() WHERE id = $2"
    )
    .bind(&payload.publish_at)
    .bind(upload.id)
    .execute(&state.db_pool)
    .await
    .ok();

    Ok(Json(json!({
        "success": true,
        "message": "Video scheduled successfully",
        "video_id": video_id,
        "publish_at": payload.publish_at
    })))
}

// ============================================================================
// Resumable Upload Handlers
// ============================================================================

/// Initiate resumable upload for large files
///
/// POST /api/youtube/upload/resumable
pub async fn initiate_resumable_upload(
    Extension(state): Extension<Arc<AppState>>,
    Extension(claims): Extension<crate::models::auth::Claims>,
    Json(payload): Json<crate::models::youtube::InitiateResumableUploadRequest>,
) -> Result<Json<serde_json::Value>, (StatusCode, Json<serde_json::Value>)> {
    let user_id = claims.sub.parse::<i32>().unwrap_or(0);

    let youtube = state.youtube_client.as_ref().ok_or_else(|| {
        (
            StatusCode::SERVICE_UNAVAILABLE,
            Json(json!({"success": false, "message": "YouTube client not initialized"})),
        )
    })?;

    // Get channel
    let channel = sqlx::query_as::<_, crate::models::youtube::ConnectedYouTubeChannel>(
        "SELECT * FROM connected_youtube_channels WHERE id = $1 AND user_id = $2 AND is_active = true"
    )
    .bind(payload.channel_id)
    .bind(user_id)
    .fetch_optional(&state.db_pool)
    .await
    .map_err(|_| {
        (
            StatusCode::INTERNAL_SERVER_ERROR,
            Json(json!({"success": false, "message": "Database error"})),
        )
    })?
    .ok_or_else(|| {
        (
            StatusCode::NOT_FOUND,
            Json(json!({"success": false, "message": "Channel not found"})),
        )
    })?;

    // Initiate resumable upload session
    let session_response = youtube.initiate_resumable_upload(
        &channel.access_token,
        &payload.title,
        payload.description.as_deref().unwrap_or(""),
        &payload.privacy_status,
        payload.category.as_deref(),
        payload.tags.clone(),
        payload.file_size,
    )
    .await
    .map_err(|e| {
        (
            StatusCode::BAD_GATEWAY,
            Json(json!({"success": false, "message": format!("YouTube API error: {}", e)})),
        )
    })?;

    // Create upload record in database
    let upload_id: i32 = sqlx::query_scalar(
        "INSERT INTO youtube_uploads (
            user_id, channel_id, local_video_path, video_title, video_description,
            privacy_status, video_category, upload_status, upload_session_url,
            total_bytes, bytes_uploaded, is_resumable
        ) VALUES ($1, $2, $3, $4, $5, $6, $7, 'uploading', $8, $9, 0, true)
        RETURNING id"
    )
    .bind(user_id)
    .bind(payload.channel_id)
    .bind(&payload.video_path)
    .bind(&payload.title)
    .bind(payload.description.as_ref())
    .bind(&payload.privacy_status)
    .bind(payload.category.as_ref())
    .bind(&session_response.session_url)
    .bind(payload.file_size)
    .fetch_one(&state.db_pool)
    .await
    .map_err(|e| {
        tracing::error!("Failed to create upload record: {}", e);
        (
            StatusCode::INTERNAL_SERVER_ERROR,
            Json(json!({"success": false, "message": "Database error"})),
        )
    })?;

    Ok(Json(json!({
        "success": true,
        "message": "Resumable upload session initiated",
        "upload_id": upload_id,
        "session_url": session_response.session_url,
        "total_bytes": payload.file_size
    })))
}

/// Upload a chunk for resumable upload
///
/// PUT /api/youtube/upload/resumable/:upload_id/chunk
pub async fn upload_chunk(
    Path(upload_id): Path<i32>,
    Extension(state): Extension<Arc<AppState>>,
    Extension(claims): Extension<crate::models::auth::Claims>,
    body: axum::body::Bytes,
) -> Result<Json<serde_json::Value>, (StatusCode, Json<serde_json::Value>)> {
    let user_id = claims.sub.parse::<i32>().unwrap_or(0);

    let youtube = state.youtube_client.as_ref().ok_or_else(|| {
        (
            StatusCode::SERVICE_UNAVAILABLE,
            Json(json!({"success": false, "message": "YouTube client not initialized"})),
        )
    })?;

    // Get upload record
    let upload = sqlx::query_as::<_, crate::models::youtube::YouTubeUpload>(
        "SELECT * FROM youtube_uploads WHERE id = $1 AND user_id = $2 AND is_resumable = true"
    )
    .bind(upload_id)
    .bind(user_id)
    .fetch_optional(&state.db_pool)
    .await
    .map_err(|_| {
        (
            StatusCode::INTERNAL_SERVER_ERROR,
            Json(json!({"success": false, "message": "Database error"})),
        )
    })?
    .ok_or_else(|| {
        (
            StatusCode::NOT_FOUND,
            Json(json!({"success": false, "message": "Upload session not found"})),
        )
    })?;

    let session_url = upload.upload_session_url.ok_or_else(|| {
        (
            StatusCode::BAD_REQUEST,
            Json(json!({"success": false, "message": "No upload session URL"})),
        )
    })?;

    let total_bytes = upload.total_bytes.ok_or_else(|| {
        (
            StatusCode::BAD_REQUEST,
            Json(json!({"success": false, "message": "Total bytes not set"})),
        )
    })?;

    let bytes_uploaded = upload.bytes_uploaded.unwrap_or(0);
    let chunk_size = body.len() as i64;
    let start_byte = bytes_uploaded;
    let end_byte = start_byte + chunk_size - 1;

    // Upload chunk
    let chunk_response = youtube.upload_resumable_chunk(
        &session_url,
        body.to_vec(),
        start_byte,
        end_byte,
        total_bytes,
    )
    .await
    .map_err(|e| {
        (
            StatusCode::BAD_GATEWAY,
            Json(json!({"success": false, "message": format!("YouTube API error: {}", e)})),
        )
    })?;

    // Update progress in database
    let new_bytes_uploaded = end_byte + 1;
    let progress = ((new_bytes_uploaded as f64 / total_bytes as f64) * 100.0) as i32;

    sqlx::query(
        "UPDATE youtube_uploads SET bytes_uploaded = $1, upload_progress = $2, updated_at = NOW() WHERE id = $3"
    )
    .bind(new_bytes_uploaded)
    .bind(progress)
    .bind(upload_id)
    .execute(&state.db_pool)
    .await
    .ok();

    if chunk_response.complete {
        // Upload complete
        let video_response = chunk_response.video_response.ok_or_else(|| {
            (
                StatusCode::INTERNAL_SERVER_ERROR,
                Json(json!({"success": false, "message": "Upload complete but no video response"})),
            )
        })?;

        let youtube_url = format!("https://www.youtube.com/watch?v={}", video_response.id);

        // Update final status
        sqlx::query(
            "UPDATE youtube_uploads SET
             upload_status = 'completed',
             upload_progress = 100,
             youtube_video_id = $1,
             youtube_url = $2,
             published_at = NOW(),
             updated_at = NOW()
             WHERE id = $3"
        )
        .bind(&video_response.id)
        .bind(&youtube_url)
        .bind(upload_id)
        .execute(&state.db_pool)
        .await
        .ok();

        Ok(Json(json!({
            "success": true,
            "complete": true,
            "message": "Upload completed successfully",
            "video_id": video_response.id,
            "youtube_url": youtube_url
        })))
    } else {
        Ok(Json(json!({
            "success": true,
            "complete": false,
            "message": "Chunk uploaded successfully",
            "bytes_uploaded": new_bytes_uploaded,
            "total_bytes": total_bytes,
            "progress": progress
        })))
    }
}

// ============================================================================
// YouTube Coming Soon Page
// ============================================================================

pub async fn youtube_coming_soon_page() -> Html<String> {
    let html = r###"
<!DOCTYPE html>
<html lang="en">
<head>
    <meta charset="UTF-8">
    <meta name="viewport" content="width=device-width, initial-scale=1.0">
    <title>YouTube Features - Coming Soon | VideoSync</title>
    <style>
        * { margin: 0; padding: 0; box-sizing: border-box; }
        body {
            font-family: -apple-system, BlinkMacSystemFont, 'Segoe UI', Roboto, sans-serif;
            background: linear-gradient(135deg, #1a1a2e 0%, #16213e 50%, #0f1419 100%);
            color: #e8e8e8;
            min-height: 100vh;
            display: flex;
            align-items: center;
            justify-content: center;
            padding: 20px;
        }
        .container {
            max-width: 800px;
            background: rgba(30, 30, 52, 0.9);
            border: 1px solid rgba(59, 130, 246, 0.3);
            border-radius: 20px;
            padding: 3rem;
            text-align: center;
            backdrop-filter: blur(10px);
        }
        .icon {
            font-size: 5rem;
            margin-bottom: 1rem;
            animation: pulse 2s infinite;
        }
        @keyframes pulse {
            0%, 100% { transform: scale(1); }
            50% { transform: scale(1.1); }
        }
        h1 {
            font-size: 2.5rem;
            margin-bottom: 1rem;
            color: #3b82f6;
        }
        .subtitle {
            font-size: 1.2rem;
            color: #cbd5e1;
            margin-bottom: 2rem;
        }
        .features-box {
            background: rgba(15, 20, 25, 0.8);
            border: 1px solid rgba(59, 130, 246, 0.2);
            border-radius: 15px;
            padding: 2rem;
            margin: 2rem 0;
            text-align: left;
        }
        .features-box h2 {
            color: #3b82f6;
            margin-bottom: 1rem;
            text-align: center;
        }
        .feature-list {
            display: grid;
            grid-template-columns: repeat(auto-fit, minmax(200px, 1fr));
            gap: 1rem;
            margin-top: 1rem;
        }
        .feature-item {
            display: flex;
            align-items: center;
            gap: 0.5rem;
            color: #cbd5e1;
        }
        .feature-item::before {
            content: "✓";
            color: #10b981;
            font-weight: bold;
        }
        .other-features {
            margin-top: 2rem;
            text-align: left;
        }
        .other-features h3 {
            color: #f8fafc;
            margin-bottom: 1rem;
            text-align: center;
        }
        .tools-grid {
            display: grid;
            grid-template-columns: repeat(auto-fit, minmax(150px, 1fr));
            gap: 0.5rem;
            margin-top: 1rem;
        }
        .tool-item {
            color: #94a3b8;
            padding: 0.5rem;
            background: rgba(59, 130, 246, 0.1);
            border-radius: 8px;
            font-size: 0.9rem;
            text-align: center;
        }
        .cta-buttons {
            margin-top: 2rem;
            display: flex;
            gap: 1rem;
            justify-content: center;
            flex-wrap: wrap;
        }
        .btn {
            padding: 0.75rem 2rem;
            border-radius: 25px;
            text-decoration: none;
            font-weight: 600;
            transition: all 0.3s;
        }
        .btn-primary {
            background: linear-gradient(135deg, #3b82f6, #1d4ed8);
            color: white;
        }
        .btn-primary:hover {
            transform: translateY(-2px);
            box-shadow: 0 10px 20px rgba(59, 130, 246, 0.3);
        }
        .btn-secondary {
            background: rgba(30, 30, 52, 0.8);
            color: #e8e8e8;
            border: 2px solid rgba(59, 130, 246, 0.3);
        }
        .btn-secondary:hover {
            border-color: rgba(59, 130, 246, 0.6);
        }
        .info-banner {
            background: rgba(59, 130, 246, 0.1);
            border: 1px solid rgba(59, 130, 246, 0.3);
            border-radius: 10px;
            padding: 1rem;
            margin-top: 2rem;
            color: #94a3b8;
        }
    </style>
</head>
<body>
    <div class="container">
        <div class="icon">🎥</div>
        <h1>YouTube Integration Coming Soon!</h1>
        <p class="subtitle">We're preparing an amazing YouTube experience for you</p>

        <div class="features-box">
            <h2>What You'll Be Able to Do:</h2>
            <div class="feature-list">
                <div class="feature-item">Upload videos directly</div>
                <div class="feature-item">Manage video metadata</div>
                <div class="feature-item">Track analytics</div>
                <div class="feature-item">Create & organize playlists</div>
                <div class="feature-item">Moderate comments</div>
                <div class="feature-item">Upload captions</div>
                <div class="feature-item">Custom thumbnails</div>
                <div class="feature-item">Schedule publishing</div>
                <div class="feature-item">Real-time statistics</div>
                <div class="feature-item">Multi-channel support</div>
            </div>
        </div>

        <div class="other-features">
            <h3>In the Meantime, Explore These Features:</h3>
            <div class="tools-grid">
                <div class="tool-item">🎬 Video Trimming</div>
                <div class="tool-item">📦 Video Merging</div>
                <div class="tool-item">🎨 Filters & Effects</div>
                <div class="tool-item">📝 Text Overlays</div>
                <div class="tool-item">🔊 Audio Editing</div>
                <div class="tool-item">🖼️ Image Overlays</div>
                <div class="tool-item">⚡ Speed Adjustment</div>
                <div class="tool-item">🔄 Format Conversion</div>
                <div class="tool-item">✂️ Video Cropping</div>
                <div class="tool-item">🎭 Green Screen</div>
                <div class="tool-item">📐 Resize & Scale</div>
                <div class="tool-item">🎵 Background Music</div>
            </div>
        </div>

        <div class="cta-buttons">
            <a href="/dashboard" class="btn btn-primary">Go to Dashboard</a>
            <a href="/" class="btn btn-secondary">Back to Home</a>
        </div>

        <div class="info-banner">
            <strong>📧 Want Early Access?</strong><br>
            YouTube features are currently in testing mode. Contact your administrator to get whitelisted for early access.
        </div>
    </div>
</body>
</html>
    "###;

    Html(html.to_string())
}
