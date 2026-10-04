//! YouTube cookie health monitoring (owner directive Oct 2026).
//!
//! Cookie exports rot — Google rotates sessions, faster with continued browser
//! use and foreign-IP reuse. When `YTDLP_COOKIES_B64` goes stale, every YouTube
//! download fails and renders die. This module watches two signals and emails
//! the admin (once per cooldown) instead of letting failures pile up silently:
//!   * failure bursts: `note_download_failure` counts consecutive download
//!     failures; at threshold it emails and stamps `last_reminded_at`.
//!     `note_download_success` resets the counter.
//!   * age: `check_cookie_age` (called from the campaign worker tick) emails
//!     when the deployed cookies are older than the max age.
//! All sends are elected via atomic `UPDATE ... RETURNING`, so N Fargate tasks
//! never double-email. Env knobs (all optional, sane defaults):
//!   `COOKIE_FAIL_THRESHOLD` (3), `COOKIE_REMIND_COOLDOWN_HOURS` (24),
//!   `COOKIE_MAX_AGE_DAYS` (14), `COOKIE_AGE_REMIND_DAYS` (7),
//!   `ADMIN_NOTIFY_EMAIL` (else first superuser email).


fn env_usize(name: &str, default: usize) -> usize {
    std::env::var(name)
        .ok()
        .and_then(|v| v.trim().parse::<usize>().ok())
        .unwrap_or(default)
}

/// Resolve the admin notification address: explicit env first, else the
/// oldest superuser account. Returns None when nobody can be notified.
pub async fn resolve_admin_email(pool: &sqlx::PgPool) -> Option<String> {
    if let Ok(addr) = std::env::var("ADMIN_NOTIFY_EMAIL") {
        let addr = addr.trim().to_string();
        if !addr.is_empty() {
            return Some(addr);
        }
    }
    sqlx::query_scalar::<_, Option<String>>(
        "SELECT email FROM users WHERE is_superuser = TRUE ORDER BY id ASC LIMIT 1",
    )
    .fetch_optional(pool)
    .await
    .ok()
    .flatten()
    .flatten()
    .filter(|e| !e.trim().is_empty())
}

async fn send_cookie_alert(
    pool: &sqlx::PgPool,
    subject: String,
    body: String,
) -> bool {
    let Some(to) = resolve_admin_email(pool).await else {
        tracing::warn!("🍪 cookie alert suppressed: no admin email resolvable");
        return false;
    };
    match crate::services::email_service::send_email(&to, &subject, &body, Some(pool), None).await
    {
        Ok(_) => {
            tracing::info!("🍪 cookie alert emailed to {}", to);
            true
        }
        Err(e) => {
            tracing::warn!("🍪 cookie alert email failed: {}", e);
            false
        }
    }
}

fn alert_body(kind: &str, detail: String) -> String {
    format!(
        "Hi,\n\nVideoSync's YouTube downloads are failing ({kind}).\n\n{detail}\n\n\
         The `YTDLP_COOKIES_B64` session in S3 has likely gone stale — Google rotates \
         session cookies, faster with continued browser use.\n\n\
         To fix (5 min):\n\
         1. In your browser, log OUT of YouTube/Google and log back IN (mints a fresh session).\n\
         2. Immediately export cookies (youtube.com scope) and send the file over.\n\
         3. It will be pushed to S3 and the media fleet refreshed; renders resume.\n\n\
         This is an automated reminder from your VideoSync API (throttled to avoid spam).\n"
    )
}

/// Call when an R2 source download succeeds — clears the failure streak.
pub async fn note_download_success(pool: &sqlx::PgPool) {
    let _ = sqlx::query(
        "UPDATE ytdlp_cookie_health SET consecutive_failures = 0 WHERE id = 1",
    )
    .execute(pool)
    .await;
}

/// Call when the download pipeline fails. At threshold (default 3 consecutive)
/// with cooldown expired (default 24h), emails the admin once.
pub async fn note_download_failure(pool: &sqlx::PgPool, error_summary: &str) {
    let threshold = env_usize("COOKIE_FAIL_THRESHOLD", 3).max(1);
    let cooldown_h = env_usize("COOKIE_REMIND_COOLDOWN_HOURS", 24).max(1) as i64;
    let row: Option<(i32, Option<chrono::DateTime<chrono::Utc>>)> = sqlx::query_as(
        "UPDATE ytdlp_cookie_health \
         SET consecutive_failures = consecutive_failures + 1 \
         WHERE id = 1 \
         RETURNING consecutive_failures, last_reminded_at",
    )
    .fetch_optional(pool)
    .await
    .ok()
    .flatten();
    let Some((count, last_reminded)) = row else { return };
    if count < threshold as i32 {
        return;
    }
    let cooldown_ok = match last_reminded {
        None => true,
        Some(t) => {
            chrono::Utc::now().signed_duration_since(t) > chrono::Duration::hours(cooldown_h)
        }
    };
    if !cooldown_ok {
        return;
    }
    // Elect a single sender across tasks: only the row-winner emails.
    let won: Option<bool> = sqlx::query_scalar(
        "UPDATE ytdlp_cookie_health \
         SET last_reminded_at = NOW() \
         WHERE id = 1 AND (last_reminded_at IS NULL OR last_reminded_at < NOW() - make_interval(hours => $1)) \
         RETURNING TRUE",
    )
    .bind(cooldown_h as i32)
    .fetch_optional(pool)
    .await
    .ok()
    .flatten();
    if won != Some(true) {
        return;
    }
    let _ = send_cookie_alert(
        pool,
        "VideoSync: YouTube downloads failing — cookies may need refresh".to_string(),
        alert_body(
            "failure burst",
            format!(
                "{} consecutive download-pipeline failures (latest: {}).",
                count,
                error_summary.chars().take(300).collect::<String>()
            ),
        ),
    )
    .await;
    // Small backoff so the counter doesn't re-trigger instantly on the next
    // failure while the admin is still reacting; successes reset it anyway.
    let _ = sqlx::query(
        "UPDATE ytdlp_cookie_health SET consecutive_failures = 0 WHERE id = 1",
    )
    .execute(pool)
    .await;
}

/// Daily age check — call from a periodic worker tick. Emails when the
/// deployed cookies are older than the max age (default 14 days), throttled
/// to one reminder per 7 days. No-op when never deployed (nothing to age).
pub async fn check_cookie_age(pool: &sqlx::PgPool) {
    let max_age_d = env_usize("COOKIE_MAX_AGE_DAYS", 14).max(1) as i64;
    let cooldown_d = env_usize("COOKIE_AGE_REMIND_DAYS", 7).max(1) as i64;
    let row: Option<(Option<chrono::DateTime<chrono::Utc>>, Option<chrono::DateTime<chrono::Utc>>)> =
        sqlx::query_as(
            "SELECT last_deployed_at, last_reminded_at FROM ytdlp_cookie_health WHERE id = 1",
        )
        .fetch_optional(pool)
        .await
        .ok()
        .flatten();
    let Some((Some(deployed), last_reminded)) = row else {
        return;
    };
    if chrono::Utc::now().signed_duration_since(deployed)
        <= chrono::Duration::hours(max_age_d * 24)
    {
        return;
    }
    let cooldown_ok = match last_reminded {
        None => true,
        Some(t) => {
            chrono::Utc::now().signed_duration_since(t)
                > chrono::Duration::hours(cooldown_d * 24)
        }
    };
    if !cooldown_ok {
        return;
    }
    let won: Option<bool> = sqlx::query_scalar(
        "UPDATE ytdlp_cookie_health \
         SET last_reminded_at = NOW() \
         WHERE id = 1 AND (last_reminded_at IS NULL OR last_reminded_at < NOW() - make_interval(hours => $1)) \
         RETURNING TRUE",
    )
    .bind((cooldown_d * 24) as i32)
    .fetch_optional(pool)
    .await
    .ok()
    .flatten();
    if won != Some(true) {
        return;
    }
    let age_days =
        chrono::Utc::now().signed_duration_since(deployed).num_days();
    let _ = send_cookie_alert(
        pool,
        "VideoSync: YouTube cookies are getting old — consider refreshing".to_string(),
        alert_body(
            "age",
            format!(
                "Current cookies were deployed {} days ago (older than the {}-day freshness window). \
                 Downloads still work, but rotation risk grows daily.",
                age_days, max_age_d
            ),
        ),
    )
    .await;
}

/// Stamp a rotation (called by the admin endpoint after pushing fresh cookies
/// to S3 + refreshing the fleet). Resets failures and the age clock.
pub async fn record_rotation(pool: &sqlx::PgPool, by: &str) -> bool {
    sqlx::query(
        "UPDATE ytdlp_cookie_health \
         SET last_deployed_at = NOW(), deployed_by = $1, consecutive_failures = 0 \
         WHERE id = 1",
    )
    .bind(by)
    .execute(pool)
    .await
    .map(|r| r.rows_affected() > 0)
    .unwrap_or(false)
}
