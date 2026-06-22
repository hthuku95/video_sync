use crate::zernio_client::{PlatformTarget, ZernioClient};
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
use tracing::warn;
use uuid::Uuid;

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

/// Best-effort auto-publish of a completed delivery to Zernio-linked social accounts.
/// Reads the delivery's `zernio_profile_id` and `zernio_account_ids`, and if both are set,
/// publishes the output video via Zernio. Never fails the delivery — only logs on error.
pub async fn try_publish_delivery_to_zernio(delivery_id: Uuid, state: &Arc<AppState>) {
    let Some(zernio) = state.zernio_client.clone() else {
        return;
    };

    let row = sqlx::query_as::<_, (Option<String>, Option<serde_json::Value>, Option<String>)>(
        "SELECT zernio_profile_id, zernio_account_ids, output_r2_url FROM deliveries WHERE id = $1",
    )
    .bind(delivery_id)
    .fetch_optional(&state.db_pool)
    .await;

    let (profile_id, account_ids, output_url) = match row {
        Ok(Some(r)) => r,
        _ => return,
    };

    let Some(profile_id) = profile_id else { return };
    let Some(account_ids) = account_ids else { return };
    let Some(output_url) = output_url else { return };

    let accounts: Vec<PlatformTarget> = match account_ids {
        serde_json::Value::Array(arr) => arr
            .into_iter()
            .filter_map(|v| {
                let obj = v.as_object()?;
                Some(PlatformTarget {
                    platform: obj.get("platform")?.as_str()?.to_string(),
                    accountId: obj.get("account_id")?.as_str()?.to_string(),
                })
            })
            .collect(),
        _ => return,
    };

    if accounts.is_empty() {
        return;
    }

    let text = format!(
        "New video render completed! 🎬\n\nAutomated delivery via VideoSync.\n\n{}",
        output_url
    );

    match zernio
        .publish_to_accounts(&profile_id, &text, accounts, vec![output_url])
        .await
    {
        Ok(resp) => {
            tracing::info!(
                "📱 Delivery {} auto-published to Zernio (post_id={})",
                delivery_id,
                resp.post.id
            );
        }
        Err(e) => {
            warn!("📱 Zernio auto-publish failed for delivery {}: {}", delivery_id, e);
        }
    }
}
