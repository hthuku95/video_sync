//! Telegram MTProto (Client API / userbot) — monitors configured
//! channels for paid-gig opportunities and pings the admin via the
//! Telegram Bot API with a pre-written custom DM.
//!
//! Architecture:
//!   1. Admin POSTs /api/admin/telegram/login/start with their phone.
//!      We create a grammers Client, request a login code, stash the
//!      Client + LoginToken keyed by phone in a process-wide mutex.
//!   2. Admin receives code, POSTs /api/admin/telegram/login/verify
//!      with phone + code. We sign in, serialize the session, save
//!      to `telegram_sessions` with `authorized = TRUE`.
//!   3. Background worker (`start_watcher`) boots on app start; loads
//!      the most recent authorized session and streams updates.
//!
//! Uses grammers from git master (post-0.8.0) where the sqlite
//! backing is optional. Session state serialized via serde + stored
//! as JSON in the `session_blob` BYTEA column.

use crate::telegram_bot;
use crate::AppState;
use grammers_client::{Client, SignInError};
use grammers_session::MemorySession;
use lazy_static::lazy_static;
use std::collections::HashMap;
use std::sync::Arc;
use tokio::sync::Mutex;

const NOTIFY_SCORE_THRESHOLD: i32 = 60;

const DEFAULT_KEYWORD_REGEX: &str =
    r"(?i)(need|looking\s+for|hiring|paying|budget|pay\s+\$?\d).*(editor|clipper|video|explainer|thumbnail|animation|motion|ugc|mockup|landing\s*page|saas\s*video)";

lazy_static! {
    static ref PENDING_LOGINS: Mutex<HashMap<String, PendingLogin>> = Mutex::new(HashMap::new());
}

struct PendingLogin {
    client: Client,
    token:  grammers_client::client::LoginToken,
    at:     chrono::DateTime<chrono::Utc>,
}

fn api_creds() -> Option<(i32, String)> {
    let id   = std::env::var("TELEGRAM_API_ID").ok()?.parse::<i32>().ok()?;
    let hash = std::env::var("TELEGRAM_API_HASH").ok()?;
    Some((id, hash))
}

async fn connect_client(session: MemorySession) -> Result<Client, String> {
    let (api_id, api_hash) = api_creds()
        .ok_or_else(|| "TELEGRAM_API_ID / TELEGRAM_API_HASH not set".to_string())?;
    Client::connect(grammers_client::client::ClientConfiguration {
        session,
        api_id,
        api_hash: api_hash.clone(),
    })
    .await
    .map_err(|e| format!("Telegram connect failed: {e}"))
}

fn serialize_session(session: &MemorySession) -> Result<Vec<u8>, String> {
    serde_json::to_vec(session).map_err(|e| format!("Session serialize failed: {e}"))
}

fn deserialize_session(bytes: &[u8]) -> Result<MemorySession, String> {
    serde_json::from_slice(bytes).map_err(|e| format!("Session deserialize failed: {e}"))
}

// ────────────────────────────────────────────────────────────────────────────
// Login — two-step phone / code flow
// ────────────────────────────────────────────────────────────────────────────

pub async fn login_start(state: &Arc<AppState>, phone: &str) -> Result<(), String> {
    prune_expired_logins().await;

    let (_, api_hash) = api_creds()
        .ok_or_else(|| "TELEGRAM_API_ID / TELEGRAM_API_HASH not set on server".to_string())?;

    let client = connect_client(MemorySession::default()).await?;

    let token = client.request_login_code(phone, &api_hash).await
        .map_err(|e| format!("request_login_code failed: {e}"))?;

    let _ = sqlx::query(
        "INSERT INTO telegram_sessions (phone, phone_code_hash, authorized)
         VALUES ($1, $2, FALSE)
         ON CONFLICT DO NOTHING"
    )
    .bind(phone)
    .bind("pending")
    .execute(&state.db_pool)
    .await;

    PENDING_LOGINS.lock().await.insert(
        phone.to_string(),
        PendingLogin { client, token, at: chrono::Utc::now() },
    );

    Ok(())
}

pub async fn login_verify(state: &Arc<AppState>, phone: &str, code: &str) -> Result<(), String> {
    let pending = PENDING_LOGINS.lock().await.remove(phone)
        .ok_or_else(|| "No pending login for this phone — start over with /login/start".to_string())?;

    match pending.client.sign_in(&pending.token, code).await {
        Ok(_user) => {}
        Err(SignInError::PasswordRequired(_)) => {
            return Err("Your Telegram account has 2FA enabled. Disable 2FA temporarily, retry, then re-enable.".to_string());
        }
        Err(SignInError::InvalidCode) => {
            return Err("Invalid code. Start over with /login/start.".to_string());
        }
        Err(e) => {
            return Err(format!("sign_in failed: {e}"));
        }
    }

    // Serialize the authenticated session from the live client.
    // client.session() returns the Session impl; we use serde via the
    // grammers-session "serde" feature. If the API drift means .save()
    // is gone, Render's build error will surface the correct method.
    let bytes = {
        let session_ref = pending.client.session();
        serde_json::to_vec(session_ref)
            .map_err(|e| format!("Session serialize failed: {e}"))?
    };

    let _ = sqlx::query("UPDATE telegram_sessions SET authorized = FALSE WHERE authorized = TRUE")
        .execute(&state.db_pool).await;

    sqlx::query(
        "INSERT INTO telegram_sessions (phone, session_blob, authorized, last_poll_at)
         VALUES ($1, $2, TRUE, NULL)"
    )
    .bind(phone)
    .bind(&bytes)
    .execute(&state.db_pool)
    .await
    .map_err(|e| format!("DB insert session failed: {e}"))?;

    Ok(())
}

async fn prune_expired_logins() {
    let cutoff = chrono::Utc::now() - chrono::Duration::minutes(10);
    let mut guard = PENDING_LOGINS.lock().await;
    guard.retain(|_, v| v.at > cutoff);
}

// ────────────────────────────────────────────────────────────────────────────
// Status
// ────────────────────────────────────────────────────────────────────────────

pub async fn status(state: &Arc<AppState>) -> serde_json::Value {
    let row = sqlx::query_as::<_, (i32, String, bool, Option<chrono::DateTime<chrono::Utc>>, Option<String>)>(
        "SELECT id, phone, authorized, last_poll_at, last_error
         FROM telegram_sessions
         ORDER BY created_at DESC LIMIT 1"
    )
    .fetch_optional(&state.db_pool)
    .await
    .ok()
    .flatten();

    match row {
        Some((id, phone, authorized, last_poll, last_error)) => {
            serde_json::json!({
                "authorized":   authorized,
                "phone":        mask_phone(&phone),
                "session_id":   id,
                "last_poll_at": last_poll.map(|t| t.to_rfc3339()),
                "last_error":   last_error,
            })
        }
        None => serde_json::json!({"authorized": false, "message": "No Telegram session yet. POST /api/admin/telegram/login/start to begin."}),
    }
}

fn mask_phone(phone: &str) -> String {
    if phone.len() < 4 { return "***".to_string(); }
    let tail = &phone[phone.len()-4..];
    format!("***{tail}")
}

// ────────────────────────────────────────────────────────────────────────────
// Watcher — streams Telegram updates
// ────────────────────────────────────────────────────────────────────────────

pub async fn start_watcher(state: Arc<AppState>) {
    if api_creds().is_none() {
        tracing::info!("Telegram watcher: TELEGRAM_API_ID / TELEGRAM_API_HASH not set — skipping");
        return;
    }

    let (client, session_id) = loop {
        match load_authorized_session(&state).await {
            Some(pair) => break pair,
            None => {
                tokio::time::sleep(std::time::Duration::from_secs(30)).await;
                continue;
            }
        }
    };

    tracing::info!("Telegram watcher started (session id {})", session_id);

    let regex = regex::Regex::new(DEFAULT_KEYWORD_REGEX)
        .expect("DEFAULT_KEYWORD_REGEX compile failed");

    // Stream-based update loop (master API)
    loop {
        match client.next_update().await {
            Ok(update) => {
                handle_update(&state, &client, &update, &regex).await;
                let _ = sqlx::query(
                    "UPDATE telegram_sessions SET last_poll_at = NOW() WHERE id = $1"
                )
                .bind(session_id)
                .execute(&state.db_pool)
                .await;
            }
            Err(e) => {
                tracing::warn!("Telegram update error: {e} — sleeping 5s");
                let _ = sqlx::query(
                    "UPDATE telegram_sessions SET last_error = $1 WHERE id = $2"
                )
                .bind(e.to_string())
                .bind(session_id)
                .execute(&state.db_pool)
                .await;
                tokio::time::sleep(std::time::Duration::from_secs(5)).await;
            }
        }
    }
}

async fn load_authorized_session(state: &Arc<AppState>) -> Option<(Client, i32)> {
    let row: Option<(i32, Vec<u8>)> = sqlx::query_as(
        "SELECT id, session_blob FROM telegram_sessions
         WHERE authorized = TRUE AND session_blob IS NOT NULL
         ORDER BY created_at DESC LIMIT 1"
    )
    .fetch_optional(&state.db_pool)
    .await
    .ok()
    .flatten();

    let (id, bytes) = row?;
    let session = deserialize_session(&bytes).ok()?;
    let client = connect_client(session).await.ok()?;
    Some((client, id))
}

async fn handle_update(
    state:   &Arc<AppState>,
    _client: &Client,
    update:  &grammers_client::Update,
    regex:   &regex::Regex,
) {
    let msg = match update {
        grammers_client::Update::NewMessage(m) if !m.outgoing() => m,
        _ => return,
    };

    let text = msg.text();
    if text.is_empty() { return; }

    let chat = msg.chat();
    let channel_name = chat.username().map(|s| s.to_string())
        .unwrap_or_else(|| chat.name().to_string());

    if !is_watched(state, &channel_name).await { return; }
    if !regex.is_match(text) { return; }

    let (score, reason, service) = crate::handlers::prospects::score_telegram_opportunity_public(
        state, &channel_name, text,
    ).await;

    let sender = msg.sender().map(|s| s.name().to_string());
    let msg_id = msg.id() as i64;
    let link   = format!("https://t.me/{}/{}", channel_name, msg.id());

    let _ = sqlx::query(
        "INSERT INTO telegram_opportunities
           (channel, message_id, sender, message, matched_kw, link,
            score, score_reason, service_type, status, source)
         VALUES ($1, $2, $3, $4, $5, $6, $7, $8, $9, 'new', 'watcher')
         ON CONFLICT DO NOTHING"
    )
    .bind(&channel_name)
    .bind(msg_id)
    .bind(sender.as_deref())
    .bind(text)
    .bind(regex.as_str())
    .bind(&link)
    .bind(score)
    .bind(&reason)
    .bind(service.as_deref())
    .execute(&state.db_pool)
    .await;

    if score < NOTIFY_SCORE_THRESHOLD { return; }

    let suggested_dm = build_suggested_dm(state, &channel_name, text, &service).await;

    let msg_preview: String = text.chars().take(280).collect();
    let notify_text = format!(
        "🔥 *Telegram opportunity — score {score}/100*\n\
        Channel: @{channel_name}\n\
        From: {sender}\n\
        Service pick: {service_pick}\n\
        Link: {link}\n\n\
        *Original message:*\n`{msg_preview}`\n\n\
        *Suggested DM (copy + send):*\n{dm}",
        score        = score,
        channel_name = channel_name,
        sender       = sender.as_deref().unwrap_or("(unknown)"),
        service_pick = service.as_deref().unwrap_or("unknown"),
        link         = link,
        msg_preview  = msg_preview,
        dm           = suggested_dm.unwrap_or_else(|| "(DM generation failed)".to_string()),
    );
    tokio::spawn(async move { telegram_bot::notify_admin(&notify_text).await; });
}

async fn is_watched(state: &Arc<AppState>, channel: &str) -> bool {
    let n: i64 = sqlx::query_scalar(
        "SELECT COUNT(*) FROM telegram_watch_channels
         WHERE enabled = TRUE AND LOWER(channel) = LOWER($1)"
    )
    .bind(channel)
    .fetch_one(&state.db_pool)
    .await
    .unwrap_or(0);
    n > 0
}

async fn build_suggested_dm(
    state:   &Arc<AppState>,
    channel: &str,
    message: &str,
    service: &Option<String>,
) -> Option<String> {
    let service_hint = service.as_deref().unwrap_or("any of our services");
    let prompt = format!(
        r#"You're writing a cold-outbound Telegram DM in response to this message posted in @{channel}:

"""
{message}
"""

We think the best service fit is: {service_hint}.

Write a 3-4 sentence reply that:
- References a concrete detail from their message (don't be generic)
- Mentions the service type + a rough price range (see below)
- Ends with a specific ask (portfolio link, call, sample turnaround)
- Sounds like one founder messaging another — lowercase ok, no corporate fluff

Service pricing cheat-sheet:
- clipping: $297-$899/mo (30-50 shorts)
- animations: $50-$150 each
- thumbnails: $25-$50 each
- ugc: $200-$500 each
- product_mockup: $100-$300 each
- landing_page: $200-$600 each
- full_stack: $1500-$3000/mo

Output ONLY the DM body."#,
        channel      = channel,
        message      = message.chars().take(400).collect::<String>(),
        service_hint = service_hint,
    );

    crate::llm_utils::generate_text_best_effort(
        state.nvidia_nim_client.as_ref(),
        state.gemma_client.as_ref(),
        state.gemini_client.as_ref(),
        &prompt,
    ).await.ok().map(|s| s.trim().to_string())
}
