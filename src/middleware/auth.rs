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
    // Try to extract token from Authorization header first
    let token = if let Some(auth_header) = headers.get("Authorization") {
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
        if auth_str.starts_with("Bearer ") {
            auth_str[7..].to_string()
        } else {
            return Err((
                StatusCode::UNAUTHORIZED,
                Json(ErrorResponse {
                    success: false,
                    message: "Invalid Authorization header format. Expected 'Bearer <token>'".to_string(),
                }),
            ));
        }
    } else {
        // Fallback: Try to extract token from query parameter (for OAuth redirects)
        let query = request.uri().query().unwrap_or("");
        let mut token_from_query = None;

        for pair in query.split('&') {
            if let Some((key, value)) = pair.split_once('=') {
                if key == "token" {
                    token_from_query = Some(value.to_string());
                    break;
                }
            }
        }

        match token_from_query {
            Some(t) => t,
            None => {
                return Err((
                    StatusCode::UNAUTHORIZED,
                    Json(ErrorResponse {
                        success: false,
                        message: "Missing Authorization header or token query parameter".to_string(),
                    }),
                ));
            }
        }
    };

    // Verify the JWT token
    let claims = match verify_jwt_token(&token) {
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