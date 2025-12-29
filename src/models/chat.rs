// src/models/chat.rs
use serde::{Deserialize, Serialize};
use sqlx::FromRow;

#[derive(Debug, Serialize, Deserialize, FromRow)]
pub struct ChatMessage {
    pub id: i32,
    pub session_id: i32,
    pub user_message: String,
    pub ai_message: String,
}

#[derive(Debug, Serialize, Deserialize, FromRow)]
pub struct ChatSession {
    pub id: i32,
    pub user_id: i32,
    pub title: String,
}
