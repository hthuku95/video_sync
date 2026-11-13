use crate::models::auth::Claims;
use crate::AppState;
use axum::{
    extract::{Extension, Request},
    http::StatusCode,
    middleware::Next,
    response::{IntoResponse, Json, Response},
};
use serde_json::json;
use std::sync::Arc;

pub async fn youtube_access_middleware(
    Extension(state): Extension<Arc<AppState>>,
    request: Request,
    next: Next,
) -> Result<Response, impl IntoResponse> {
    // Get user claims (set by auth_middleware)
    let claims = request.extensions().get::<Claims>();

    let claims = match claims {
        Some(c) => c,
        None => {
            return Err((
                StatusCode::UNAUTHORIZED,
                Json(json!({
                    "success": false,
                    "message": "Authentication required"
                })),
            ));
        }
    };

    // Admins always have access (staff or superuser)
    if claims.is_staff || claims.is_superuser {
        return Ok(next.run(request).await);
    }

    // Whitelist-only (owner directive Nov 13 2026): the youtube_features_enabled
    // toggle bypass was REMOVED — this system is not customer-facing. Access is
    // admins/staff and whitelisted emails, ALWAYS.
    let is_whitelisted = sqlx::query_scalar::<_, bool>(
        "SELECT EXISTS(SELECT 1 FROM whitelist_emails WHERE email = $1)",
    )
    .bind(&claims.email)
    .fetch_one(&state.db_pool)
    .await
    .map_err(|e| {
        tracing::error!("Failed to check whitelist: {}", e);
        (
            StatusCode::INTERNAL_SERVER_ERROR,
            Json(json!({
                "success": false,
                "message": "Failed to verify access"
            })),
        )
    })?;

    if is_whitelisted {
        Ok(next.run(request).await)
    } else {
        Err((
            StatusCode::FORBIDDEN,
            Json(json!({
                "success": false,
                "message": "YouTube features are currently in testing mode. Contact your administrator for access.",
                "feature": "youtube_integration",
                "requires_admin": true,
                "coming_soon_url": "/youtube/coming-soon"
            })),
        ))
    }
}
