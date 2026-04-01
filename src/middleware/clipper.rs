use crate::models::auth::{Claims, ErrorResponse};
use axum::{
    extract::Request,
    http::StatusCode,
    middleware::Next,
    response::{IntoResponse, Json, Response},
};

/// Allows clippers, staff, and superusers. Requires auth_middleware to run first.
pub async fn clipper_middleware(
    request: Request,
    next: Next,
) -> Result<Response, impl IntoResponse> {
    let claims = request.extensions().get::<Claims>();

    match claims {
        Some(claims) => {
            if claims.is_clipper || claims.is_staff || claims.is_superuser {
                Ok(next.run(request).await)
            } else {
                Err((
                    StatusCode::FORBIDDEN,
                    Json(ErrorResponse {
                        success: false,
                        message: "Clipper access required.".to_string(),
                    }),
                ))
            }
        }
        None => Err((
            StatusCode::UNAUTHORIZED,
            Json(ErrorResponse {
                success: false,
                message: "Authentication required.".to_string(),
            }),
        )),
    }
}
