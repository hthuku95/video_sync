use crate::handlers::auth::verify_jwt_token;
use crate::models::auth::{Claims, ErrorResponse};
use axum::{
    extract::Request,
    http::{HeaderMap, StatusCode, header},
    middleware::Next,
    response::{Html, IntoResponse, Json, Response},
};

/// Extracts JWT claims from a request without requiring authentication.
/// Returns (Option<Claims>, Option<Response>) — if the response is Some,
/// the caller should return it immediately (error case).
fn extract_jwt_claims(headers: &HeaderMap, uri: &axum::http::Uri) -> (Option<Claims>, Option<Response>) {
    let token = if let Some(auth_header) = headers.get("Authorization") {
        let auth_str = match auth_header.to_str() {
            Ok(str) => str,
            Err(_) => return (None, Some(unauthenticated(headers))),
        };
        if auth_str.starts_with("Bearer ") {
            auth_str[7..].to_string()
        } else {
            return (None, Some(unauthenticated(headers)));
        }
    } else if let Some(token) = headers.get("Cookie")
        .and_then(|c| c.to_str().ok())
        .and_then(|c| {
            c.split(';').find_map(|pair| {
                let p = pair.trim();
                if let Some(value) = p.strip_prefix("token=") {
                    Some(value.to_string())
                } else {
                    None
                }
            })
        })
    {
        token
    } else {
        let query = uri.query().unwrap_or("");
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
            None => return (None, None), // No token found — not an error, just unauthenticated
        }
    };

    match verify_jwt_token(&token) {
        Ok(claims) => (Some(claims), None),
        Err(e) => {
            tracing::warn!("JWT verification failed: {}", e);
            (None, Some(unauthenticated(headers)))
        }
    }
}

pub async fn auth_middleware(
    headers: HeaderMap,
    mut request: Request,
    next: Next,
) -> Result<Response, impl IntoResponse> {
    let (claims, err_response) = extract_jwt_claims(&headers, request.uri());
    if let Some(err) = err_response {
        return Err(err);
    }
    match claims {
        Some(c) => {
            request.extensions_mut().insert(c);
            Ok(next.run(request).await)
        }
        None => Err(unauthenticated(&headers)),
    }
}

/// Like auth_middleware but does NOT redirect or error when no token is present.
/// If a valid JWT is found, Claims are injected into request extensions.
/// If not, the request passes through without Claims — handlers use Option<Extension<Claims>>.
pub async fn optional_auth_middleware(
    headers: HeaderMap,
    mut request: Request,
    next: Next,
) -> Response {
    let (claims, _err_response) = extract_jwt_claims(&headers, request.uri());
    if let Some(c) = claims {
        request.extensions_mut().insert(c);
    }
    next.run(request).await
}

fn is_browser_request(headers: &HeaderMap) -> bool {
    headers
        .get(header::ACCEPT)
        .and_then(|v| v.to_str().ok())
        .map(|v| v.contains("text/html"))
        .unwrap_or(false)
}

fn unauthenticated(headers: &HeaderMap) -> Response {
    if is_browser_request(headers) {
        (
            StatusCode::FOUND,
            [(header::LOCATION, "/login")],
            Html(""),
        )
            .into_response()
    } else {
        (
            StatusCode::UNAUTHORIZED,
            [(header::CONTENT_TYPE, "application/json")],
            Json(ErrorResponse {
                success: false,
                message: "Missing Authorization header or token query parameter".to_string(),
            }),
        )
            .into_response()
    }
}
