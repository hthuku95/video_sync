use crate::models::auth::{Claims, ErrorResponse};
use axum::{
    extract::Request,
    http::StatusCode,
    middleware::Next,
    response::{IntoResponse, Json, Response},
};

pub async fn admin_middleware(
    request: Request,
    next: Next,
) -> Result<Response, impl IntoResponse> {
    // Get the claims from request extensions (set by auth middleware)
    let claims = request.extensions().get::<Claims>();
    
    match claims {
        Some(claims) => {
            if claims.is_superuser || claims.is_staff {
                Ok(next.run(request).await)
            } else {
                Err((
                    StatusCode::FORBIDDEN,
                    Json(ErrorResponse {
                        success: false,
                        message: "Admin access required. You must be staff or superuser.".to_string(),
                    }),
                ))
            }
        }
        None => {
            Err((
                StatusCode::UNAUTHORIZED,
                Json(ErrorResponse {
                    success: false,
                    message: "Authentication required for admin access.".to_string(),
                }),
            ))
        }
    }
}

pub async fn superuser_middleware(
    request: Request,
    next: Next,
) -> Result<Response, impl IntoResponse> {
    // Get the claims from request extensions (set by auth middleware)
    let claims = request.extensions().get::<Claims>();
    
    match claims {
        Some(claims) => {
            if claims.is_superuser {
                Ok(next.run(request).await)
            } else {
                Err((
                    StatusCode::FORBIDDEN,
                    Json(ErrorResponse {
                        success: false,
                        message: "Superuser access required.".to_string(),
                    }),
                ))
            }
        }
        None => {
            Err((
                StatusCode::UNAUTHORIZED,
                Json(ErrorResponse {
                    success: false,
                    message: "Authentication required for superuser access.".to_string(),
                }),
            ))
        }
    }
}