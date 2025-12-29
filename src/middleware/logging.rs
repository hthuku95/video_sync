use axum::{
    extract::{Request, MatchedPath},
    http::StatusCode,
    middleware::Next,
    response::Response,
};
use std::time::Instant;
use uuid::Uuid;

/// Request logging middleware that adds structured logging for all HTTP requests
pub async fn request_logging_middleware(
    req: Request,
    next: Next,
) -> Result<Response, StatusCode> {
    let start = Instant::now();
    let request_id = Uuid::new_v4().to_string();
    
    // Extract request information before moving req
    let method = req.method().clone();
    let uri = req.uri().clone();
    let matched_path = req.extensions().get::<MatchedPath>()
        .map(|mp| mp.as_str().to_owned())
        .unwrap_or_else(|| "unknown".to_owned());
    let user_agent = req.headers()
        .get("user-agent")
        .and_then(|v| v.to_str().ok())
        .unwrap_or("unknown")
        .to_owned();
    let remote_addr = req.extensions()
        .get::<axum::extract::ConnectInfo<std::net::SocketAddr>>()
        .map(|ci| ci.0.to_string())
        .unwrap_or_else(|| "unknown".to_string());
    
    // Log the incoming request
    tracing::info!(
        request_id = %request_id,
        method = %method,
        path = %matched_path,
        uri = %uri,
        user_agent = %user_agent,
        remote_addr = %remote_addr,
        "incoming request"
    );
    
    // Process the request
    let response = next.run(req).await;
    
    // Calculate response time
    let duration = start.elapsed();
    let status = response.status();
    
    // Log the response based on status code
    match status.as_u16() {
        200..=299 => {
            tracing::info!(
                request_id = %request_id,
                method = %method,
                path = %matched_path,
                uri = %uri,
                status = %status.as_u16(),
                duration_ms = %duration.as_millis(),
                "request completed"
            );
        }
        300..=399 => {
            tracing::info!(
                request_id = %request_id,
                method = %method,
                path = %matched_path,
                uri = %uri,
                status = %status.as_u16(),
                duration_ms = %duration.as_millis(),
                "request completed (redirect)"
            );
        }
        400..=499 => {
            tracing::warn!(
                request_id = %request_id,
                method = %method,
                path = %matched_path,
                uri = %uri,
                status = %status.as_u16(),
                duration_ms = %duration.as_millis(),
                "request completed (client error)"
            );
        }
        500..=599 => {
            tracing::error!(
                request_id = %request_id,
                method = %method,
                path = %matched_path,
                uri = %uri,
                status = %status.as_u16(),
                duration_ms = %duration.as_millis(),
                "request completed (server error)"
            );
        }
        _ => {
            tracing::info!(
                request_id = %request_id,
                method = %method,
                path = %matched_path,
                uri = %uri,
                status = %status.as_u16(),
                duration_ms = %duration.as_millis(),
                "request completed (unknown status)"
            );
        }
    }
    
    Ok(response)
}

/// Global error handler that provides structured error logging
pub async fn global_error_handler(
    err: Box<dyn std::error::Error + Send + Sync>,
) -> (StatusCode, String) {
    let error_id = Uuid::new_v4();
    
    tracing::error!(
        error_id = %error_id,
        error = %err,
        "unhandled error occurred"
    );
    
    // In production, don't expose internal error details
    let message = if cfg!(debug_assertions) {
        format!("Internal server error: {} (ID: {})", err, error_id)
    } else {
        format!("Internal server error (ID: {})", error_id)
    };
    
    (StatusCode::INTERNAL_SERVER_ERROR, message)
}