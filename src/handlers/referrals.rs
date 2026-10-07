use std::sync::Arc;
use axum::{Extension, Json};
use axum::routing::{get, post};
use axum::Router;
use serde::Deserialize;
use sqlx::Row;
use uuid::Uuid;

use crate::middleware::auth::auth_middleware;
use crate::models::auth::Claims;
use crate::AppState;

pub fn referral_routes() -> Router {
    Router::new()
        .route("/api/referrals/my-code", get(api_get_my_referral_code).post(api_create_my_referral_code))
        .route("/api/referrals/my-codes", get(api_list_my_referral_codes))
        .route("/api/referrals/my-commissions", get(api_get_my_commissions))
        .layer(axum::middleware::from_fn(auth_middleware))
}

/// The five campaign apps (owner directive Oct 2026 §65).
pub const REFERRAL_APPS: &[(&str, &str)] = &[
    ("clips", "VideoSync Clips"),
    ("shorts", "VideoSync Shorts"),
    ("app", "Website Video"),
    ("learn", "VideoSync Learn"),
    ("motion", "VideoSync Motion"),
];

pub fn referral_app_host(app: &str) -> Option<&'static str> {
    match app {
        "clips" => Some("https://clips.videosync.ink"),
        "shorts" => Some("https://shorts.videosync.ink"),
        "app" => Some("https://app.videosync.ink"),
        "learn" => Some("https://learn.videosync.ink"),
        "motion" => Some("https://motion.videosync.ink"),
        _ => None,
    }
}

fn ref_url_for(app: Option<&str>, code: &str) -> String {
    match app {
        Some(a) => format!("/ref/{a}/{code}"),
        None => format!("/ref/{code}"),
    }
}

async fn check_whitelisted(state: &Arc<AppState>, email: &str) -> bool {
    let in_whitelist = sqlx::query_scalar::<_, bool>(
        "SELECT EXISTS(SELECT 1 FROM whitelist_emails WHERE email = $1)",
    )
    .bind(email)
    .fetch_one(&state.db_pool)
    .await
    .unwrap_or(false);
    in_whitelist
}

/// GET /api/referrals/my-codes — all five app links for the current user.
/// Missing apps are auto-created so the dashboard always shows 5 copyable links.
async fn api_list_my_referral_codes(
    Extension(state): Extension<Arc<AppState>>,
    Extension(claims): Extension<Claims>,
) -> Json<serde_json::Value> {
    if !claims.is_superuser && !claims.is_staff && !check_whitelisted(&state, &claims.email).await {
        return Json(serde_json::json!({"success": false, "error": "Access restricted. Email not whitelisted."}));
    }
    let user_id: i32 = match claims.sub.parse() {
        Ok(id) => id,
        Err(_) => return Json(serde_json::json!({"success": false, "error": "Invalid user ID in token"})),
    };
    let mut out = Vec::new();
    for (app, name) in REFERRAL_APPS {
        let row = sqlx::query(
            "INSERT INTO referral_codes (user_id, code, app_slug) \
             VALUES ($1, $2, $3) \
             ON CONFLICT (user_id, app_slug) WHERE app_slug IS NOT NULL \
             DO UPDATE SET code = referral_codes.code \
             RETURNING code",
        )
        .bind(user_id)
        .bind(format!("ref-{app}-{}", &Uuid::new_v4().to_string()[..6]))
        .bind(*app)
        .fetch_optional(&state.db_pool)
        .await
        .ok()
        .flatten();
        // Fallback: read existing (e.g. legacy row won the race — then create fresh below).
        let code: Option<String> = match row {
            Some(r) => Some(r.get("code")),
            None => sqlx::query_scalar::<_, String>(
                "SELECT code FROM referral_codes WHERE user_id = $1 AND app_slug = $2",
            )
            .bind(user_id)
            .bind(*app)
            .fetch_optional(&state.db_pool)
            .await
            .ok()
            .flatten(),
        };
        if let Some(code) = code {
            let host = referral_app_host(app).unwrap_or("https://videosync.ink");
            out.push(serde_json::json!({
                "app": app,
                "app_name": name,
                "code": code,
                "ref_url": ref_url_for(Some(app), &code),
                "landing_url": format!("{host}/?ref={code}"),
            }));
        }
    }
    Json(serde_json::json!({"success": true, "codes": out}))
}

/// GET /api/referrals/my-code
async fn api_get_my_referral_code(
    Extension(state): Extension<Arc<AppState>>,
    Extension(claims): Extension<Claims>,
) -> Json<serde_json::Value> {
    if !claims.is_superuser && !claims.is_staff && !check_whitelisted(&state, &claims.email).await {
        return Json(serde_json::json!({"success": false, "error": "Access restricted. Email not whitelisted."}));
    }

    let user_id: i32 = match claims.sub.parse() {
        Ok(id) => id,
        Err(_) => return Json(serde_json::json!({"success": false, "error": "Invalid user ID in token"})),
    };

    let existing = sqlx::query(
        "SELECT id, code, created_at FROM referral_codes WHERE user_id = $1 ORDER BY created_at DESC LIMIT 1",
    )
    .bind(user_id)
    .fetch_optional(&state.db_pool)
    .await;

    match existing {
        Ok(Some(row)) => {
            let id: Uuid = row.get("id");
            let code: String = row.get("code");
            let created_at: chrono::DateTime<chrono::Utc> = row.get("created_at");
            Json(serde_json::json!({
                "success": true,
                "code": {
                    "id": id,
                    "code": code,
                    "ref_url": format!("/ref/{code}"),
                    "created_at": created_at,
                }
            }))
        }
        Ok(None) => {
            Json(serde_json::json!({"success": true, "code": null}))
        }
        Err(e) => Json(serde_json::json!({"success": false, "error": format!("Database error: {e}")})),
    }
}

/// POST /api/referrals/my-code — Auto-generate a referral code for the current user
#[derive(Deserialize)]
struct CreateMyReferralCodeRequest {
    code: Option<String>,
    /// App slug (clips|shorts|app|learn|motion). Omitted = legacy all-link.
    app_slug: Option<String>,
}

async fn api_create_my_referral_code(
    Extension(state): Extension<Arc<AppState>>,
    Extension(claims): Extension<Claims>,
    Json(req): Json<CreateMyReferralCodeRequest>,
) -> Json<serde_json::Value> {
    if !claims.is_superuser && !claims.is_staff && !check_whitelisted(&state, &claims.email).await {
        return Json(serde_json::json!({"success": false, "error": "Access restricted. Email not whitelisted."}));
    }

    let user_id: i32 = match claims.sub.parse() {
        Ok(id) => id,
        Err(_) => return Json(serde_json::json!({"success": false, "error": "Invalid user ID in token"})),
    };

    let code = req.code.unwrap_or_else(|| {
        let suffix = &Uuid::new_v4().to_string()[..8];
        format!("ref-{suffix}")
    });
    let app_slug: Option<String> = req.app_slug.and_then(|a| {
        let a = a.to_ascii_lowercase();
        if referral_app_host(&a).is_some() {
            Some(a)
        } else {
            None
        }
    });

    let result = sqlx::query(
        "INSERT INTO referral_codes (user_id, code, app_slug) VALUES ($1, $2, $3) ON CONFLICT (code) DO NOTHING RETURNING id, code",
    )
    .bind(user_id)
    .bind(&code)
    .bind(&app_slug)
    .fetch_optional(&state.db_pool)
    .await;

    match result {
        Ok(Some(row)) => {
            let id: Uuid = row.get("id");
            let code: String = row.get("code");
            Json(serde_json::json!({
                "success": true,
                "code": {
                    "id": id,
                    "code": code,
                    "ref_url": ref_url_for(app_slug.as_deref(), &code),
                }
            }))
        }
        Ok(None) => Json(serde_json::json!({"success": false, "error": "Code already taken or create failed"})),
        Err(e) => Json(serde_json::json!({"success": false, "error": format!("Failed to create referral code: {e}")})),
    }
}

/// GET /api/referrals/my-commissions
async fn api_get_my_commissions(
    Extension(state): Extension<Arc<AppState>>,
    Extension(claims): Extension<Claims>,
) -> Json<serde_json::Value> {
    if !claims.is_superuser && !claims.is_staff && !check_whitelisted(&state, &claims.email).await {
        return Json(serde_json::json!({"success": false, "error": "Access restricted. Email not whitelisted."}));
    }

    let user_id: i32 = match claims.sub.parse() {
        Ok(id) => id,
        Err(_) => return Json(serde_json::json!({"success": false, "error": "Invalid user ID in token"})),
    };

    let rows = sqlx::query_as::<_, (Uuid, Uuid, i32, f64, String, Option<chrono::DateTime<chrono::Utc>>, chrono::DateTime<chrono::Utc>)>(
        "SELECT rc.id, rc.prospect_id, rc.deal_amount_cents, rc.commission_rate, rc.status, rc.paid_at, rc.created_at \
         FROM referral_commission rc \
         WHERE rc.referrer_user_id = $1 \
         ORDER BY rc.created_at DESC",
    )
    .bind(user_id)
    .fetch_all(&state.db_pool)
    .await;

    match rows {
        Ok(rows) => {
            let total_earned: i64 = rows.iter().map(|(_, _, deal, rate, status, _, _)| {
                if status == "paid" {
                    (*deal as f64 * rate) as i64
                } else {
                    0
                }
            }).sum();

            let commissions: Vec<serde_json::Value> = rows
                .into_iter()
                .map(|(id, prospect_id, deal_amount_cents, commission_rate, status, paid_at, created_at)| {
                    let commission_cents = (deal_amount_cents as f64 * commission_rate) as i64;
                    serde_json::json!({
                        "id": id,
                        "prospect_id": prospect_id,
                        "deal_amount_cents": deal_amount_cents,
                        "commission_rate": commission_rate,
                        "commission_cents": commission_cents,
                        "status": status,
                        "paid_at": paid_at,
                        "created_at": created_at,
                    })
                })
                .collect();

            Json(serde_json::json!({
                "success": true,
                "commissions": commissions,
                "total_earned_cents": total_earned,
            }))
        }
        Err(e) => Json(serde_json::json!({"success": false, "error": format!("Failed to list commissions: {e}")})),
    }
}
