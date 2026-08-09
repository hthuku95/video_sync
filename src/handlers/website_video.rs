//! Website-URL→Video credits service.
//!
//! Monetizes the `landing_page` Managed Campaign service as an a-la-carte
//! bundle: buyers pay $50 for 10 videos or $100 for up to 30 videos generated
//! from their website URL. Credits are stored per-user in `website_video_bundles`
//! and consumed atomically at generation time.
//!
//! Payment rails:
//!   * PayPal / card — POST /api/website-video/payment/paypal/create then
//!     /capture (JS SDK flow). The existing `paypal` module provides the offer
//!     catalog + helpers; this module adds user-scoped bundle tracking.
//!   * USDC (Base) — GET /api/website-video/pay-spec then POST /api/website-video/settle
//!     with the X-Payment header. Reuses crate::x402 (same rail as campaigns).
//!
//! Every route requires a valid user JWT (auth_middleware).

use crate::handlers::paypal;
use crate::models::auth::Claims;
use crate::AppState;
use axum::{
    extract::{Extension, Query},
    http::{HeaderMap, StatusCode},
    response::Json,
    routing::{get, post},
    Router,
};
use serde::Deserialize;
use serde_json::{json, Value};
use sqlx::Row;
use std::sync::Arc;
use uuid::Uuid;

pub fn website_video_routes() -> Router {
    Router::new()
        .route("/api/website-video/credits", get(credits))
        .route("/api/website-video/pay-spec", get(pay_spec))
        .route("/api/website-video/settle", post(settle))
        .route(
            "/api/website-video/payment/paypal/create",
            post(paypal_create),
        )
        .route(
            "/api/website-video/payment/paypal/capture",
            post(paypal_capture),
        )
        .route("/api/website-video/generate", post(generate))
        .route("/api/website-video/videos", get(list_videos))
        .layer(axum::middleware::from_fn(crate::middleware::auth::auth_middleware))
}

/// Map an offer id to the number of videos (credits) it grants.
fn credits_for_offer(offer_id: &str) -> Option<i32> {
    match offer_id {
        "website-video-10" => Some(10),
        "website-video-30" => Some(30),
        _ => None,
    }
}

fn user_id(claims: &Claims) -> Result<i32, (StatusCode, Json<Value>)> {
    claims
        .sub
        .parse::<i32>()
        .map_err(|_| (StatusCode::UNAUTHORIZED, Json(json!({"success": false, "error": "Invalid user token"}))))
}

/// Read PUBLIC_BASE_URL, falling back to the live ALB (videosync.video is expired).
fn base_url() -> String {
    std::env::var("PUBLIC_BASE_URL")
        .unwrap_or_else(|_| "http://video-editor-api-1737481385.us-east-1.elb.amazonaws.com".to_string())
}

/// Aggregate credit ledger for the authenticated user.
async fn credits(
    Extension(claims): Extension<Claims>,
    Extension(state): Extension<Arc<AppState>>,
) -> (StatusCode, Json<Value>) {
    let uid = match user_id(&claims) {
        Ok(u) => u,
        Err(e) => return e,
    };

    let row = match sqlx::query(
        "SELECT COALESCE(SUM(credits_purchased), 0)::BIGINT AS purchased,
                COALESCE(SUM(credits_used), 0)::BIGINT AS used
         FROM website_video_bundles
         WHERE user_id = $1 AND payment_status = 'completed'",
    )
    .bind(uid)
    .fetch_one(&state.db_pool)
    .await
    {
        Ok(r) => r,
        Err(e) => {
            return (
                StatusCode::INTERNAL_SERVER_ERROR,
                Json(json!({"success": false, "error": format!("DB error: {e}")})),
            )
        }
    };

    let purchased: i64 = row.get("purchased");
    let used: i64 = row.get("used");
    let remaining = purchased - used;

    (
        StatusCode::OK,
        Json(json!({
            "success": true,
            "purchased": purchased,
            "used": used,
            "remaining": remaining.max(0),
        })),
    )
}

// ── USDC (x402) ──────────────────────────────────────────────────────────────

#[derive(Debug, Deserialize)]
struct OfferQuery {
    offer_id: String,
}

/// GET /api/website-video/pay-spec?offer_id=website-video-10
/// Returns the real x402 PaymentRequiredResponse (same shape as campaigns/api-access).
async fn pay_spec(
    Query(q): Query<OfferQuery>,
    Extension(_state): Extension<Arc<AppState>>,
) -> (StatusCode, Json<Value>) {
    let credits_count = match credits_for_offer(&q.offer_id) {
        Some(c) => c,
        None => {
            return (
                StatusCode::BAD_REQUEST,
                Json(json!({"success": false, "error": "Unknown offer id"})),
            )
        }
    };
    let price_cents = match paypal::get_offer_price_cents(&q.offer_id) {
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
                Json(json!({"success": false, "error": "X402_RECIPIENT_ADDRESS not configured"})),
            )
        }
    };

    let offer_name = paypal::PAYPAL_OFFERS
        .iter()
        .find(|o| o.id == q.offer_id)
        .map(|o| o.name)
        .unwrap_or("Website Video Bundle");
    let resource_url = format!("{}/api/website-video/settle", base_url());
    let description = format!(
        "{} — {} videos generated from your website URL",
        offer_name, credits_count
    );

    let spec = crate::x402::build_payment_required(price_cents, &recipient, &resource_url, &description);
    (
        StatusCode::OK,
        Json(serde_json::to_value(spec).unwrap_or(json!({"error": "spec serialise failed"}))),
    )
}

#[derive(Debug, Deserialize)]
struct SettleRequest {
    offer_id: String,
}

/// POST /api/website-video/settle — X-Payment header + {offer_id} body.
/// Verifies via the Coinbase facilitator, records a completed bundle, grants credits.
async fn settle(
    Extension(claims): Extension<Claims>,
    headers: HeaderMap,
    Extension(state): Extension<Arc<AppState>>,
    Json(req): Json<SettleRequest>,
) -> (StatusCode, Json<Value>) {
    let uid = match user_id(&claims) {
        Ok(u) => u,
        Err(e) => return e,
    };
    let credits_count = match credits_for_offer(&req.offer_id) {
        Some(c) => c,
        None => {
            return (
                StatusCode::BAD_REQUEST,
                Json(json!({"success": false, "error": "Unknown offer id"})),
            )
        }
    };
    let price_cents = match paypal::get_offer_price_cents(&req.offer_id) {
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
                Json(json!({"success": false, "error": "Missing X-Payment header. Fetch /pay-spec first."})),
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

    let offer_name = paypal::PAYPAL_OFFERS
        .iter()
        .find(|o| o.id == req.offer_id)
        .map(|o| o.name)
        .unwrap_or("Website Video Bundle");
    let resource_url = format!("{}/api/website-video/settle", base_url());
    let description = format!("{} — Website Video bundle", offer_name);
    let spec = crate::x402::build_payment_required(price_cents, &recipient, &resource_url, &description);
    let requirement = match spec.accepts.first() {
        Some(r) => r.clone(),
        None => {
            return (
                StatusCode::INTERNAL_SERVER_ERROR,
                Json(json!({"success": false, "error": "Failed to build payment requirements"})),
            )
        }
    };

    let tx_hash = match crate::x402::settle_or_reject(&x_payment, &requirement).await {
        Ok(h) => h,
        Err(e) => {
            return (
                StatusCode::PAYMENT_REQUIRED,
                Json(json!({"success": false, "error": e})),
            )
        }
    };

    // Extract payer address from the signed authorization payload.
    let payer_address: Option<String> = (|| {
        use base64::Engine;
        let bytes = base64::engine::general_purpose::STANDARD.decode(&x_payment).ok()?;
        let parsed: Value = serde_json::from_slice(&bytes).ok()?;
        parsed
            .pointer("/payload/authorization/from")
            .and_then(|v| v.as_str())
            .map(|s| s.to_string())
    })();

    let _bundle_id: Uuid = match sqlx::query_scalar(
        "INSERT INTO website_video_bundles
         (user_id, offer_id, credits_purchased, payment_status, payment_method, tx_hash, amount_cents, paid_at)
         VALUES ($1, $2, $3, 'completed', 'usdc_base', $4, $5, NOW())
         RETURNING id",
    )
    .bind(uid)
    .bind(&req.offer_id)
    .bind(credits_count)
    .bind(&tx_hash)
    .bind(price_cents as i32)
    .fetch_one(&state.db_pool)
    .await
    {
        Ok(id) => id,
        Err(e) => {
            return (
                StatusCode::INTERNAL_SERVER_ERROR,
                Json(json!({"success": false, "error": format!("DB insert failed: {e}")})),
            )
        }
    };

    let remaining = match remaining_credits(&state, uid).await {
        Ok(r) => r,
        Err(e) => {
            return (
                StatusCode::INTERNAL_SERVER_ERROR,
                Json(json!({"success": false, "error": format!("DB error: {e}")})),
            )
        }
    };

    let tx_short = tx_hash.chars().take(10).collect::<String>();
    let notify_text = format!(
        "💰 *USDC Website Video payment — ${:.2}*\n\
         Bundle: {} ({} videos)\n\
         Tx: `{}...`\n\
         Payer: {}",
        price_cents as f64 / 100.0,
        req.offer_id,
        credits_count,
        tx_short,
        payer_address.as_deref().unwrap_or("unknown"),
    );
    tokio::spawn(async move {
        crate::telegram_bot::notify_admin(&notify_text).await;
    });

    (
        StatusCode::OK,
        Json(json!({
            "success": true,
            "tx_hash": tx_hash,
            "credits_purchased": credits_count,
            "credits_remaining": remaining,
        })),
    )
}

// ── PayPal / card ───────────────────────────────────────────────────────────

#[derive(Debug, Deserialize)]
struct PayPalCreateRequest {
    offer_id: String,
}

/// POST /api/website-video/payment/paypal/create
/// Creates a pending bundle row + a PayPal order. Returns bundle_id + order id
/// so the JS SDK can continue on approve.
async fn paypal_create(
    Extension(claims): Extension<Claims>,
    Extension(state): Extension<Arc<AppState>>,
    Json(req): Json<PayPalCreateRequest>,
) -> (StatusCode, Json<Value>) {
    let uid = match user_id(&claims) {
        Ok(u) => u,
        Err(e) => return e,
    };
    let offer = match paypal::PAYPAL_OFFERS.iter().find(|o| o.id == req.offer_id) {
        Some(o) => *o,
        None => {
            return (
                StatusCode::BAD_REQUEST,
                Json(json!({"success": false, "error": "Unknown PayPal offer id"})),
            )
        }
    };
    let credits_count = match credits_for_offer(&req.offer_id) {
        Some(c) => c,
        None => {
            return (
                StatusCode::BAD_REQUEST,
                Json(json!({"success": false, "error": "Offer is not a Website Video bundle"})),
            )
        }
    };

    // Create the pending bundle row first so we can link the order to it.
    let bundle_id: Uuid = match sqlx::query_scalar(
        "INSERT INTO website_video_bundles
         (user_id, offer_id, credits_purchased, payment_status, amount_cents)
         VALUES ($1, $2, $3, 'pending', $4)
         RETURNING id",
    )
    .bind(uid)
    .bind(req.offer_id.as_str())
    .bind(credits_count)
    .bind(offer.amount.parse::<f64>().unwrap_or(0.0) as i32 * 100)
    .fetch_one(&state.db_pool)
    .await
    {
        Ok(id) => id,
        Err(e) => {
            return (
                StatusCode::INTERNAL_SERVER_ERROR,
                Json(json!({"success": false, "error": format!("DB insert failed: {e}")})),
            )
        }
    };

    let env = paypal::get_paypal_env(&state).await;
    let base_url = paypal::paypal_base_url(&env);
    let (client_id, client_secret) = paypal::paypal_credentials(&env);
    if client_id.is_empty() || client_secret.is_empty() {
        return (
            StatusCode::INTERNAL_SERVER_ERROR,
            Json(json!({"success": false, "error": "PayPal not configured"})),
        );
    }

    let access_token = match paypal::get_paypal_access_token(base_url, &client_id, &client_secret).await {
        Ok(t) => t,
        Err(e) => {
            return (
                StatusCode::INTERNAL_SERVER_ERROR,
                Json(json!({"success": false, "error": e})),
            )
        }
    };

    let order_body = json!({
        "intent": "CAPTURE",
        "purchase_units": [{
            "reference_id": offer.id,
            "description": offer.name,
            "amount": {
                "currency_code": "USD",
                "value": offer.amount,
                "breakdown": {
                    "item_total": {
                        "currency_code": "USD",
                        "value": offer.amount
                    }
                }
            },
            "items": [{
                "name": offer.name,
                "description": offer.description,
                "unit_amount": {
                    "currency_code": "USD",
                    "value": offer.amount
                },
                "quantity": "1",
                "category": "DIGITAL_GOODS"
            }]
        }],
        "payment_source": {
            "paypal": {
                "experience_context": {
                    "payment_method_preference": "IMMEDIATE_PAYMENT_REQUIRED",
                    "brand_name": "VideoSync Website Video",
                    "locale": "en-US",
                    "landing_page": "LOGIN",
                    "user_action": "PAY_NOW"
                }
            }
        }
    });

    let order_resp = match reqwest::Client::new()
        .post(format!("{}/v2/checkout/orders", base_url))
        .header("Content-Type", "application/json")
        .bearer_auth(&access_token)
        .json(&order_body)
        .send()
        .await
    {
        Ok(r) => r,
        Err(e) => {
            return (
                StatusCode::INTERNAL_SERVER_ERROR,
                Json(json!({"success": false, "error": format!("PayPal order failed: {e}")})),
            )
        }
    };

    let order_json: Value = match order_resp.json().await {
        Ok(b) => b,
        Err(e) => {
            return (
                StatusCode::INTERNAL_SERVER_ERROR,
                Json(json!({"success": false, "error": format!("PayPal order parse failed: {e}")})),
            )
        }
    };

    let paypal_order_id = match order_json.get("id").and_then(|id| id.as_str()) {
        Some(id) => id.to_string(),
        None => {
            return (
                StatusCode::INTERNAL_SERVER_ERROR,
                Json(json!({"success": false, "error": "PayPal did not return an order ID", "detail": order_json})),
            )
        }
    };

    let _ = sqlx::query(
        "UPDATE website_video_bundles SET paypal_order_id = $1, updated_at = NOW() WHERE id = $2",
    )
    .bind(&paypal_order_id)
    .bind(bundle_id)
    .execute(&state.db_pool)
    .await;

    (
        StatusCode::OK,
        Json(json!({
            "success": true,
            "bundle_id": bundle_id.to_string(),
            "paypal_order_id": paypal_order_id,
        })),
    )
}

#[derive(Debug, Deserialize)]
struct PayPalCaptureRequest {
    bundle_id: String,
}

/// POST /api/website-video/payment/paypal/capture
/// Captures the PayPal order for the user's pending bundle, grants credits on success.
async fn paypal_capture(
    Extension(claims): Extension<Claims>,
    Extension(state): Extension<Arc<AppState>>,
    Json(req): Json<PayPalCaptureRequest>,
) -> (StatusCode, Json<Value>) {
    let uid = match user_id(&claims) {
        Ok(u) => u,
        Err(e) => return e,
    };
    let bundle_id = match Uuid::parse_str(&req.bundle_id) {
        Ok(id) => id,
        Err(_) => {
            return (
                StatusCode::BAD_REQUEST,
                Json(json!({"success": false, "error": "Invalid bundle id"})),
            )
        }
    };

    // Load the bundle (must belong to the user and be pending).
    let row = match sqlx::query(
        "SELECT offer_id, credits_purchased, paypal_order_id, payment_status
         FROM website_video_bundles WHERE id = $1 AND user_id = $2",
    )
    .bind(bundle_id)
    .bind(uid)
    .fetch_optional(&state.db_pool)
    .await
    {
        Ok(Some(r)) => r,
        Ok(None) => {
            return (
                StatusCode::NOT_FOUND,
                Json(json!({"success": false, "error": "Bundle not found"})),
            )
        }
        Err(e) => {
            return (
                StatusCode::INTERNAL_SERVER_ERROR,
                Json(json!({"success": false, "error": format!("DB error: {e}")})),
            )
        }
    };

    let offer_id: String = row.get("offer_id");
    let credits_count: i32 = row.get("credits_purchased");
    let payment_status: String = row.get("payment_status");
    if payment_status == "completed" {
        let remaining = remaining_credits(&state, uid).await.unwrap_or(0);
        return (
            StatusCode::OK,
            Json(json!({"success": true, "credits_purchased": credits_count, "credits_remaining": remaining})),
        );
    }
    let paypal_order_id: String = row.get("paypal_order_id");

    let env = paypal::get_paypal_env(&state).await;
    let base_url = paypal::paypal_base_url(&env);
    let (client_id, client_secret) = paypal::paypal_credentials(&env);
    if client_id.is_empty() || client_secret.is_empty() {
        return (
            StatusCode::INTERNAL_SERVER_ERROR,
            Json(json!({"success": false, "error": "PayPal not configured"})),
        );
    }

    let access_token = match paypal::get_paypal_access_token(base_url, &client_id, &client_secret).await {
        Ok(t) => t,
        Err(e) => {
            return (
                StatusCode::INTERNAL_SERVER_ERROR,
                Json(json!({"success": false, "error": e})),
            )
        }
    };

    let capture_resp = match reqwest::Client::new()
        .post(format!("{}/v2/checkout/orders/{}/capture", base_url, paypal_order_id))
        .header("Content-Type", "application/json")
        .bearer_auth(&access_token)
        .send()
        .await
    {
        Ok(r) => r,
        Err(e) => {
            return (
                StatusCode::INTERNAL_SERVER_ERROR,
                Json(json!({"success": false, "error": format!("PayPal capture failed: {e}")})),
            )
        }
    };

    let capture_body: Value = match capture_resp.json().await {
        Ok(b) => b,
        Err(e) => {
            return (
                StatusCode::INTERNAL_SERVER_ERROR,
                Json(json!({"success": false, "error": format!("PayPal capture parse failed: {e}")})),
            )
        }
    };

    let capture_status = capture_body
        .get("status")
        .and_then(|s| s.as_str())
        .unwrap_or("UNKNOWN");
    if capture_status != "COMPLETED" {
        return (
            StatusCode::BAD_REQUEST,
            Json(json!({"success": false, "error": format!("PayPal capture not completed: {capture_status}"), "capture": capture_body})),
        );
    }

    let capture_id = capture_body
        .pointer("/purchase_units/0/payments/captures/0/id")
        .and_then(|v| v.as_str())
        .map(|s| s.to_string());
    let buyer_email = capture_body
        .pointer("/payer/email_address")
        .and_then(|v| v.as_str())
        .map(|s| s.to_string());

    let _ = sqlx::query(
        "UPDATE website_video_bundles
         SET payment_status = 'completed', paypal_capture_id = $1, paid_at = NOW(), updated_at = NOW()
         WHERE id = $2",
    )
    .bind(capture_id.as_deref())
    .bind(bundle_id)
    .execute(&state.db_pool)
    .await;

    let remaining = match remaining_credits(&state, uid).await {
        Ok(r) => r,
        Err(e) => {
            return (
                StatusCode::INTERNAL_SERVER_ERROR,
                Json(json!({"success": false, "error": format!("DB error: {e}")})),
            )
        }
    };

    let dollars = credits_count as f64 * 5.0; // $5/video → $50 or $100
    let notify_text = format!(
        "💰 *PayPal Website Video payment — ${:.2}*\n\
         Offer: {} ({} videos)\n\
         PayPal Order: {}\n\
         Payer: {}",
        dollars,
        offer_id,
        credits_count,
        paypal_order_id,
        buyer_email.unwrap_or_else(|| "unknown".to_string()),
    );
    tokio::spawn(async move {
        crate::telegram_bot::notify_admin(&notify_text).await;
    });

    (
        StatusCode::OK,
        Json(json!({
            "success": true,
            "credits_purchased": credits_count,
            "credits_remaining": remaining,
        })),
    )
}

// ── Generation ──────────────────────────────────────────────────────────────

#[derive(Debug, Deserialize)]
pub struct GenerateVideoRequest {
    pub source_url: String,
    pub product_name: Option<String>,
    pub brief: Option<String>,
    pub style: Option<String>,
    pub duration: Option<f64>,
}

/// POST /api/website-video/generate
/// Validates credits, creates a landing_page delivery, starts the
/// AgenticServicePipeline, and atomically consumes one credit.
async fn generate(
    Extension(claims): Extension<Claims>,
    Extension(state): Extension<Arc<AppState>>,
    Json(req): Json<GenerateVideoRequest>,
) -> (StatusCode, Json<Value>) {
    let uid = match user_id(&claims) {
        Ok(u) => u,
        Err(e) => return e,
    };

    let has_product_url = req.source_url.starts_with("http://") || req.source_url.starts_with("https://");
    if !has_product_url {
        return (
            StatusCode::BAD_REQUEST,
            Json(json!({"success": false, "error": "source_url must be a valid http(s) URL"})),
        );
    }

    // Atomic credit check + consume in a single UPDATE ... RETURNING.
    let consumed: Option<Uuid> = sqlx::query_scalar(
        "UPDATE website_video_bundles
            SET credits_used = credits_used + 1, updated_at = NOW()
          WHERE id = (
                SELECT id FROM website_video_bundles
                 WHERE user_id = $1 AND payment_status = 'completed'
                   AND credits_used < credits_purchased
                 ORDER BY created_at ASC
                 LIMIT 1
          )
         RETURNING id",
    )
    .bind(uid)
    .fetch_optional(&state.db_pool)
    .await
    .unwrap_or(None);

    if consumed.is_none() {
        return (
            StatusCode::PAYMENT_REQUIRED,
            Json(json!({"success": false, "error": "No video credits remaining. Purchase a bundle to continue."})),
        );
    }

    let product_name = req.product_name.unwrap_or_else(|| "the product".to_string());
    let brief = req.brief.unwrap_or_default();
    let style = req.style.unwrap_or_else(|| "premium SaaS explainer, cinematic, clean motion graphics".to_string());
    let duration = req.duration.unwrap_or(60.0);

    // Deep-crawl the website via BrowserBase so the agent gets design tokens,
    // all pages, and full content (mirrors the prospect landing_page flow).
    let mut scraped_site = None;
    match crate::browserbase_client::crawl_website(&req.source_url).await {
        Ok(crawl) => {
            let mut ctx = String::new();
            ctx.push_str(&format!("## Website Crawl: {}\n\n", req.source_url));
            if !crawl.css_info.is_empty() {
                ctx.push_str(&format!("### Design Tokens\n{}\n\n", crawl.css_info));
            }
            ctx.push_str(&format!("### Pages ({})\n", crawl.pages.len()));
            for p in &crawl.pages {
                ctx.push_str(&format!("- {} ({})\n", p.title, p.url));
            }
            ctx.push_str(&format!("\n### Full Content\n{}", crawl.combined_markdown));
            scraped_site = Some(ctx);
        }
        Err(e) => {
            tracing::warn!("Website crawl failed for {}: {e}", req.source_url);
        }
    }

    let hero = crate::handlers::prospects::fetch_landing_page_hero(&req.source_url).await;
    let mut extra = json!({
        "source": "website_video",
        "website_video": true,
        "source_url": req.source_url,
        "product_name": product_name.clone(),
        "scraped_website_content": scraped_site,
    });
    if let Some(hero_url) = hero {
        extra["reference_image_url"] = json!(hero_url);
    }

    let prompt = format!(
        "Generate a business landing page video for {product_name} from the website {source_url}. \
         Target ~{duration}s — can be longer if content requires it. \
         Brief: {brief}",
        source_url = req.source_url,
        duration = duration,
    );

    let delivery_id: Uuid = match sqlx::query_scalar(
        "INSERT INTO deliveries
         (client_ref, title, gig_type, prompt, style, duration, extra_args, status, source_url, user_id)
         VALUES ($1, $2, 'landing_page', $3, $4, $5, $6, 'pending', $7, $8)
         RETURNING id",
    )
    .bind(format!("website_video:{uid}"))
    .bind(format!("Website Video for {product_name}"))
    .bind(&prompt)
    .bind(&style)
    .bind(duration)
    .bind(&extra)
    .bind(&req.source_url)
    .bind(uid)
    .fetch_one(&state.db_pool)
    .await
    {
        Ok(id) => id,
        Err(e) => {
            // Refund the consumed credit so a DB failure doesn't burn it.
            let _ = sqlx::query(
                "UPDATE website_video_bundles SET credits_used = credits_used - 1, updated_at = NOW() WHERE id = $1",
            )
            .bind(consumed)
            .execute(&state.db_pool)
            .await;
            return (
                StatusCode::INTERNAL_SERVER_ERROR,
                Json(json!({"success": false, "error": format!("Delivery insert failed: {e}")})),
            )
        }
    };

    let scraped_context = scraped_site
        .map(|s| format!("\n\n## Scraped Website Content\n{}\n\nUse the above website content to understand the product and its value proposition.", s))
        .unwrap_or_default();

    let workflow_result = crate::services::AgenticServicePipeline::start(
        state.clone(),
        crate::services::ServiceType::LandingPage,
        crate::services::ServiceInput {
            title: format!("Website Video for {}", product_name),
            brief: format!("{prompt}\n\nGenerate a buyer-facing landing page video for {product_name}. Include concise narration and end with a clear CTA.{scraped_context}"),
            source_url: Some(req.source_url.clone()),
            style,
            duration_seconds: duration,
            delivery_id,
            prospect_id: None,
            session_uuid: None,
            user_id: Some(uid),
            source_table: Some("deliveries".to_string()),
            source_record_id: Some(delivery_id),
            idempotency_key: Some(format!("website-video-agentic:{delivery_id}")),
            reference_images: vec![],
        },
    )
    .await;

    match workflow_result {
        Ok(workflow_id) => {
            let _ = sqlx::query("UPDATE deliveries SET workflow_id = $1 WHERE id = $2")
                .bind(workflow_id)
                .bind(delivery_id)
                .execute(&state.db_pool)
                .await;
        }
        Err(error) => {
            let _ = sqlx::query(
                "UPDATE deliveries SET status='failed', error_message=$1, completed_at=NOW() WHERE id=$2",
            )
            .bind(&error)
            .bind(delivery_id)
            .execute(&state.db_pool)
            .await;
            tracing::error!("AgenticServicePipeline failed for delivery {delivery_id}: {error}");
        }
    }

    let remaining = match remaining_credits(&state, uid).await {
        Ok(r) => r,
        Err(_) => 0,
    };

    (
        StatusCode::OK,
        Json(json!({
            "success": true,
            "delivery_id": delivery_id.to_string(),
            "credits_remaining": remaining,
        })),
    )
}

// ── Listing ─────────────────────────────────────────────────────────────────

/// GET /api/website-video/videos — the authenticated user's deliveries.
async fn list_videos(
    Extension(claims): Extension<Claims>,
    Extension(state): Extension<Arc<AppState>>,
) -> (StatusCode, Json<Value>) {
    let uid = match user_id(&claims) {
        Ok(u) => u,
        Err(e) => return e,
    };

    let rows = match sqlx::query(
        "SELECT id, title, status, output_r2_url, preview_r2_url, error_message, created_at, completed_at
         FROM deliveries
         WHERE user_id = $1
         ORDER BY created_at DESC
         LIMIT 200",
    )
    .bind(uid)
    .fetch_all(&state.db_pool)
    .await
    {
        Ok(rows) => rows,
        Err(e) => {
            return (
                StatusCode::INTERNAL_SERVER_ERROR,
                Json(json!({"success": false, "error": format!("DB error: {e}")})),
            )
        }
    };

    let deliveries: Vec<Value> = rows
        .iter()
        .map(|row| {
            json!({
                "id": row.get::<Uuid, _>("id").to_string(),
                "title": row.get::<String, _>("title"),
                "status": row.get::<String, _>("status"),
                "output_r2_url": row.try_get::<Option<String>, _>("output_r2_url").ok().flatten(),
                "preview_r2_url": row.try_get::<Option<String>, _>("preview_r2_url").ok().flatten(),
                "error_message": row.try_get::<Option<String>, _>("error_message").ok().flatten(),
                "created_at": row.get::<chrono::DateTime<chrono::Utc>, _>("created_at").to_rfc3339(),
                "completed_at": row.try_get::<Option<chrono::DateTime<chrono::Utc>>, _>("completed_at").ok().flatten().map(|d| d.to_rfc3339()),
            })
        })
        .collect();

    (StatusCode::OK, Json(json!({"success": true, "deliveries": deliveries})))
}

async fn remaining_credits(state: &Arc<AppState>, uid: i32) -> Result<i64, sqlx::Error> {
    let row = sqlx::query(
        "SELECT COALESCE(SUM(credits_purchased), 0)::BIGINT AS purchased,
                COALESCE(SUM(credits_used), 0)::BIGINT AS used
         FROM website_video_bundles
         WHERE user_id = $1 AND payment_status = 'completed'",
    )
    .bind(uid)
    .fetch_one(&state.db_pool)
    .await?;
    let purchased: i64 = row.get("purchased");
    let used: i64 = row.get("used");
    Ok((purchased - used).max(0))
}
