use crate::AppState;
use axum::{
    extract::{Extension, Path},
    http::StatusCode,
    response::Json,
    routing::{get, post},
    Router,
};
use chrono::Utc;
use serde::{Deserialize, Serialize};
use serde_json::{json, Value};
use sqlx::Row;
use std::sync::Arc;
use uuid::Uuid;

pub fn paypal_routes() -> Router {
    Router::new()
        .route("/api/paypal/config", get(paypal_config))
        .route("/api/paypal/orders", post(create_paypal_order))
        .route("/api/paypal/orders/:order_id/capture", post(capture_paypal_order))
}

#[derive(Debug, Deserialize)]
pub struct CreatePayPalOrderRequest {
    pub offer_id: String,
}

#[derive(Debug, Serialize, Clone, Copy)]
pub(crate) struct PayPalOffer {
    pub(crate) id: &'static str,
    pub(crate) name: &'static str,
    pub(crate) amount: &'static str,
    pub(crate) description: &'static str,
}

pub(crate) const PAYPAL_OFFERS: &[PayPalOffer] = &[
    PayPalOffer {
        id: "saas-demo-starter",
        name: "SaaS/App Demo Starter",
        amount: "399.00",
        description: "Starter product demo video from URL, screenshots, or brief.",
    },
    PayPalOffer {
        id: "saas-demo-launch",
        name: "SaaS/App Demo Launch Pack",
        amount: "699.00",
        description: "Launch demo pack with hooks, thumbnail/hero concept, and delivery page.",
    },
    PayPalOffer {
        id: "agency-3-videos",
        name: "Website-to-Video Agency Pack",
        amount: "1500.00",
        description: "Three client website/app demo videos with delivery pages.",
    },
    PayPalOffer {
        id: "product-mockup-standard",
        name: "Product Mockup Video Pack",
        amount: "599.00",
        description: "Animated UI/product mockup video package.",
    },
    PayPalOffer {
        id: "education-explainer-standard",
        name: "Education Explainer Pack",
        amount: "750.00",
        description: "Visual explainer with diagrams, narration, and review-ready delivery.",
    },
    PayPalOffer {
        id: "blender-scene-standard",
        name: "Blender 2D/3D Scene Pack",
        amount: "1200.00",
        description: "Blender scene pack with multiple shots and camera/lighting polish.",
    },
    PayPalOffer {
        id: "clip-enhancement-standard",
        name: "Clip Enhancement Pack",
        amount: "600.00",
        description: "Short-form clip enhancement pack with captions, graphics, and variants.",
    },
    PayPalOffer {
        id: "audio-standard",
        name: "Voice & Audio Production Pack",
        amount: "300.00",
        description: "Narration, voiceover, summary, or audio-backed video package.",
    },
];

/// Return the price in US cents for a given offer ID.
pub fn get_offer_price_cents(offer_id: &str) -> Option<u64> {
    let offer = PAYPAL_OFFERS.iter().find(|o| o.id == offer_id)?;
    let clean: String = offer.amount.chars().filter(|c| c.is_ascii_digit() || *c == '.').collect();
    let dollars: f64 = clean.parse().ok()?;
    Some((dollars * 100.0) as u64)
}

/// Read paypal_env from app_config table, falling back to PAYPAL_ENV env var.
async fn get_paypal_env(state: &AppState) -> String {
    let db_value: Option<String> = sqlx::query_scalar(
        "SELECT value FROM app_config WHERE key = 'paypal_env'"
    )
    .fetch_optional(&state.db_pool)
    .await
    .ok()
    .flatten();
    db_value.unwrap_or_else(|| std::env::var("PAYPAL_ENV").unwrap_or_else(|_| "sandbox".to_string()))
}

fn paypal_base_url(env: &str) -> &'static str {
    match env {
        "live" | "production" => "https://api-m.paypal.com",
        _ => "https://api-m.sandbox.paypal.com",
    }
}

fn paypal_credentials(env: &str) -> (String, String) {
    let client_id = match env {
        "live" | "production" => std::env::var("PAYPAL_CLIENT_ID_LIVE")
            .or_else(|_| std::env::var("PAYPAL_LIVE_CLIENT_ID"))
            .or_else(|_| std::env::var("PAYPAL_CLIENT_ID"))
            .unwrap_or_default(),
        _ => std::env::var("PAYPAL_CLIENT_ID_SANDBOX")
            .or_else(|_| std::env::var("PAYPAL_SANDBOX_CLIENT_ID"))
            .or_else(|_| std::env::var("PAYPAL_CLIENT_ID"))
            .unwrap_or_default(),
    };
    let client_secret = match env {
        "live" | "production" => std::env::var("PAYPAL_CLIENT_SECRET_LIVE")
            .or_else(|_| std::env::var("PAYPAL_LIVE_CLIENT_SECRET"))
            .or_else(|_| std::env::var("PAYPAL_CLIENT_SECRET"))
            .unwrap_or_default(),
        _ => std::env::var("PAYPAL_CLIENT_SECRET_SANDBOX")
            .or_else(|_| std::env::var("PAYPAL_SANDBOX_CLIENT_SECRET"))
            .or_else(|_| std::env::var("PAYPAL_CLIENT_SECRET"))
            .unwrap_or_default(),
    };
    (client_id, client_secret)
}

async fn get_paypal_access_token(base_url: &str, client_id: &str, client_secret: &str) -> Result<String, String> {
    let resp = reqwest::Client::new()
        .post(format!("{}/v1/oauth2/token", base_url))
        .header("Accept", "application/json")
        .header("Content-Type", "application/x-www-form-urlencoded")
        .basic_auth(client_id, Some(client_secret))
        .body("grant_type=client_credentials")
        .send()
        .await
        .map_err(|e| format!("PayPal auth request failed: {}", e))?;

    let body: Value = resp.json().await.map_err(|e| format!("PayPal auth parse failed: {}", e))?;
    body.get("access_token")
        .and_then(|t| t.as_str())
        .map(|s| s.to_string())
        .ok_or_else(|| "PayPal access token not returned".to_string())
}

async fn paypal_config(
    Extension(state): Extension<Arc<AppState>>,
) -> Json<Value> {
    let env = get_paypal_env(&state).await;
    let (client_id, _) = paypal_credentials(&env);

    Json(json!({
        "success": true,
        "environment": env,
        "client_id": client_id,
        "offers": PAYPAL_OFFERS,
    }))
}

fn find_offer(offer_id: &str) -> Option<&'static PayPalOffer> {
    PAYPAL_OFFERS.iter().find(|offer| offer.id == offer_id)
}

async fn create_paypal_order(
    Extension(state): Extension<Arc<AppState>>,
    axum::Json(payload): axum::Json<CreatePayPalOrderRequest>,
) -> (StatusCode, Json<Value>) {
    let offer = match find_offer(payload.offer_id.trim()) {
        Some(o) => o,
        None => return (
            StatusCode::BAD_REQUEST,
            Json(json!({"success": false, "message": "Unknown PayPal offer id"})),
        ),
    };

    let env = get_paypal_env(&state).await;
    let base_url = paypal_base_url(&env);
    let (client_id, client_secret) = paypal_credentials(&env);

    if client_id.is_empty() || client_secret.is_empty() {
        return (
            StatusCode::INTERNAL_SERVER_ERROR,
            Json(json!({"success": false, "message": "PayPal not configured"})),
        );
    }

    let access_token = match get_paypal_access_token(base_url, &client_id, &client_secret).await {
        Ok(t) => t,
        Err(e) => return (
            StatusCode::INTERNAL_SERVER_ERROR,
            Json(json!({"success": false, "message": e})),
        ),
    };

    let studio_url = std::env::var("STUDIO_PUBLIC_BASE_URL")
        .unwrap_or_else(|_| "https://videosync-studio-723463981172.us-central1.run.app".to_string());

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
                    "brand_name": "VideoSync Studio",
                    "locale": "en-US",
                    "landing_page": "LOGIN",
                    "user_action": "PAY_NOW",
                    "return_url": format!("{}/?paypal_return=1&offer_id={}", studio_url, offer.id),
                    "cancel_url": format!("{}/?paypal_cancel=1&offer_id={}", studio_url, offer.id)
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
        Err(e) => return (
            StatusCode::INTERNAL_SERVER_ERROR,
            Json(json!({"success": false, "message": format!("PayPal order failed: {}", e)})),
        ),
    };

    let order_body: Value = match order_resp.json().await {
        Ok(b) => b,
        Err(e) => return (
            StatusCode::INTERNAL_SERVER_ERROR,
            Json(json!({"success": false, "message": format!("PayPal order parse failed: {}", e)})),
        ),
    };

    if order_body.get("id").and_then(|id| id.as_str()).is_none() {
        return (
            StatusCode::INTERNAL_SERVER_ERROR,
            Json(json!({"success": false, "message": "PayPal did not return an order ID", "detail": order_body})),
        );
    }

    (StatusCode::OK, Json(json!({"success": true, "order": order_body, "offer": offer})))
}

async fn capture_paypal_order(
    Extension(state): Extension<Arc<AppState>>,
    Path(order_id): Path<String>,
) -> (StatusCode, Json<Value>) {
    let env = get_paypal_env(&state).await;
    let base_url = paypal_base_url(&env);
    let (client_id, client_secret) = paypal_credentials(&env);

    if client_id.is_empty() || client_secret.is_empty() {
        return (
            StatusCode::INTERNAL_SERVER_ERROR,
            Json(json!({"success": false, "message": "PayPal not configured"})),
        );
    }

    let access_token = match get_paypal_access_token(base_url, &client_id, &client_secret).await {
        Ok(t) => t,
        Err(e) => return (
            StatusCode::INTERNAL_SERVER_ERROR,
            Json(json!({"success": false, "message": e})),
        ),
    };

    let capture_resp = match reqwest::Client::new()
        .post(format!("{}/v2/checkout/orders/{}/capture", base_url, order_id))
        .header("Content-Type", "application/json")
        .bearer_auth(&access_token)
        .send()
        .await
    {
        Ok(r) => r,
        Err(e) => return (
            StatusCode::INTERNAL_SERVER_ERROR,
            Json(json!({"success": false, "message": format!("PayPal capture failed: {}", e)})),
        ),
    };

    let capture_body: Value = match capture_resp.json().await {
        Ok(b) => b,
        Err(e) => return (
            StatusCode::INTERNAL_SERVER_ERROR,
            Json(json!({"success": false, "message": format!("PayPal capture parse failed: {}", e)})),
        ),
    };

    let capture_status = capture_body
        .get("status")
        .and_then(|s| s.as_str())
        .unwrap_or("UNKNOWN");

    let is_completed = capture_status == "COMPLETED";

    // Record payment in database
    if is_completed {
        let offer_id = capture_body
            .pointer("/purchase_units/0/reference_id")
            .and_then(|v| v.as_str())
            .unwrap_or("unknown");
        let capture_id = capture_body
            .pointer("/purchase_units/0/payments/captures/0/id")
            .and_then(|v| v.as_str());
        let payer_email = capture_body
            .pointer("/payer/email_address")
            .and_then(|v| v.as_str());
        let payer_name = capture_body
            .pointer("/payer/name/given_name")
            .and_then(|v| v.as_str());

        let amount_cents = get_offer_price_cents(offer_id).unwrap_or(0);
        let offer_name = PAYPAL_OFFERS
            .iter()
            .find(|o| o.id == offer_id)
            .map(|o| o.name)
            .unwrap_or(offer_id);

        let payment_method = match env.as_str() {
            "live" | "production" => "paypal_live",
            _ => "paypal_sandbox",
        };

        let payment_id: Option<Uuid> = sqlx::query_scalar(
            "INSERT INTO studio_payments
             (offer_id, offer_name, amount_cents, currency, payment_method, status,
              paypal_order_id, paypal_capture_id, buyer_email, buyer_name, raw_meta, completed_at)
             VALUES ($1, $2, $3, 'USD', $4, 'completed', $5, $6, $7, $8, $9, NOW())
             RETURNING id",
        )
        .bind(offer_id)
        .bind(offer_name)
        .bind(amount_cents as i32)
        .bind(payment_method)
        .bind(&order_id)
        .bind(capture_id)
        .bind(payer_email)
        .bind(payer_name)
        .bind(serde_json::to_value(&capture_body).ok())
        .fetch_optional(&state.db_pool)
        .await
        .ok()
        .flatten();

        // Fulfill: create a delivery for the purchased offer
        let gig_type = match offer_id {
            "saas-demo-starter" | "saas-demo-launch" => "product_demo",
            "agency-3-videos" => "product_demo",
            "product-mockup-standard" => "product_mockup",
            "education-explainer-standard" => "educational_explainer",
            "blender-scene-standard" => "blender_scene",
            "clip-enhancement-standard" => "clip_enhancement",
            "audio-standard" => "voice_audio",
            _ => "product_demo",
        };
        let delivery_title = format!("{} — {}", offer_name, payer_email.unwrap_or("Buyer"));
        let delivery_id: Option<Uuid> = sqlx::query_scalar(
            "INSERT INTO deliveries (client_ref, title, gig_type, prompt, style, duration, extra_args, status)
             VALUES ($1, $2, $3, $4, 'professional', 30.0, $5, 'pending')
             RETURNING id",
        )
        .bind(payer_email.unwrap_or("anonymous"))
        .bind(&delivery_title)
        .bind(gig_type)
        .bind(format!("PayPal purchase: {}. Offer: {}. Payer: {}. Order: {}.",
            offer_name, offer_id, payer_email.unwrap_or("unknown"), order_id))
        .bind(json!({
            "source": "paypal",
            "offer_id": offer_id,
            "paypal_order_id": order_id,
            "paypal_capture_id": capture_id,
            "buyer_email": payer_email,
            "buyer_name": payer_name,
        }))
        .fetch_optional(&state.db_pool)
        .await
        .ok()
        .flatten();

        // Link delivery to payment
        if let (Some(pid), Some(did)) = (payment_id, delivery_id) {
            let _ = sqlx::query(
                "UPDATE studio_payments SET delivery_id = $1 WHERE id = $2",
            )
            .bind(did)
            .bind(pid)
            .execute(&state.db_pool)
            .await;
        }

        let dollars = amount_cents as f64 / 100.0;
        let payer = payer_email.unwrap_or("unknown");

        let notify_text = format!(
            "💰 *PayPal {} payment — ${:.2}*\n\
             Offer: {}\n\
             PayPal Order: {}\n\
             Payer: {}",
            payment_method, dollars, offer_name, order_id, payer,
        );
        tokio::spawn(async move {
            crate::telegram_bot::notify_admin(&notify_text).await;
        });

        let email_subject = format!(
            "New PayPal payment: ${:.2} — {}",
            dollars, offer_name,
        );
        let email_body = format!(
            "A new payment has been received.\n\n\
             Method: PayPal ({})\n\
             Offer: {}\n\
             Amount: ${:.2}\n\
             Payer: {}\n\
             Order ID: {}\n\
             Time: {}",
            payment_method,
            offer_name,
            dollars,
            payer,
            order_id,
            chrono::Utc::now().format("%Y-%m-%d %H:%M:%S UTC"),
        );
        tokio::spawn(async move {
            crate::email::notify_admin(&email_subject, &email_body).await;
        });
    }

    (StatusCode::OK, Json(json!({
        "success": is_completed,
        "status": capture_status,
        "capture": capture_body,
    })))
}
