use axum::{
    extract::{ConnectInfo, Request},
    http::StatusCode,
    middleware::Next,
    response::{IntoResponse, Json, Response},
};
use serde_json::json;
use std::collections::HashMap;
use std::net::SocketAddr;
use std::sync::{Arc, Mutex};
use std::time::{Duration, Instant};

#[derive(Clone)]
pub struct RateLimiter {
    // Store IP -> (request_count, window_start)
    clients: Arc<Mutex<HashMap<String, (u32, Instant)>>>,
    max_requests: u32,
    window_duration: Duration,
}

impl RateLimiter {
    pub fn new(max_requests: u32, window_seconds: u64) -> Self {
        Self {
            clients: Arc::new(Mutex::new(HashMap::new())),
            max_requests,
            window_duration: Duration::from_secs(window_seconds),
        }
    }

    pub fn check_rate_limit(&self, client_ip: &str) -> bool {
        let mut clients = self.clients.lock().unwrap();
        let now = Instant::now();

        match clients.get_mut(client_ip) {
            Some((count, window_start)) => {
                // Check if window has expired
                if now.duration_since(*window_start) > self.window_duration {
                    *count = 1;
                    *window_start = now;
                    true
                } else if *count >= self.max_requests {
                    false
                } else {
                    *count += 1;
                    true
                }
            }
            None => {
                clients.insert(client_ip.to_string(), (1, now));
                true
            }
        }
    }

    // Clean up old entries periodically
    pub fn cleanup_expired(&self) {
        let mut clients = self.clients.lock().unwrap();
        let now = Instant::now();
        
        clients.retain(|_, (_, window_start)| {
            now.duration_since(*window_start) <= self.window_duration
        });
    }
}

pub async fn rate_limit_middleware(
    ConnectInfo(addr): ConnectInfo<SocketAddr>,
    request: Request,
    next: Next,
) -> Result<Response, impl IntoResponse> {
    // Create a basic rate limiter - 100 requests per minute per IP
    static RATE_LIMITER: std::sync::OnceLock<RateLimiter> = std::sync::OnceLock::new();
    let rate_limiter = RATE_LIMITER.get_or_init(|| RateLimiter::new(100, 60));

    let client_ip = addr.ip().to_string();

    if !rate_limiter.check_rate_limit(&client_ip) {
        tracing::warn!("Rate limit exceeded for IP: {}", client_ip);
        return Err((
            StatusCode::TOO_MANY_REQUESTS,
            Json(json!({
                "success": false,
                "message": "Rate limit exceeded. Please try again later.",
                "retry_after": 60
            })),
        ));
    }

    // Occasionally clean up expired entries
    if rand::random::<u8>() < 10 {
        rate_limiter.cleanup_expired();
    }

    Ok(next.run(request).await)
}

// More aggressive rate limiting for sensitive endpoints
pub async fn strict_rate_limit_middleware(
    ConnectInfo(addr): ConnectInfo<SocketAddr>,
    request: Request,
    next: Next,
) -> Result<Response, impl IntoResponse> {
    // Stricter rate limiter - 10 requests per minute per IP
    static STRICT_RATE_LIMITER: std::sync::OnceLock<RateLimiter> = std::sync::OnceLock::new();
    let rate_limiter = STRICT_RATE_LIMITER.get_or_init(|| RateLimiter::new(10, 60));

    let client_ip = addr.ip().to_string();

    if !rate_limiter.check_rate_limit(&client_ip) {
        tracing::warn!("Strict rate limit exceeded for IP: {}", client_ip);
        return Err((
            StatusCode::TOO_MANY_REQUESTS,
            Json(json!({
                "success": false,
                "message": "Rate limit exceeded for sensitive operations. Please try again later.",
                "retry_after": 60
            })),
        ));
    }

    Ok(next.run(request).await)
}