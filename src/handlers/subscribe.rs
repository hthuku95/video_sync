//! $15/mo USDC subscription for regular users (post-trial paywall).
//!
//! Architecturally this is the **user-facing** twin of `api_access.rs`:
//! same x402 flow, same EIP-3009 wallet dance, but the buyer is an
//! authenticated user (JWT already in Claims) and we update `users.*`
//! subscription columns directly instead of a separate subscriptions
//! table — because regular-user subscription state belongs on the user row.

use crate::handlers::paypal::{self, paypal_credentials, get_paypal_env, paypal_base_url, get_paypal_access_token};
use crate::middleware::auth::auth_middleware;
use crate::models::auth::Claims;
use crate::services::monetization::CREATOR_MONTHLY_USDC_CENTS;
use crate::x402;
use crate::AppState;
use axum::{
    extract::Extension,
    http::{HeaderMap, StatusCode},
    response::{Html, Json},
    routing::{get, post},
    Router,
};
use serde::Deserialize;
use serde_json::{json, Value};
use std::sync::Arc;

/// Flat-rate USD price for the regular-user subscription. 15 USDC → 30 days.
pub fn subscribe_routes() -> Router {
    // Page is PUBLIC (buyer might not even be logged in yet — we render
    // the pricing and kick them to /login if they try to pay without auth).
    let public = Router::new().route("/subscribe", get(subscribe_page));

    // Unlock-spec + unlock both REQUIRE auth — we tie the subscription
    // to the caller's user_id, so we must know who they are.
    let protected = Router::new()
        .route("/api/subscribe/unlock-spec", get(subscribe_unlock_spec))
        .route("/api/subscribe/unlock", post(subscribe_unlock))
        .route("/api/subscribe/paypal-order", post(subscribe_paypal_order))
        .route("/api/subscribe/paypal-capture", post(subscribe_paypal_capture))
        .layer(axum::middleware::from_fn(auth_middleware));

    public.merge(protected)
}

async fn subscribe_page() -> Html<String> {
    Html(include_str!("subscribe_page.html").to_string())
}

/// GET /api/subscribe/unlock-spec — returns the x402 PaymentRequirements.
async fn subscribe_unlock_spec(
    Extension(_state): Extension<Arc<AppState>>,
    Extension(_claims): Extension<Claims>,
) -> Json<serde_json::Value> {
    let recipient = match std::env::var("X402_RECIPIENT_ADDRESS") {
        Ok(a) if !a.is_empty() => a,
        _ => return Json(json!({"error": "X402_RECIPIENT_ADDRESS not configured on server"})),
    };

    let base_url =
        std::env::var("PUBLIC_BASE_URL").unwrap_or_else(|_| "https://videosync.video".to_string());
    let resource = format!("{}/api/subscribe/unlock", base_url);
    let description = "VideoSync monthly subscription — 30 days access".to_string();

    let spec = x402::build_payment_required(
        CREATOR_MONTHLY_USDC_CENTS,
        &recipient,
        &resource,
        &description,
    );
    Json(serde_json::to_value(spec).unwrap_or(json!({"error": "spec serialise failed"})))
}

/// POST /api/subscribe/unlock — verifies X-Payment via Coinbase facilitator,
/// flips the user's subscription_status to 'active' for 30 days.
async fn subscribe_unlock(
    Extension(state): Extension<Arc<AppState>>,
    Extension(claims): Extension<Claims>,
    headers: HeaderMap,
) -> (StatusCode, Json<serde_json::Value>) {
    let user_id: i32 = match claims.sub.parse() {
        Ok(id) => id,
        Err(_) => {
            return (
                StatusCode::UNAUTHORIZED,
                Json(json!({"success": false, "error": "invalid_user_id"})),
            )
        }
    };

    let x_payment = match headers.get("X-Payment").and_then(|h| h.to_str().ok()) {
        Some(s) if !s.is_empty() => s.to_string(),
        _ => {
            return (
                StatusCode::PAYMENT_REQUIRED,
                Json(
                    json!({"success": false, "error": "Missing X-Payment header. Fetch /unlock-spec first."}),
                ),
            )
        }
    };

    // Rebuild the same PaymentRequirements we sent the wallet so the
    // facilitator can sanity-check the signature matches what we asked for.
    let recipient = match std::env::var("X402_RECIPIENT_ADDRESS") {
        Ok(a) if !a.is_empty() => a,
        _ => {
            return (
                StatusCode::INTERNAL_SERVER_ERROR,
                Json(json!({"success": false, "error": "X402_RECIPIENT_ADDRESS not configured"})),
            )
        }
    };
    let base_url =
        std::env::var("PUBLIC_BASE_URL").unwrap_or_else(|_| "https://videosync.video".to_string());
    let resource = format!("{}/api/subscribe/unlock", base_url);
    let description = "VideoSync monthly subscription — 30 days access".to_string();
    let spec = x402::build_payment_required(
        CREATOR_MONTHLY_USDC_CENTS,
        &recipient,
        &resource,
        &description,
    );
    let req = match spec.accepts.first() {
        Some(r) => r.clone(),
        None => {
            return (
                StatusCode::INTERNAL_SERVER_ERROR,
                Json(json!({"success": false, "error": "Failed to build payment requirements"})),
            )
        }
    };

    let tx_hash = match x402::settle_or_reject(&x_payment, &req).await {
        Ok(h) => h,
        Err(e) => {
            return (
                StatusCode::PAYMENT_REQUIRED,
                Json(json!({"success": false, "error": e})),
            )
        }
    };

    // Flip the user row — active for the next 30 days.
    let _ = sqlx::query(
        "UPDATE users
         SET subscription_status       = 'active',
             subscription_tier         = 'monthly_15',
             subscription_active_until = NOW() + INTERVAL '30 days',
             last_payment_receipt_id   = $1,
             last_payment_at           = NOW(),
             updated_at                = NOW()
         WHERE id = $2",
    )
    .bind(&tx_hash)
    .bind(user_id)
    .execute(&state.db_pool)
    .await;

    // Append to audit log.
    let _ = sqlx::query(
        "INSERT INTO user_payment_events (user_id, event_type, amount_usdc, tx_hash)
         VALUES ($1, 'paid', $2, $3)",
    )
    .bind(user_id)
    .bind(sqlx::types::Decimal::new(
        CREATOR_MONTHLY_USDC_CENTS as i64,
        2,
    ))
    .bind(&tx_hash)
    .execute(&state.db_pool)
    .await;

    // Ping admin on Telegram — fire-and-forget so payment response
    // isn't held up by Telegram latency.
    let tx_preview = tx_hash.chars().take(10).collect::<String>();
    let notify_text = format!(
        "💰 *New $15 USDC subscription*\n\
         User id: {}\n\
         Tx: `{}...`\n\
         Active until: 30 days from now",
        user_id, tx_preview
    );
    tokio::spawn(async move {
        crate::telegram_bot::notify_admin(&notify_text).await;
    });

    (
        StatusCode::OK,
        Json(json!({
            "success":         true,
            "tx_hash":         tx_hash,
            "active_for_days": 30,
            "message":         "Welcome aboard — your access is unlocked for 30 days. Reload the app to clear the paywall.",
        })),
    )
}

/// POST /api/subscribe/paypal-order — creates a $15 PayPal order for subscription.
async fn subscribe_paypal_order(
    Extension(state): Extension<Arc<AppState>>,
    Extension(claims): Extension<Claims>,
) -> (StatusCode, Json<Value>) {
    let user_id: i32 = match claims.sub.parse() {
        Ok(id) => id,
        Err(_) => return (StatusCode::UNAUTHORIZED, Json(json!({"success": false, "error": "invalid_user_id"}))),
    };

    let env = get_paypal_env(&state).await;
    let base_url = paypal_base_url(&env);
    let (client_id, client_secret) = paypal_credentials(&env);
    if client_id.is_empty() || client_secret.is_empty() {
        return (StatusCode::INTERNAL_SERVER_ERROR, Json(json!({"success": false, "message": "PayPal not configured"})));
    }

    let token = match get_paypal_access_token(base_url, &client_id, &client_secret).await {
        Ok(t) => t,
        Err(e) => return (StatusCode::INTERNAL_SERVER_ERROR, Json(json!({"success": false, "message": e}))),
    };

    let studio_url = std::env::var("PUBLIC_BASE_URL").unwrap_or_else(|_| "https://videosync.video".to_string());
    let dollars = CREATOR_MONTHLY_USDC_CENTS as f64 / 100.0;

    let resp = match reqwest::Client::new()
        .post(format!("{}/v2/checkout/orders", base_url))
        .header("Content-Type", "application/json")
        .bearer_auth(&token)
        .json(&json!({
            "intent": "CAPTURE",
            "purchase_units": [{
                "reference_id": "subscription_monthly",
                "description": "VideoSync monthly subscription",
                "amount": { "currency_code": "USD", "value": format!("{:.2}", dollars) }
            }],
            "payment_source": {
                "paypal": {
                    "experience_context": {
                        "payment_method_preference": "IMMEDIATE_PAYMENT_REQUIRED",
                        "landing_page": "LOGIN",
                        "user_action": "PAY_NOW",
                        "return_url": format!("{}/subscribe?paypal_return=1", studio_url),
                        "cancel_url": format!("{}/subscribe?paypal_cancel=1", studio_url)
                    }
                }
            }
        }))
        .send()
        .await
    {
        Ok(r) => r,
        Err(e) => return (StatusCode::INTERNAL_SERVER_ERROR, Json(json!({"success": false, "message": format!("PayPal order failed: {}", e)}))),
    };

    let body: Value = match resp.json().await {
        Ok(b) => b,
        Err(e) => return (StatusCode::INTERNAL_SERVER_ERROR, Json(json!({"success": false, "message": format!("PayPal parse failed: {}", e)}))),
    };

    if !body.get("id").and_then(|v| v.as_str()).is_some() {
        return (StatusCode::BAD_REQUEST, Json(json!({"success": false, "error": body})));
    }

    (StatusCode::OK, Json(body))
}

/// POST /api/subscribe/paypal-capture — captures the PayPal order and activates subscription.
async fn subscribe_paypal_capture(
    Extension(state): Extension<Arc<AppState>>,
    Extension(claims): Extension<Claims>,
    axum::Json(payload): axum::Json<CaptureRequest>,
) -> (StatusCode, Json<Value>) {
    let user_id: i32 = match claims.sub.parse() {
        Ok(id) => id,
        Err(_) => return (StatusCode::UNAUTHORIZED, Json(json!({"success": false, "error": "invalid_user_id"}))),
    };

    let env = get_paypal_env(&state).await;
    let base_url = paypal_base_url(&env);
    let (client_id, client_secret) = paypal_credentials(&env);
    if client_id.is_empty() || client_secret.is_empty() {
        return (StatusCode::INTERNAL_SERVER_ERROR, Json(json!({"success": false, "message": "PayPal not configured"})));
    }

    let token = match get_paypal_access_token(base_url, &client_id, &client_secret).await {
        Ok(t) => t,
        Err(e) => return (StatusCode::INTERNAL_SERVER_ERROR, Json(json!({"success": false, "message": e}))),
    };

    let resp = match reqwest::Client::new()
        .post(format!("{}/v2/checkout/orders/{}/capture", base_url, payload.order_id))
        .header("Content-Type", "application/json")
        .bearer_auth(&token)
        .send()
        .await
    {
        Ok(r) => r,
        Err(e) => return (StatusCode::INTERNAL_SERVER_ERROR, Json(json!({"success": false, "message": format!("PayPal capture failed: {}", e)}))),
    };

    let body: Value = match resp.json().await {
        Ok(b) => b,
        Err(e) => return (StatusCode::INTERNAL_SERVER_ERROR, Json(json!({"success": false, "message": format!("Capture parse failed: {}", e)}))),
    };

    let capture_status = body.get("status").and_then(|s| s.as_str()).unwrap_or("UNKNOWN");
    let is_completed = capture_status == "COMPLETED";

    if is_completed {
        let payer_email = body.pointer("/payer/email_address").and_then(|v| v.as_str());
        let capture_id = body.pointer("/purchase_units/0/payments/captures/0/id").and_then(|v| v.as_str());

        // Flip the user to active
        let _ = sqlx::query(
            "UPDATE users
             SET subscription_status       = 'active',
                 subscription_tier         = 'monthly_15',
                 subscription_active_until = NOW() + INTERVAL '30 days',
                 last_payment_receipt_id   = $1,
                 last_payment_at           = NOW(),
                 updated_at                = NOW()
             WHERE id = $2",
        )
        .bind(capture_id.unwrap_or("paypal"))
        .bind(user_id)
        .execute(&state.db_pool)
        .await;

        let _ = sqlx::query(
            "INSERT INTO user_payment_events (user_id, event_type, amount_usdc, tx_hash)
             VALUES ($1, 'paid_paypal', 1500, $2)",
        )
        .bind(user_id)
        .bind(payload.order_id.clone())
        .execute(&state.db_pool)
        .await;

        let notify_text = format!(
            "💰 *New $15 PayPal subscription*\nUser id: {}\nPayer: {}\nOrder: {}\nActive until: 30 days from now",
            user_id, payer_email.unwrap_or("unknown"), payload.order_id
        );
        tokio::spawn(async move {
            crate::telegram_bot::notify_admin(&notify_text).await;
        });
    }

    (StatusCode::OK, Json(json!({
        "success": is_completed,
        "status": capture_status,
    })))
}

#[derive(Debug, Deserialize)]
struct CaptureRequest {
    order_id: String,
}
