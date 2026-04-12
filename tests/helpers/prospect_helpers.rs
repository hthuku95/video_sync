// Prospect Finder & Instagram Leads integration test helpers
// Mirrors admin_helpers.rs patterns but targets production API and persists
// results to the test_runs / test_results tables for admin visibility.

use reqwest::Client;
use serde::{Deserialize, Serialize};
use sqlx::{PgPool, Row};
use uuid::Uuid;

// ─── Auth ──────────────────────────────────────────────────────────────────

#[derive(Deserialize)]
struct LoginResponse {
    pub success: bool,
    pub token: String,
}

/// Ensure the test admin user exists on the target DB, then POST login → JWT.
/// Creates the user + whitelist entry if absent (safe to call repeatedly).
pub async fn ensure_and_login(
    pool: &PgPool,
    base_url: &str,
    email: &str,
    password: &str,
) -> Result<String, String> {
    // Hash the password
    let hash = bcrypt::hash(password, bcrypt::DEFAULT_COST)
        .map_err(|e| format!("bcrypt error: {}", e))?;

    // Whitelist the email
    sqlx::query(
        "INSERT INTO whitelist_emails (email, created_at)
         VALUES ($1, NOW())
         ON CONFLICT (email) DO NOTHING",
    )
    .bind(email)
    .execute(pool)
    .await
    .map_err(|e| format!("whitelist insert error: {}", e))?;

    // Upsert admin user
    sqlx::query(
        "INSERT INTO users
           (email, username, password_hash, is_superuser, is_staff, is_active, created_at, updated_at)
         VALUES ($1, $2, $3, true, true, true, NOW(), NOW())
         ON CONFLICT (email) DO UPDATE
           SET password_hash = EXCLUDED.password_hash,
               is_superuser  = true,
               is_staff      = true,
               is_active     = true,
               updated_at    = NOW()",
    )
    .bind(email)
    .bind(email.split('@').next().unwrap_or("testadmin"))
    .bind(&hash)
    .execute(pool)
    .await
    .map_err(|e| format!("user upsert error: {}", e))?;

    login(base_url, email, password).await
}

/// POST {base_url}/api/auth/login → JWT string
pub async fn login(base_url: &str, email: &str, password: &str) -> Result<String, String> {
    #[derive(Serialize)]
    struct Req<'a> {
        email: &'a str,
        password: &'a str,
    }

    let client = http_client(60)?;
    let res = client
        .post(format!("{}/api/auth/login", base_url))
        .json(&Req { email, password })
        .send()
        .await
        .map_err(|e| format!("Login request failed: {}", e))?;

    let status = res.status();
    if !status.is_success() {
        let body = res.text().await.unwrap_or_default();
        return Err(format!("Login {} — {}", status, body));
    }

    let r: LoginResponse = res
        .json()
        .await
        .map_err(|e| format!("Login JSON parse failed: {}", e))?;

    if !r.success {
        return Err("Login returned success=false".to_string());
    }

    Ok(r.token)
}

// ─── Test Run / Result persistence ─────────────────────────────────────────

/// Insert a new test_run row; return its UUID.
pub async fn create_test_run(pool: &PgPool, name: &str) -> Result<Uuid, String> {
    let id: Uuid = sqlx::query_scalar(
        "INSERT INTO test_runs (name, status, total_tests, passed_tests, failed_tests)
         VALUES ($1, 'running', 0, 0, 0)
         RETURNING id",
    )
    .bind(name)
    .fetch_one(pool)
    .await
    .map_err(|e| format!("create_test_run DB error: {}", e))?;

    Ok(id)
}

/// Insert a test_result row with status='running'; return its UUID.
pub async fn start_test_result(
    pool: &PgPool,
    run_id: Uuid,
    test_name: &str,
    prompt: &str,
) -> Result<Uuid, String> {
    // Bump total_tests on the parent run
    sqlx::query(
        "UPDATE test_runs SET total_tests = total_tests + 1 WHERE id = $1",
    )
    .bind(run_id)
    .execute(pool)
    .await
    .map_err(|e| format!("bump total_tests error: {}", e))?;

    let id: Uuid = sqlx::query_scalar(
        "INSERT INTO test_results
           (run_id, test_name, gig_type, prompt, status)
         VALUES ($1, $2, 'prospect_finder', $3, 'running')
         RETURNING id",
    )
    .bind(run_id)
    .bind(test_name)
    .bind(prompt)
    .fetch_one(pool)
    .await
    .map_err(|e| format!("start_test_result DB error: {}", e))?;

    Ok(id)
}

/// Mark a test_result as passed and store detail text as llm_review_feedback.
pub async fn pass_test_result(pool: &PgPool, result_id: Uuid, run_id: Uuid, detail: &str) {
    let _ = sqlx::query(
        "UPDATE test_results
         SET status='passed', llm_review_feedback=$1, completed_at=NOW()
         WHERE id=$2",
    )
    .bind(detail)
    .bind(result_id)
    .execute(pool)
    .await;

    let _ = sqlx::query(
        "UPDATE test_runs SET passed_tests = passed_tests + 1 WHERE id = $1",
    )
    .bind(run_id)
    .execute(pool)
    .await;
}

/// Mark a test_result as failed and store the error message.
pub async fn fail_test_result(pool: &PgPool, result_id: Uuid, run_id: Uuid, error: &str) {
    let _ = sqlx::query(
        "UPDATE test_results
         SET status='failed', error_message=$1, completed_at=NOW()
         WHERE id=$2",
    )
    .bind(error)
    .bind(result_id)
    .execute(pool)
    .await;

    let _ = sqlx::query(
        "UPDATE test_runs SET failed_tests = failed_tests + 1 WHERE id = $1",
    )
    .bind(run_id)
    .execute(pool)
    .await;
}

/// Close out a test_run: set status to 'completed' or 'failed' depending on
/// whether any test_results have status='failed'.
pub async fn finalize_run(pool: &PgPool, run_id: Uuid) {
    let row = sqlx::query(
        "SELECT passed_tests, failed_tests FROM test_runs WHERE id = $1",
    )
    .bind(run_id)
    .fetch_optional(pool)
    .await
    .ok()
    .flatten();

    if let Some(row) = row {
        let failed: i32 = row.try_get("failed_tests").unwrap_or(0);
        let status = if failed > 0 { "failed" } else { "completed" };
        let _ = sqlx::query(
            "UPDATE test_runs SET status=$1, completed_at=NOW() WHERE id=$2",
        )
        .bind(status)
        .bind(run_id)
        .execute(pool)
        .await;
    }
}

// ─── PhantomBuster job polling ──────────────────────────────────────────────

/// Poll `phantombuster_jobs.status` every 15 s until `completed` / `failed` or timeout.
/// Returns `true` if status reached 'completed', `false` on timeout or failure.
pub async fn poll_phantombuster_job(pool: &PgPool, job_id: Uuid, timeout_secs: u64) -> bool {
    let deadline = tokio::time::Instant::now()
        + tokio::time::Duration::from_secs(timeout_secs);

    loop {
        let status: Option<String> = sqlx::query_scalar(
            "SELECT status FROM phantombuster_jobs WHERE id = $1",
        )
        .bind(job_id)
        .fetch_optional(pool)
        .await
        .ok()
        .flatten();

        match status.as_deref() {
            Some("completed") => return true,
            Some("failed") => {
                eprintln!("PhantomBuster job {} failed", job_id);
                return false;
            }
            _ => {}
        }

        if tokio::time::Instant::now() >= deadline {
            eprintln!(
                "PhantomBuster job {} timed out after {}s (still {:?})",
                job_id, timeout_secs, status
            );
            return false;
        }

        tokio::time::sleep(tokio::time::Duration::from_secs(15)).await;
    }
}

/// Poll multiple PhantomBuster jobs; returns true when all reach 'completed'.
pub async fn poll_phantombuster_jobs_for_run(
    pool: &PgPool,
    pb_job_ids: &[Uuid],
    timeout_secs: u64,
) -> bool {
    let deadline = tokio::time::Instant::now()
        + tokio::time::Duration::from_secs(timeout_secs);

    loop {
        let mut all_done = true;
        let mut any_failed = false;

        for &job_id in pb_job_ids {
            let status: Option<String> = sqlx::query_scalar(
                "SELECT status FROM phantombuster_jobs WHERE id = $1",
            )
            .bind(job_id)
            .fetch_optional(pool)
            .await
            .ok()
            .flatten();

            match status.as_deref() {
                Some("completed") => {}
                Some("failed") => {
                    eprintln!("PhantomBuster job {} failed", job_id);
                    any_failed = true;
                }
                _ => {
                    all_done = false;
                }
            }
        }

        if any_failed { return false; }
        if all_done   { return true;  }

        if tokio::time::Instant::now() >= deadline {
            eprintln!("PhantomBuster jobs timed out after {}s", timeout_secs);
            return false;
        }

        tokio::time::sleep(tokio::time::Duration::from_secs(15)).await;
    }
}

// ─── HTTP client helper ────────────────────────────────────────────────────

pub fn http_client(timeout_secs: u64) -> Result<Client, String> {
    Client::builder()
        .timeout(std::time::Duration::from_secs(timeout_secs))
        .build()
        .map_err(|e| format!("Failed to build HTTP client: {}", e))
}
