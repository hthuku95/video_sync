use aws_sdk_sesv2::types::{Body, Content, Destination, EmailContent, Message};
use sqlx::PgPool;
use std::time::Duration;

const SES_TIMEOUT: Duration = Duration::from_secs(30);

/// Default sender address. Must be verified in AWS SES.
/// Override with `SES_FROM_EMAIL` env var.
const DEFAULT_FROM: &str = "VideoSync <noreply@videosync.ink>";

fn from_address() -> String {
    std::env::var("SES_FROM_EMAIL").unwrap_or_else(|_| DEFAULT_FROM.to_string())
}

/// Named sender profiles (owner directive Oct 2026 — the app sends from
/// multiple @videosync.ink addresses; SES is verified for the whole domain).
/// Prospect-facing mail uses support@ so replies land in the monitored inbox.
pub const SUPPORT_FROM: &str = "VideoSync <support@videosync.ink>";
pub const HARRY_FROM: &str = "Harry Thuku <harry@videosync.ink>";

pub fn sender_for(profile: &str) -> String {
    match profile {
        "support" => SUPPORT_FROM.to_string(),
        "harry" => HARRY_FROM.to_string(),
        _ => from_address(),
    }
}

/// Determine which email provider to use. Defaults to Resend if RESEND_API_KEY is
/// set, otherwise falls back to SES. Override with EMAIL_PROVIDER env var
/// ("resend" | "ses").
fn email_provider() -> &'static str {
    match std::env::var("EMAIL_PROVIDER").as_deref() {
        Ok("ses") => "ses",
        Ok("resend") => "resend",
        _ => {
            if std::env::var("RESEND_API_KEY").is_ok() {
                "resend"
            } else {
                "ses"
            }
        }
    }
}

/// Build an SESv2 client inline (same pattern as SQS — SDK discovers creds from env).
async fn ses_client() -> Result<aws_sdk_sesv2::Client, String> {
    let config = aws_config::load_from_env().await;
    Ok(aws_sdk_sesv2::Client::new(&config))
}

/// Send a single email via Resend (https://resend.com). Returns message_id.
async fn resend_send(
    to: &str,
    subject: &str,
    body: &str,
    html: Option<&str>,
    from_addr: &str,
) -> Result<String, String> {
    let api_key = std::env::var("RESEND_API_KEY")
        .map_err(|_| "RESEND_API_KEY not set".to_string())?;
    let client = reqwest::Client::builder()
        .timeout(std::time::Duration::from_secs(30))
        .build()
        .unwrap_or_default();
    let mut payload = serde_json::json!({
        "from": from_addr,
        "to": [to],
        "subject": subject,
        "text": body,
    });
    if let Some(h) = html {
        payload["html"] = serde_json::Value::String(h.to_string());
    }
    let resp = client
        .post("https://api.resend.com/emails")
        .bearer_auth(&api_key)
        .json(&payload)
        .send()
        .await
        .map_err(|e| format!("Resend request failed: {e}"))?;
    if !resp.status().is_success() {
        let status = resp.status();
        let body = resp.text().await.unwrap_or_default();
        return Err(format!("Resend error {status}: {body}"));
    }
    let v: serde_json::Value = resp.json().await.map_err(|e| format!("Resend parse failed: {e}"))?;
    Ok(v.get("id").and_then(|x| x.as_str()).unwrap_or("resend-unknown").to_string())
}

/// Send a single plain-text email via AWS SESv2 and log the result.
///
/// `to` — recipient email address.
/// `subject` — email subject line.
/// `body` — plain-text body.
/// `db` — optional DB pool; if provided, logs the send to `email_log` table.
/// `prospect_id` — optional prospect FK for the log.
/// `from` — optional sender profile ("support") or full address; defaults to
///   `SES_FROM_EMAIL`. Any @videosync.ink address works (domain verified).
///
/// Returns `Ok((message_id, log_id))` on success. `log_id` is None if no DB given.
pub async fn send_email(
    to: &str,
    subject: &str,
    body: &str,
    html: Option<&str>,
    db: Option<&PgPool>,
    prospect_id: Option<uuid::Uuid>,
    from: Option<&str>,
) -> Result<(String, Option<uuid::Uuid>), String> {
    let from_addr = match from {
        Some("support") => SUPPORT_FROM.to_string(),
        Some(addr) => addr.to_string(),
        None => from_address(),
    };
    // Pre-insert log entry if DB is available
    let log_id = if let Some(pool) = db {
        let id = uuid::Uuid::new_v4();
        let result = sqlx::query(
            "INSERT INTO email_log (id, prospect_id, to_email, subject, status, from_email) \
             VALUES ($1, $2, $3, $4, 'sending', $5)",
        )
        .bind(id)
        .bind(prospect_id)
        .bind(to)
        .bind(subject)
        .bind(&from_addr)
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

    let provider = email_provider();
    let send_result: Result<String, String> = if provider == "resend" {
        resend_send(to, subject, body, html, &from_addr).await
    } else {
        // SES path
        let client = ses_client().await?;
        let dest = Destination::builder().to_addresses(to).build();
        let subject_content = Content::builder()
            .data(subject)
            .charset("UTF-8")
            .build()
            .map_err(|e| format!("Failed to build subject content: {e}"))?;
        let text_content = Content::builder()
            .data(body)
            .charset("UTF-8")
            .build()
            .map_err(|e| format!("Failed to build body content: {e}"))?;
        let mut body_builder = Body::builder().text(text_content);
        if let Some(html_body) = html {
            let html_content = Content::builder()
                .data(html_body)
                .charset("UTF-8")
                .build()
                .map_err(|e| format!("Failed to build html content: {e}"))?;
            body_builder = body_builder.html(html_content);
        }
        let body = body_builder.build();
        let msg = Message::builder()
            .subject(subject_content)
            .body(body)
            .build();
        match tokio::time::timeout(SES_TIMEOUT, async {
            client
                .send_email()
                .from_email_address(&from_addr)
                .destination(dest)
                .content(EmailContent::builder().simple(msg).build())
                .send()
                .await
        })
        .await
        {
            Ok(Ok(response)) => Ok(response.message_id().unwrap_or("unknown").to_string()),
            Ok(Err(e)) => Err(format!("SES send error: {e}")),
            Err(_) => Err("SES send timed out".to_string()),
        }
    };

    match send_result {
        Ok(message_id) => {
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
        Err(err) => {
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

/// Send a personalized email to a prospect and log it.
pub async fn send_prospect_email(
    pool: &PgPool,
    to_email: &str,
    prospect_name: &str,
    email_script: &str,
    prospect_id: uuid::Uuid,
) -> Result<(String, uuid::Uuid), String> {
    let (subject, body) = split_email_script(email_script, prospect_name);
    let html = build_prospect_html_email(&subject, &body);
    let (message_id, log_id) = send_email(
        to_email,
        &subject,
        &body,
        Some(&html),
        Some(pool),
        Some(prospect_id),
        Some("support"),
    )
    .await?;
    Ok((message_id, log_id.unwrap_or_else(uuid::Uuid::new_v4)))
}

/// Build a professional HTML email body using the VideoSync brand palette
/// (indigo/blue accent, light card, footer with contact + privacy/terms links).
fn build_prospect_html_email(subject: &str, body: &str) -> String {
    let body_html = body
        .lines()
        .map(|line| {
            let escaped = line
                .replace('&', "&amp;")
                .replace('<', "&lt;")
                .replace('>', "&gt;");
            if escaped.trim().is_empty() {
                "<br>".to_string()
            } else {
                format!("<p style=\"margin:0 0 12px 0;line-height:1.6;color:#1f2937;\">{}</p>", escaped)
            }
        })
        .collect::<Vec<_>>()
        .join("");
    format!(
        "<!DOCTYPE html><html lang=\"en\"><head><meta charset=\"UTF-8\"><meta name=\"viewport\" content=\"width=device-width,initial-scale=1.0\"><title>{subject}</title></head><body style=\"margin:0;padding:0;background:#f3f4f6;font-family:-apple-system,BlinkMacSystemFont,'Segoe UI',Roboto,sans-serif;\"><table role=\"presentation\" width=\"100%\" cellpadding=\"0\" cellspacing=\"0\" style=\"background:#f3f4f6;padding:32px 16px;\"><tr><td align=\"center\"><table role=\"presentation\" width=\"600\" cellpadding=\"0\" cellspacing=\"0\" style=\"max-width:600px;background:#ffffff;border-radius:12px;overflow:hidden;box-shadow:0 4px 24px rgba(0,0,0,0.06);\"><tr><td style=\"background:linear-gradient(135deg,#3b82f6,#6366f1);padding:24px 32px;\"><h1 style=\"margin:0;color:#ffffff;font-size:20px;font-weight:700;\">VideoSync</h1><p style=\"margin:4px 0 0;color:rgba(255,255,255,0.85);font-size:13px;\">AI video production for your content</p></td></tr><tr><td style=\"padding:32px;\">{body_html}</td></tr><tr><td style=\"padding:20px 32px;border-top:1px solid #e5e7eb;background:#f9fafb;\"><p style=\"margin:0 0 8px;font-size:12px;color:#6b7280;\">Questions? Reply to this email or reach us at <a href=\"mailto:support@videosync.ink\" style=\"color:#3b82f6;text-decoration:none;\">support@videosync.ink</a>.</p><p style=\"margin:0;font-size:11px;color:#9ca3af;\">&copy; VideoSync &middot; <a href=\"https://videosync.ink/privacy\" style=\"color:#9ca3af;text-decoration:underline;\">Privacy Policy</a> &middot; <a href=\"https://videosync.ink/terms\" style=\"color:#9ca3af;text-decoration:underline;\">Terms</a></p></td></tr></table></td></tr></table></body></html>",
        body_html = body_html,
    )
}

/// Fetch recent email log entries for the admin dashboard.
pub async fn list_email_logs(
    pool: &PgPool,
    limit: i64,
) -> Result<Vec<serde_json::Value>, String> {
    let rows = sqlx::query_as::<_, (uuid::Uuid, Option<uuid::Uuid>, String, String, String, Option<String>, Option<chrono::DateTime<chrono::Utc>>, chrono::DateTime<chrono::Utc>, Option<String>)>(
        "SELECT id, prospect_id, to_email, subject, status, error_message, opened_at, created_at, from_email \
         FROM email_log ORDER BY created_at DESC LIMIT $1",
    )
    .bind(limit)
    .fetch_all(pool)
    .await
    .map_err(|e| format!("Failed to fetch email logs: {e}"))?;

    Ok(rows
        .into_iter()
        .map(
            |(id, prospect_id, to_email, subject, status, error_message, opened_at, created_at, from_email)| {
                serde_json::json!({
                    "id": id,
                    "prospect_id": prospect_id,
                    "to_email": to_email,
                    "subject": subject,
                    "status": status,
                    "error_message": error_message,
                    "opened_at": opened_at,
                    "created_at": created_at,
                    "from_email": from_email,
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
