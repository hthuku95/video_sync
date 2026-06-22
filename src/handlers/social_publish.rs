use crate::zernio_client::{PlatformTarget, ZernioClient};
use crate::AppState;
use axum::{
    extract::{Extension, Json, Path, Query},
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
        // Client-facing delivery Zernio self-service
        .route("/api/deliveries/:id/social-status", get(get_delivery_social_status))
        .route("/api/deliveries/:id/social-profile", post(create_delivery_zernio_profile))
        .route("/api/deliveries/:id/social-targets", post(set_delivery_social_targets))
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

// ── Client-facing delivery Zernio self-service types ───────────────────────

#[derive(Debug, Deserialize)]
pub struct DeliverySocialTargetsPayload {
    pub accounts: Vec<DeliveryAccountTarget>,
}

#[derive(Debug, Deserialize, Serialize, Clone)]
pub struct DeliveryAccountTarget {
    pub platform: String,
    pub account_id: String,
}

// ── Delivery social status ─────────────────────────────────────────────────

/// Get the Zernio social status for a delivery: profile + connected accounts.
pub async fn get_delivery_social_status(
    Extension(state): Extension<Arc<AppState>>,
    Path(delivery_id): Path<Uuid>,
) -> Json<serde_json::Value> {
    let row = sqlx::query(
        "SELECT zernio_profile_id, zernio_account_ids, status, title, output_r2_url
         FROM deliveries WHERE id = $1",
    )
    .bind(delivery_id)
    .fetch_optional(&state.db_pool)
    .await;

    let (profile_id, account_ids, delivery_status, title, output_url) = match row {
        Ok(Some(r)) => (
            r.get::<Option<String>, _>("zernio_profile_id"),
            r.get::<Option<serde_json::Value>, _>("zernio_account_ids"),
            r.get::<String, _>("status"),
            r.get::<String, _>("title"),
            r.get::<Option<String>, _>("output_r2_url"),
        ),
        Ok(None) => return Json(json!({"success": false, "error": "Delivery not found"})),
        Err(e) => return Json(json!({"success": false, "error": e.to_string()})),
    };

    let Some(zernio) = state.zernio_client.clone() else {
        return Json(json!({"success": false, "error": "Zernio not configured"}));
    };

    // Fetch all profiles and accounts from Zernio
    let (profiles, all_accounts) = match (
        zernio.list_profiles().await,
        zernio.list_accounts().await,
    ) {
        (Ok(p), Ok(a)) => (p.profiles, a.accounts),
        (Err(e), _) | (_, Err(e)) => {
            return Json(json!({"success": false, "error": format!("Zernio error: {e}")}));
        }
    };

    // If delivery has a profile, find it and its accounts
    let zernio_profile = profile_id
        .as_ref()
        .and_then(|pid| profiles.iter().find(|p| p.id == *pid).cloned());

    let connected_accounts: Vec<serde_json::Value> = zernio_profile
        .as_ref()
        .map(|p| {
            all_accounts
                .iter()
                .filter(|a| a.profile_id.as_deref() == Some(&p.id))
                .map(|a| {
                    json!({
                        "_id": a.id,
                        "platform": a.platform,
                        "username": a.username,
                        "connected": a.connected,
                    })
                })
                .collect()
        })
        .unwrap_or_default();

    // Parse existing targets on the delivery
    let existing_targets: Vec<DeliveryAccountTarget> = match &account_ids {
        Some(serde_json::Value::Array(arr)) => arr
            .iter()
            .filter_map(|v| {
                Some(DeliveryAccountTarget {
                    platform: v.get("platform")?.as_str()?.to_string(),
                    account_id: v.get("account_id")?.as_str()?.to_string(),
                })
            })
            .collect(),
        _ => vec![],
    };

    Json(json!({
        "success": true,
        "delivery_status": delivery_status,
        "title": title,
        "has_output": output_url.is_some(),
        "profile_id": profile_id,
        "profile": zernio_profile,
        "connected_accounts": connected_accounts,
        "existing_targets": existing_targets,
        "all_profiles": profiles,
    }))
}

// ── Create delivery Zernio profile ─────────────────────────────────────────

/// Auto-create a Zernio profile for this delivery and save the profile_id.
pub async fn create_delivery_zernio_profile(
    Extension(state): Extension<Arc<AppState>>,
    Path(delivery_id): Path<Uuid>,
) -> Json<serde_json::Value> {
    let z = match &state.zernio_client {
        Some(c) => c.clone(),
        None => return Json(json!({"success": false, "error": "Zernio not configured"})),
    };

    // Check if delivery already has a profile
    let existing = sqlx::query(
        "SELECT zernio_profile_id, title FROM deliveries WHERE id = $1",
    )
    .bind(delivery_id)
    .fetch_optional(&state.db_pool)
    .await;

    let (existing_profile_id, title) = match existing {
        Ok(Some(r)) => (
            r.get::<Option<String>, _>("zernio_profile_id"),
            r.get::<String, _>("title"),
        ),
        Ok(None) => return Json(json!({"success": false, "error": "Delivery not found"})),
        Err(e) => return Json(json!({"success": false, "error": e.to_string()})),
    };

    // If delivery already has a profile, return it
    if let Some(pid) = existing_profile_id {
        return Json(json!({"success": true, "profile_id": pid, "already_existed": true}));
    }

    // Create profile on Zernio
    let profile_name = format!("Delivery: {}", title);
    match z.create_profile(&profile_name, Some("Auto-created for delivery self-service")).await {
        Ok(resp) => {
            let profile_id = resp.profile.id.clone();
            let result = sqlx::query("UPDATE deliveries SET zernio_profile_id = $1 WHERE id = $2")
                .bind(&profile_id)
                .bind(delivery_id)
                .execute(&state.db_pool)
                .await;

            match result {
                Ok(_) => Json(json!({
                    "success": true,
                    "profile_id": profile_id,
                    "profile": resp.profile,
                })),
                Err(e) => Json(json!({"success": false, "error": format!("DB update failed: {e}")})),
            }
        }
        Err(e) => Json(json!({"success": false, "error": format!("Zernio create profile failed: {e}")})),
    }
}

// ── Set delivery social targets ────────────────────────────────────────────

/// Set which Zernio accounts to auto-publish to for this delivery.
pub async fn set_delivery_social_targets(
    Extension(state): Extension<Arc<AppState>>,
    Path(delivery_id): Path<Uuid>,
    Json(payload): Json<DeliverySocialTargetsPayload>,
) -> Json<serde_json::Value> {
    // First ensure delivery has a profile_id
    let profile_row = sqlx::query(
        "SELECT zernio_profile_id FROM deliveries WHERE id = $1",
    )
    .bind(delivery_id)
    .fetch_optional(&state.db_pool)
    .await;

    let profile_id = match profile_row {
        Ok(Some(r)) => match r.get::<Option<String>, _>("zernio_profile_id") {
            Some(pid) => pid,
            None => return Json(json!({"success": false, "error": "No Zernio profile for this delivery. Create one first."})),
        },
        Ok(None) => return Json(json!({"success": false, "error": "Delivery not found"})),
        Err(e) => return Json(json!({"success": false, "error": e.to_string()})),
    };

    let account_ids = serde_json::to_value(&payload.accounts).unwrap_or_default();
    let result = sqlx::query(
        "UPDATE deliveries SET zernio_profile_id = $1, zernio_account_ids = $2 WHERE id = $3",
    )
    .bind(&profile_id)
    .bind(&account_ids)
    .bind(delivery_id)
    .execute(&state.db_pool)
    .await;

    match result {
        Ok(r) if r.rows_affected() > 0 => Json(json!({
            "success": true,
            "profile_id": profile_id,
            "targets": payload.accounts,
        })),
        Ok(_) => Json(json!({"success": false, "error": "Delivery not found"})),
        Err(e) => Json(json!({"success": false, "error": e.to_string()})),
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
