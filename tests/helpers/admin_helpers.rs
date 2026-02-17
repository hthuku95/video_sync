// Admin API test helpers
// Provides functions for interacting with admin endpoints

use serde::{Deserialize, Serialize};
use sqlx::{PgPool, Row};
use std::error::Error;

#[derive(Debug, Clone)]
pub struct TestAdmin {
    pub id: i32,
    pub email: String,
    pub username: String,
    pub token: Option<String>,
}

#[derive(Debug, Deserialize)]
pub struct JobsResponse {
    pub success: bool,
    pub jobs: Vec<JobSummary>,
    pub pagination: PaginationInfo,
}

#[derive(Debug, Deserialize)]
pub struct JobSummary {
    pub id: i32,
    pub user_id: i32,
    pub username: String,
    pub linkage_id: i32,
    pub source_channel_name: String,
    pub dest_channel_name: String,
    pub source_video_id: String,
    pub source_video_title: Option<String>,
    pub status: String,
    pub error_message: Option<String>,
    pub retry_count: i32,
    pub created_at: String,
    pub updated_at: String,
}

#[derive(Debug, Deserialize)]
pub struct PaginationInfo {
    pub page: u32,
    pub limit: u32,
    pub total: i64,
    pub total_pages: i64,
}

#[derive(Debug, Deserialize)]
pub struct JobDetailResponse {
    pub success: bool,
    pub job: JobDetail,
}

#[derive(Debug, Deserialize)]
pub struct JobDetail {
    pub id: i32,
    pub linkage_id: i32,
    pub source_video_id: String,
    pub source_video_title: Option<String>,
    pub status: String,
    pub error_message: Option<String>,
    pub retry_count: i32,
    pub created_at: String,
    pub updated_at: String,
}

#[derive(Debug, Deserialize)]
pub struct AdminLoginResponse {
    pub success: bool,
    pub token: String,
    pub user: AdminUser,
}

#[derive(Debug, Deserialize)]
pub struct AdminUser {
    pub id: i32,
    pub email: String,
    pub username: String,
    pub is_superuser: bool,
}

#[derive(Debug, Deserialize)]
pub struct ApiResponse {
    pub success: bool,
    pub message: Option<String>,
}

/// Create a test admin user programmatically
pub async fn create_test_admin(
    pool: &PgPool,
    email: &str,
    username: &str,
    password: &str,
) -> Result<TestAdmin, String> {
    // Hash password
    let password_hash = bcrypt::hash(password, bcrypt::DEFAULT_COST)
        .map_err(|e| format!("Failed to hash password: {}", e))?;

    // Whitelist the email (required by login handler)
    sqlx::query(
        "INSERT INTO whitelist_emails (email, created_at)
         VALUES ($1, NOW())
         ON CONFLICT (email) DO NOTHING"
    )
    .bind(email)
    .execute(pool)
    .await
    .map_err(|e| format!("Failed to whitelist email: {}", e))?;

    // Insert admin user (upsert to handle reruns gracefully)
    let result = sqlx::query(
        "INSERT INTO users (email, username, password_hash, is_superuser, is_staff, is_active, created_at, updated_at)
         VALUES ($1, $2, $3, true, true, true, NOW(), NOW())
         ON CONFLICT (email) DO UPDATE
           SET password_hash = EXCLUDED.password_hash,
               is_superuser = true,
               is_staff = true,
               is_active = true,
               updated_at = NOW()
         RETURNING id, email, username"
    )
    .bind(email)
    .bind(username)
    .bind(&password_hash)
    .fetch_one(pool)
    .await
    .map_err(|e| format!("Failed to create admin user: {}", e))?;

    Ok(TestAdmin {
        id: result.get("id"),
        email: result.get("email"),
        username: result.get("username"),
        token: None,
    })
}

/// Login as admin and get JWT token
pub async fn admin_login(email: &str, password: &str) -> Result<String, String> {
    let client = reqwest::Client::builder()
        .timeout(std::time::Duration::from_secs(30))
        .build()
        .map_err(|e| format!("Failed to build HTTP client: {}", e))?;

    #[derive(Serialize)]
    struct LoginRequest {
        email: String,
        password: String,
    }

    let response = client
        .post("http://127.0.0.1:3000/api/auth/login")
        .json(&LoginRequest {
            email: email.to_string(),
            password: password.to_string(),
        })
        .send()
        .await
        .map_err(|e| format!("Failed to send login request: {}", e))?;

    if !response.status().is_success() {
        let status = response.status();
        let body = response.text().await.unwrap_or_default();
        return Err(format!("Login failed with status {}: {}", status, body));
    }

    let login_response: AdminLoginResponse = response
        .json()
        .await
        .map_err(|e| format!("Failed to parse login response: {}", e))?;

    if !login_response.success {
        return Err("Login failed: success=false".to_string());
    }

    Ok(login_response.token)
}

/// Get all jobs with optional filters
pub async fn get_jobs(
    token: &str,
    status: Option<&str>,
    user_id: Option<i32>,
    page: Option<u32>,
    limit: Option<u32>,
) -> Result<JobsResponse, String> {
    let client = reqwest::Client::builder()
        .timeout(std::time::Duration::from_secs(30))
        .build()
        .map_err(|e| format!("Failed to build HTTP client: {}", e))?;

    let mut url = "http://127.0.0.1:3000/api/admin/clipping/jobs".to_string();
    let mut params = Vec::new();

    if let Some(s) = status {
        params.push(format!("status={}", s));
    }
    if let Some(u) = user_id {
        params.push(format!("user_id={}", u));
    }
    if let Some(p) = page {
        params.push(format!("page={}", p));
    }
    if let Some(l) = limit {
        params.push(format!("limit={}", l));
    }

    if !params.is_empty() {
        url.push('?');
        url.push_str(&params.join("&"));
    }

    let response = client
        .get(&url)
        .header("Authorization", format!("Bearer {}", token))
        .send()
        .await
        .map_err(|e| {
            if e.is_timeout() {
                format!("Request timed out after 30s")
            } else if e.is_connect() {
                format!("Connection failed: {} (is server running on 127.0.0.1:3000?)", e)
            } else if e.is_request() {
                format!("Request error: {}", e)
            } else {
                format!("Failed to get jobs: {} (kind: {:?})", e, e.source())
            }
        })?;

    if !response.status().is_success() {
        let status = response.status();
        let body = response.text().await.unwrap_or_default();
        return Err(format!("Get jobs failed with status {}: {}", status, body));
    }

    let jobs_response: JobsResponse = response
        .json()
        .await
        .map_err(|e| format!("Failed to parse jobs response: {}", e))?;

    Ok(jobs_response)
}

/// Get detailed information about a specific job
pub async fn get_job_details(token: &str, job_id: i32) -> Result<JobDetail, String> {
    let client = reqwest::Client::builder()
        .timeout(std::time::Duration::from_secs(30))
        .build()
        .map_err(|e| format!("Failed to build HTTP client: {}", e))?;

    let url = format!("http://127.0.0.1:3000/api/admin/clipping/jobs/{}", job_id);

    let response = client
        .get(&url)
        .header("Authorization", format!("Bearer {}", token))
        .send()
        .await
        .map_err(|e| format!("Failed to get job details: {}", e))?;

    if !response.status().is_success() {
        let status = response.status();
        let body = response.text().await.unwrap_or_default();
        return Err(format!("Get job details failed with status {}: {}", status, body));
    }

    let detail_response: JobDetailResponse = response
        .json()
        .await
        .map_err(|e| format!("Failed to parse job details: {}", e))?;

    Ok(detail_response.job)
}

/// Cancel a job (admin only)
pub async fn cancel_job(token: &str, job_id: i32) -> Result<(), String> {
    let client = reqwest::Client::builder()
        .timeout(std::time::Duration::from_secs(30))
        .build()
        .map_err(|e| format!("Failed to build HTTP client: {}", e))?;

    let url = format!("http://127.0.0.1:3000/api/admin/clipping/jobs/{}/cancel", job_id);

    let response = client
        .post(&url)
        .header("Authorization", format!("Bearer {}", token))
        .send()
        .await
        .map_err(|e| format!("Failed to cancel job: {}", e))?;

    if !response.status().is_success() {
        let status = response.status();
        let body = response.text().await.unwrap_or_default();
        return Err(format!("Cancel job failed with status {}: {}", status, body));
    }

    let api_response: ApiResponse = response
        .json()
        .await
        .map_err(|e| format!("Failed to parse cancel response: {}", e))?;

    if !api_response.success {
        return Err("Cancel failed: success=false".to_string());
    }

    Ok(())
}

/// Retry a failed job (admin only)
pub async fn retry_job(token: &str, job_id: i32) -> Result<(), String> {
    let client = reqwest::Client::builder()
        .timeout(std::time::Duration::from_secs(30))
        .build()
        .map_err(|e| format!("Failed to build HTTP client: {}", e))?;

    let url = format!("http://127.0.0.1:3000/api/admin/clipping/jobs/{}/retry", job_id);

    let response = client
        .post(&url)
        .header("Authorization", format!("Bearer {}", token))
        .send()
        .await
        .map_err(|e| format!("Failed to retry job: {}", e))?;

    if !response.status().is_success() {
        let status = response.status();
        let body = response.text().await.unwrap_or_default();
        return Err(format!("Retry job failed with status {}: {}", status, body));
    }

    let api_response: ApiResponse = response
        .json()
        .await
        .map_err(|e| format!("Failed to parse retry response: {}", e))?;

    if !api_response.success {
        return Err("Retry failed: success=false".to_string());
    }

    Ok(())
}

/// Delete admin user and whitelist entry (cleanup)
pub async fn delete_test_admin(pool: &PgPool, admin_id: i32) -> Result<(), String> {
    // Get email before deleting user
    let row = sqlx::query("SELECT email FROM users WHERE id = $1")
        .bind(admin_id)
        .fetch_optional(pool)
        .await
        .map_err(|e| format!("Failed to fetch admin email: {}", e))?;

    sqlx::query("DELETE FROM users WHERE id = $1")
        .bind(admin_id)
        .execute(pool)
        .await
        .map_err(|e| format!("Failed to delete admin user: {}", e))?;

    // Also remove from whitelist
    if let Some(row) = row {
        let email: String = row.get("email");
        sqlx::query("DELETE FROM whitelist_emails WHERE email = $1")
            .bind(&email)
            .execute(pool)
            .await
            .map_err(|e| format!("Failed to remove from whitelist: {}", e))?;
    }

    Ok(())
}
