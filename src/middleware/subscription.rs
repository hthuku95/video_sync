//! Regular-user subscription gate.
//!
//! Runs AFTER `auth_middleware` so `Claims` is in the request extensions.
//! Reads the caller's subscription row and decides whether the request
//! is allowed through:
//!
//! * `grandfathered`                             → always allow (team / pre-paywall signups).
//! * `trial` AND trial_ends_at > NOW()           → allow.
//! * `active` AND subscription_active_until > NOW() → allow.
//! * Admins / staff / whitelisted clippers       → always allow.
//! * Otherwise                                   → HTTP 402 with upgrade_url.
//!
//! When a trial's timestamp has passed we flip the status to `expired`
//! inline so the admin dashboard sees it without a separate sweeper.

use crate::models::auth::Claims;
use crate::AppState;
use axum::{
    extract::Request,
    http::StatusCode,
    middleware::Next,
    response::{IntoResponse, Json, Response},
};
use serde_json::json;
use sqlx::Row;
use std::sync::Arc;

pub async fn subscription_middleware(
    request: Request,
    next: Next,
) -> Result<Response, impl IntoResponse> {
    // Pull Claims (JWT already verified upstream by auth_middleware) and
    // AppState out of request extensions. If either is missing the request
    // is malformed — bail out rather than crash.
    let claims = match request.extensions().get::<Claims>() {
        Some(c) => c.clone(),
        None => {
            return Err((
                StatusCode::UNAUTHORIZED,
                Json(json!({"success": false, "error": "auth_required"})),
            ))
        }
    };

    let state = match request.extensions().get::<Arc<AppState>>() {
        Some(s) => s.clone(),
        None => {
            return Err((
                StatusCode::INTERNAL_SERVER_ERROR,
                Json(json!({"success": false, "error": "app_state_missing"})),
            ))
        }
    };

    // Staff + superusers bypass the paywall entirely — they're running ops.
    if claims.is_superuser || claims.is_staff {
        return Ok(next.run(request).await);
    }

    let user_id: i32 = match claims.sub.parse() {
        Ok(id) => id,
        Err(_) => {
            return Err((
                StatusCode::UNAUTHORIZED,
                Json(json!({"success": false, "error": "invalid_user_id"})),
            ))
        }
    };

    let row = match sqlx::query(
        "SELECT subscription_status, trial_ends_at, subscription_active_until
         FROM users WHERE id = $1",
    )
    .bind(user_id)
    .fetch_optional(&state.db_pool)
    .await
    {
        Ok(Some(r)) => r,
        Ok(None) => {
            return Err((
                StatusCode::UNAUTHORIZED,
                Json(json!({"success": false, "error": "user_not_found"})),
            ))
        }
        Err(e) => {
            tracing::warn!("subscription_middleware DB error: {}", e);
            return Err((
                StatusCode::INTERNAL_SERVER_ERROR,
                Json(json!({"success": false, "error": "subscription_lookup_failed"})),
            ));
        }
    };

    // NULL status → grandfathered (the migration backfills NULLs, but a
    // race with a fresh signup before the register handler runs is safe
    // to treat as grandfathered).
    let status: String = row
        .try_get::<Option<String>, _>("subscription_status")
        .ok()
        .flatten()
        .unwrap_or_else(|| "grandfathered".to_string());

    let now = chrono::Utc::now();

    match status.as_str() {
        "grandfathered" => Ok(next.run(request).await),

        "trial" => {
            let trial_end = row
                .try_get::<Option<chrono::DateTime<chrono::Utc>>, _>("trial_ends_at")
                .ok()
                .flatten();
            match trial_end {
                Some(t) if t > now => Ok(next.run(request).await),
                _ => {
                    // Trial expired — flip to `expired` so admin UI and
                    // future requests see the correct status without
                    // running an overnight sweeper.
                    let _ = sqlx::query(
                        "UPDATE users SET subscription_status = 'expired'
                         WHERE id = $1 AND subscription_status = 'trial'",
                    )
                    .bind(user_id)
                    .execute(&state.db_pool)
                    .await;
                    let _ = sqlx::query(
                        "INSERT INTO user_payment_events (user_id, event_type)
                         VALUES ($1, 'expired')",
                    )
                    .bind(user_id)
                    .execute(&state.db_pool)
                    .await;
                    Err(payment_required())
                }
            }
        }

        "active" => {
            let until = row
                .try_get::<Option<chrono::DateTime<chrono::Utc>>, _>("subscription_active_until")
                .ok()
                .flatten();
            match until {
                Some(t) if t > now => Ok(next.run(request).await),
                _ => {
                    // Subscription lapsed — demote to expired.
                    let _ = sqlx::query(
                        "UPDATE users SET subscription_status = 'expired'
                         WHERE id = $1 AND subscription_status = 'active'",
                    )
                    .bind(user_id)
                    .execute(&state.db_pool)
                    .await;
                    let _ = sqlx::query(
                        "INSERT INTO user_payment_events (user_id, event_type)
                         VALUES ($1, 'expired')",
                    )
                    .bind(user_id)
                    .execute(&state.db_pool)
                    .await;
                    Err(payment_required())
                }
            }
        }

        // 'expired' | 'cancelled' | anything else → block.
        _ => Err(payment_required()),
    }
}

fn payment_required() -> (StatusCode, Json<serde_json::Value>) {
    (
        StatusCode::PAYMENT_REQUIRED,
        Json(json!({
            "success":      false,
            "error":        "subscription_required",
            "message":      "Your free trial has ended. Subscribe for $15/mo USDC to continue.",
            "upgrade_url":  "/subscribe",
        })),
    )
}
