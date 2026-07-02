use crate::models::auth::Claims;
use crate::AppState;
use axum::{
    extract::Request,
    http::{StatusCode, header},
    middleware::Next,
    response::{Html, IntoResponse, Json, Response},
};
use serde_json::json;
use sqlx::Row;
use std::sync::Arc;

pub async fn subscription_middleware(
    request: Request,
    next: Next,
) -> Result<Response, impl IntoResponse> {
    let claims = match request.extensions().get::<Claims>() {
        Some(c) => c.clone(),
        None => {
            return Err(payment_required(&request));
        }
    };

    let state = match request.extensions().get::<Arc<AppState>>() {
        Some(s) => s.clone(),
        None => {
            return Err(json_error(
                StatusCode::INTERNAL_SERVER_ERROR,
                json!({"success": false, "error": "app_state_missing"}),
            ));
        }
    };

    if claims.is_superuser || claims.is_staff {
        return Ok(next.run(request).await);
    }

    let user_id: i32 = match claims.sub.parse() {
        Ok(id) => id,
        Err(_) => {
            return Err(json_error(
                StatusCode::UNAUTHORIZED,
                json!({"success": false, "error": "invalid_user_id"}),
            ));
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
            return Err(json_error(
                StatusCode::UNAUTHORIZED,
                json!({"success": false, "error": "user_not_found"}),
            ));
        }
        Err(e) => {
            tracing::warn!("subscription_middleware DB error: {}", e);
            return Err(json_error(
                StatusCode::INTERNAL_SERVER_ERROR,
                json!({"success": false, "error": "subscription_lookup_failed"}),
            ));
        }
    };

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
                    Err(payment_required(&request))
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
                    Err(payment_required(&request))
                }
            }
        }

        _ => Err(payment_required(&request)),
    }
}

fn is_browser_request(request: &Request) -> bool {
    request
        .headers()
        .get(header::ACCEPT)
        .and_then(|v| v.to_str().ok())
        .map(|v| v.contains("text/html"))
        .unwrap_or(false)
}

fn json_error(status: StatusCode, body: serde_json::Value) -> Response {
    (status, Json(body)).into_response()
}

fn payment_required(request: &Request) -> Response {
    if is_browser_request(request) {
        (
            StatusCode::FOUND,
            [(header::LOCATION, "/subscribe")],
            Html(""),
        )
            .into_response()
    } else {
        (
            StatusCode::PAYMENT_REQUIRED,
            Json(json!({
                "success":      false,
                "error":        "subscription_required",
                "message":      "Your free trial has ended. Subscribe for $15/mo USDC to continue.",
                "upgrade_url":  "/subscribe",
            })),
        )
            .into_response()
    }
}
