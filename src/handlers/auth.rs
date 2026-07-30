use crate::middleware::rate_limit::strict_rate_limit_middleware;
use crate::models::{admin::SystemSetting, auth::*};
use crate::youtube_client;
use crate::AppState;
use axum::{
    extract::{Extension, Query},
    http::{HeaderMap, StatusCode},
    response::{Html, Json, Redirect},
    routing::{get, post, Router},
};
use base64::Engine;
use bcrypt::{hash, verify, DEFAULT_COST};
use chrono::{Duration, Utc};
use jsonwebtoken::{decode, encode, DecodingKey, EncodingKey, Header, Validation};
use serde::Deserialize;
use serde_json::json;
use sqlx::{FromRow, Row};
use std::sync::Arc;

/// Validates redirect URLs to prevent open redirect attacks
/// Allows:
/// - Relative paths (e.g., /dashboard) for videosync.video frontend
/// - Full URLs from allowed origins (configured via ALLOWED_REDIRECT_ORIGINS env)
fn is_allowed_redirect_url(url: &str) -> bool {
    // Allow relative paths (for videosync.video frontend)
    if url.starts_with('/') {
        tracing::debug!("✅ Allowing relative redirect: {}", url);
        return true;
    }

    // Parse as full URL
    if let Ok(parsed) = url::Url::parse(url) {
        let host = parsed.host_str().unwrap_or("");

        // Construct host:port string for comparison
        let host_with_port = if let Some(port) = parsed.port() {
            format!("{}:{}", host, port)
        } else {
            host.to_string()
        };

        // Get allowed origins from environment
        let allowed_origins = std::env::var("ALLOWED_REDIRECT_ORIGINS")
            .unwrap_or_else(|_| {
                "localhost:5173,localhost:3000,content-machine-pbjp.vercel.app,www.videosync.video,videosync.video".to_string()
            });

        tracing::debug!(
            "🔍 Validating redirect URL - host: {}, host_with_port: {}, allowed_origins: {}",
            host,
            host_with_port,
            allowed_origins
        );

        // Check if host:port or just host matches any allowed origin
        for allowed in allowed_origins.split(',') {
            let allowed = allowed.trim();
            if host == allowed || host_with_port == allowed {
                tracing::debug!("✅ Matched allowed origin: {}", allowed);
                return true;
            }
        }

        tracing::warn!(
            "🚫 Rejected redirect to disallowed domain: {} ({}). Allowed origins: {}",
            host,
            host_with_port,
            allowed_origins
        );
        return false;
    }

    // If can't parse and isn't relative, reject
    tracing::warn!("🚫 Invalid redirect URL format: {}", url);
    false
}

pub fn auth_routes() -> Router {
    Router::new()
        .route("/api/auth/register", post(register))
        .route("/api/auth/login", post(login))
        .route("/api/auth/verify", get(verify_token))
        .route("/api/auth/google", get(initiate_google_oauth))
        .route("/api/auth/google/callback", get(google_oauth_callback))
        .route("/api/auth/register/clipper", post(register_clipper))
        .layer(axum::middleware::from_fn(strict_rate_limit_middleware))
}

pub fn clipper_invite_routes() -> Router {
    use crate::middleware::admin::admin_middleware;
    use crate::middleware::auth::auth_middleware;
    Router::new()
        .route("/api/admin/clipper-invites", post(create_clipper_invite))
        .route("/api/admin/clipper-invites", get(list_clipper_invites))
        .route(
            "/api/admin/clipper-invites/:token",
            axum::routing::delete(revoke_clipper_invite),
        )
        .layer(axum::middleware::from_fn(admin_middleware))
        .layer(axum::middleware::from_fn(auth_middleware))
}

async fn register(
    headers: HeaderMap,
    Extension(state): Extension<Arc<AppState>>,
    Json(payload): Json<RegisterRequest>,
) -> Result<Json<AuthResponse>, (StatusCode, Json<ErrorResponse>)> {
    // Validate input
    if payload.email.is_empty() || payload.username.is_empty() || payload.password.is_empty() {
        return Err((
            StatusCode::BAD_REQUEST,
            Json(ErrorResponse {
                success: false,
                message: "Email, username, and password are required".to_string(),
            }),
        ));
    }

    if payload.password.len() < 6 {
        return Err((
            StatusCode::BAD_REQUEST,
            Json(ErrorResponse {
                success: false,
                message: "Password must be at least 6 characters long".to_string(),
            }),
        ));
    }

    // Validate password confirmation
    if payload.password != payload.confirm_password {
        return Err((
            StatusCode::BAD_REQUEST,
            Json(ErrorResponse {
                success: false,
                message: "Passwords do not match".to_string(),
            }),
        ));
    }

    // Only Content Machine signups are whitelist-restricted.
    if request_requires_content_machine_whitelist(&headers) {
        if let Err(e) = check_whitelist_enabled(&state, &payload.email).await {
            return Err(e);
        }
    }

    // Check if user already exists
    let existing_user = sqlx::query("SELECT id FROM users WHERE email = $1 OR username = $2")
        .bind(&payload.email)
        .bind(&payload.username)
        .fetch_optional(&state.db_pool)
        .await;

    match existing_user {
        Ok(Some(_)) => {
            return Err((
                StatusCode::CONFLICT,
                Json(ErrorResponse {
                    success: false,
                    message: "User with this email or username already exists".to_string(),
                }),
            ));
        }
        Ok(None) => {} // User doesn't exist, proceed
        Err(e) => {
            tracing::error!("Database error checking existing user: {}", e);
            return Err((
                StatusCode::INTERNAL_SERVER_ERROR,
                Json(ErrorResponse {
                    success: false,
                    message: "Internal server error".to_string(),
                }),
            ));
        }
    }

    // Hash the password
    let password_hash = match hash(&payload.password, DEFAULT_COST) {
        Ok(hash) => hash,
        Err(e) => {
            tracing::error!("Error hashing password: {}", e);
            return Err((
                StatusCode::INTERNAL_SERVER_ERROR,
                Json(ErrorResponse {
                    success: false,
                    message: "Internal server error".to_string(),
                }),
            ));
        }
    };

    // Insert new user. Starts a 7-day free trial immediately — the
    // subscription_middleware will flip status to 'expired' after
    // trial_ends_at passes. (Existing users were grandfathered by the
    // 20260415000001 migration and never see this flow.)
    let user_row = sqlx::query(
        "INSERT INTO users (email, username, password_hash, is_active, is_superuser,
                            is_staff, is_clipper, created_at, updated_at,
                            subscription_status, trial_ends_at)
         VALUES ($1, $2, $3, true, false, false, false, NOW(), NOW(),
                 'trial', NOW() + INTERVAL '7 days')
         RETURNING id, email, username, password_hash, is_active, is_superuser, is_staff, is_clipper, created_at, updated_at,
                   subscription_status, trial_ends_at, subscription_active_until, subscription_tier, last_payment_at, is_dfy_customer"
    )
    .bind(&payload.email)
    .bind(&payload.username)
    .bind(&password_hash)
    .fetch_one(&state.db_pool)
    .await;

    let user = match user_row {
        Ok(row) => {
            let mut user = User::from_row(&row).map_err(|e| {
                tracing::error!("Error converting row to User: {}", e);
                (
                    StatusCode::INTERNAL_SERVER_ERROR,
                    Json(ErrorResponse {
                        success: false,
                        message: "Failed to create user".to_string(),
                    }),
                )
            })?;
            user.password_hash = String::new(); // Don't include password hash in response
                                                // Audit-log the trial start. Best-effort — non-fatal if it fails.
            let _ = sqlx::query(
                "INSERT INTO user_payment_events (user_id, event_type) VALUES ($1, 'trial_started')"
            )
            .bind(user.id)
            .execute(&state.db_pool)
            .await;
            user
        }
        Err(e) => {
            tracing::error!("Error creating user: {}", e);
            return Err((
                StatusCode::INTERNAL_SERVER_ERROR,
                Json(ErrorResponse {
                    success: false,
                    message: "Failed to create user".to_string(),
                }),
            ));
        }
    };

    // Generate JWT token
    let token = generate_jwt_token(&user)?;

    Ok(Json(AuthResponse {
        success: true,
        message: "User registered successfully".to_string(),
        user: UserResponse::from(user),
        token,
    }))
}

async fn login(
    headers: HeaderMap,
    Extension(state): Extension<Arc<AppState>>,
    Json(payload): Json<LoginRequest>,
) -> Result<(HeaderMap, Json<AuthResponse>), (StatusCode, Json<ErrorResponse>)> {
    // Validate input
    if payload.email.is_empty() || payload.password.is_empty() {
        return Err((
            StatusCode::BAD_REQUEST,
            Json(ErrorResponse {
                success: false,
                message: "Email and password are required".to_string(),
            }),
        ));
    }

    // Only Content Machine logins are whitelist-restricted.
    if request_requires_content_machine_whitelist(&headers) {
        if let Err(e) = check_whitelist_for_existing_user_login(&state, &payload.email).await {
            return Err(e);
        }
    }

    // Find user by email
    let user_row = sqlx::query(
        "SELECT id, email, username, password_hash, google_id, is_active, is_superuser, is_staff, is_clipper, created_at, updated_at,
                subscription_status, trial_ends_at, subscription_active_until, subscription_tier,
                last_payment_at, is_dfy_customer
         FROM users WHERE email = $1 AND is_active = true"
    )
    .bind(&payload.email)
    .fetch_optional(&state.db_pool)
    .await;

    let user = match user_row {
        Ok(Some(row)) => {
            let has_google_id = row
                .try_get::<Option<String>, _>("google_id")
                .ok()
                .flatten()
                .map(|value| !value.trim().is_empty())
                .unwrap_or(false);
            let password_hash = row
                .try_get::<String, _>("password_hash")
                .unwrap_or_default();

            if password_hash.trim().is_empty() && has_google_id {
                return Err((
                    StatusCode::UNAUTHORIZED,
                    Json(ErrorResponse {
                        success: false,
                        message: "This account uses Google sign-in. Please continue with Google."
                            .to_string(),
                    }),
                ));
            }

            // Use try_into to convert the row to User struct
            User::from_row(&row).map_err(|e| {
                tracing::error!("Error converting row to User: {}", e);
                (
                    StatusCode::INTERNAL_SERVER_ERROR,
                    Json(ErrorResponse {
                        success: false,
                        message: "Internal server error".to_string(),
                    }),
                )
            })?
        }
        Ok(None) => {
            return Err((
                StatusCode::UNAUTHORIZED,
                Json(ErrorResponse {
                    success: false,
                    message: "Invalid email or password".to_string(),
                }),
            ));
        }
        Err(e) => {
            tracing::error!("Database error finding user: {}", e);
            return Err((
                StatusCode::INTERNAL_SERVER_ERROR,
                Json(ErrorResponse {
                    success: false,
                    message: "Internal server error".to_string(),
                }),
            ));
        }
    };

    // Verify password
    match verify(&payload.password, &user.password_hash) {
        Ok(true) => {} // Password is correct
        Ok(false) => {
            return Err((
                StatusCode::UNAUTHORIZED,
                Json(ErrorResponse {
                    success: false,
                    message: "Invalid email or password".to_string(),
                }),
            ));
        }
        Err(e) => {
            tracing::error!("Error verifying password: {}", e);
            return Err((
                StatusCode::INTERNAL_SERVER_ERROR,
                Json(ErrorResponse {
                    success: false,
                    message: "Internal server error".to_string(),
                }),
            ));
        }
    }

    // Generate JWT token
    let token = generate_jwt_token(&user)?;

    // Set HttpOnly cookie so browser page navigations carry the JWT
    let mut headers = HeaderMap::new();
    let cookie_value = format!(
        "token={}; HttpOnly; Secure; Path=/; SameSite=Lax; Max-Age=86400",
        token
    );
    headers.insert(
        axum::http::header::SET_COOKIE,
        axum::http::HeaderValue::from_str(&cookie_value).unwrap(),
    );

    Ok((headers, Json(AuthResponse {
        success: true,
        message: "Login successful".to_string(),
        user: UserResponse::from(user),
        token,
    })))
}

fn generate_jwt_token(user: &User) -> Result<String, (StatusCode, Json<ErrorResponse>)> {
    let jwt_secret = std::env::var("JWT_SECRET").unwrap_or_else(|_| "default_secret".to_string());

    let expiration = Utc::now()
        .checked_add_signed(Duration::hours(24))
        .expect("valid timestamp")
        .timestamp();

    let claims = Claims {
        sub: user.id.to_string(),
        username: user.username.clone(),
        email: user.email.clone(),
        is_superuser: user.is_superuser,
        is_staff: user.is_staff,
        is_clipper: user.is_clipper,
        exp: expiration as usize,
        iat: Utc::now().timestamp() as usize,
    };

    match encode(
        &Header::default(),
        &claims,
        &EncodingKey::from_secret(jwt_secret.as_ref()),
    ) {
        Ok(token) => Ok(token),
        Err(e) => {
            tracing::error!("Error generating JWT token: {}", e);
            Err((
                StatusCode::INTERNAL_SERVER_ERROR,
                Json(ErrorResponse {
                    success: false,
                    message: "Failed to generate authentication token".to_string(),
                }),
            ))
        }
    }
}

async fn verify_token(
    headers: HeaderMap,
    Extension(state): Extension<Arc<AppState>>,
) -> Result<Json<serde_json::Value>, (StatusCode, Json<ErrorResponse>)> {
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
                message: "Invalid Authorization header format. Expected 'Bearer <token>'"
                    .to_string(),
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

    // Get user from database
    let user_row = sqlx::query(
        "SELECT id, email, username, password_hash, is_active, is_superuser, is_staff, is_clipper, created_at, updated_at,
                subscription_status, trial_ends_at, subscription_active_until, subscription_tier, last_payment_at, is_dfy_customer
         FROM users WHERE id = $1 AND is_active = true"
    )
    .bind(claims.sub.parse::<i32>().unwrap_or(0))
    .fetch_optional(&state.db_pool)
    .await;

    let user = match user_row {
        Ok(Some(row)) => {
            let mut user = User::from_row(&row).map_err(|e| {
                tracing::error!("Error converting row to User: {}", e);
                (
                    StatusCode::INTERNAL_SERVER_ERROR,
                    Json(ErrorResponse {
                        success: false,
                        message: "Internal server error".to_string(),
                    }),
                )
            })?;
            user.password_hash = String::new(); // Don't include password hash
            user
        }
        Ok(None) => {
            return Err((
                StatusCode::UNAUTHORIZED,
                Json(ErrorResponse {
                    success: false,
                    message: "User not found".to_string(),
                }),
            ));
        }
        Err(e) => {
            tracing::error!("Database error finding user: {}", e);
            return Err((
                StatusCode::INTERNAL_SERVER_ERROR,
                Json(ErrorResponse {
                    success: false,
                    message: "Internal server error".to_string(),
                }),
            ));
        }
    };

    Ok(Json(serde_json::json!({
        "success": true,
        "user": UserResponse::from(user)
    })))
}

pub fn verify_jwt_token(token: &str) -> Result<Claims, jsonwebtoken::errors::Error> {
    let jwt_secret = std::env::var("JWT_SECRET").unwrap_or_else(|_| "default_secret".to_string());

    let token_data = decode::<Claims>(
        token,
        &DecodingKey::from_secret(jwt_secret.as_ref()),
        &Validation::default(),
    )?;

    Ok(token_data.claims)
}

async fn check_whitelist_enabled(
    state: &Arc<AppState>,
    email: &str,
) -> Result<(), (StatusCode, Json<ErrorResponse>)> {
    // Get whitelist enabled status
    let setting = sqlx::query_as::<_, SystemSetting>(
        "SELECT * FROM system_settings WHERE setting_key = 'whitelist_enabled'",
    )
    .fetch_optional(&state.db_pool)
    .await
    .map_err(|e| {
        tracing::error!("Database error checking whitelist setting: {}", e);
        (
            StatusCode::INTERNAL_SERVER_ERROR,
            Json(ErrorResponse {
                success: false,
                message: "Internal server error".to_string(),
            }),
        )
    })?;

    let whitelist_enabled = setting
        .map(|s| s.as_bool().unwrap_or(false))
        .unwrap_or(false);

    // If whitelist is not enabled, allow all emails
    if !whitelist_enabled {
        return Ok(());
    }

    // Check if email is in whitelist
    let whitelisted = sqlx::query("SELECT id FROM whitelist_emails WHERE email = $1")
        .bind(email)
        .fetch_optional(&state.db_pool)
        .await
        .map_err(|e| {
            tracing::error!("Database error checking whitelist: {}", e);
            (
                StatusCode::INTERNAL_SERVER_ERROR,
                Json(ErrorResponse {
                    success: false,
                    message: "Internal server error".to_string(),
                }),
            )
        })?;

    if whitelisted.is_none() {
        return Err((
            StatusCode::FORBIDDEN,
            Json(ErrorResponse {
                success: false,
                message: "Access restricted. Email not whitelisted.".to_string(),
            }),
        ));
    }

    Ok(())
}

async fn has_existing_active_user_with_email(
    state: &Arc<AppState>,
    email: &str,
) -> Result<bool, (StatusCode, Json<ErrorResponse>)> {
    sqlx::query_scalar::<_, i64>("SELECT COUNT(*) FROM users WHERE email = $1 AND is_active = true")
        .bind(email)
        .fetch_one(&state.db_pool)
        .await
        .map(|count| count > 0)
        .map_err(|e| {
            tracing::error!(
                "Database error checking existing active user for whitelist login bypass: {}",
                e
            );
            (
                StatusCode::INTERNAL_SERVER_ERROR,
                Json(ErrorResponse {
                    success: false,
                    message: "Internal server error".to_string(),
                }),
            )
        })
}

async fn check_whitelist_for_existing_user_login(
    state: &Arc<AppState>,
    email: &str,
) -> Result<(), (StatusCode, Json<ErrorResponse>)> {
    match check_whitelist_enabled(state, email).await {
        Ok(()) => Ok(()),
        Err(err) => {
            if has_existing_active_user_with_email(state, email).await? {
                tracing::info!(
                    email = %email,
                    "Allowing existing active user through whitelist login gate"
                );
                Ok(())
            } else {
                Err(err)
            }
        }
    }
}

fn request_requires_content_machine_whitelist(headers: &HeaderMap) -> bool {
    let origin = headers
        .get("origin")
        .and_then(|v| v.to_str().ok())
        .unwrap_or_default()
        .to_lowercase();
    let referer = headers
        .get("referer")
        .and_then(|v| v.to_str().ok())
        .unwrap_or_default()
        .to_lowercase();

    let matches_content_machine = |value: &str| {
        value.contains("content-machine-pbjp.vercel.app")
            || value.contains("localhost:5173")
            || value.contains("localhost:4173")
    };

    matches_content_machine(&origin) || matches_content_machine(&referer)
}

// ============================================================================
// Google OAuth Login/Signup
// ============================================================================

#[derive(Deserialize)]
pub struct GoogleOAuthQuery {
    pub redirect_to: Option<String>,
}

#[derive(Deserialize)]
pub struct GoogleCallbackQuery {
    pub code: Option<String>,
    pub state: Option<String>,
    pub error: Option<String>,
}

/// Initiate Google OAuth login/signup
pub async fn initiate_google_oauth(
    Query(params): Query<GoogleOAuthQuery>,
    Extension(state): Extension<Arc<AppState>>,
) -> Result<Redirect, (StatusCode, Json<serde_json::Value>)> {
    // Check if Google OAuth is configured
    let client_id = state.google_oauth_client_id.as_ref().ok_or_else(|| {
        (
            StatusCode::SERVICE_UNAVAILABLE,
            Json(json!({
                "success": false,
                "message": "Google OAuth not configured"
            })),
        )
    })?;

    // Validate and get redirect URL
    let redirect_to = params.redirect_to.unwrap_or("/dashboard".to_string());

    tracing::info!(
        "🔐 Initiating Google OAuth login with redirect_to: {}",
        redirect_to
    );

    if !is_allowed_redirect_url(&redirect_to) {
        tracing::error!("🚫 Rejected invalid redirect URL: {}", redirect_to);
        return Err((
            StatusCode::BAD_REQUEST,
            Json(json!({
                "success": false,
                "message": "Invalid redirect URL"
            })),
        ));
    }

    // Detect source app from redirect URL to ensure proper redirection
    let source_app = if redirect_to.contains("content-machine-pbjp.vercel.app")
    {
        "content_machine"
    } else if redirect_to.contains("localhost:5173") || redirect_to.contains("localhost:4173") {
        "content_machine_local"
    } else {
        "videosync"
    };

    // Generate state parameter with redirect URL and source app
    let state_data = json!({
        "redirect_to": redirect_to,
        "source_app": source_app,
        "timestamp": chrono::Utc::now().timestamp()
    });
    let state_param = base64::prelude::BASE64_URL_SAFE_NO_PAD.encode(state_data.to_string());

    // Required scopes for login
    let scopes = [
        "https://www.googleapis.com/auth/userinfo.email",
        "https://www.googleapis.com/auth/userinfo.profile",
        "openid",
    ];

    let redirect_uri = std::env::var("GOOGLE_OAUTH_REDIRECT_URI_AUTH")
        .unwrap_or_else(|_| "http://localhost:3000/api/auth/google/callback".to_string());

    let auth_url =
        youtube_client::build_google_oauth_url(client_id, &redirect_uri, &scopes, &state_param);

    Ok(Redirect::to(&auth_url))
}

/// Handle Google OAuth callback for login/signup
pub async fn google_oauth_callback(
    Query(params): Query<GoogleCallbackQuery>,
    Extension(state): Extension<Arc<AppState>>,
) -> Result<(HeaderMap, Html<String>), (StatusCode, Html<String>)> {
    // Check for OAuth error
    if let Some(error) = params.error {
        tracing::error!("Google OAuth error: {}", error);
        return Ok((HeaderMap::new(), Html(format!(
            r#"<!DOCTYPE html><html><head><title>Login Failed</title>
            <style>body {{ font-family: Arial; max-width: 600px; margin: 100px auto; text-align: center; }}</style>
            </head><body>
            <h1>❌ Login Failed</h1><p>Error: {}</p>
            <a href="/login">Try Again</a>
            </body></html>"#,
            error
        ))));
    }

    let code = params.code.ok_or_else(|| {
        (
            StatusCode::BAD_REQUEST,
            Html("<h1>Missing authorization code</h1>".to_string()),
        )
    })?;

    // Decode state parameter
    let state_json = params.state.ok_or_else(|| {
        (
            StatusCode::BAD_REQUEST,
            Html("<h1>Missing state parameter</h1>".to_string()),
        )
    })?;

    let state_bytes = base64::prelude::BASE64_URL_SAFE_NO_PAD
        .decode(&state_json)
        .map_err(|_| {
            (
                StatusCode::BAD_REQUEST,
                Html("<h1>Invalid state</h1>".to_string()),
            )
        })?;

    let state_str = String::from_utf8(state_bytes).map_err(|_| {
        (
            StatusCode::BAD_REQUEST,
            Html("<h1>Invalid state</h1>".to_string()),
        )
    })?;

    let state_data: serde_json::Value = serde_json::from_str(&state_str).map_err(|_| {
        (
            StatusCode::BAD_REQUEST,
            Html("<h1>Invalid state</h1>".to_string()),
        )
    })?;

    let redirect_to = state_data["redirect_to"]
        .as_str()
        .unwrap_or("/dashboard")
        .to_string();

    let source_app = state_data["source_app"].as_str().unwrap_or("videosync");

    tracing::info!(
        "🔄 OAuth callback received - redirect_to: {}, source_app: {}",
        redirect_to,
        source_app
    );

    // Validate redirect URL for security
    let redirect_to = if is_allowed_redirect_url(&redirect_to) {
        tracing::info!("✅ Redirect URL validated: {}", redirect_to);
        redirect_to
    } else {
        // Provide appropriate fallback based on source app
        let fallback = match source_app {
            "content_machine" => "https://content-machine-pbjp.vercel.app/",
            "content_machine_local" => "http://localhost:5173/",
            _ => "/dashboard",
        };
        tracing::warn!(
            "🚫 Invalid redirect URL in callback, falling back to {} for app {}: {}",
            fallback,
            source_app,
            redirect_to
        );
        fallback.to_string()
    };

    // Exchange code for tokens
    let client_id = state.google_oauth_client_id.as_ref().unwrap();
    let client_secret = state.google_oauth_client_secret.as_ref().unwrap();
    let redirect_uri = std::env::var("GOOGLE_OAUTH_REDIRECT_URI_AUTH")
        .unwrap_or_else(|_| "http://localhost:3000/api/auth/google/callback".to_string());

    let client = reqwest::Client::new();
    let token_response = youtube_client::exchange_code_for_token(
        &client,
        &code,
        client_id,
        client_secret,
        &redirect_uri,
    )
    .await
    .map_err(|e| {
        tracing::error!("Failed to exchange code: {}", e);
        (
            StatusCode::INTERNAL_SERVER_ERROR,
            Html(format!("<h1>Failed to exchange code: {}</h1>", e)),
        )
    })?;

    // Get user info from Google
    let user_info = youtube_client::get_google_user_info(&client, &token_response.access_token)
        .await
        .map_err(|e| {
            tracing::error!("Failed to get user info: {}", e);
            (
                StatusCode::INTERNAL_SERVER_ERROR,
                Html(format!("<h1>Failed to get user info: {}</h1>", e)),
            )
        })?;

    // Only Content Machine Google sign-ins should be whitelist-restricted.
    if source_app == "content_machine" {
        if let Err(_e) = check_whitelist_for_existing_user_login(&state, &user_info.email).await {
            return Ok((HeaderMap::new(), Html(r#"
<!DOCTYPE html><html><head><title>Access Restricted</title>
<style>body { font-family: Arial; max-width: 600px; margin: 100px auto; text-align: center; }</style>
</head><body>
<h1>❌ Access Restricted</h1>
<p>Your email is not whitelisted. Please contact the administrator.</p>
<a href="/login">Back to Login</a>
</body></html>
        "#.to_string())));
        }
    }

    // Calculate token expiry
    let token_expiry = chrono::Utc::now() + chrono::Duration::seconds(token_response.expires_in);

    // Check if user exists with this Google ID
    let existing_user = sqlx::query_as::<_, User>("SELECT * FROM users WHERE google_id = $1")
        .bind(&user_info.id)
        .fetch_optional(&state.db_pool)
        .await
        .map_err(|e| {
            tracing::error!("Database error: {}", e);
            (
                StatusCode::INTERNAL_SERVER_ERROR,
                Html("<h1>Database error</h1>".to_string()),
            )
        })?;

    let user = if let Some(user) = existing_user {
        // Update existing user's Google tokens
        sqlx::query(
            "UPDATE users
             SET google_access_token = $1, google_refresh_token = $2, google_token_expiry = $3,
                 google_email = $4, google_picture = $5, updated_at = NOW()
             WHERE id = $6",
        )
        .bind(&token_response.access_token)
        .bind(&token_response.refresh_token)
        .bind(token_expiry)
        .bind(&user_info.email)
        .bind(&user_info.picture)
        .bind(user.id)
        .execute(&state.db_pool)
        .await
        .ok();

        tracing::info!("👤 Existing user logged in via Google: {}", user.email);
        user
    } else {
        // Check if email already exists (link accounts)
        let email_user = sqlx::query_as::<_, User>("SELECT * FROM users WHERE email = $1")
            .bind(&user_info.email)
            .fetch_optional(&state.db_pool)
            .await
            .map_err(|_| {
                (
                    StatusCode::INTERNAL_SERVER_ERROR,
                    Html("<h1>Database error</h1>".to_string()),
                )
            })?;

        if let Some(user) = email_user {
            // Link Google account to existing user
            sqlx::query(
                "UPDATE users
                 SET google_id = $1, google_access_token = $2, google_refresh_token = $3,
                     google_token_expiry = $4, google_email = $5, google_picture = $6, updated_at = NOW()
                 WHERE id = $7"
            )
            .bind(&user_info.id)
            .bind(&token_response.access_token)
            .bind(&token_response.refresh_token)
            .bind(token_expiry)
            .bind(&user_info.email)
            .bind(&user_info.picture)
            .bind(user.id)
            .execute(&state.db_pool)
            .await
            .ok();

            tracing::info!("🔗 Linked Google account to existing user: {}", user.email);
            user
        } else {
            // Create new user from Google account
            let username = user_info.email.split('@').next().unwrap_or(&user_info.name);

            let user_row = sqlx::query(
                "INSERT INTO users (
                    email, username, password_hash, is_active,
                    google_id, google_email, google_picture,
                    google_access_token, google_refresh_token, google_token_expiry,
                    subscription_status, trial_ends_at,
                    created_at, updated_at
                )
                VALUES ($1, $2, $3, true, $4, $5, $6, $7, $8, $9,
                        'trial', NOW() + INTERVAL '7 days',
                        NOW(), NOW())
                RETURNING id, email, username, password_hash, is_active, is_superuser, is_staff, is_clipper, created_at, updated_at,
                          subscription_status, trial_ends_at, subscription_active_until, subscription_tier, last_payment_at, is_dfy_customer"
            )
            .bind(&user_info.email)
            .bind(username)
            .bind("") // No password for Google users
            .bind(&user_info.id)
            .bind(&user_info.email)
            .bind(&user_info.picture)
            .bind(&token_response.access_token)
            .bind(&token_response.refresh_token)
            .bind(token_expiry)
            .fetch_one(&state.db_pool)
            .await
            .map_err(|e| {
                tracing::error!("Failed to create user: {}", e);
                (StatusCode::INTERNAL_SERVER_ERROR, Html(format!("<h1>Failed to create user: {}</h1>", e)))
            })?;

            let user = User {
                id: user_row.get("id"),
                email: user_row.get("email"),
                username: user_row.get("username"),
                password_hash: user_row.get("password_hash"),
                is_active: user_row.get("is_active"),
                is_superuser: user_row.get("is_superuser"),
                is_staff: user_row.get("is_staff"),
                is_clipper: user_row.get("is_clipper"),
                created_at: user_row.get("created_at"),
                updated_at: user_row.get("updated_at"),
                subscription_status: user_row.try_get("subscription_status").ok().flatten(),
                trial_ends_at: user_row.try_get("trial_ends_at").ok().flatten(),
                subscription_active_until: user_row.try_get("subscription_active_until").ok().flatten(),
                subscription_tier: user_row.try_get("subscription_tier").ok().flatten(),
                last_payment_at: user_row.try_get("last_payment_at").ok().flatten(),
                is_dfy_customer: user_row.try_get("is_dfy_customer").unwrap_or(false),
            };

            tracing::info!("✨ Created new user via Google OAuth: {}", user.email);
            user
        }
    };

    // Generate JWT token
    let jwt_secret = std::env::var("JWT_SECRET").expect("JWT_SECRET must be set");

    let claims = Claims {
        sub: user.id.to_string(),
        email: user.email.clone(),
        username: user.username.clone(),
        is_superuser: user.is_superuser,
        is_staff: user.is_staff,
        is_clipper: user.is_clipper,
        exp: (Utc::now() + Duration::days(30)).timestamp() as usize,
        iat: Utc::now().timestamp() as usize,
    };

    let token = encode(
        &Header::default(),
        &claims,
        &EncodingKey::from_secret(jwt_secret.as_ref()),
    )
    .map_err(|e| {
        tracing::error!("Failed to generate token: {}", e);
        (
            StatusCode::INTERNAL_SERVER_ERROR,
            Html("<h1>Failed to generate token</h1>".to_string()),
        )
    })?;

    // Pass token and user data via URL hash to frontend
    let user_json = json!({
        "id": user.id,
        "email": user.email,
        "username": user.username,
        "is_staff": user.is_staff,
        "is_superuser": user.is_superuser,
        "is_clipper": user.is_clipper
    })
    .to_string();

    // URL encode the token and user data for the hash fragment
    let encoded_token = urlencoding::encode(&token);
    let encoded_user = urlencoding::encode(&user_json);

    // Construct redirect URL with hash parameters
    let final_redirect = format!(
        "{}#token={}&user={}",
        redirect_to, encoded_token, encoded_user
    );

    // Set HttpOnly cookie so browser page navigations carry the JWT
    let cookie_value = format!(
        "token={}; HttpOnly; Secure; Path=/; SameSite=Lax; Max-Age=86400",
        token
    );

    // Return HTML that redirects with token in URL hash + cookie for page navs
    let mut headers = HeaderMap::new();
    headers.insert(
        axum::http::header::SET_COOKIE,
        axum::http::HeaderValue::from_str(&cookie_value).unwrap(),
    );

    Ok((
        headers,
        Html(format!(
            r#"<!DOCTYPE html><html><head><title>Login Successful</title>
        <style>body {{ font-family: Arial; max-width: 600px; margin: 100px auto; text-align: center; }}</style>
        </head><body>
        <h1>✅ Successfully logged in with Google</h1>
        <p>Redirecting...</p>
        <script>
            setTimeout(() => window.location.href = '{}', 1000);
        </script>
        </body></html>"#,
            final_redirect
        )),
    ))
}

// ============================================================================
// Clipper Invite System
// ============================================================================

#[derive(Debug, Deserialize)]
struct CreateInviteRequest {
    label: Option<String>,
    expires_days: Option<i64>,
}

#[derive(Debug, Deserialize)]
pub struct ClipperRegisterRequest {
    pub token: String,
    pub email: String,
    pub username: String,
    pub password: String,
    pub confirm_password: String,
}

async fn create_clipper_invite(
    Extension(state): Extension<Arc<AppState>>,
    Extension(claims): Extension<Claims>,
    Json(payload): Json<CreateInviteRequest>,
) -> Result<Json<serde_json::Value>, (StatusCode, Json<ErrorResponse>)> {
    use rand::Rng;
    let token: String = rand::thread_rng()
        .sample_iter(&rand::distributions::Alphanumeric)
        .take(48)
        .map(char::from)
        .collect();

    let expires_days = payload.expires_days.unwrap_or(30);
    let admin_id: i32 = claims.sub.parse().unwrap_or(0);

    sqlx::query(
        "INSERT INTO clipper_invite_tokens (token, label, created_by_admin_id, expires_at)
         VALUES ($1, $2, $3, NOW() + $4 * INTERVAL '1 day')",
    )
    .bind(&token)
    .bind(&payload.label)
    .bind(admin_id)
    .bind(expires_days)
    .execute(&state.db_pool)
    .await
    .map_err(|e| {
        tracing::error!("Failed to create invite token: {}", e);
        (
            StatusCode::INTERNAL_SERVER_ERROR,
            Json(ErrorResponse {
                success: false,
                message: "Failed to create token".to_string(),
            }),
        )
    })?;

    Ok(Json(serde_json::json!({
        "success": true,
        "token": token,
        "signup_url": format!("/signup/clipper?token={}", token),
        "expires_days": expires_days
    })))
}

async fn list_clipper_invites(
    Extension(state): Extension<Arc<AppState>>,
) -> Result<Json<serde_json::Value>, (StatusCode, Json<ErrorResponse>)> {
    let rows = sqlx::query(
        "SELECT t.id, t.token, t.label, t.expires_at, t.used_at, t.created_at,
                u.email AS used_by_email
         FROM clipper_invite_tokens t
         LEFT JOIN users u ON u.id = t.used_by_user_id
         ORDER BY t.created_at DESC",
    )
    .fetch_all(&state.db_pool)
    .await
    .map_err(|e| {
        tracing::error!("Failed to list invites: {}", e);
        (
            StatusCode::INTERNAL_SERVER_ERROR,
            Json(ErrorResponse {
                success: false,
                message: "Failed to fetch tokens".to_string(),
            }),
        )
    })?;

    let invites: Vec<serde_json::Value> = rows
        .iter()
        .map(|r| {
            let used_at: Option<chrono::DateTime<chrono::Utc>> = r.get("used_at");
            serde_json::json!({
                "id": r.get::<uuid::Uuid, _>("id").to_string(),
                "token": r.get::<String, _>("token"),
                "label": r.get::<Option<String>, _>("label"),
                "expires_at": r.get::<chrono::DateTime<chrono::Utc>, _>("expires_at"),
                "used": used_at.is_some(),
                "used_at": used_at,
                "used_by_email": r.get::<Option<String>, _>("used_by_email"),
                "created_at": r.get::<chrono::DateTime<chrono::Utc>, _>("created_at"),
            })
        })
        .collect();

    Ok(Json(
        serde_json::json!({ "success": true, "invites": invites }),
    ))
}

async fn revoke_clipper_invite(
    Extension(state): Extension<Arc<AppState>>,
    axum::extract::Path(token): axum::extract::Path<String>,
) -> Result<Json<serde_json::Value>, (StatusCode, Json<ErrorResponse>)> {
    let result =
        sqlx::query("DELETE FROM clipper_invite_tokens WHERE token = $1 AND used_at IS NULL")
            .bind(&token)
            .execute(&state.db_pool)
            .await
            .map_err(|e| {
                tracing::error!("Failed to revoke invite: {}", e);
                (
                    StatusCode::INTERNAL_SERVER_ERROR,
                    Json(ErrorResponse {
                        success: false,
                        message: "Failed to revoke token".to_string(),
                    }),
                )
            })?;

    if result.rows_affected() == 0 {
        return Err((
            StatusCode::NOT_FOUND,
            Json(ErrorResponse {
                success: false,
                message: "Token not found or already used".to_string(),
            }),
        ));
    }

    Ok(Json(
        serde_json::json!({ "success": true, "message": "Token revoked" }),
    ))
}

async fn register_clipper(
    Extension(state): Extension<Arc<AppState>>,
    Json(payload): Json<ClipperRegisterRequest>,
) -> Result<Json<AuthResponse>, (StatusCode, Json<ErrorResponse>)> {
    // Validate inputs
    if payload.email.is_empty() || payload.username.is_empty() || payload.password.is_empty() {
        return Err((
            StatusCode::BAD_REQUEST,
            Json(ErrorResponse {
                success: false,
                message: "Email, username, and password are required".to_string(),
            }),
        ));
    }
    if payload.password.len() < 6 {
        return Err((
            StatusCode::BAD_REQUEST,
            Json(ErrorResponse {
                success: false,
                message: "Password must be at least 6 characters".to_string(),
            }),
        ));
    }
    if payload.password != payload.confirm_password {
        return Err((
            StatusCode::BAD_REQUEST,
            Json(ErrorResponse {
                success: false,
                message: "Passwords do not match".to_string(),
            }),
        ));
    }

    // Validate invite token
    let token_row =
        sqlx::query("SELECT id, expires_at, used_at FROM clipper_invite_tokens WHERE token = $1")
            .bind(&payload.token)
            .fetch_optional(&state.db_pool)
            .await
            .map_err(|e| {
                tracing::error!("DB error checking invite token: {}", e);
                (
                    StatusCode::INTERNAL_SERVER_ERROR,
                    Json(ErrorResponse {
                        success: false,
                        message: "Internal server error".to_string(),
                    }),
                )
            })?;

    let token_row = token_row.ok_or_else(|| {
        (
            StatusCode::BAD_REQUEST,
            Json(ErrorResponse {
                success: false,
                message: "Invalid invite token".to_string(),
            }),
        )
    })?;

    let used_at: Option<chrono::DateTime<chrono::Utc>> = token_row.get("used_at");
    if used_at.is_some() {
        return Err((
            StatusCode::BAD_REQUEST,
            Json(ErrorResponse {
                success: false,
                message: "Invite token has already been used".to_string(),
            }),
        ));
    }
    let expires_at: chrono::DateTime<chrono::Utc> = token_row.get("expires_at");
    if expires_at < chrono::Utc::now() {
        return Err((
            StatusCode::BAD_REQUEST,
            Json(ErrorResponse {
                success: false,
                message: "Invite token has expired".to_string(),
            }),
        ));
    }
    let token_id: uuid::Uuid = token_row.get("id");

    // Check if user already exists
    let existing = sqlx::query("SELECT id FROM users WHERE email = $1 OR username = $2")
        .bind(&payload.email)
        .bind(&payload.username)
        .fetch_optional(&state.db_pool)
        .await
        .map_err(|e| {
            tracing::error!("DB error: {}", e);
            (
                StatusCode::INTERNAL_SERVER_ERROR,
                Json(ErrorResponse {
                    success: false,
                    message: "Internal server error".to_string(),
                }),
            )
        })?;

    if existing.is_some() {
        return Err((
            StatusCode::CONFLICT,
            Json(ErrorResponse {
                success: false,
                message: "Email or username already taken".to_string(),
            }),
        ));
    }

    let password_hash = hash(&payload.password, DEFAULT_COST).map_err(|e| {
        tracing::error!("Hash error: {}", e);
        (
            StatusCode::INTERNAL_SERVER_ERROR,
            Json(ErrorResponse {
                success: false,
                message: "Internal server error".to_string(),
            }),
        )
    })?;

    // Create clipper user
    let user_row = sqlx::query(
        "INSERT INTO users (email, username, password_hash, is_active, is_superuser, is_staff, is_clipper,
                            subscription_status, trial_ends_at, created_at, updated_at)
         VALUES ($1, $2, $3, true, false, false, true,
                 'trial', NOW() + INTERVAL '7 days', NOW(), NOW())
         RETURNING id, email, username, password_hash, is_active, is_superuser, is_staff, is_clipper, created_at, updated_at,
                   subscription_status, trial_ends_at, subscription_active_until, subscription_tier, last_payment_at, is_dfy_customer"
    )
    .bind(&payload.email)
    .bind(&payload.username)
    .bind(&password_hash)
    .fetch_one(&state.db_pool)
    .await
    .map_err(|e| {
        tracing::error!("Failed to create clipper user: {}", e);
        (StatusCode::INTERNAL_SERVER_ERROR, Json(ErrorResponse { success: false, message: "Failed to create user".to_string() }))
    })?;

    let mut user = User::from_row(&user_row).map_err(|e| {
        tracing::error!("Row conversion error: {}", e);
        (
            StatusCode::INTERNAL_SERVER_ERROR,
            Json(ErrorResponse {
                success: false,
                message: "Internal server error".to_string(),
            }),
        )
    })?;
    user.password_hash = String::new();

    let user_id = user.id;

    // Mark token as used
    sqlx::query(
        "UPDATE clipper_invite_tokens SET used_by_user_id = $1, used_at = NOW() WHERE id = $2",
    )
    .bind(user_id)
    .bind(token_id)
    .execute(&state.db_pool)
    .await
    .ok();

    let token = generate_jwt_token(&user)?;
    tracing::info!("✅ Clipper registered: {}", user.email);

    Ok(Json(AuthResponse {
        success: true,
        message: "Clipper account created successfully".to_string(),
        user: UserResponse::from(user),
        token,
    }))
}
