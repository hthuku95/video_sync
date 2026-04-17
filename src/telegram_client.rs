//! Telegram MTProto (Client API / userbot) — monitors configured
//! channels for paid-gig opportunities and pings the admin via the
//! Telegram Bot API with a pre-written custom DM.
//!
//! NOTE: The grammers-client master API underwent a major redesign
//! (SenderPool + UpdateStream pattern replacing Client::connect +
//! next_update). This module is currently STUBBED — login and watcher
//! return informative errors. The Telegram Bot API side (telegram_bot.rs)
//! still works for outbound notifications.
//!
//! TODO: implement SenderPool::new → Client::new → stream_updates flow.

use crate::AppState;
use std::sync::Arc;

pub async fn login_start(_state: &Arc<AppState>, _phone: &str) -> Result<(), String> {
    Err("Telegram MTProto login is not yet available — the grammers-client library underwent a major API redesign. Use the Telegram Bot API for now.".to_string())
}

pub async fn login_verify(_state: &Arc<AppState>, _phone: &str, _code: &str) -> Result<(), String> {
    Err("Telegram MTProto login is not yet available.".to_string())
}

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
        None => serde_json::json!({
            "authorized": false,
            "message": "Telegram MTProto watcher is pending — grammers API migration in progress."
        }),
    }
}

fn mask_phone(phone: &str) -> String {
    if phone.len() < 4 { return "***".to_string(); }
    let tail = &phone[phone.len()-4..];
    format!("***{tail}")
}

pub async fn start_watcher(_state: Arc<AppState>) {
    tracing::info!("Telegram MTProto watcher: STUBBED — grammers API migration pending. Bot API notifications still active.");
}
