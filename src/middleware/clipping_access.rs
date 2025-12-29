// Access control middleware for YouTube Clipping feature
// CRITICAL: Only whitelisted users and admins can access clipping
// This is independent of the youtube_features_enabled toggle

use axum::{
    extract::Request,
    http::StatusCode,
    middleware::Next,
    response::{IntoResponse, Response},
    Extension, Json,
};
use crate::models::auth::Claims;
use serde_json::json;
use sqlx::PgPool;
use std::sync::Arc;

/// Middleware to check if user has access to clipping features
/// Access granted to: is_staff, is_superuser, OR whitelisted users
/// Regular users are ALWAYS denied regardless of YouTube feature toggle
pub async fn clipping_access_middleware(
    Extension(pool): Extension<Arc<PgPool>>,
    mut request: Request,
    next: Next,
) -> Result<Response, impl IntoResponse> {
    // Get user claims from auth middleware
    let claims = match request.extensions().get::<Claims>().cloned() {
        Some(claims) => claims,
        None => {
            return Err((
                StatusCode::UNAUTHORIZED,
                Json(json!({
                    "error": "Authentication required",
                    "message": "Please log in to access YouTube Clipping features"
                })),
            ));
        }
    };

    // Admins and staff ALWAYS have access
    if claims.is_staff || claims.is_superuser {
        tracing::debug!(
            "Clipping access granted to admin user: {}",
            claims.username
        );
        request.extensions_mut().insert(claims);
        return Ok(next.run(request).await);
    }

    // Check if user is whitelisted
    let is_whitelisted = sqlx::query_scalar::<_, bool>(
        "SELECT EXISTS(SELECT 1 FROM whitelist_emails WHERE email = $1)",
    )
    .bind(&claims.email)
    .fetch_one(pool.as_ref())
    .await
    .unwrap_or(false);

    if is_whitelisted {
        tracing::debug!(
            "Clipping access granted to whitelisted user: {}",
            claims.username
        );
        request.extensions_mut().insert(claims);
        Ok(next.run(request).await)
    } else {
        tracing::warn!(
            "Clipping access denied for non-whitelisted user: {}",
            claims.username
        );
        Err((
            StatusCode::FORBIDDEN,
            Json(json!({
                "error": "Access Denied",
                "message": "YouTube Clipping is currently in beta and only available to whitelisted users. Contact support for early access.",
                "feature": "youtube_clipping",
                "requires_whitelist": true,
                "user_email": claims.email
            })),
        ))
    }
}
