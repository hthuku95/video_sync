use crate::handlers::auth::verify_jwt_token;
use crate::models::auth::ErrorResponse;
use axum::{
    extract::Request,
    http::{HeaderMap, header},
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
                return Err(json_or_redirect(&headers, "Invalid Authorization header format"));
            }
        };

        if auth_str.starts_with("Bearer ") {
            auth_str[7..].to_string()
        } else {
            return Err(json_or_redirect(
                &headers,
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
                    "Missing Authorization header or token query parameter",
                ));
            }
        }
    };

    let claims = match verify_jwt_token(&token) {
        Ok(claims) => claims,
        Err(e) => {
            tracing::warn!("JWT verification failed: {}", e);
            return Err(json_or_redirect(&headers, "Invalid or expired token"));
        }
    };

    request.extensions_mut().insert(claims);
    Ok(next.run(request).await)
}

fn is_browser_request(headers: &HeaderMap) -> bool {
    headers
        .get(header::ACCEPT)
        .and_then(|v| v.to_str().ok())
        .map(|v| v.contains("text/html"))
        .unwrap_or(false)
}

fn json_or_redirect(
    headers: &HeaderMap,
    message: &str,
) -> axum::response::Response {
    if is_browser_request(headers) {
        Redirect::to("/login").into_response()
    } else {
        Json(ErrorResponse {
            success: false,
            message: message.to_string(),
        }).into_response()
    }
}
