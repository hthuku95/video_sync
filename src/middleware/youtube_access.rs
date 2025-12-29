use crate::models::auth::Claims;
use crate::models::admin::SystemSetting;
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
                }))
            ));
        }
    };

    // Admins always have access (staff or superuser)
    if claims.is_staff || claims.is_superuser {
        return Ok(next.run(request).await);
    }

    // Check if YouTube features are enabled globally
    let setting = sqlx::query_as::<_, SystemSetting>(
        "SELECT * FROM system_settings WHERE setting_key = 'youtube_features_enabled'"
    )
    .fetch_optional(&state.db_pool)
    .await
    .map_err(|e| {
        tracing::error!("Failed to check YouTube feature setting: {}", e);
        (
            StatusCode::INTERNAL_SERVER_ERROR,
            Json(json!({
                "success": false,
                "message": "Failed to check feature availability"
            }))
        )
    })?;

    let features_enabled = setting
        .map(|s| s.as_bool().unwrap_or(false))
        .unwrap_or(false);

    if features_enabled {
        // Feature is enabled for everyone
        return Ok(next.run(request).await);
    }

    // Feature is disabled - check whitelist
    let is_whitelisted = sqlx::query_scalar::<_, bool>(
        "SELECT EXISTS(SELECT 1 FROM whitelist_emails WHERE email = $1)"
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
            }))
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
            }))
        ))
    }
}
