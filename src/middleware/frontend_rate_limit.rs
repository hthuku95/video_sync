use axum::{
    extract::{ConnectInfo, Request},
    http::{StatusCode, Uri},
    middleware::Next,
    response::{IntoResponse, Json, Response},
};
use serde_json::json;
use std::collections::HashMap;
use std::net::SocketAddr;
use std::sync::{Arc, Mutex};
use std::time::{Duration, Instant};

#[derive(Clone)]
pub struct FrontendRateLimiter {
    // Store IP -> (operation_type -> (request_count, window_start))
    clients: Arc<Mutex<HashMap<String, HashMap<String, (u32, Instant)>>>>,
    limits: HashMap<String, (u32, Duration)>, // operation_type -> (max_requests, window_duration)
}

impl FrontendRateLimiter {
    pub fn new() -> Self {
        let mut limits = HashMap::new();
        
        // Define different rate limits for different frontend operations
        // These are more generous than API limits but still protect against abuse
        
        // Chat operations (most token-intensive)
        limits.insert("chat".to_string(), (30, Duration::from_secs(60))); // 30 per minute
        limits.insert("websocket".to_string(), (100, Duration::from_secs(60))); // 100 messages per minute
        
        // File operations
        limits.insert("upload".to_string(), (20, Duration::from_secs(300))); // 20 uploads per 5 minutes
        limits.insert("download".to_string(), (50, Duration::from_secs(60))); // 50 downloads per minute
        
        // UI operations (most lenient)
        limits.insert("ui".to_string(), (200, Duration::from_secs(60))); // 200 page loads per minute
        limits.insert("api".to_string(), (150, Duration::from_secs(60))); // 150 API calls per minute
        
        // Authentication (strict but reasonable)
        limits.insert("auth".to_string(), (5, Duration::from_secs(60))); // 5 auth attempts per minute
        
        // Admin operations (very strict)
        limits.insert("admin".to_string(), (20, Duration::from_secs(60))); // 20 admin operations per minute
        
        Self {
            clients: Arc::new(Mutex::new(HashMap::new())),
            limits,
        }
    }

    pub fn check_rate_limit(&self, client_ip: &str, operation_type: &str) -> bool {
        let (max_requests, window_duration) = match self.limits.get(operation_type) {
            Some(limit) => *limit,
            None => (100, Duration::from_secs(60)), // Default fallback
        };

        let mut clients = self.clients.lock().unwrap();
        let now = Instant::now();

        let client_operations = clients.entry(client_ip.to_string()).or_insert_with(HashMap::new);

        match client_operations.get_mut(operation_type) {
            Some((count, window_start)) => {
                // Check if window has expired
                if now.duration_since(*window_start) > window_duration {
                    *count = 1;
                    *window_start = now;
                    true
                } else if *count >= max_requests {
                    false
                } else {
                    *count += 1;
                    true
                }
            }
            None => {
                client_operations.insert(operation_type.to_string(), (1, now));
                true
            }
        }
    }

    pub fn cleanup_expired(&self) {
        let mut clients = self.clients.lock().unwrap();
        let now = Instant::now();
        
        clients.retain(|_, operations| {
            operations.retain(|operation_type, (_, window_start)| {
                let window_duration = self.limits.get(operation_type)
                    .map(|(_, duration)| *duration)
                    .unwrap_or(Duration::from_secs(60));
                now.duration_since(*window_start) <= window_duration
            });
            !operations.is_empty()
        });
    }

    pub fn get_remaining_requests(&self, client_ip: &str, operation_type: &str) -> Option<u32> {
        let (max_requests, _) = self.limits.get(operation_type)?;
        let clients = self.clients.lock().unwrap();
        
        if let Some(client_operations) = clients.get(client_ip) {
            if let Some((used_requests, _)) = client_operations.get(operation_type) {
                return Some(max_requests.saturating_sub(*used_requests));
            }
        }
        
        Some(*max_requests)
    }
}

// Determine operation type based on the request path and method
fn get_operation_type(uri: &Uri) -> String {
    let path = uri.path();
    
    match path {
        // Admin operations
        path if path.starts_with("/admin") => "admin".to_string(),
        
        // Authentication
        path if path.starts_with("/api/auth") => "auth".to_string(),
        path if path.contains("login") || path.contains("signup") => "auth".to_string(),
        
        // Chat and AI operations (most token-intensive)
        "/ws" => "websocket".to_string(),
        path if path.starts_with("/api/chat") => "chat".to_string(),
        path if path.contains("/chat") => "chat".to_string(),
        
        // File operations
        path if path.starts_with("/upload") => "upload".to_string(),
        path if path.starts_with("/api/files") => "download".to_string(),
        path if path.contains("files") && path.contains("session") => "download".to_string(),
        
        // General API
        path if path.starts_with("/api/") => "api".to_string(),
        
        // UI operations (most lenient)
        _ => "ui".to_string(),
    }
}

pub async fn frontend_rate_limit_middleware(
    ConnectInfo(addr): ConnectInfo<SocketAddr>,
    request: Request,
    next: Next,
) -> Result<Response, impl IntoResponse> {
    // Create/get the rate limiter instance
    static FRONTEND_RATE_LIMITER: std::sync::OnceLock<FrontendRateLimiter> = std::sync::OnceLock::new();
    let rate_limiter = FRONTEND_RATE_LIMITER.get_or_init(|| FrontendRateLimiter::new());

    let client_ip = addr.ip().to_string();
    let operation_type = get_operation_type(request.uri());

    if !rate_limiter.check_rate_limit(&client_ip, &operation_type) {
        let remaining = rate_limiter.get_remaining_requests(&client_ip, &operation_type).unwrap_or(0);
        
        tracing::warn!(
            "Frontend rate limit exceeded for IP: {} on operation: {} (remaining: {})", 
            client_ip, operation_type, remaining
        );

        // Provide helpful error messages based on operation type
        let message = match operation_type.as_str() {
            "chat" | "websocket" => {
                "Chat rate limit exceeded. AI operations consume significant resources. Please wait before sending more messages."
            }
            "upload" => {
                "File upload rate limit exceeded. Please wait before uploading more files."
            }
            "auth" => {
                "Authentication rate limit exceeded. Too many login attempts. Please wait before trying again."
            }
            "admin" => {
                "Admin operation rate limit exceeded. Please wait before performing more administrative actions."
            }
            _ => {
                "Rate limit exceeded. Please slow down your requests."
            }
        };

        return Err((
            StatusCode::TOO_MANY_REQUESTS,
            Json(json!({
                "success": false,
                "message": message,
                "operation_type": operation_type,
                "retry_after": 60,
                "help": "This application uses AI services that consume tokens. Rate limiting helps ensure fair usage and optimal performance for everyone."
            })),
        ));
    }

    // Occasionally clean up expired entries (10% chance)
    if rand::random::<u8>() < 26 {
        rate_limiter.cleanup_expired();
    }

    Ok(next.run(request).await)
}

// Create a specialized AI rate limiter with stricter limits
#[derive(Clone)]
pub struct AIRateLimiter {
    clients: Arc<Mutex<HashMap<String, HashMap<String, (u32, Instant)>>>>,
    limits: HashMap<String, (u32, Duration)>,
}

impl AIRateLimiter {
    pub fn new() -> Self {
        let mut limits = HashMap::new();
        
        // Stricter limits for AI operations due to token consumption
        limits.insert("ai_chat".to_string(), (20, Duration::from_secs(60))); // 20 AI messages per minute
        limits.insert("ai_processing".to_string(), (10, Duration::from_secs(300))); // 10 video processing per 5 minutes
        limits.insert("websocket".to_string(), (50, Duration::from_secs(60))); // 50 WebSocket messages per minute
        
        Self {
            clients: Arc::new(Mutex::new(HashMap::new())),
            limits,
        }
    }

    pub fn check_rate_limit(&self, client_ip: &str, operation_type: &str) -> bool {
        let (max_requests, window_duration) = match self.limits.get(operation_type) {
            Some(limit) => *limit,
            None => (20, Duration::from_secs(60)), // Stricter default for AI operations
        };

        let mut clients = self.clients.lock().unwrap();
        let now = Instant::now();

        let client_operations = clients.entry(client_ip.to_string()).or_insert_with(HashMap::new);

        match client_operations.get_mut(operation_type) {
            Some((count, window_start)) => {
                if now.duration_since(*window_start) > window_duration {
                    *count = 1;
                    *window_start = now;
                    true
                } else if *count >= max_requests {
                    false
                } else {
                    *count += 1;
                    true
                }
            }
            None => {
                client_operations.insert(operation_type.to_string(), (1, now));
                true
            }
        }
    }
}

// Middleware specifically for high-cost AI operations
pub async fn ai_operation_rate_limit_middleware(
    ConnectInfo(addr): ConnectInfo<SocketAddr>,
    request: Request,
    next: Next,
) -> Result<Response, impl IntoResponse> {
    // More restrictive rate limiter for AI operations
    static AI_RATE_LIMITER: std::sync::OnceLock<AIRateLimiter> = std::sync::OnceLock::new();
    let rate_limiter = AI_RATE_LIMITER.get_or_init(|| AIRateLimiter::new());

    let client_ip = addr.ip().to_string();
    let operation_type = if request.uri().path() == "/ws" { 
        "websocket".to_string() 
    } else { 
        "ai_chat".to_string() 
    };

    if !rate_limiter.check_rate_limit(&client_ip, &operation_type) {
        tracing::warn!("AI operation rate limit exceeded for IP: {} on operation: {}", client_ip, operation_type);

        return Err((
            StatusCode::TOO_MANY_REQUESTS,
            Json(json!({
                "success": false,
                "message": "AI operation rate limit exceeded. Video editing AI consumes significant computational resources and tokens. Please wait before sending more AI requests.",
                "operation_type": "ai_operation",
                "retry_after": 60,
                "token_conservation": "Each AI video editing operation uses multiple API tokens. Rate limiting helps manage costs and ensures service availability.",
                "help": "This stricter rate limit applies to AI-powered video editing operations to ensure optimal performance and cost management."
            })),
        ));
    }

    Ok(next.run(request).await)
}