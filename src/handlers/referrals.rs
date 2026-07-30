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
        .route("/api/referrals/my-commissions", get(api_get_my_commissions))
        .layer(axum::middleware::from_fn(auth_middleware))
}

/// GET /api/referrals/my-code
async fn api_get_my_referral_code(
    Extension(state): Extension<Arc<AppState>>,
    Extension(claims): Extension<Claims>,
) -> Json<serde_json::Value> {
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
}

async fn api_create_my_referral_code(
    Extension(state): Extension<Arc<AppState>>,
    Extension(claims): Extension<Claims>,
    Json(req): Json<CreateMyReferralCodeRequest>,
) -> Json<serde_json::Value> {
    let user_id: i32 = match claims.sub.parse() {
        Ok(id) => id,
        Err(_) => return Json(serde_json::json!({"success": false, "error": "Invalid user ID in token"})),
    };

    let code = req.code.unwrap_or_else(|| {
        let suffix = &Uuid::new_v4().to_string()[..8];
        format!("ref-{suffix}")
    });

    let result = sqlx::query(
        "INSERT INTO referral_codes (user_id, code) VALUES ($1, $2) ON CONFLICT (code) DO NOTHING RETURNING id, code",
    )
    .bind(user_id)
    .bind(&code)
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
                    "ref_url": format!("/ref/{code}"),
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
