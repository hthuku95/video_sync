// src/agent/conversation_manager.rs
use rust_decimal::Decimal;
use serde_json::Value;
use sqlx::PgPool;
use thiserror::Error;

#[derive(Error, Debug)]
pub enum ConversationError {
    #[error("Database error: {0}")]
    DatabaseError(#[from] sqlx::Error),
    #[error("Serialization error: {0}")]
    SerializationError(#[from] serde_json::Error),
}

/// Message types following LangChain/LangGraph patterns
#[derive(Debug, Clone, PartialEq)]
pub enum MessageRole {
    System,
    Human,
    Assistant,
    Function,
}

impl MessageRole {
    pub fn to_string(&self) -> String {
        match self {
            MessageRole::System => "system".to_string(),
            MessageRole::Human => "user".to_string(), // Gemini uses "user" for human messages
            MessageRole::Assistant => "model".to_string(), // Gemini uses "model" for AI responses
            MessageRole::Function => "function".to_string(),
        }
    }

    pub fn from_string(role: &str) -> Self {
        match role {
            "system" => MessageRole::System,
            "user" | "human" => MessageRole::Human,
            "model" | "assistant" => MessageRole::Assistant,
            "function" => MessageRole::Function,
            _ => MessageRole::Human, // Default fallback
        }
    }
}

/// Individual conversation message following modern patterns
#[derive(Debug, Clone)]
pub struct ConversationMessage {
    pub id: Option<i32>,
    pub session_id: String,
    pub role: MessageRole,
    pub content: String,
    pub metadata: Option<Value>, // Store tool calls, function responses, etc.
    pub created_at: Option<chrono::DateTime<chrono::Utc>>,

    // Token usage and cost tracking
    pub prompt_tokens: Option<i32>,
    pub completion_tokens: Option<i32>,
    pub total_tokens: Option<i32>,
    pub model: Option<String>,
    pub cost_usd: Option<Decimal>,
}

impl ConversationMessage {
    pub fn new_human(session_id: String, content: String) -> Self {
        Self {
            id: None,
            session_id,
            role: MessageRole::Human,
            content,
            metadata: None,
            created_at: None,
            prompt_tokens: None,
            completion_tokens: None,
            total_tokens: None,
            model: None,
            cost_usd: None,
        }
    }

    pub fn new_assistant(session_id: String, content: String) -> Self {
        Self {
            id: None,
            session_id,
            role: MessageRole::Assistant,
            content,
            metadata: None,
            created_at: None,
            prompt_tokens: None,
            completion_tokens: None,
            total_tokens: None,
            model: None,
            cost_usd: None,
        }
    }

}

/// Manages conversation history following LangChain/LangGraph patterns
pub struct ConversationManager {
    db_pool: PgPool,
}

impl ConversationManager {
    pub fn new(db_pool: PgPool) -> Self {
        Self { db_pool }
    }

    /// Create the new conversation messages table schema
    pub async fn initialize_schema(&self) -> Result<(), ConversationError> {
        // Create the table first
        sqlx::query(
            r#"
            CREATE TABLE IF NOT EXISTS conversation_messages (
                id SERIAL PRIMARY KEY,
                session_id INTEGER NOT NULL,
                role VARCHAR(20) NOT NULL, -- 'system', 'user', 'assistant', 'function'
                content TEXT NOT NULL,
                metadata JSONB, -- Store function calls, tool responses, etc.
                created_at TIMESTAMPTZ NOT NULL DEFAULT NOW(),
                
                -- Token usage tracking
                prompt_tokens INTEGER,
                completion_tokens INTEGER,
                total_tokens INTEGER,
                model VARCHAR(50),
                cost_usd DECIMAL(10, 6),

                FOREIGN KEY (session_id) REFERENCES chat_sessions(id) ON DELETE CASCADE
            )
        "#,
        )
        .execute(&self.db_pool)
        .await?;

        // Create indexes separately (SQLx doesn't allow multiple commands in one prepared statement)
        sqlx::query("CREATE INDEX IF NOT EXISTS idx_conversation_messages_session_id ON conversation_messages(session_id)")
            .execute(&self.db_pool)
            .await?;

        sqlx::query("CREATE INDEX IF NOT EXISTS idx_conversation_messages_role ON conversation_messages(role)")
            .execute(&self.db_pool)
            .await?;

        sqlx::query("CREATE INDEX IF NOT EXISTS idx_conversation_messages_created_at ON conversation_messages(created_at)")
            .execute(&self.db_pool)
            .await?;

        Ok(())
    }

    /// Save a message to the database
    pub async fn save_message(
        &self,
        message: &ConversationMessage,
    ) -> Result<ConversationMessage, ConversationError> {
        tracing::debug!(
            "💾 Saving message to DB - session: {}, role: {:?}, content_len: {}",
            message.session_id,
            message.role,
            message.content.len()
        );

        let session_db_id = self.get_session_db_id(&message.session_id).await?;

        let metadata_json = match &message.metadata {
            Some(meta) => Some(sqlx::types::Json(meta.clone())),
            None => None,
        };

        let row = sqlx::query_as::<_, (i32, chrono::DateTime<chrono::Utc>)>(
            "INSERT INTO conversation_messages 
             (session_id, role, content, metadata, created_at, prompt_tokens, completion_tokens, total_tokens, model, cost_usd)
             VALUES ($1, $2, $3, $4, NOW(), $5, $6, $7, $8, $9)
             RETURNING id, created_at"
        )
        .bind(session_db_id)
        .bind(message.role.to_string())
        .bind(&message.content)
        .bind(metadata_json)
        .bind(message.prompt_tokens)
        .bind(message.completion_tokens)
        .bind(message.total_tokens)
        .bind(&message.model)
        .bind(message.cost_usd)
        .fetch_one(&self.db_pool)
        .await?;

        let mut saved_message = message.clone();
        saved_message.id = Some(row.0);
        saved_message.created_at = Some(row.1);

        Ok(saved_message)
    }

    /// Get conversation history for a session (following LangChain pattern)
    pub async fn get_conversation_history(
        &self,
        session_id: &str,
        limit: Option<i32>,
    ) -> Result<Vec<ConversationMessage>, ConversationError> {
        let session_db_id = self.get_session_db_id(session_id).await?;
        let limit = limit.unwrap_or(50);

        let rows = sqlx::query_as::<_, (
            i32, String, String, Option<sqlx::types::Json<Value>>, chrono::DateTime<chrono::Utc>,
            Option<i32>, Option<i32>, Option<i32>, Option<String>, Option<Decimal>
        )>(
            "SELECT id, role, content, metadata, created_at, prompt_tokens, completion_tokens, total_tokens, model, cost_usd
             FROM conversation_messages
             WHERE session_id = $1
             ORDER BY created_at ASC
             LIMIT $2"
        )
        .bind(session_db_id)
        .bind(limit)
        .fetch_all(&self.db_pool)
        .await?;

        let messages = rows
            .into_iter()
            .map(
                |(
                    id,
                    role,
                    content,
                    metadata,
                    created_at,
                    prompt_tokens,
                    completion_tokens,
                    total_tokens,
                    model,
                    cost_usd,
                )| {
                    ConversationMessage {
                        id: Some(id),
                        session_id: session_id.to_string(),
                        role: MessageRole::from_string(&role),
                        content,
                        metadata: metadata.map(|json| json.0),
                        created_at: Some(created_at),
                        prompt_tokens,
                        completion_tokens,
                        total_tokens,
                        model,
                        cost_usd,
                    }
                },
            )
            .collect();

        Ok(messages)
    }

    /// Get session database ID
    async fn get_session_db_id(&self, session_uuid: &str) -> Result<i32, ConversationError> {
        let row =
            sqlx::query_as::<_, (i32,)>("SELECT id FROM chat_sessions WHERE session_uuid = $1")
                .bind(session_uuid)
                .fetch_one(&self.db_pool)
                .await?;

        Ok(row.0)
    }

}
