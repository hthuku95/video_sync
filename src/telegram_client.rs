//! Telegram MTProto (Client API / userbot) — monitors configured
//! channels for paid-gig opportunities and pings the admin via the
//! Telegram Bot API with a pre-written custom DM.
//!
//! Uses grammers-client 0.9 (SenderPool + UpdateStream pattern).
//!
//! Flow:
//!   1. Admin POSTs /admin/prospect-finder/telegram/login/start  → login_start()
//!   2. Telegram sends a code to the phone
//!   3. Admin POSTs /admin/prospect-finder/telegram/login/verify → login_verify()
//!   4. Session bytes are persisted in Postgres (telegram_sessions)
//!   5. start_watcher() runs as a background task, restores session from DB,
//!      connects, streams channel updates, AI-scores them, inserts matches.

use crate::AppState;
use futures::StreamExt;
use std::collections::HashMap;
use std::sync::Arc;

use grammers_client::client::UpdatesConfiguration;
use grammers_client::update::Update;
use grammers_client::{tl, Client, SenderPool, SignInError};
use grammers_session::storages::SqliteSession;
use regex::Regex;

// ---------------------------------------------------------------------------
// Constants
// ---------------------------------------------------------------------------

/// Default keywords that indicate a paid-gig opportunity.
const DEFAULT_KEYWORD_REGEX: &str = r"(?i)(need\s+(a\s+)?(video|editor|clipping|clips|thumbnail|animation|ugc|content\s+creator)|hiring\s+(video|editor|freelanc)|looking\s+for\s+(a\s+)?(video|editor|freelanc)|pay(ing)?\s+(in\s+)?(usdc|usd|crypto|\$)|budget\s*[:=]\s*\$?\d|dm\s+me\s+(for|your)\s+(rate|portfolio)|freelance\s+(video|editor|gig))";

/// Minimum AI score to notify admin.
const NOTIFY_SCORE_THRESHOLD: i32 = 60;

#[derive(Debug, Clone)]
pub struct TelegramDiscoveredChannel {
    pub query: String,
    pub channel_id: Option<i64>,
    pub username: Option<String>,
    pub title: String,
    pub is_broadcast: bool,
    pub is_megagroup: bool,
    pub participants_count: Option<i32>,
}

// ---------------------------------------------------------------------------
// Pending login state (held between start and verify)
// ---------------------------------------------------------------------------

/// Stashed between login_start and login_verify.
struct PendingLogin {
    created_at: std::time::Instant,
}

lazy_static::lazy_static! {
    /// Keyed by phone number.  Holds just the session file path so we can
    /// re-open the SqliteSession on verify.  grammers Client is NOT Send so
    /// we recreate it in verify rather than trying to store it.
    static ref PENDING_LOGINS: std::sync::Mutex<HashMap<String, PendingLogin>> =
        std::sync::Mutex::new(HashMap::new());
}

// ---------------------------------------------------------------------------
// Helpers
// ---------------------------------------------------------------------------

fn api_id() -> Result<i32, String> {
    std::env::var("TELEGRAM_API_ID")
        .map_err(|_| "TELEGRAM_API_ID env var not set".to_string())?
        .parse::<i32>()
        .map_err(|e| format!("TELEGRAM_API_ID is not a valid i32: {e}"))
}

fn api_hash() -> Result<String, String> {
    std::env::var("TELEGRAM_API_HASH").map_err(|_| "TELEGRAM_API_HASH env var not set".to_string())
}

/// Derive a stable session file path from the phone number.
fn session_path(phone: &str) -> String {
    use sha2::{Digest, Sha256};
    let hash = hex::encode(Sha256::digest(phone.as_bytes()));
    format!("/tmp/videosync_tg_{}.session", &hash[..16])
}

fn mask_phone(phone: &str) -> String {
    if phone.len() < 4 {
        return "***".to_string();
    }
    let tail = &phone[phone.len() - 4..];
    format!("***{tail}")
}

// ---------------------------------------------------------------------------
// login_start
// ---------------------------------------------------------------------------

pub async fn login_start(state: &Arc<AppState>, phone: &str) -> Result<(), String> {
    let id = api_id()?;
    let hash = api_hash()?;
    let path = session_path(phone);

    // Remove stale session file if present so we get a fresh auth handshake.
    let _ = tokio::fs::remove_file(&path).await;

    let session = Arc::new(
        SqliteSession::open(&path)
            .await
            .map_err(|e| format!("session open: {e}"))?,
    );

    let pool = SenderPool::new(session, id);

    // Runner must be spawned before any API call.
    let runner = pool.runner;
    tokio::spawn(async move { runner.run().await });

    let client = Client::new(pool.handle);

    let token = client
        .request_login_code(phone, &hash)
        .await
        .map_err(|e| format!("request_login_code: {e}"))?;

    // Persist the phone_code_hash from the token so we can recreate a
    // LoginToken on verify.  grammers LoginToken is not easily stored, but
    // we can serialize the phone_code_hash string that Telegram sends back.
    // Unfortunately LoginToken doesn't expose that field — we work around
    // it by keeping the session file alive (which stores the auth key) and
    // stashing a PendingLogin.

    // Upsert a pending row in DB so the UI can show "code sent" state.
    sqlx::query(
        "INSERT INTO telegram_sessions (phone, authorized, created_at)
         VALUES ($1, FALSE, NOW())
         ON CONFLICT DO NOTHING",
    )
    .bind(phone)
    .execute(&state.db_pool)
    .await
    .map_err(|e| format!("db insert: {e}"))?;

    // Stash for verify.  We can only hold the session path — Client is !Send
    // in some builds and LoginToken has no public serialization.  We'll
    // recreate the client from the session file in verify, but we ALSO need
    // to keep the current client alive so the auth key stays valid.  The
    // trick: we leak the client into a Box that we drop on verify.
    //
    // Actually, a simpler approach: we serialize the LoginToken via
    // serde if it implements it.  Since it may not, we instead keep the
    // entire tokio task alive by storing a oneshot sender.  On verify,
    // we send the code through the channel.
    //
    // Simplest correct approach: use a channel.

    let (code_tx, code_rx) = tokio::sync::oneshot::channel::<String>();

    // Spawn a task that waits for the code then signs in.
    let phone_owned = phone.to_string();
    let db = state.db_pool.clone();
    let sess_path = path.clone();
    tokio::spawn(async move {
        match code_rx.await {
            Ok(code) => {
                let result = client.sign_in(&token, &code).await;
                match result {
                    Ok(_user) => {
                        tracing::info!("Telegram login success for {}", mask_phone(&phone_owned));
                        // Read session file bytes and persist to DB.
                        if let Ok(blob) = tokio::fs::read(&sess_path).await {
                            let _ = sqlx::query(
                                "UPDATE telegram_sessions
                                 SET session_blob = $1, authorized = TRUE, last_error = NULL, updated_at = NOW()
                                 WHERE phone = $2",
                            )
                            .bind(&blob)
                            .bind(&phone_owned)
                            .execute(&db)
                            .await;
                        }
                    }
                    Err(SignInError::PasswordRequired(_pwd_token)) => {
                        tracing::warn!(
                            "Telegram 2FA password required for {}",
                            mask_phone(&phone_owned)
                        );
                        let _ = sqlx::query(
                            "UPDATE telegram_sessions SET last_error = '2FA password required — not yet supported', updated_at = NOW() WHERE phone = $1",
                        )
                        .bind(&phone_owned)
                        .execute(&db)
                        .await;
                    }
                    Err(SignInError::InvalidCode) => {
                        tracing::warn!("Telegram invalid code for {}", mask_phone(&phone_owned));
                        let _ = sqlx::query(
                            "UPDATE telegram_sessions SET last_error = 'Invalid code', updated_at = NOW() WHERE phone = $1",
                        )
                        .bind(&phone_owned)
                        .execute(&db)
                        .await;
                    }
                    Err(e) => {
                        tracing::error!(
                            "Telegram sign_in error for {}: {e:?}",
                            mask_phone(&phone_owned)
                        );
                        let _ = sqlx::query(
                            "UPDATE telegram_sessions SET last_error = $1, updated_at = NOW() WHERE phone = $2",
                        )
                        .bind(format!("{e:?}"))
                        .bind(&phone_owned)
                        .execute(&db)
                        .await;
                    }
                }
            }
            Err(_) => {
                // Sender was dropped — login_verify was never called within timeout.
                tracing::warn!(
                    "Telegram login code channel dropped for {}",
                    mask_phone(&phone_owned)
                );
            }
        }
    });

    // Stash the sender so login_verify can send the code.
    {
        let mut map = PENDING_LOGINS.lock().unwrap();
        // Replace any existing PendingLogin for this phone — also drop old
        // CODE_SENDERS entry.
        map.insert(
            phone.to_string(),
            PendingLogin {
                created_at: std::time::Instant::now(),
            },
        );
    }
    {
        let mut senders = CODE_SENDERS.lock().unwrap();
        senders.insert(phone.to_string(), code_tx);
    }

    Ok(())
}

lazy_static::lazy_static! {
    /// Oneshot senders keyed by phone.  login_start inserts, login_verify takes.
    static ref CODE_SENDERS: std::sync::Mutex<HashMap<String, tokio::sync::oneshot::Sender<String>>> =
        std::sync::Mutex::new(HashMap::new());
}

// ---------------------------------------------------------------------------
// login_verify
// ---------------------------------------------------------------------------

pub async fn login_verify(_state: &Arc<AppState>, phone: &str, code: &str) -> Result<(), String> {
    // Remove the pending login entry.
    {
        let mut map = PENDING_LOGINS.lock().unwrap();
        let pending = map.remove(phone).ok_or_else(|| {
            "No pending login found for this phone. Call /login/start first.".to_string()
        })?;
        // Check timeout (10 minutes).
        if pending.created_at.elapsed() > std::time::Duration::from_secs(600) {
            return Err(
                "Login code expired (>10 minutes). Please restart the login flow.".to_string(),
            );
        }
    }

    // Send the code to the spawned task that holds the Client + LoginToken.
    let tx = {
        let mut senders = CODE_SENDERS.lock().unwrap();
        senders.remove(phone).ok_or_else(|| {
            "Code channel not found — login may have already completed or timed out.".to_string()
        })?
    };

    tx.send(code.to_string()).map_err(|_| {
        "Login task is no longer running. Please restart the login flow.".to_string()
    })?;

    // The spawned task will sign in and update the DB.  We return success
    // immediately — the UI can poll /status to see when authorized flips to
    // true.  (sign_in happens asynchronously in the background task.)
    Ok(())
}

// ---------------------------------------------------------------------------
// status
// ---------------------------------------------------------------------------

pub async fn status(state: &Arc<AppState>) -> serde_json::Value {
    let row = sqlx::query_as::<
        _,
        (
            i32,
            String,
            bool,
            Option<chrono::DateTime<chrono::Utc>>,
            Option<String>,
        ),
    >(
        "SELECT id, phone, authorized, last_poll_at, last_error
         FROM telegram_sessions
         ORDER BY created_at DESC LIMIT 1",
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
        None => serde_json::json!({
            "authorized": false,
            "message": "No Telegram session found. Use /login/start to connect."
        }),
    }
}

// ---------------------------------------------------------------------------
// start_watcher (background task)
// ---------------------------------------------------------------------------

pub async fn start_watcher(state: Arc<AppState>) {
    tracing::info!("Telegram MTProto watcher: starting background loop");

    // Wait for an authorized session to appear in DB, then connect and stream.
    loop {
        match try_run_watcher(&state).await {
            Ok(()) => {
                // Watcher exited cleanly (shouldn't normally happen).
                tracing::warn!("Telegram watcher exited cleanly — restarting in 30s");
            }
            Err(e) => {
                tracing::error!("Telegram watcher error: {e} — retrying in 30s");
                // Store error in DB.
                let _ = sqlx::query(
                    "UPDATE telegram_sessions SET last_error = $1, updated_at = NOW()
                     WHERE authorized = TRUE ORDER BY created_at DESC LIMIT 1",
                )
                .bind(format!("{e}"))
                .execute(&state.db_pool)
                .await;
            }
        }
        tokio::time::sleep(std::time::Duration::from_secs(30)).await;
    }
}

/// Use the logged-in MTProto user session to search Telegram's public
/// directory. Bot API cannot do this; it must run as an authorized user.
pub async fn discover_public_channels(
    state: &Arc<AppState>,
    queries: Vec<String>,
    limit_per_query: i32,
) -> Result<Vec<TelegramDiscoveredChannel>, String> {
    let row = sqlx::query_as::<_, (i32, String, Vec<u8>)>(
        "SELECT id, phone, session_blob
         FROM telegram_sessions
         WHERE authorized = TRUE AND session_blob IS NOT NULL
         ORDER BY created_at DESC LIMIT 1",
    )
    .fetch_optional(&state.db_pool)
    .await
    .map_err(|e| format!("db query: {e}"))?;

    let (session_id, phone, blob) = row
        .ok_or_else(|| "No authorized Telegram MTProto user session. Log in from Prospect Finder first.".to_string())?;

    let path = session_path(&phone);
    tokio::fs::write(&path, &blob)
        .await
        .map_err(|e| format!("write session file: {e}"))?;

    let session = Arc::new(
        SqliteSession::open(&path)
            .await
            .map_err(|e| format!("session open: {e}"))?,
    );

    let pool = SenderPool::new(session, api_id()?);
    let runner = pool.runner;
    tokio::spawn(async move { runner.run().await });
    let client = Client::new(pool.handle);

    let mut discovered = Vec::new();
    for query in queries {
        let q = query.trim();
        if q.is_empty() {
            continue;
        }

        let result = client
            .invoke(&tl::functions::contacts::Search {
                q: q.to_string(),
                limit: limit_per_query,
                bots: false,
                broadcasts: true,
            })
            .await
            .map_err(|e| format!("contacts.search({q}): {e}"))?;

        let tl::enums::contacts::Found::Found(found) = result;
        for chat in found.chats {
            match chat {
                tl::enums::Chat::Channel(channel) => {
                    if channel.scam || channel.fake || channel.restricted {
                        continue;
                    }
                    discovered.push(TelegramDiscoveredChannel {
                        query: q.to_string(),
                        channel_id: Some(channel.id),
                        username: channel.username.clone(),
                        title: channel.title.clone(),
                        is_broadcast: channel.broadcast,
                        is_megagroup: channel.megagroup,
                        participants_count: channel.participants_count,
                    });
                }
                tl::enums::Chat::Chat(chat) => {
                    discovered.push(TelegramDiscoveredChannel {
                        query: q.to_string(),
                        channel_id: Some(chat.id),
                        username: None,
                        title: chat.title.clone(),
                        is_broadcast: false,
                        is_megagroup: true,
                        participants_count: Some(chat.participants_count),
                    });
                }
                _ => {}
            }
        }
    }

    let _ = sqlx::query(
        "UPDATE telegram_sessions SET last_poll_at = NOW(), last_error = NULL, updated_at = NOW() WHERE id = $1",
    )
    .bind(session_id)
    .execute(&state.db_pool)
    .await;

    Ok(discovered)
}

async fn try_run_watcher(state: &Arc<AppState>) -> Result<(), String> {
    // 1. Fetch the latest authorized session blob from DB.
    let row = sqlx::query_as::<_, (i32, String, Vec<u8>)>(
        "SELECT id, phone, session_blob
         FROM telegram_sessions
         WHERE authorized = TRUE AND session_blob IS NOT NULL
         ORDER BY created_at DESC LIMIT 1",
    )
    .fetch_optional(&state.db_pool)
    .await
    .map_err(|e| format!("db query: {e}"))?;

    let (session_id, phone, blob) = match row {
        Some(r) => r,
        None => {
            return Err("No authorized Telegram session in DB yet".to_string());
        }
    };

    tracing::info!(
        "Telegram watcher: restoring session {} for {}",
        session_id,
        mask_phone(&phone)
    );

    // 2. Write blob to temp file and open SqliteSession.
    let path = session_path(&phone);
    tokio::fs::write(&path, &blob)
        .await
        .map_err(|e| format!("write session file: {e}"))?;

    let id = api_id()?;

    let session = Arc::new(
        SqliteSession::open(&path)
            .await
            .map_err(|e| format!("session open: {e}"))?,
    );

    let pool = SenderPool::new(session, id);
    let updates_rx = pool.updates;

    let runner = pool.runner;
    tokio::spawn(async move { runner.run().await });

    let client = Client::new(pool.handle);

    // 3. Load watch channels and keyword regexes.
    let channels = load_watch_channels(state).await;
    tracing::info!("Telegram watcher: monitoring {} channels", channels.len());

    // Build a combined regex from channel-specific overrides + default.
    let default_re =
        Regex::new(DEFAULT_KEYWORD_REGEX).map_err(|e| format!("bad default regex: {e}"))?;

    // 4. Stream updates.
    let config = UpdatesConfiguration {
        catch_up: true,
        ..Default::default()
    };
    let mut stream = client.stream_updates(updates_rx, config)
        .await
        .map_err(|e| format!("stream_updates error: {e}"))?;

    // Mark last_poll_at on connect.
    let _ = sqlx::query(
        "UPDATE telegram_sessions SET last_poll_at = NOW(), last_error = NULL, updated_at = NOW() WHERE id = $1",
    )
    .bind(session_id)
    .execute(&state.db_pool)
    .await;

    tracing::info!("Telegram watcher: streaming updates");

    loop {
        let update = stream
            .next()
            .await
            .map_err(|e| format!("stream error: {e}"))?;

        match update {
            Update::NewMessage(msg) => {
                // Update last_poll_at periodically (every message).
                let _ =
                    sqlx::query("UPDATE telegram_sessions SET last_poll_at = NOW() WHERE id = $1")
                        .bind(session_id)
                        .execute(&state.db_pool)
                        .await;

                let text = msg.text();
                if text.is_empty() {
                    continue;
                }

                let chat_name = msg
                    .peer()
                    .and_then(|p| p.name().or(p.username()))
                    .unwrap_or("unknown")
                    .to_string();

                // Check if this channel is in our watch list.
                let channel_config = channels.get(&chat_name.to_lowercase());
                // If we have watch channels configured, only process those.
                // If the list is empty, process all channels (catch-all mode).
                if !channels.is_empty() && channel_config.is_none() {
                    continue;
                }

                // Pick the regex: channel-specific override or default.
                let re = match channel_config {
                    Some(Some(override_re)) => override_re,
                    _ => &default_re,
                };

                // Check keyword match.
                let kw_match = re.find(text);
                if kw_match.is_none() {
                    continue;
                }
                let matched_kw = kw_match.unwrap().as_str().to_string();

                let msg_id = msg.id();
                let sender_name = msg
                    .sender()
                    .and_then(|s| s.name().map(|n| n.to_string()))
                    .unwrap_or_default();
                let link = format!("https://t.me/{}/{}", &chat_name, msg_id);

                tracing::info!(
                    "Telegram watcher: keyword match in @{} msg#{}: \"{}...\"",
                    &chat_name,
                    msg_id,
                    &text[..text.len().min(80)]
                );

                // Deduplicate by message_id + channel.
                let exists = sqlx::query_scalar::<_, bool>(
                    "SELECT EXISTS(SELECT 1 FROM telegram_opportunities WHERE channel = $1 AND message_id = $2)",
                )
                .bind(&chat_name)
                .bind(msg_id as i64)
                .fetch_one(&state.db_pool)
                .await
                .unwrap_or(false);

                if exists {
                    continue;
                }

                // AI scoring.
                let (score, reason, service) =
                    crate::handlers::prospects::score_telegram_opportunity_public(
                        state, &chat_name, text,
                    )
                    .await;

                // Insert.
                let _ = sqlx::query(
                    "INSERT INTO telegram_opportunities
                       (channel, message_id, sender, message, matched_kw, link,
                        score, score_reason, service_type, status, source)
                     VALUES ($1, $2, $3, $4, $5, $6, $7, $8, $9, 'new', 'watcher')",
                )
                .bind(&chat_name)
                .bind(msg_id as i64)
                .bind(&sender_name)
                .bind(text)
                .bind(&matched_kw)
                .bind(&link)
                .bind(score)
                .bind(&reason)
                .bind(&service)
                .execute(&state.db_pool)
                .await;

                // Notify admin if score >= threshold.
                if score >= NOTIFY_SCORE_THRESHOLD {
                    let dm = build_suggested_dm(state, &chat_name, text, service.as_deref()).await;
                    let notification = format!(
                        "🎯 *Telegram Lead* (score {score}/100)\n\
                         Channel: @{chat_name}\n\
                         Sender: {sender_name}\n\
                         Match: `{matched_kw}`\n\
                         Service: {}\n\n\
                         _{}_\n\n\
                         *Suggested DM:*\n{}",
                        service.as_deref().unwrap_or("unknown"),
                        &text[..text.len().min(300)],
                        dm
                    );
                    crate::telegram_bot::notify_admin(&notification).await;
                }
            }
            _ => {
                // Ignore edits, deletes, etc. for now.
            }
        }
    }
}

// ---------------------------------------------------------------------------
// Watch channel loader
// ---------------------------------------------------------------------------

/// Returns a map of lowercase channel name -> optional compiled regex override.
/// `None` value means "use default regex".
async fn load_watch_channels(state: &Arc<AppState>) -> HashMap<String, Option<Regex>> {
    let rows = sqlx::query_as::<_, (String, Option<String>)>(
        "SELECT channel, keyword_re FROM telegram_watch_channels WHERE enabled = TRUE",
    )
    .fetch_all(&state.db_pool)
    .await
    .unwrap_or_default();

    let mut map = HashMap::new();
    for (ch, kw_re) in rows {
        let compiled = kw_re.and_then(|re_str| {
            Regex::new(&re_str)
                .map_err(|e| {
                    tracing::warn!("Bad regex for channel {ch}: {e} — using default");
                    e
                })
                .ok()
        });
        map.insert(ch.to_lowercase(), compiled);
    }
    map
}

// ---------------------------------------------------------------------------
// DM generation helper
// ---------------------------------------------------------------------------

/// Use the LLM fallback chain to generate a short, professional DM that the
/// admin can copy-paste to the lead.
async fn build_suggested_dm(
    state: &Arc<AppState>,
    channel: &str,
    message: &str,
    service_type: Option<&str>,
) -> String {
    let prompt = format!(
        "You are a freelance video editor reaching out to a potential client on Telegram.\n\
         Write a short (3-4 sentences), professional, friendly DM responding to this message \
         posted in the @{channel} channel. The service you offer is: {service}.\n\n\
         Message:\n\"{msg}\"\n\n\
         Rules:\n\
         - Mention you saw their post in @{channel}\n\
         - Briefly highlight your relevant skill\n\
         - Include a soft CTA (e.g., happy to share portfolio)\n\
         - Keep it under 60 words\n\
         - Do NOT use emojis\n\
         - Output ONLY the DM text, nothing else",
        channel = channel,
        service = service_type.unwrap_or("video editing / clipping"),
        msg = &message[..message.len().min(500)],
    );

    match crate::llm_utils::generate_text_best_effort(
        state.ollama_fast_client.as_ref(),
        state.ollama_client.as_ref(),
        state.nvidia_nim_client.as_ref(),
        state.gemma_client.as_ref(),
        state.gemini_client.as_ref(),
        state.deepseek_client.as_ref(),
        &prompt,
    )
    .await
    {
        Ok(dm) => dm,
        Err(e) => {
            tracing::warn!("DM generation failed: {e}");
            format!(
                "Hi! I saw your post in @{channel} — I specialize in {service} and would love to help. \
                 Happy to share my portfolio if you're interested.",
                channel = channel,
                service = service_type.unwrap_or("video editing"),
            )
        }
    }
}
