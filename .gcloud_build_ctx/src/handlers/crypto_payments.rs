use crate::handlers::paypal;
use crate::x402;
use crate::AppState;
use axum::{
    extract::Extension,
    http::{HeaderMap, StatusCode},
    response::Json,
    routing::post,
    Router,
};
use serde::Deserialize;
use serde_json::json;
use std::sync::Arc;
use uuid::Uuid;

pub fn crypto_routes() -> Router {
    Router::new()
        .route("/api/crypto/unlock-spec", post(crypto_unlock_spec))
        .route("/api/crypto/unlock", post(crypto_unlock))
}

#[derive(Debug, Deserialize)]
struct UnlockSpecRequest {
    offer_id: String,
}

#[derive(Debug, Deserialize)]
struct UnlockRequest {
    offer_id: String,
}

/// POST /api/crypto/unlock-spec — returns x402 PaymentRequirements for an offer.
#[allow(unused_variables)]
async fn crypto_unlock_spec(
    Extension(state): Extension<Arc<AppState>>,
    axum::Json(payload): axum::Json<UnlockSpecRequest>,
) -> (StatusCode, Json<serde_json::Value>) {
    let offer_id = payload.offer_id.trim().to_string();

    let price_cents = match paypal::get_offer_price_cents(&offer_id) {
        Some(p) => p,
        None => {
            return (
                StatusCode::BAD_REQUEST,
                Json(json!({"success": false, "error": "Unknown offer id"})),
            )
        }
    };

    let recipient = match std::env::var("X402_RECIPIENT_ADDRESS") {
        Ok(a) if !a.is_empty() => a,
        _ => {
            return (
                StatusCode::INTERNAL_SERVER_ERROR,
                Json(json!({"success": false, "error": "X402_RECIPIENT_ADDRESS not configured on server"})),
            )
        }
    };

    let base_url =
        std::env::var("PUBLIC_BASE_URL").unwrap_or_else(|_| "https://videosync.video".to_string());
    let resource = format!("{}/api/crypto/unlock", base_url);
    let offer_name = paypal::PAYPAL_OFFERS
        .iter()
        .find(|o| o.id == offer_id)
        .map(|o| o.name)
        .unwrap_or("VideoSync Studio Pack");
    let description = format!("{} — VideoSync Studio", offer_name);

    let spec = x402::build_payment_required(price_cents, &recipient, &resource, &description);
    (
        StatusCode::OK,
        Json(json!({
            "success": true,
            "offer_id": offer_id,
            "price_usd_cents": price_cents,
            "x402": spec,
        })),
    )
}

/// POST /api/crypto/unlock — verifies X-Payment via Coinbase facilitator
/// and records the payment in studio_payments.
async fn crypto_unlock(
    Extension(state): Extension<Arc<AppState>>,
    headers: HeaderMap,
    axum::Json(payload): axum::Json<UnlockRequest>,
) -> (StatusCode, Json<serde_json::Value>) {
    let offer_id = payload.offer_id.trim().to_string();

    let price_cents = match paypal::get_offer_price_cents(&offer_id) {
        Some(p) => p,
        None => {
            return (
                StatusCode::BAD_REQUEST,
                Json(json!({"success": false, "error": "Unknown offer id"})),
            )
        }
    };

    let x_payment = match headers.get("X-Payment").and_then(|h| h.to_str().ok()) {
        Some(s) if !s.is_empty() => s.to_string(),
        _ => {
            return (
                StatusCode::PAYMENT_REQUIRED,
                Json(json!({"success": false, "error": "Missing X-Payment header. Fetch /unlock-spec first."})),
            )
        }
    };

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
    let resource = format!("{}/api/crypto/unlock", base_url);
    let offer_name = paypal::PAYPAL_OFFERS
        .iter()
        .find(|o| o.id == offer_id)
        .map(|o| o.name)
        .unwrap_or("VideoSync Studio Pack");
    let description = format!("{} — VideoSync Studio", offer_name);

    let spec = x402::build_payment_required(price_cents, &recipient, &resource, &description);
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

    // Extract payer address from the signed authorization payload
    let payer_address: Option<String> = (|| {
        use base64::Engine;
        let bytes = base64::engine::general_purpose::STANDARD.decode(&x_payment).ok()?;
        let parsed: serde_json::Value = serde_json::from_slice(&bytes).ok()?;
        parsed
            .pointer("/payload/authorization/from")
            .and_then(|v| v.as_str())
            .map(|s| s.to_string())
    })();

    // Record payment in studio_payments
    let _ = sqlx::query(
        "INSERT INTO studio_payments
         (offer_id, offer_name, amount_cents, currency, payment_method, status,
          tx_hash, payer_address, raw_meta, completed_at)
         VALUES ($1, $2, $3, 'USD', 'usdc_base', 'completed', $4, $5, $6, NOW())",
    )
    .bind(&offer_id)
    .bind(offer_name)
    .bind(price_cents as i32)
    .bind(&tx_hash)
    .bind(&payer_address)
    .bind(serde_json::json!({"network": "base", "asset": req.asset}))
    .execute(&state.db_pool)
    .await;

    let dollars = price_cents as f64 / 100.0;
    let payer = payer_address.as_deref().unwrap_or("unknown");
    let tx_short = tx_hash.chars().take(10).collect::<String>();

    let notify_text = format!(
        "💰 *USDC Studio payment — ${:.2}*\n\
         Offer: {}\n\
         Tx: `{}...`\n\
         Payer: {}",
        dollars, offer_name, tx_short, payer,
    );
    tokio::spawn(async move {
        crate::telegram_bot::notify_admin(&notify_text).await;
    });

    let email_subject = format!(
        "New USDC payment: ${:.2} — {}",
        dollars, offer_name,
    );
    let email_body = format!(
        "A new payment has been received.\n\n\
         Method: USDC (base)\n\
         Offer: {}\n\
         Amount: ${:.2}\n\
         Payer: {}\n\
         Transaction: {}\n\
         Time: {}",
        offer_name,
        dollars,
        payer,
        tx_hash,
        chrono::Utc::now().format("%Y-%m-%d %H:%M:%S UTC"),
    );
    tokio::spawn(async move {
        crate::email::notify_admin(&email_subject, &email_body).await;
    });

    // Fulfill: create a delivery for the purchased offer
    let gig_type = match offer_id.as_str() {
        "saas-demo-starter" | "saas-demo-launch" => "product_demo",
        "agency-3-videos" => "product_demo",
        "product-mockup-standard" => "product_mockup",
        "education-explainer-standard" => "educational_explainer",
        "blender-scene-standard" => "blender_scene",
        "clip-enhancement-standard" => "clip_enhancement",
        "audio-standard" => "voice_audio",
        _ => "product_demo",
    };
    let delivery_title = format!("{} — {}", offer_name, payer);
    let delivery_id: Option<Uuid> = sqlx::query_scalar(
        "INSERT INTO deliveries (client_ref, title, gig_type, prompt, style, duration, extra_args, status)
         VALUES ($1, $2, $3, $4, 'professional', 30.0, $5, 'pending')
         RETURNING id",
    )
    .bind(&payer)
    .bind(&delivery_title)
    .bind(gig_type)
    .bind(format!("USDC purchase: {}. Offer: {}. Payer: {}. Tx: {}.",
        offer_name, offer_id, payer, tx_hash))
    .bind(serde_json::json!({
        "source": "usdc",
        "offer_id": offer_id,
        "tx_hash": tx_hash,
        "payer_address": payer,
    }))
    .fetch_optional(&state.db_pool)
    .await
    .ok()
    .flatten();

    // Auto-fulfillment: kick off delivery generation workflow
    if let Some(did) = delivery_id {
        let render_state = state.clone();
        tokio::spawn(async move {
            let _ = crate::handlers::admin::ensure_delivery_workflow(&render_state, did).await;
            crate::handlers::admin::run_delivery_job(did, render_state).await;
        });
    }

    let delivery_id_val = delivery_id.map(|id| id.to_string());
    (
        StatusCode::OK,
        Json(json!({
            "success": true,
            "tx_hash": tx_hash,
            "offer_id": offer_id,
            "price_usd_cents": price_cents,
            "delivery_id": delivery_id_val,
            "message": format!("Payment received for {}. Your order is confirmed.", offer_name),
        })),
    )
}
