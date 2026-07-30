use aws_sdk_sesv2::types::{Body, Content, Destination, EmailContent, Message};
use sqlx::PgPool;
use std::sync::Arc;
use std::time::Duration;

const SES_TIMEOUT: Duration = Duration::from_secs(30);

/// Default sender address. Must be verified in AWS SES.
/// Override with `SES_FROM_EMAIL` env var.
const DEFAULT_FROM: &str = "VideoSync <noreply@videosync.video>";

fn from_address() -> String {
    std::env::var("SES_FROM_EMAIL").unwrap_or_else(|_| DEFAULT_FROM.to_string())
}

/// Build an SESv2 client inline (same pattern as SQS — SDK discovers creds from env).
async fn ses_client() -> Result<aws_sdk_sesv2::Client, String> {
    let config = aws_config::load_from_env().await;
    Ok(aws_sdk_sesv2::Client::new(&config))
}

/// Send a single plain-text email via AWS SESv2 and log the result.
///
/// `to` — recipient email address.
/// `subject` — email subject line.
/// `body` — plain-text body.
/// `db` — optional DB pool; if provided, logs the send to `email_log` table.
/// `prospect_id` — optional prospect FK for the log.
///
/// Returns `Ok((message_id, log_id))` on success. `log_id` is None if no DB given.
pub async fn send_email(
    to: &str,
    subject: &str,
    body: &str,
    db: Option<&PgPool>,
    prospect_id: Option<uuid::Uuid>,
) -> Result<(String, Option<uuid::Uuid>), String> {
    // Pre-insert log entry if DB is available
    let log_id = if let Some(pool) = db {
        let id = uuid::Uuid::new_v4();
        let result = sqlx::query(
            "INSERT INTO email_log (id, prospect_id, to_email, subject, status) \
             VALUES ($1, $2, $3, $4, 'sending')",
        )
        .bind(id)
        .bind(prospect_id)
        .bind(to)
        .bind(subject)
        .execute(pool)
        .await;
        match result {
            Ok(_) => Some(id),
            Err(e) => {
                tracing::warn!("Failed to insert email_log: {e}");
                None
            }
        }
    } else {
        None
    };

    let client = ses_client().await?;

    let dest = Destination::builder().to_addresses(to).build();
    let subject_content = Content::builder()
        .data(subject)
        .charset("UTF-8")
        .build()
        .map_err(|e| format!("Failed to build subject content: {e}"))?;
    let body_content = Content::builder()
        .data(body)
        .charset("UTF-8")
        .build()
        .map_err(|e| format!("Failed to build body content: {e}"))?;
    let msg = Message::builder()
        .subject(subject_content)
        .body(Body::builder().text(body_content).build())
        .build()
        .map_err(|e| format!("Failed to build message: {e}"))?;

    match tokio::time::timeout(SES_TIMEOUT, async {
        client
            .send_email()
            .from_email_address(from_address())
            .destination(dest)
            .content(EmailContent::builder().simple(msg).build())
            .send()
            .await
    })
    .await
    {
        Ok(Ok(response)) => {
            let message_id = response.message_id().unwrap_or("unknown").to_string();
            // Update log entry to 'sent'
            if let (Some(pool), Some(lid)) = (db, log_id) {
                let _ = sqlx::query(
                    "UPDATE email_log SET status = 'sent', message_id = $1 WHERE id = $2",
                )
                .bind(&message_id)
                .bind(lid)
                .execute(pool)
                .await;
            }
            Ok((message_id, log_id))
        }
        Ok(Err(e)) => {
            let err = format!("SES send error: {e}");
            // Update log entry to 'failed'
            if let (Some(pool), Some(lid)) = (db, log_id) {
                let _ = sqlx::query(
                    "UPDATE email_log SET status = 'failed', error_message = $1 WHERE id = $2",
                )
                .bind(&err)
                .bind(lid)
                .execute(pool)
                .await;
            }
            Err(err)
        }
        Err(_) => {
            let err = "SES send timed out".to_string();
            if let (Some(pool), Some(lid)) = (db, log_id) {
                let _ = sqlx::query(
                    "UPDATE email_log SET status = 'failed', error_message = $1 WHERE id = $2",
                )
                .bind(&err)
                .bind(lid)
                .execute(pool)
                .await;
            }
            Err(err)
        }
    }
}

/// Send a personalized email to a prospect and log it.
pub async fn send_prospect_email(
    pool: &PgPool,
    to_email: &str,
    prospect_name: &str,
    email_script: &str,
    prospect_id: uuid::Uuid,
) -> Result<(String, uuid::Uuid), String> {
    let (subject, body) = split_email_script(email_script, prospect_name);
    let (message_id, log_id) = send_email(
        to_email,
        &subject,
        &body,
        Some(pool),
        Some(prospect_id),
    )
    .await?;
    Ok((message_id, log_id.unwrap_or_else(uuid::Uuid::new_v4)))
}

/// Split an email script into subject + body.
/// The first non-empty line is treated as the subject.
fn split_email_script(script: &str, _name: &str) -> (String, String) {
    let trimmed = script.trim();
    if let Some(first_newline) = trimmed.find('\n') {
        let subject = trimmed[..first_newline].trim().to_string();
        let body = trimmed[first_newline..].trim().to_string();
        (subject, body)
    } else {
        ("Your free video sample from VideoSync".to_string(), trimmed.to_string())
    }
}

/// Fetch recent email log entries for the admin dashboard.
pub async fn list_email_logs(
    pool: &PgPool,
    limit: i64,
) -> Result<Vec<serde_json::Value>, String> {
    let rows = sqlx::query_as::<_, (uuid::Uuid, Option<uuid::Uuid>, String, String, String, Option<String>, Option<chrono::DateTime<chrono::Utc>>, chrono::DateTime<chrono::Utc>)>(
        "SELECT id, prospect_id, to_email, subject, status, error_message, opened_at, created_at \
         FROM email_log ORDER BY created_at DESC LIMIT $1",
    )
    .bind(limit)
    .fetch_all(pool)
    .await
    .map_err(|e| format!("Failed to fetch email logs: {e}"))?;

    Ok(rows
        .into_iter()
        .map(
            |(id, prospect_id, to_email, subject, status, error_message, opened_at, created_at)| {
                serde_json::json!({
                    "id": id,
                    "prospect_id": prospect_id,
                    "to_email": to_email,
                    "subject": subject,
                    "status": status,
                    "error_message": error_message,
                    "opened_at": opened_at,
                    "created_at": created_at,
                })
            },
        )
        .collect())
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_split_email_script_with_subject() {
        let script = "Subject: Check out your free sample\n\nHi John,\n\nHere is your video sample.\n\nBest";
        let (subject, body) = split_email_script(script, "John");
        assert_eq!(subject, "Subject: Check out your free sample");
        assert!(body.contains("Hi John"));
    }

    #[test]
    fn test_split_email_script_no_newline() {
        let script = "Just a body with no subject";
        let (subject, body) = split_email_script(script, "Test");
        assert_eq!(subject, "Your free video sample from VideoSync");
        assert_eq!(body, "Just a body with no subject");
    }
}
