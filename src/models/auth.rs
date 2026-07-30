use serde::{Deserialize, Serialize};
use sqlx::FromRow;

#[derive(Debug, Serialize, Deserialize, FromRow)]
pub struct User {
    pub id: i32,
    pub email: String,
    pub username: String,
    pub password_hash: String,
    pub is_active: bool,
    pub is_superuser: bool,
    pub is_staff: bool,
    pub is_clipper: bool,
    pub created_at: chrono::DateTime<chrono::Utc>,
    pub updated_at: chrono::DateTime<chrono::Utc>,
    pub subscription_status: Option<String>,
    pub trial_ends_at: Option<chrono::DateTime<chrono::Utc>>,
    pub subscription_active_until: Option<chrono::DateTime<chrono::Utc>>,
    pub subscription_tier: Option<String>,
    pub last_payment_at: Option<chrono::DateTime<chrono::Utc>>,
    pub is_dfy_customer: bool,
}

#[derive(Debug, Serialize, Deserialize, sqlx::FromRow)]
pub struct UserResponse {
    pub id: i32,
    pub email: String,
    pub username: String,
    pub is_active: bool,
    pub is_superuser: bool,
    pub is_staff: bool,
    pub is_clipper: bool,
    pub created_at: chrono::DateTime<chrono::Utc>,
    pub subscription_status: Option<String>,
    pub trial_ends_at: Option<chrono::DateTime<chrono::Utc>>,
    pub subscription_active_until: Option<chrono::DateTime<chrono::Utc>>,
    pub subscription_tier: Option<String>,
    pub last_payment_at: Option<chrono::DateTime<chrono::Utc>>,
    pub is_dfy_customer: bool,
}

#[derive(Debug, Deserialize)]
pub struct RegisterRequest {
    pub email: String,
    pub username: String,
    pub password: String,
    pub confirm_password: String,
    pub referred_by: Option<String>,
}

#[derive(Debug, Deserialize)]
pub struct LoginRequest {
    pub email: String,
    pub password: String,
}

#[derive(Debug, Serialize)]
pub struct AuthResponse {
    pub success: bool,
    pub message: String,
    pub user: UserResponse,
    pub token: String,
}

#[derive(Debug, Serialize)]
pub struct ErrorResponse {
    pub success: bool,
    pub message: String,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct Claims {
    pub sub: String, // Subject (user id)
    pub username: String,
    pub email: String,
    pub is_superuser: bool,
    pub is_staff: bool,
    pub is_clipper: bool,
    pub exp: usize, // Expiration time
    pub iat: usize, // Issued at
}

impl From<User> for UserResponse {
    fn from(user: User) -> Self {
        UserResponse {
            id: user.id,
            email: user.email,
            username: user.username,
            is_active: user.is_active,
            is_superuser: user.is_superuser,
            is_staff: user.is_staff,
            is_clipper: user.is_clipper,
            created_at: user.created_at,
            subscription_status: user.subscription_status,
            trial_ends_at: user.trial_ends_at,
            subscription_active_until: user.subscription_active_until,
            subscription_tier: user.subscription_tier,
            last_payment_at: user.last_payment_at,
            is_dfy_customer: user.is_dfy_customer,
        }
    }
}
