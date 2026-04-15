//! Public API/platform license sales — agencies pay USDC on Base, get an
//! API key in seconds. Reuses crate::x402 (same module that powers the
//! /delivery/:id paywall) so there's only one payment rail to maintain.
//!
//! Flow:
//!   1. Buyer visits /api-access (public SSR page) — sees the pricing table.
//!   2. Buyer clicks "Buy Starter / Pro" → POST /api/agency/subscribe creates
//!      a `pending` row in api_subscriptions, returns the subscription_id.
//!   3. Page calls GET /api/agency/subscribe/:id/unlock-spec → returns the
//!      x402 PaymentRequiredResponse with the tier price.
//!   4. Wallet (Phantom EVM, MetaMask, Coinbase Wallet) signs EIP-3009
//!      transferWithAuthorization. Page POSTs back with X-Payment header.
//!   5. POST /api/agency/subscribe/:id/unlock verifies via Coinbase facilitator,
//!      flips status='active', generates the API key, returns it to the page.
//!
//! NOTE: the API key isn't yet wired into the clipping/thumbnail handlers as
//! an alternative auth path — that wiring is the next-session task. Until
//! then, the key is a "founder pass" — early agencies pay to lock in the
//! pricing and get notified when the API endpoints land.

use crate::x402;
use crate::AppState;
use axum::{
    extract::{Extension, Path},
    http::HeaderMap,
    http::StatusCode,
    response::{Html, Json},
    routing::{get, post},
    Router,
};
use serde::Deserialize;
use serde_json::json;
use sqlx::Row;
use std::sync::Arc;

pub fn api_access_routes() -> Router {
    Router::new()
        .route("/api-access", get(api_access_page))
        .route("/api/agency/subscribe", post(create_subscription))
        .route("/api/agency/subscribe/:id/unlock-spec", get(subscription_unlock_spec))
        .route("/api/agency/subscribe/:id/unlock", post(subscription_unlock))
}

// ────────────────────────────────────────────────────────────────────────────
// SSR page — pricing + buy buttons + reused wallet JS
// ────────────────────────────────────────────────────────────────────────────

async fn api_access_page() -> Html<String> {
    let html = include_str!("api_access_page.html");
    Html(html.to_string())
}

// ────────────────────────────────────────────────────────────────────────────
// Tier definitions — single source of truth so price + quotas stay in sync
// across the SSR page, the create-subscription endpoint, and the unlock-spec.
// ────────────────────────────────────────────────────────────────────────────

#[derive(Debug, Clone, Copy)]
struct TierSpec {
    name:         &'static str,
    price_cents:  u64,
    clip_quota:   i32,
    thumb_quota:  i32,
    anim_quota:   i32,
}

const STARTER: TierSpec = TierSpec {
    name:         "starter",
    price_cents:  9900, // $99
    clip_quota:   1000,
    thumb_quota:  500,
    anim_quota:   50,
};

const PRO: TierSpec = TierSpec {
    name:         "pro",
    price_cents:  19900, // $199
    clip_quota:   5000,
    thumb_quota:  2500,
    anim_quota:   200,
};

fn tier_for(name: &str) -> Option<TierSpec> {
    match name {
        "starter" => Some(STARTER),
        "pro"     => Some(PRO),
        _ => None,
    }
}

// ────────────────────────────────────────────────────────────────────────────
// Create-subscription — public, no auth. Inserts a pending row and returns
// the subscription_id the page uses for the unlock dance.
// ────────────────────────────────────────────────────────────────────────────

#[derive(Debug, Deserialize)]
struct CreateSubReq {
    tier:           String,
    email:          Option<String>,
    contact_handle: Option<String>,
}

async fn create_subscription(
    Extension(state): Extension<Arc<AppState>>,
    Json(req):        Json<CreateSubReq>,
) -> Json<serde_json::Value> {
    let Some(tier) = tier_for(&req.tier) else {
        return Json(json!({"success": false, "error": "Invalid tier — must be 'starter' or 'pro'"}));
    };

    // Pre-generate the API key now even though we won't expose it until the
    // payment lands. Using a simple `vk_` prefix so callers can grep their
    // env files easily. 32 hex chars = 128 bits of entropy, plenty.
    let api_key = format!(
        "vk_{}",
        hex::encode(rand::random::<[u8; 16]>())
    );

    let id = match sqlx::query_scalar::<_, uuid::Uuid>(
        "INSERT INTO api_subscriptions
           (email, contact_handle, api_key, tier, monthly_clip_quota,
            monthly_thumbnail_quota, monthly_animation_quota,
            payment_amount_usdc, status)
         VALUES ($1, $2, $3, $4, $5, $6, $7, $8, 'pending')
         RETURNING id"
    )
    .bind(req.email.as_deref())
    .bind(req.contact_handle.as_deref())
    .bind(&api_key)
    .bind(tier.name)
    .bind(tier.clip_quota)
    .bind(tier.thumb_quota)
    .bind(tier.anim_quota)
    .bind(sqlx::types::Decimal::new(tier.price_cents as i64, 2))
    .fetch_one(&state.db_pool)
    .await {
        Ok(id) => id,
        Err(e) => return Json(json!({"success": false, "error": format!("DB insert failed: {}", e)})),
    };

    Json(json!({
        "success":         true,
        "subscription_id": id.to_string(),
        "tier":            tier.name,
        "price_usd":       tier.price_cents as f64 / 100.0,
        "next_step":       format!("/api/agency/subscribe/{}/unlock-spec", id),
    }))
}

// ────────────────────────────────────────────────────────────────────────────
// Unlock spec — returns the x402 PaymentRequirements for the buyer's wallet
// to sign. Mirrors /delivery/:id/unlock-spec exactly except the resource and
// description.
// ────────────────────────────────────────────────────────────────────────────

async fn subscription_unlock_spec(
    Path(id):         Path<String>,
    Extension(state): Extension<Arc<AppState>>,
) -> Json<serde_json::Value> {
    let uuid = match uuid::Uuid::parse_str(&id) {
        Ok(u) => u,
        Err(_) => return Json(json!({"error": "Invalid subscription id"})),
    };

    let row = match sqlx::query(
        "SELECT tier, payment_amount_usdc, status FROM api_subscriptions WHERE id = $1"
    )
    .bind(uuid)
    .fetch_optional(&state.db_pool)
    .await {
        Ok(Some(r)) => r,
        Ok(None)    => return Json(json!({"error": "Subscription not found"})),
        Err(e)      => return Json(json!({"error": format!("DB error: {}", e)})),
    };

    let status: String = row.get("status");
    if status == "active" {
        return Json(json!({"error": "Subscription is already active"}));
    }
    let tier_name: String = row.get("tier");
    let price_dec: Option<sqlx::types::Decimal> = row.try_get("payment_amount_usdc").ok().flatten();
    let price_cents: u64 = price_dec
        .and_then(|d| {
            use sqlx::types::Decimal;
            (d * Decimal::new(100, 0)).trunc().to_string().parse::<u64>().ok()
        })
        .unwrap_or(STARTER.price_cents);

    let recipient = match std::env::var("X402_RECIPIENT_ADDRESS") {
        Ok(a) if !a.is_empty() => a,
        _ => return Json(json!({"error": "X402_RECIPIENT_ADDRESS not configured on server"})),
    };

    let base_url = std::env::var("PUBLIC_BASE_URL")
        .unwrap_or_else(|_| "https://videosync.video".to_string());
    let resource = format!("{}/api/agency/subscribe/{}/unlock", base_url, uuid);
    let description = format!("VideoSync API access — {} tier (30-day pass)", tier_name);

    let spec = x402::build_payment_required(price_cents, &recipient, &resource, &description);
    Json(serde_json::to_value(spec).unwrap_or(json!({"error": "spec serialise failed"})))
}

// ────────────────────────────────────────────────────────────────────────────
// Unlock — verifies X-Payment via Coinbase facilitator, settles on-chain,
// activates the subscription, returns the API key.
// ────────────────────────────────────────────────────────────────────────────

async fn subscription_unlock(
    Path(id):         Path<String>,
    headers:          HeaderMap,
    Extension(state): Extension<Arc<AppState>>,
) -> (StatusCode, Json<serde_json::Value>) {
    let uuid = match uuid::Uuid::parse_str(&id) {
        Ok(u) => u,
        Err(_) => return (StatusCode::BAD_REQUEST, Json(json!({"success": false, "error": "Invalid subscription id"}))),
    };

    let x_payment = match headers.get("X-Payment").and_then(|h| h.to_str().ok()) {
        Some(s) if !s.is_empty() => s.to_string(),
        _ => return (StatusCode::PAYMENT_REQUIRED,
                     Json(json!({"success": false, "error": "Missing X-Payment header. Fetch /unlock-spec first."}))),
    };

    let row = match sqlx::query(
        "SELECT tier, payment_amount_usdc, api_key, status FROM api_subscriptions WHERE id = $1"
    )
    .bind(uuid)
    .fetch_optional(&state.db_pool)
    .await {
        Ok(Some(r)) => r,
        Ok(None)    => return (StatusCode::NOT_FOUND, Json(json!({"success": false, "error": "Subscription not found"}))),
        Err(e)      => return (StatusCode::INTERNAL_SERVER_ERROR, Json(json!({"success": false, "error": format!("DB error: {}", e)}))),
    };

    let status: String = row.get("status");
    if status == "active" {
        let key: String = row.get("api_key");
        return (StatusCode::OK, Json(json!({"success": true, "api_key": key, "message": "Subscription is already active"})));
    }

    let tier_name: String = row.get("tier");
    let price_dec: Option<sqlx::types::Decimal> = row.try_get("payment_amount_usdc").ok().flatten();
    let price_cents: u64 = price_dec
        .and_then(|d| {
            use sqlx::types::Decimal;
            (d * Decimal::new(100, 0)).trunc().to_string().parse::<u64>().ok()
        })
        .unwrap_or(STARTER.price_cents);

    let recipient = match std::env::var("X402_RECIPIENT_ADDRESS") {
        Ok(a) if !a.is_empty() => a,
        _ => return (StatusCode::INTERNAL_SERVER_ERROR,
                     Json(json!({"success": false, "error": "X402_RECIPIENT_ADDRESS not configured"}))),
    };

    let base_url = std::env::var("PUBLIC_BASE_URL")
        .unwrap_or_else(|_| "https://videosync.video".to_string());
    let resource = format!("{}/api/agency/subscribe/{}/unlock", base_url, uuid);
    let description = format!("VideoSync API access — {} tier (30-day pass)", tier_name);

    let spec = x402::build_payment_required(price_cents, &recipient, &resource, &description);
    let req  = match spec.accepts.first() {
        Some(r) => r.clone(),
        None    => return (StatusCode::INTERNAL_SERVER_ERROR,
                           Json(json!({"success": false, "error": "Failed to build payment requirements"}))),
    };

    let tx_hash = match x402::settle_or_reject(&x_payment, &req).await {
        Ok(h)  => h,
        Err(e) => return (StatusCode::PAYMENT_REQUIRED,
                          Json(json!({"success": false, "error": e}))),
    };

    // Activate — 30-day pass starting now.
    let api_key: String = row.get("api_key");
    let _ = sqlx::query(
        "UPDATE api_subscriptions
         SET status = 'active', active_until = NOW() + INTERVAL '30 days',
             payment_receipt_id = $1, updated_at = NOW()
         WHERE id = $2"
    )
    .bind(&tx_hash)
    .bind(uuid)
    .execute(&state.db_pool)
    .await;

    (StatusCode::OK, Json(json!({
        "success":           true,
        "api_key":           api_key,
        "tier":              tier_name,
        "tx_hash":           tx_hash,
        "active_for_days":   30,
        "next_step":         "Save your API key. It's the same shape as a Bearer token: `Authorization: Bearer <key>`. API endpoints land in the next deploy — you'll get an email when they do.",
    })))
}
