use crate::handlers::auth::verify_jwt_token;
use crate::models::auth::ErrorResponse;
use axum::{
    extract::Request,
    http::{HeaderMap, StatusCode, header},
    middleware::Next,
    response::{IntoResponse, Json, Redirect, Response},
};

pub async fn auth_middleware(
    headers: HeaderMap,
    mut request: Request,
    next: Next,
) -> Result<Response, impl IntoResponse> {
    let token = if let Some(auth_header) = headers.get("Authorization") {
        let auth_str = match auth_header.to_str() {
            Ok(str) => str,
            Err(_) => {
                return Err(json_or_redirect(
                    &headers,
                    StatusCode::UNAUTHORIZED,
                    "Invalid Authorization header format",
                ));
            }
        };

        if auth_str.starts_with("Bearer ") {
            auth_str[7..].to_string()
        } else {
            return Err(json_or_redirect(
                &headers,
                StatusCode::UNAUTHORIZED,
                "Invalid Authorization header format. Expected 'Bearer <token>'",
            ));
        }
    } else {
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
                return Err(json_or_redirect(
                    &headers,
                    StatusCode::UNAUTHORIZED,
                    "Missing Authorization header or token query parameter",
                ));
            }
        }
    };

    let claims = match verify_jwt_token(&token) {
        Ok(claims) => claims,
        Err(e) => {
            tracing::warn!("JWT verification failed: {}", e);
            return Err(json_or_redirect(
                &headers,
                StatusCode::UNAUTHORIZED,
                "Invalid or expired token",
            ));
        }
    };

    request.extensions_mut().insert(claims);
    Ok(next.run(request).await)
}

fn json_or_redirect(
    headers: &HeaderMap,
    status: StatusCode,
    message: &str,
) -> (StatusCode, axum::response::Response) {
    let accepts_html = headers
        .get(header::ACCEPT)
        .and_then(|v| v.to_str().ok())
        .map(|v| v.contains("text/html"))
        .unwrap_or(false);

    if accepts_html {
        let target = if message.contains("expired") || message.contains("Invalid") {
            "/login"
        } else {
            "/login"
        };
        (status, Redirect::to(target).into_response())
    } else {
        (status, Json(ErrorResponse {
            success: false,
            message: message.to_string(),
        }).into_response())
    }
}
