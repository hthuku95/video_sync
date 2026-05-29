//! Telegram Bot API client + long-poll worker for @videosync_sales_bot.
//!
//! Pure HTTP (Bot API, not MTProto) — safe, stateless, no ban risk,
//! no interactive login required. Two jobs:
//!
//! 1. **Outbound: admin pings** — when a user pays via x402 (subscribe,
//!    api-access, delivery unlock), `notify_admin()` DMs the configured
//!    admin user with tx details so you know to follow up.
//!
//! 2. **Inbound: AI sales/support responder** — anyone who messages
//!    `@videosync_sales_bot` gets an AI reply built from our LLM stack
//!    (NVIDIA NIM → Gemma → Gemini) with the product pitch baked in.
//!    Runs as a background long-poll task; each new message is handled
//!    independently.
//!
//! Env vars consumed:
//! * `TELEGRAM_BOT_TOKEN`     — from @BotFather (required)
//! * `TELEGRAM_ADMIN_USER_ID` — numeric user id from @userinfobot (required
//!                              for admin pings; optional for inbound AI)

use crate::llm_utils::generate_text_best_effort;
use crate::services::monetization::telegram_system_pitch;
use crate::AppState;
use serde::{Deserialize, Serialize};
use serde_json::json;
use std::sync::Arc;

const TELEGRAM_API_BASE: &str = "https://api.telegram.org";

fn bot_token() -> Option<String> {
    std::env::var("TELEGRAM_BOT_TOKEN")
        .ok()
        .filter(|s| !s.is_empty())
}

fn admin_chat_id() -> Option<i64> {
    std::env::var("TELEGRAM_ADMIN_USER_ID")
        .ok()
        .and_then(|s| s.parse().ok())
}

fn bot_enabled() -> bool {
    bot_token().is_some()
}

/// Send a Markdown-formatted message to the configured admin. No-op if
/// the bot env vars aren't set — safe to call from any payment handler.
pub async fn notify_admin(text: &str) {
    let (Some(token), Some(chat_id)) = (bot_token(), admin_chat_id()) else {
        return;
    };

    let client = match reqwest::Client::builder()
        .timeout(std::time::Duration::from_secs(10))
        .build()
    {
        Ok(c) => c,
        Err(_) => return,
    };

    let body = json!({
        "chat_id":                  chat_id,
        "text":                     text,
        "parse_mode":               "Markdown",
        "disable_web_page_preview": true,
    });

    let url = format!("{}/bot{}/sendMessage", TELEGRAM_API_BASE, token);
    match client.post(&url).json(&body).send().await {
        Ok(r) if r.status().is_success() => {}
        Ok(r) => tracing::warn!("Telegram admin notify non-2xx: {}", r.status()),
        Err(e) => tracing::warn!("Telegram admin notify failed: {}", e),
    }
}

// ────────────────────────────────────────────────────────────────────────────
// Long-poll worker — receives DMs to the bot and replies via AI
// ────────────────────────────────────────────────────────────────────────────

#[derive(Debug, Deserialize)]
struct GetUpdatesResponse {
    ok: bool,
    result: Option<Vec<Update>>,
}

#[derive(Debug, Deserialize)]
struct Update {
    update_id: i64,
    message: Option<Message>,
}

#[derive(Debug, Deserialize)]
struct Message {
    message_id: i64,
    from: Option<User>,
    chat: Chat,
    text: Option<String>,
}

#[derive(Debug, Deserialize, Serialize)]
struct User {
    id: i64,
    first_name: Option<String>,
    username: Option<String>,
}

#[derive(Debug, Deserialize)]
struct Chat {
    id: i64,
}

/// Main worker loop — started from main.rs once per process. Long-polls
/// Telegram's `getUpdates` endpoint; for each text message from a regular
/// user (i.e. not a group, not a bot), sends the text + SYSTEM_PITCH to
/// our LLM and posts the reply back.
///
/// Bot API long-poll is resilient: 30s timeout, Telegram returns early
/// when a new message arrives. If we miss updates after a restart Telegram
/// keeps them for 24h.
pub async fn start_worker(state: Arc<AppState>) {
    if !bot_enabled() {
        tracing::info!("Telegram bot disabled (TELEGRAM_BOT_TOKEN not set) — skipping worker");
        return;
    }
    tracing::info!("✈️ Telegram sales bot worker started");

    let client = match reqwest::Client::builder()
        .timeout(std::time::Duration::from_secs(45)) // > long-poll timeout
        .build()
    {
        Ok(c) => c,
        Err(e) => {
            tracing::error!("Telegram bot: HTTP client init failed: {}", e);
            return;
        }
    };

    let token = bot_token().unwrap();
    let url = format!("{}/bot{}/getUpdates", TELEGRAM_API_BASE, token);

    // Offset persists across iterations so Telegram only returns updates
    // we haven't acked yet. Reset to 0 on startup — Telegram will replay
    // up to 24h of unprocessed messages.
    let mut offset: i64 = 0;

    loop {
        let query = json!({
            "offset":          offset,
            "timeout":         30,     // seconds — Telegram's max long-poll
            "allowed_updates": ["message"],
        });

        let resp = match client.post(&url).json(&query).send().await {
            Ok(r) => r,
            Err(e) => {
                tracing::warn!("Telegram getUpdates request failed: {} — retrying in 5s", e);
                tokio::time::sleep(std::time::Duration::from_secs(5)).await;
                continue;
            }
        };

        let parsed: GetUpdatesResponse = match resp.json().await {
            Ok(p) => p,
            Err(e) => {
                tracing::warn!("Telegram getUpdates parse failed: {}", e);
                tokio::time::sleep(std::time::Duration::from_secs(5)).await;
                continue;
            }
        };

        if !parsed.ok {
            tracing::warn!("Telegram getUpdates returned ok=false");
            tokio::time::sleep(std::time::Duration::from_secs(5)).await;
            continue;
        }

        let updates = parsed.result.unwrap_or_default();
        for u in updates {
            // Always advance offset so a single bad message doesn't loop us.
            offset = u.update_id + 1;

            let Some(msg) = u.message else {
                continue;
            };
            let Some(text) = msg.text.as_deref() else {
                continue;
            };
            let user_id = msg.from.as_ref().map(|u| u.id).unwrap_or(0);

            // Only DMs, not groups. Groups have negative chat ids.
            if msg.chat.id < 0 {
                continue;
            }

            // Spawn per-message handling so one slow LLM call doesn't
            // block the long-poll.
            let state_clone = state.clone();
            let text_owned = text.to_string();
            let chat_id = msg.chat.id;
            let msg_id = msg.message_id;
            let username = msg
                .from
                .as_ref()
                .and_then(|u| u.username.clone())
                .unwrap_or_default();
            tokio::spawn(async move {
                handle_dm(state_clone, chat_id, msg_id, user_id, username, text_owned).await;
            });
        }
    }
}

async fn handle_dm(
    state: Arc<AppState>,
    chat_id: i64,
    reply_to: i64,
    _user_id: i64,
    username: String,
    text: String,
) {
    // First ping admin so they know someone DM'd the bot — helpful when
    // the AI response isn't good enough and they want to jump in.
    let admin_blurb = format!(
        "📨 *New Telegram DM to @videosync\\_sales\\_bot*\n\
        From: {user}\n\
        Message: `{msg}`",
        user = if username.is_empty() {
            format!("user {}", chat_id)
        } else {
            format!("@{}", username)
        },
        msg = text.chars().take(200).collect::<String>(),
    );
    // Fire-and-forget — don't block the reply on the admin ping.
    tokio::spawn(async move {
        notify_admin(&admin_blurb).await;
    });

    // Generate an AI reply using our LLM stack.
    let prompt = format!(
        "{}\n\nIncoming message from user:\n\"\"\"\n{}\n\"\"\"\n\nYour reply:",
        telegram_system_pitch(),
        text
    );
    let reply = match generate_text_best_effort(
        state.nvidia_nim_client.as_ref(),
        state.gemma_client.as_ref(),
        state.gemini_client.as_ref(),
        state.deepseek_client.as_ref(),
        &prompt,
    )
    .await
    {
        Ok(r) => r.trim().to_string(),
        Err(e) => {
            tracing::warn!("Telegram bot AI reply failed: {}", e);
            "Hey — I'm having a blip generating a reply. Can you try again in a minute, or tag @hthuku directly?".to_string()
        }
    };

    // Send the reply — no Markdown so the LLM's output doesn't accidentally
    // hit a Telegram parse error from unbalanced formatting.
    send_text_reply(chat_id, reply_to, &reply).await;
}

async fn send_text_reply(chat_id: i64, reply_to: i64, text: &str) {
    let Some(token) = bot_token() else {
        return;
    };
    let client = match reqwest::Client::builder()
        .timeout(std::time::Duration::from_secs(10))
        .build()
    {
        Ok(c) => c,
        Err(_) => return,
    };
    let url = format!("{}/bot{}/sendMessage", TELEGRAM_API_BASE, token);
    let body = json!({
        "chat_id":                  chat_id,
        "text":                     text,
        "reply_to_message_id":      reply_to,
        "disable_web_page_preview": true,
    });
    if let Err(e) = client.post(&url).json(&body).send().await {
        tracing::warn!("Telegram send_text_reply failed: {}", e);
    }
}
