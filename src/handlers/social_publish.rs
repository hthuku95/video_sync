use crate::models::auth::Claims;
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
use sqlx::Row;
use std::sync::Arc;
use tracing::{info, warn};
use uuid::Uuid;
use chrono::{DateTime, Utc};

pub fn social_routes() -> Router {
    // Public admin-facing endpoints
    let public = Router::new()
        .route("/api/social/create-profile", post(create_profile))
        .route("/api/social/profiles", get(list_profiles))
        .route("/api/social/connect-url", get(get_connect_url))
        .route("/api/social/accounts", get(list_accounts))
        .route("/api/social/publish", post(publish_post))
        // Client-facing delivery Zernio self-service
        .route("/api/deliveries/:id/social-status", get(get_delivery_social_status))
        .route("/api/deliveries/:id/social-profile", post(create_delivery_zernio_profile))
        .route("/api/deliveries/:id/social-targets", post(set_delivery_social_targets))
        // Manual retry for failed/partial Zernio posts
        .route("/api/social/posts/:id/retry", post(retry_post))
        // Zernio webhook for push-based status updates (replaces polling)
        .route("/api/social/webhook", post(zernio_webhook));
    // User-level self-service (requires auth)
    let user_routes = Router::new()
        .route("/api/social/my-profile", post(get_or_create_my_zernio_profile))
        .route("/api/social/my-accounts", get(get_my_social_accounts))
        .route("/api/social/sync-accounts", post(sync_my_social_accounts))
        .layer(axum::middleware::from_fn(crate::middleware::auth::auth_middleware));
    public.merge(user_routes)
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
    pub redirect_url: Option<String>,
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
    match z.get_connect_url(&query.platform, &query.profile_id, query.redirect_url.as_deref()).await {
        Ok(resp) => Ok(Json(json!({
            "success": true,
            "authUrl": resp.auth_url,
            "state": resp.state,
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
    match z.list_accounts(None).await {
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
        zernio.list_accounts(None).await,
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
                .filter(|a| a.profile_id.as_ref().map(|pid| pid.id()) == Some(p.id.as_str()))
                .map(|a| {
                    json!({
                        "_id": a.id,
                        "platform": a.platform,
                        "username": a.username,
                        "connected": a.is_active,
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

// ── User-level self-service handlers ────────────────────────────────────────

/// Get or create the authenticated user's Zernio profile.
/// Stores the mapping in `user_zernio_profiles` table so it persists across sessions.
async fn ensure_user_zernio_profile(
    state: &Arc<AppState>,
    user_id: i32,
) -> Result<String, (StatusCode, Json<serde_json::Value>)> {
    // Check existing mapping
    let existing = sqlx::query(
        "SELECT zernio_profile_id FROM user_zernio_profiles WHERE user_id = $1",
    )
    .bind(user_id)
    .fetch_optional(&state.db_pool)
    .await
    .map_err(|e| {
        (
            StatusCode::INTERNAL_SERVER_ERROR,
            Json(json!({"success": false, "error": format!("DB error: {e}")})),
        )
    })?;

    if let Some(row) = existing {
        return Ok(row.get::<String, _>("zernio_profile_id"));
    }

    let z = state.zernio_client.clone().ok_or_else(|| {
        (
            StatusCode::SERVICE_UNAVAILABLE,
            Json(json!({"success": false, "error": "Zernio not configured"})),
        )
    })?;

    // Search for an existing profile on Zernio before creating a new one
    match z.list_profiles().await {
        Ok(profiles_resp) => {
            for profile in &profiles_resp.profiles {
                // Check if the profile name matches our naming convention:
                // "{username} — VideoSync"
                if profile.name.contains("— VideoSync") {
                    // Found a matching profile — adopt it
                    sqlx::query(
                        "INSERT INTO user_zernio_profiles (user_id, zernio_profile_id) VALUES ($1, $2) \
                         ON CONFLICT DO NOTHING",
                    )
                    .bind(user_id)
                    .bind(&profile.id)
                    .execute(&state.db_pool)
                    .await
                    .ok();
                    info!(
                        "Adopted existing Zernio profile '{}' ({}) for user {}",
                        profile.name, profile.id, user_id
                    );
                    return Ok(profile.id.clone());
                }
            }
        }
        Err(e) => warn!("Failed to list Zernio profiles (will create new): {e}"),
    }

    // Auto-create a Zernio profile
    let z = state.zernio_client.clone().ok_or_else(|| {
        (
            StatusCode::SERVICE_UNAVAILABLE,
            Json(json!({"success": false, "error": "Zernio not configured"})),
        )
    })?;

    // Get user info for profile name
    let user_row = sqlx::query("SELECT username, email FROM users WHERE id = $1")
        .bind(user_id)
        .fetch_optional(&state.db_pool)
        .await
        .map_err(|e| {
            (
                StatusCode::INTERNAL_SERVER_ERROR,
                Json(json!({"success": false, "error": format!("DB error: {e}")})),
            )
        })?;

    let profile_name = match user_row {
        Some(r) => {
            let username: Option<String> = r.get("username");
            let email: String = r.get("email");
            username.unwrap_or_else(|| email.split('@').next().unwrap_or("User").to_string())
        }
        None => format!("User {}", user_id),
    };

    match z
        .create_profile(
            &format!("{} — VideoSync", profile_name),
            Some("Auto-created for social publishing"),
        )
        .await
    {
        Ok(resp) => {
            let profile_id = resp.profile.id.clone();
            sqlx::query(
                "INSERT INTO user_zernio_profiles (user_id, zernio_profile_id) VALUES ($1, $2)",
            )
            .bind(user_id)
            .bind(&profile_id)
            .execute(&state.db_pool)
            .await
            .map_err(|e| {
                (
                    StatusCode::INTERNAL_SERVER_ERROR,
                    Json(json!({"success": false, "error": format!("DB insert error: {e}")})),
                )
            })?;
            info!(
                "Created Zernio profile '{}' ({}) for user {}",
                profile_name, profile_id, user_id
            );
            Ok(profile_id)
        }
        Err(e) => Err((
            StatusCode::BAD_GATEWAY,
            Json(json!({"success": false, "error": format!("Zernio create profile failed: {e}")})),
        )),
    }
}

pub async fn get_or_create_my_zernio_profile(
    Extension(state): Extension<Arc<AppState>>,
    Extension(claims): Extension<Claims>,
) -> Json<serde_json::Value> {
    let user_id: i32 = match claims.sub.parse() {
        Ok(id) => id,
        Err(_) => return Json(json!({"success": false, "error": "Invalid user ID in token"})),
    };
    match ensure_user_zernio_profile(&state, user_id).await {
        Ok(profile_id) => Json(json!({"success": true, "profile": {"id": profile_id}})),
        Err((_code, err)) => err,
    }
}

pub async fn get_my_social_accounts(
    Extension(state): Extension<Arc<AppState>>,
    Extension(claims): Extension<Claims>,
) -> Json<serde_json::Value> {
    let user_id: i32 = match claims.sub.parse() {
        Ok(id) => id,
        Err(_) => return Json(json!({"success": false, "error": "Invalid user ID in token"})),
    };
    let rows = sqlx::query_as::<_, (String, String, Option<String>, Option<String>, Option<String>, bool, Option<chrono::DateTime<chrono::Utc>>)>(
        "SELECT zernio_account_id, platform, username, display_name, profile_picture, is_active, synced_at \
         FROM user_zernio_accounts WHERE user_id = $1 ORDER BY platform",
    )
    .bind(user_id)
    .fetch_all(&state.db_pool)
    .await;
    match rows {
        Ok(accounts) => {
            let accounts_json: Vec<serde_json::Value> = accounts
                .into_iter()
                .map(|(id, platform, username, display_name, profile_picture, is_active, synced_at)| {
                    json!({
                        "id": id,
                        "platform": platform,
                        "account_name": display_name.or(username).unwrap_or_else(|| platform.clone()),
                        "avatar_url": profile_picture,
                        "status": if is_active { "active" } else { "inactive" },
                        "created_at": synced_at.map(|t| t.to_rfc3339()),
                    })
                })
                .collect();
            Json(json!({"success": true, "accounts": accounts_json}))
        }
        Err(e) => Json(json!({"success": false, "error": format!("DB error: {e}")})),
    }
}

pub async fn sync_my_social_accounts(
    Extension(state): Extension<Arc<AppState>>,
    Extension(claims): Extension<Claims>,
) -> Json<serde_json::Value> {
    let user_id: i32 = match claims.sub.parse() {
        Ok(id) => id,
        Err(_) => return Json(json!({"success": false, "error": "Invalid user ID in token"})),
    };
    let Some(zernio) = state.zernio_client.clone() else {
        return Json(json!({"success": false, "error": "Zernio not configured"}));
    };

    // Get the user's Zernio profile_id for scoped sync
    let profile_id = match ensure_user_zernio_profile(&state, user_id).await {
        Ok(pid) => pid,
        Err((_code, err)) => return err,
    };

    let accounts = match zernio.list_accounts(Some(&profile_id)).await {
        Ok(r) => r.accounts,
        Err(e) => return Json(json!({"success": false, "error": format!("Zernio error: {e}" )})),
    };
    // Clear and re-insert for this user
    let _ = sqlx::query("DELETE FROM user_zernio_accounts WHERE user_id = $1")
        .bind(user_id)
        .execute(&state.db_pool)
        .await;
    for acc in &accounts {
        let _ = sqlx::query(
            "INSERT INTO user_zernio_accounts \
             (user_id, zernio_account_id, platform, username, display_name, profile_picture, is_active) \
             VALUES ($1, $2, $3, $4, $5, $6, $7) \
             ON CONFLICT (user_id, zernio_account_id) DO UPDATE SET \
             username = EXCLUDED.username, display_name = EXCLUDED.display_name, is_active = EXCLUDED.is_active, synced_at = NOW()",
        )
        .bind(user_id)
        .bind(&acc.id)
        .bind(&acc.platform)
        .bind(&acc.username)
        .bind(&acc.display_name)
        .bind(&acc.profile_picture)
        .bind(acc.is_active)
        .execute(&state.db_pool)
        .await;
    }
    info!("Synced {} Zernio accounts for user {}", accounts.len(), user_id);
    Json(json!({"success": true, "synced_count": accounts.len(), "accounts": accounts}))
}

/// POST /api/social/posts/:id/retry — Manually retry a failed or partially-published Zernio post.
async fn retry_post(
    Extension(state): Extension<Arc<AppState>>,
    Path(post_id): Path<String>,
) -> Result<Json<serde_json::Value>, (StatusCode, Json<serde_json::Value>)> {
    let z = client(&state)?;
    match z.retry_post(&post_id).await {
        Ok(resp) => Ok(Json(json!({
            "success": true,
            "post": resp.post,
        }))),
        Err(e) => Ok(Json(json!({
            "success": false,
            "error": e.to_string(),
        }))),
    }
}

/// POST /api/social/webhook — Receives push-based status updates from Zernio.
///
/// Zernio calls this endpoint on post lifecycle transitions. Events per docs.zernio.com:
///   post.scheduled, post.published, post.partial, post.failed, post.cancelled,
///   post.recycled, post.platform.published, post.platform.failed,
///   post.tiktok.url_resolved, post.platform.deleted
///
/// This replaces (or supplements) the polling-based `check_zernio_post_status()` in campaign_engine.
#[derive(Deserialize)]
struct ZernioWebhookPayload {
    #[serde(default)]
    event: String,                          // e.g. "post.published", "post.platform.failed"
    post_id: Option<String>,                // Zernio's post ID
    status: Option<String>,                 // "published" | "failed" | "partial" (legacy, some events use this)
    #[serde(default)]
    platforms: Vec<serde_json::Value>,      // Per-platform status array
    #[serde(default)]
    error: Option<String>,                  // Top-level error message
    /// Per-platform event details (populated for post.platform.* events)
    platform: Option<String>,
    account_id: Option<String>,
}

async fn zernio_webhook(
    Extension(state): Extension<Arc<AppState>>,
    Json(payload): Json<ZernioWebhookPayload>,
) -> Json<serde_json::Value> {
    // Acknowledge receipt immediately (Zernio expects 2xx fast)
    let Some(post_id) = payload.post_id else {
        return Json(json!({"success": true, "message": "ignored — no post_id"}));
    };

    let event_lower = payload.event.to_lowercase();
    info!(post_id = %post_id, event = %event_lower, "Zernio webhook received");

    // Map Zernio event to internal campaign_posts status
    let mapped = match event_lower.as_str() {
        "post.published" => "published",
        "post.partial" => "published",
        "post.failed" => "failed",
        "post.cancelled" => "failed",
        "post.scheduled" | "post.recycled" => {
            // Scheduled or recycled — keep current status, just log
            info!(post_id = %post_id, event = %event_lower, "Zernio webhook: non-terminal event, no status change");
            return Json(json!({"success": true, "message": "event acknowledged"}));
        }
        // Per-platform events are logged but don't change post-level status
        ev if ev.starts_with("post.platform.") || ev == "post.tiktok.url_resolved" || ev == "post.platform.deleted" => {
            info!(post_id = %post_id, event = %event_lower, platform = ?payload.platform, "Zernio per-platform event");
            // Still update extra_args with platform data but don't change post status
            update_platform_results(&state.db_pool, &post_id, &payload.platforms, &payload.platform).await;
            return Json(json!({"success": true, "message": "per-platform event acknowledged"}));
        }
        _ => {
            warn!(post_id = %post_id, event = %event_lower, "Zernio webhook: unhandled event");
            return Json(json!({"success": true, "message": "unhandled event — ignored"}));
        }
    };

    // Update campaign_posts status
    let now = chrono::Utc::now();
    let updated = sqlx::query(
        "UPDATE campaign_posts SET status = $1, published_at = COALESCE(published_at, $2), updated_at = $2 WHERE zernio_post_id = $3",
    )
    .bind(mapped)
    .bind(now)
    .bind(&post_id)
    .execute(&state.db_pool)
    .await;

    match updated {
        Ok(r) => {
            if r.rows_affected() > 0 {
                info!(post_id = %post_id, status = %mapped, rows = %r.rows_affected(), "Updated campaign_post via webhook");
            } else {
                // Not a campaign_post — may be a delivery auto-publish
                info!(post_id = %post_id, "No campaign_post matched; checking deliveries");
            }
        }
        Err(e) => {
            warn!(post_id = %post_id, error = %e, "Failed to update campaign_post via webhook");
        }
    }

    // Update platform results in extra_args
    update_platform_results(&state.db_pool, &post_id, &payload.platforms, &payload.platform).await;

    Json(json!({"success": true, "message": "processed"}))
}

/// Helper: write per-platform results into deliveries.extra_args.zernio_platform_results
async fn update_platform_results(
    db_pool: &sqlx::PgPool,
    post_id: &str,
    platforms: &[serde_json::Value],
    single_platform: &Option<String>,
) {
    if platforms.is_empty() && single_platform.is_none() {
        return;
    }

    let now = chrono::Utc::now();

    if !platforms.is_empty() {
        let _ = sqlx::query(
            "UPDATE deliveries SET extra_args = jsonb_set(COALESCE(extra_args, '{}'::jsonb), '{zernio_platform_results}', $1::jsonb), updated_at = $2 WHERE id IN (SELECT delivery_id FROM campaign_posts WHERE zernio_post_id = $3)",
        )
        .bind(serde_json::json!(platforms))
        .bind(now)
        .bind(post_id)
        .execute(db_pool)
        .await;
    }
}
