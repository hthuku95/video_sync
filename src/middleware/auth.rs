use crate::handlers::auth::verify_jwt_token;
use crate::models::auth::{Claims, ErrorResponse};
use axum::{
    extract::Request,
    http::{HeaderMap, StatusCode},
    middleware::Next,
    response::{IntoResponse, Json, Response},
};

pub async fn auth_middleware(
    headers: HeaderMap,
    mut request: Request,
    next: Next,
) -> Result<Response, impl IntoResponse> {
    // Extract the Authorization header
    let auth_header = match headers.get("Authorization") {
        Some(header) => header,
        None => {
            return Err((
                StatusCode::UNAUTHORIZED,
                Json(ErrorResponse {
                    success: false,
                    message: "Missing Authorization header".to_string(),
                }),
            ));
        }
    };

    // Convert header to string
    let auth_str = match auth_header.to_str() {
        Ok(str) => str,
        Err(_) => {
            return Err((
                StatusCode::UNAUTHORIZED,
                Json(ErrorResponse {
                    success: false,
                    message: "Invalid Authorization header format".to_string(),
                }),
            ));
        }
    };

    // Extract token from "Bearer <token>" format
    let token = if auth_str.starts_with("Bearer ") {
        &auth_str[7..]
    } else {
        return Err((
            StatusCode::UNAUTHORIZED,
            Json(ErrorResponse {
                success: false,
                message: "Invalid Authorization header format. Expected 'Bearer <token>'".to_string(),
            }),
        ));
    };

    // Verify the JWT token
    let claims = match verify_jwt_token(token) {
        Ok(claims) => claims,
        Err(e) => {
            tracing::warn!("JWT verification failed: {}", e);
            return Err((
                StatusCode::UNAUTHORIZED,
                Json(ErrorResponse {
                    success: false,
                    message: "Invalid or expired token".to_string(),
                }),
            ));
        }
    };

    // Add the claims to the request extensions so handlers can access them
    request.extensions_mut().insert(claims);

    // Continue to the next handler
    Ok(next.run(request).await)
}

// Extension trait to easily extract claims from request extensions
pub trait ClaimsExtractor {
    fn claims(&self) -> Option<&Claims>;
}

impl ClaimsExtractor for Request {
    fn claims(&self) -> Option<&Claims> {
        self.extensions().get::<Claims>()
    }
}