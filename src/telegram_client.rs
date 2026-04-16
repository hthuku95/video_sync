//! Telegram MTProto (Client API / userbot) — monitors configured
//! channels for paid-gig opportunities and pings the admin via the
//! Telegram Bot API with a pre-written custom DM. See telegram_bot.rs
//! for the outbound bot side.
//!
//! Architecture:
//!   1. Admin POSTs /api/admin/telegram/login/start with their phone.
//!      We create a grammers Client, request a login code (Telegram
//!      sends to the phone via SMS / in-app notification), stash the
//!      Client + LoginToken keyed by phone in a process-wide mutex.
//!   2. Admin receives code, POSTs /api/admin/telegram/login/verify
//!      with phone + code. We look up the stashed Client, sign in,
//!      serialize the session bytes, save to `telegram_sessions` row
//!      with `authorized = TRUE`.
//!   3. Background worker (`start_watcher`) boots on app start; loads
//!      the most recent authorized session, subscribes to channels
//!      from `telegram_watch_channels`, iterates `client.next_update()`
//!      and for each incoming message:
//!        - Regex-match against keyword_re (or default regex).
//!        - Score via `score_telegram_opportunity` (existing LLM pass).
//!        - If score ≥ 60, insert into `telegram_opportunities` AND
//!          push a notification to the admin's Telegram via
//!          `telegram_bot::notify_admin` with a pre-generated DM.
//!
//! Uses `grammers-client` 0.8 + `grammers-session` 0.8. Pure Rust
//! MTProto — no C deps, no phone-code step required except the
//! one-time interactive login.

use crate::telegram_bot;
use crate::AppState;
use grammers_client::{Client, Config, InitParams, SignInError};
use grammers_session::Session;
use lazy_static::lazy_static;
use std::collections::HashMap;
use std::sync::Arc;
use tokio::sync::Mutex;

/// Minimum opportunity score (0-100) before we push a Telegram
/// notification to the admin. Matches the IG leads threshold for
/// "top leads".
const NOTIFY_SCORE_THRESHOLD: i32 = 60;

/// Default keyword regex when a channel hasn't set one. Case-insensitive
/// match against combinations like "need editor", "looking for clipper",
/// "paying for animation", "hire UGC", etc.
const DEFAULT_KEYWORD_REGEX: &str =
    r"(?i)(need|looking\s+for|hiring|paying|budget|pay\s+\$?\d).*(editor|clipper|video|explainer|thumbnail|animation|motion|ugc|mockup|landing\s*page|saas\s*video)";

lazy_static! {
    /// In-flight login flows keyed by phone. Holds the grammers Client +
    /// LoginToken between /login/start and /login/verify — these can't
    /// be serialized so we stash the live objects in memory. Expired
    /// entries (stale >10 min) are dropped on each insert.
    static ref PENDING_LOGINS: Mutex<HashMap<String, PendingLogin>> = Mutex::new(HashMap::new());
}

struct PendingLogin {
    client: Client,
    token:  grammers_client::types::LoginToken,
    at:     chrono::DateTime<chrono::Utc>,
}

fn api_creds() -> Option<(i32, String)> {
    let id   = std::env::var("TELEGRAM_API_ID").ok()?.parse::<i32>().ok()?;
    let hash = std::env::var("TELEGRAM_API_HASH").ok()?;
    Some((id, hash))
}

/// Build a fresh Config for a new login attempt.
fn new_config(session: Session) -> Option<Config> {
    let (api_id, api_hash) = api_creds()?;
    Some(Config {
        session,
        api_id,
        api_hash,
        params: InitParams {
            catch_up: true,
            ..Default::default()
        },
    })
}

// ────────────────────────────────────────────────────────────────────────────
// Login — two-step phone / code flow
// ────────────────────────────────────────────────────────────────────────────

/// Step 1: starts the login, triggering Telegram to send a code to the
/// admin's phone / Telegram app. Returns Ok(()) on success; the admin
/// now waits for the code, then POSTs it to /login/verify.
pub async fn login_start(state: &Arc<AppState>, phone: &str) -> Result<(), String> {
    // Clean up any expired in-flight logins (>10 min old).
    prune_expired_logins().await;

    let config = new_config(Session::new())
        .ok_or_else(|| "TELEGRAM_API_ID / TELEGRAM_API_HASH not set on server".to_string())?;

    let client = Client::connect(config).await
        .map_err(|e| format!("Telegram connect failed: {}", e))?;

    let token = client.request_login_code(phone).await
        .map_err(|e| format!("request_login_code failed: {}", e))?;

    // Persist (phone → pending row) so /verify can look it up + we have
    // a record if the admin abandons mid-flow.
    let _ = sqlx::query(
        "INSERT INTO telegram_sessions (phone, phone_code_hash, authorized)
         VALUES ($1, $2, FALSE)
         ON CONFLICT DO NOTHING"
    )
    .bind(phone)
    .bind("pending") // actual hash is inside the in-memory LoginToken
    .execute(&state.db_pool)
    .await;

    PENDING_LOGINS.lock().await.insert(
        phone.to_string(),
        PendingLogin { client, token, at: chrono::Utc::now() },
    );

    Ok(())
}

/// Step 2: completes the login. On success, serializes the session and
/// writes it to `telegram_sessions` with `authorized = TRUE`.
pub async fn login_verify(state: &Arc<AppState>, phone: &str, code: &str) -> Result<(), String> {
    let pending = PENDING_LOGINS.lock().await.remove(phone)
        .ok_or_else(|| "No pending login for this phone — start over with /login/start".to_string())?;

    match pending.client.sign_in(&pending.token, code).await {
        Ok(_user) => {}
        Err(SignInError::PasswordRequired(_)) => {
            // 2FA password is set. Out of scope for MVP — document + error.
            return Err("Your Telegram account has 2FA enabled. 2FA login isn't supported yet — disable 2FA temporarily, retry, then re-enable.".to_string());
        }
        Err(SignInError::InvalidCode) => {
            return Err("Invalid code. Start over with /login/start.".to_string());
        }
        Err(e) => {
            return Err(format!("sign_in failed: {}", e));
        }
    }

    // Serialize + persist the session. Mark old authorized rows obsolete.
    let bytes = pending.client.session().save();

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
    .map_err(|e| format!("DB insert session failed: {}", e))?;

    Ok(())
}

async fn prune_expired_logins() {
    let cutoff = chrono::Utc::now() - chrono::Duration::minutes(10);
    let mut guard = PENDING_LOGINS.lock().await;
    guard.retain(|_, v| v.at > cutoff);
}

// ────────────────────────────────────────────────────────────────────────────
// Authorization status — used by admin UI to decide whether to show
// the login form or the "watcher active" panel
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
    format!("***{}", tail)
}

// ────────────────────────────────────────────────────────────────────────────
// Watcher — polls Telegram updates + routes matching messages into
// telegram_opportunities and the admin Telegram bot
// ────────────────────────────────────────────────────────────────────────────

/// Main entry: spawned once on app startup from main.rs. Loads the
/// most recent authorized session (if any) and runs the update loop.
/// If no session exists or the session is invalid, logs + exits — the
/// admin can re-run login from the UI, then restart the app (or we'll
/// add a reload endpoint later).
pub async fn start_watcher(state: Arc<AppState>) {
    if api_creds().is_none() {
        tracing::info!("Telegram watcher: TELEGRAM_API_ID / TELEGRAM_API_HASH not set — skipping");
        return;
    }

    // Wait until the admin has logged in. Poll DB every 30s.
    let (client, session_id) = loop {
        match load_authorized_session(&state).await {
            Some((client, id)) => break (client, id),
            None => {
                tokio::time::sleep(std::time::Duration::from_secs(30)).await;
                continue;
            }
        }
    };

    tracing::info!("✈️ Telegram watcher started (session id {})", session_id);

    // Preload watched channels. Refreshed opportunistically when `next_update`
    // returns updates from unknown chats.
    let regex = regex::Regex::new(DEFAULT_KEYWORD_REGEX)
        .expect("DEFAULT_KEYWORD_REGEX compile failed — fix the constant");

    loop {
        match client.next_update().await {
            Ok(Some(update)) => {
                handle_update(&state, &client, &update, &regex).await;
                let _ = sqlx::query(
                    "UPDATE telegram_sessions SET last_poll_at = NOW() WHERE id = $1"
                )
                .bind(session_id)
                .execute(&state.db_pool)
                .await;
            }
            Ok(None) => {
                tokio::time::sleep(std::time::Duration::from_secs(1)).await;
            }
            Err(e) => {
                tracing::warn!("Telegram next_update error: {} — sleeping 5s", e);
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
    let session = Session::load(&bytes).ok()?;
    let config = new_config(session)?;
    let client = Client::connect(config).await.ok()?;
    Some((client, id))
}

async fn handle_update(
    state:   &Arc<AppState>,
    _client: &Client,
    update:  &grammers_client::Update,
    regex:   &regex::Regex,
) {
    // Only act on new messages in channels we watch.
    let msg = match update {
        grammers_client::Update::NewMessage(m) if !m.outgoing() => m,
        _ => return,
    };

    let text = msg.text();
    if text.is_empty() { return; }

    let chat = msg.chat();
    let channel_name = chat.username().map(|s| s.to_string())
        .unwrap_or_else(|| chat.name().to_string());

    // Skip channels not in our watch list.
    let watched = is_watched(state, &channel_name).await;
    if !watched { return; }

    if !regex.is_match(text) {
        return;
    }

    // AI score the opportunity via existing helper.
    let (score, reason, service) = crate::handlers::prospects::score_telegram_opportunity_public(
        state, &channel_name, text,
    ).await;

    // Persist even low-score hits for admin review — they can still be
    // useful to know about even if AI doesn't see a perfect fit.
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

    // Only ping the admin for high-confidence matches so the bot isn't noisy.
    if score < NOTIFY_SCORE_THRESHOLD { return; }

    // Build a pre-written custom DM the admin can copy-paste. Reuses the
    // same LLM + service pitch machinery as the IG lead DM generator.
    let suggested_dm = build_suggested_dm(state, &channel_name, text, &service).await;

    let msg_preview: String = text.chars().take(280).collect();
    let notify_text = format!(
        "🔥 *Telegram opportunity — score {}/100*\n\
        Channel: @{}\n\
        From: {}\n\
        Service pick: {}\n\
        Link: {}\n\n\
        *Original message:*\n`{}`\n\n\
        *Suggested DM (copy + send):*\n{}",
        score,
        channel_name,
        sender.as_deref().unwrap_or("(unknown)"),
        service.as_deref().unwrap_or("unknown"),
        link,
        msg_preview,
        suggested_dm.unwrap_or_else(|| "(DM generation failed — write one manually)".to_string()),
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
