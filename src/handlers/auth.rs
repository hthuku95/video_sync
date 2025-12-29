use crate::models::{admin::SystemSetting, auth::*};
use crate::middleware::rate_limit::strict_rate_limit_middleware;
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

pub fn auth_routes() -> Router {
    Router::new()
        .route("/api/auth/register", post(register))
        .route("/api/auth/login", post(login))
        .route("/api/auth/verify", get(verify_token))
        .route("/api/auth/google", get(initiate_google_oauth))
        .route("/api/auth/google/callback", get(google_oauth_callback))
        .layer(axum::middleware::from_fn(strict_rate_limit_middleware))
}

async fn register(
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

    // Check whitelist if enabled
    if let Err(e) = check_whitelist_enabled(&state, &payload.email).await {
        return Err(e);
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

    // Insert new user (normal users are not staff or superuser by default)
    let user_row = sqlx::query(
        "INSERT INTO users (email, username, password_hash, is_active, is_superuser, is_staff, created_at, updated_at) 
         VALUES ($1, $2, $3, true, false, false, NOW(), NOW()) 
         RETURNING id, email, username, password_hash, is_active, is_superuser, is_staff, created_at, updated_at"
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
            user
        },
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
    Extension(state): Extension<Arc<AppState>>,
    Json(payload): Json<LoginRequest>,
) -> Result<Json<AuthResponse>, (StatusCode, Json<ErrorResponse>)> {
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

    // Check whitelist if enabled
    if let Err(e) = check_whitelist_enabled(&state, &payload.email).await {
        return Err(e);
    }

    // Find user by email
    let user_row = sqlx::query(
        "SELECT id, email, username, password_hash, is_active, is_superuser, is_staff, created_at, updated_at 
         FROM users WHERE email = $1 AND is_active = true"
    )
    .bind(&payload.email)
    .fetch_optional(&state.db_pool)
    .await;

    let user = match user_row {
        Ok(Some(row)) => {
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
        },
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

    Ok(Json(AuthResponse {
        success: true,
        message: "Login successful".to_string(),
        user: UserResponse::from(user),
        token,
    }))
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

    // Get user from database
    let user_row = sqlx::query(
        "SELECT id, email, username, is_active, is_superuser, is_staff, created_at, updated_at 
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
        },
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
        "SELECT * FROM system_settings WHERE setting_key = 'whitelist_enabled'"
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
            }))
        )
    })?;

    // Generate state parameter with redirect URL
    let state_data = json!({
        "redirect_to": params.redirect_to.unwrap_or("/dashboard".to_string()),
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

    let auth_url = youtube_client::build_google_oauth_url(
        client_id,
        &redirect_uri,
        &scopes,
        &state_param,
    );

    tracing::info!("🔐 Initiating Google OAuth login");

    Ok(Redirect::to(&auth_url))
}

/// Handle Google OAuth callback for login/signup
pub async fn google_oauth_callback(
    Query(params): Query<GoogleCallbackQuery>,
    Extension(state): Extension<Arc<AppState>>,
) -> Result<Html<String>, (StatusCode, Html<String>)> {
    // Check for OAuth error
    if let Some(error) = params.error {
        tracing::error!("Google OAuth error: {}", error);
        return Ok(Html(format!(
            r#"<!DOCTYPE html><html><head><title>Login Failed</title>
            <style>body {{ font-family: Arial; max-width: 600px; margin: 100px auto; text-align: center; }}</style>
            </head><body>
            <h1>❌ Login Failed</h1><p>Error: {}</p>
            <a href="/login">Try Again</a>
            </body></html>"#,
            error
        )));
    }

    let code = params.code.ok_or_else(|| {
        (
            StatusCode::BAD_REQUEST,
            Html("<h1>Missing authorization code</h1>".to_string())
        )
    })?;

    // Decode state parameter
    let state_json = params.state.ok_or_else(|| {
        (
            StatusCode::BAD_REQUEST,
            Html("<h1>Missing state parameter</h1>".to_string())
        )
    })?;

    let state_bytes = base64::prelude::BASE64_URL_SAFE_NO_PAD.decode(&state_json)
        .map_err(|_| (StatusCode::BAD_REQUEST, Html("<h1>Invalid state</h1>".to_string())))?;

    let state_str = String::from_utf8(state_bytes)
        .map_err(|_| (StatusCode::BAD_REQUEST, Html("<h1>Invalid state</h1>".to_string())))?;

    let state_data: serde_json::Value = serde_json::from_str(&state_str)
        .map_err(|_| (StatusCode::BAD_REQUEST, Html("<h1>Invalid state</h1>".to_string())))?;

    let redirect_to = state_data["redirect_to"]
        .as_str()
        .unwrap_or("/dashboard")
        .to_string();

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
        (StatusCode::INTERNAL_SERVER_ERROR, Html(format!("<h1>Failed to exchange code: {}</h1>", e)))
    })?;

    // Get user info from Google
    let user_info = youtube_client::get_google_user_info(&client, &token_response.access_token)
        .await
        .map_err(|e| {
            tracing::error!("Failed to get user info: {}", e);
            (StatusCode::INTERNAL_SERVER_ERROR, Html(format!("<h1>Failed to get user info: {}</h1>", e)))
        })?;

    // Check whitelist if enabled
    if let Err(_e) = check_whitelist_enabled(&state, &user_info.email).await {
        return Ok(Html(r#"
<!DOCTYPE html><html><head><title>Access Restricted</title>
<style>body { font-family: Arial; max-width: 600px; margin: 100px auto; text-align: center; }</style>
</head><body>
<h1>❌ Access Restricted</h1>
<p>Your email is not whitelisted. Please contact the administrator.</p>
<a href="/login">Back to Login</a>
</body></html>
        "#.to_string()));
    }

    // Calculate token expiry
    let token_expiry = chrono::Utc::now() + chrono::Duration::seconds(token_response.expires_in);

    // Check if user exists with this Google ID
    let existing_user = sqlx::query_as::<_, User>(
        "SELECT * FROM users WHERE google_id = $1"
    )
    .bind(&user_info.id)
    .fetch_optional(&state.db_pool)
    .await
    .map_err(|e| {
        tracing::error!("Database error: {}", e);
        (StatusCode::INTERNAL_SERVER_ERROR, Html("<h1>Database error</h1>".to_string()))
    })?;

    let user = if let Some(mut user) = existing_user {
        // Update existing user's Google tokens
        sqlx::query(
            "UPDATE users
             SET google_access_token = $1, google_refresh_token = $2, google_token_expiry = $3,
                 google_email = $4, google_picture = $5, updated_at = NOW()
             WHERE id = $6"
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
        let email_user = sqlx::query_as::<_, User>(
            "SELECT * FROM users WHERE email = $1"
        )
        .bind(&user_info.email)
        .fetch_optional(&state.db_pool)
        .await
        .map_err(|_| (StatusCode::INTERNAL_SERVER_ERROR, Html("<h1>Database error</h1>".to_string())))?;

        if let Some(mut user) = email_user {
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
                    created_at, updated_at
                )
                VALUES ($1, $2, $3, true, $4, $5, $6, $7, $8, $9, NOW(), NOW())
                RETURNING id, email, username, password_hash, is_active, is_superuser, is_staff, created_at, updated_at"
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
                created_at: user_row.get("created_at"),
                updated_at: user_row.get("updated_at"),
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
        (StatusCode::INTERNAL_SERVER_ERROR, Html("<h1>Failed to generate token</h1>".to_string()))
    })?;

    // Return HTML that stores token and redirects
    Ok(Html(format!(
        r#"<!DOCTYPE html><html><head><title>Login Successful</title>
        <style>body {{ font-family: Arial; max-width: 600px; margin: 100px auto; text-align: center; }}</style>
        </head><body>
        <h1>✅ Successfully logged in with Google</h1>
        <p>Redirecting...</p>
        <script>
            localStorage.setItem('authToken', '{}');
            localStorage.setItem('user', '{}');
            setTimeout(() => window.location.href = '{}', 1000);
        </script>
        </body></html>"#,
        token,
        json!({
            "id": user.id,
            "email": user.email,
            "username": user.username,
            "is_staff": user.is_staff,
            "is_superuser": user.is_superuser
        }).to_string().replace("'", "\\'"),
        redirect_to
    )))
}