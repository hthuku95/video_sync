use lettre::{
    message::Mailbox,
    transport::smtp::authentication::Credentials,
    AsyncSmtpTransport, AsyncTransport, Message, Tokio1Executor,
};

fn smtp_config() -> Option<(String, u16, String, String)> {
    let host = std::env::var("SMTP_HOST").ok().filter(|s| !s.is_empty())?;
    let port: u16 = std::env::var("SMTP_PORT")
        .ok()
        .and_then(|s| s.parse().ok())
        .unwrap_or(587);
    let username = std::env::var("SMTP_USERNAME").ok().filter(|s| !s.is_empty())?;
    let password = std::env::var("SMTP_PASSWORD").ok().filter(|s| !s.is_empty())?;
    Some((host, port, username, password))
}

fn notification_email() -> Option<String> {
    std::env::var("NOTIFICATION_EMAIL")
        .ok()
        .filter(|s| !s.is_empty())
}

fn smtp_enabled() -> bool {
    smtp_config().is_some() && notification_email().is_some()
}

/// Send a plain-text email to the configured NOTIFICATION_EMAIL.
/// No-op if SMTP env vars are not set — safe to call from any handler.
pub async fn notify_admin(subject: &str, body: &str) {
    let (host, port, username, password) = match smtp_config() {
        Some(c) => c,
        None => return,
    };
    let to_addr = match notification_email() {
        Some(a) => a,
        None => return,
    };
    let from_addr = username.clone();

    let mailbox_from: Mailbox = match from_addr.parse() {
        Ok(m) => m,
        Err(_) => return,
    };
    let mailbox_to: Mailbox = match to_addr.parse() {
        Ok(m) => m,
        Err(_) => return,
    };

    let message = match Message::builder()
        .from(mailbox_from)
        .to(mailbox_to)
        .subject(subject)
        .body(body.to_string())
    {
        Ok(m) => m,
        Err(e) => {
            tracing::warn!("Failed to build email: {}", e);
            return;
        }
    };

    let creds = Credentials::new(username, password);

    let mailer: AsyncSmtpTransport<Tokio1Executor> = match AsyncSmtpTransport::<Tokio1Executor>::relay(&host) {
        Ok(b) => b.credentials(creds).port(port).build(),
        Err(e) => {
            tracing::warn!("Failed to build SMTP transport: {}", e);
            return;
        }
    };

    match mailer.send(message).await {
        Ok(_) => tracing::info!("Payment notification email sent to {}", to_addr),
        Err(e) => tracing::warn!("Failed to send payment notification email: {}", e),
    }
}
