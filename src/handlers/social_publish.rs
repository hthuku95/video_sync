use crate::zernio_client::{self, PlatformTarget, ZernioClient};
use crate::AppState;
use axum::{
    extract::{Extension, Json, Query},
    http::StatusCode,
    routing::{get, post},
    Router,
};
use serde::{Deserialize, Serialize};
use serde_json::json;
use std::sync::Arc;

pub fn social_routes() -> Router {
    Router::new()
        .route("/api/social/create-profile", post(create_profile))
        .route("/api/social/profiles", get(list_profiles))
        .route("/api/social/connect-url", get(get_connect_url))
        .route("/api/social/accounts", get(list_accounts))
        .route("/api/social/publish", post(publish_post))
}

fn client(state: &Arc<AppState>) -> Result<ZernioClient, (StatusCode, Json<serde_json::Value>)> {
    state
        .zernio_client
        .clone()
        .ok_or_else(|| {
            (
                StatusCode::SERVICE_UNAVAILABLE,
                Json(json!({
                    "success": false,
                    "error": "Zernio client not configured. Set ZERNIO_API_KEY."
                })),
            )
        })
}

// ── Request / Response types ───────────────────────────────────────────────

#[derive(Debug, Deserialize)]
pub struct CreateProfilePayload {
    pub name: String,
    pub description: Option<String>,
}

#[derive(Debug, Deserialize)]
pub struct ConnectUrlQuery {
    pub platform: String,
    pub profile_id: String,
}

#[derive(Debug, Deserialize)]
pub struct PublishPayload {
    pub profile_id: String,
    pub text: String,
    pub platforms: Vec<PlatformTargetPayload>,
    #[serde(default)]
    pub media_urls: Vec<String>,
}

#[derive(Debug, Deserialize)]
pub struct PlatformTargetPayload {
    pub platform: String,
    pub account_id: String,
}

// ── Handlers ───────────────────────────────────────────────────────────────

pub async fn create_profile(
    Extension(state): Extension<Arc<AppState>>,
    Json(payload): Json<CreateProfilePayload>,
) -> Result<Json<serde_json::Value>, (StatusCode, Json<serde_json::Value>)> {
    let z = client(&state)?;
    match z
        .create_profile(&payload.name, payload.description.as_deref())
        .await
    {
        Ok(resp) => Ok(Json(json!({
            "success": true,
            "profile": resp.profile,
        }))),
        Err(e) => Ok(Json(json!({
            "success": false,
            "error": e.to_string(),
        }))),
    }
}

pub async fn list_profiles(
    Extension(state): Extension<Arc<AppState>>,
) -> Result<Json<serde_json::Value>, (StatusCode, Json<serde_json::Value>)> {
    let z = client(&state)?;
    match z.list_profiles().await {
        Ok(resp) => Ok(Json(json!({
            "success": true,
            "profiles": resp.profiles,
        }))),
        Err(e) => Ok(Json(json!({
            "success": false,
            "error": e.to_string(),
        }))),
    }
}

pub async fn get_connect_url(
    Extension(state): Extension<Arc<AppState>>,
    Query(query): Query<ConnectUrlQuery>,
) -> Result<Json<serde_json::Value>, (StatusCode, Json<serde_json::Value>)> {
    let z = client(&state)?;
    match z.get_connect_url(&query.platform, &query.profile_id).await {
        Ok(resp) => Ok(Json(json!({
            "success": true,
            "url": resp.url,
        }))),
        Err(e) => Ok(Json(json!({
            "success": false,
            "error": e.to_string(),
        }))),
    }
}

pub async fn list_accounts(
    Extension(state): Extension<Arc<AppState>>,
) -> Result<Json<serde_json::Value>, (StatusCode, Json<serde_json::Value>)> {
    let z = client(&state)?;
    match z.list_accounts().await {
        Ok(resp) => Ok(Json(json!({
            "success": true,
            "accounts": resp.accounts,
        }))),
        Err(e) => Ok(Json(json!({
            "success": false,
            "error": e.to_string(),
        }))),
    }
}

pub async fn publish_post(
    Extension(state): Extension<Arc<AppState>>,
    Json(payload): Json<PublishPayload>,
) -> Result<Json<serde_json::Value>, (StatusCode, Json<serde_json::Value>)> {
    let z = client(&state)?;

    let targets: Vec<PlatformTarget> = payload
        .platforms
        .into_iter()
        .map(|t| PlatformTarget {
            platform: t.platform,
            accountId: t.account_id,
        })
        .collect();

    match z
        .publish_to_accounts(&payload.profile_id, &payload.text, targets, payload.media_urls)
        .await
    {
        Ok(resp) => Ok(Json(json!({
            "success": true,
            "post_id": resp.post.id,
            "status": resp.post.status,
        }))),
        Err(e) => Ok(Json(json!({
            "success": false,
            "error": e.to_string(),
        }))),
    }
}
