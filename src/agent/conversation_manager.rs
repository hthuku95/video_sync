// src/agent/conversation_manager.rs
use crate::gemini_client::{Content, Part, GenerateContentRequest, GenerationConfig, ToolConfig, FunctionCallingConfig, FunctionCallingMode, Tool};
use serde_json::Value;
use sqlx::PgPool;
use thiserror::Error;
use rust_decimal::Decimal;

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
    pub fn new_system(session_id: String, content: String) -> Self {
        Self {
            id: None,
            session_id,
            role: MessageRole::System,
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

    pub fn new_function_call(session_id: String, tool_name: String, args: Value) -> Self {
        let content = format!("Called function: {}", tool_name);
        let metadata = serde_json::json!({
            "function_call": {
                "name": tool_name,
                "arguments": args
            }
        });

        Self {
            id: None,
            session_id,
            role: MessageRole::Function,
            content,
            metadata: Some(metadata),
            created_at: None,
            prompt_tokens: None,
            completion_tokens: None,
            total_tokens: None,
            model: None,
            cost_usd: None,
        }
    }

    pub fn new_function_result(session_id: String, tool_name: String, result: String) -> Self {
        let content = format!("Function {} result: {}", tool_name, result);
        let metadata = serde_json::json!({
            "function_response": {
                "name": tool_name,
                "content": result
            }
        });

        Self {
            id: None,
            session_id,
            role: MessageRole::Function,
            content,
            metadata: Some(metadata),
            created_at: None,
            prompt_tokens: None,
            completion_tokens: None,
            total_tokens: None,
            model: None,
            cost_usd: None,
        }
    }

    /// Convert to Gemini API Content structure
    pub fn to_gemini_content(&self) -> Content {
        // Handle function calls and responses in metadata
        if let Some(ref metadata) = self.metadata {
            if let Some(function_call) = metadata.get("function_call") {
                return Content {
                    parts: vec![Part::FunctionCall {
                        function_call: crate::gemini_client::FunctionCall {
                            name: function_call["name"].as_str().unwrap_or("").to_string(),
                            args: function_call["arguments"].as_object()
                                .map(|obj| obj.iter().map(|(k, v)| (k.clone(), v.clone())).collect())
                                .unwrap_or_default(),
                            thought_signature: function_call.get("thought_signature")
                                .and_then(|v| v.as_str())
                                .map(|s| s.to_string()),
                        }
                    }],
                    role: Some(self.role.to_string()),
                };
            }

            if let Some(function_response) = metadata.get("function_response") {
                return Content {
                    parts: vec![Part::FunctionResponse {
                        function_response: crate::gemini_client::FunctionResponse {
                            name: function_response["name"].as_str().unwrap_or("").to_string(),
                            response: function_response["content"].as_object()
                                .map(|obj| obj.iter().map(|(k, v)| (k.clone(), v.clone())).collect())
                                .unwrap_or_default(),
                            thought_signature: function_response.get("thought_signature")
                                .and_then(|v| v.as_str())
                                .map(|s| s.to_string()),
                        }
                    }],
                    role: Some(self.role.to_string()),
                };
            }
        }

        // Regular text message
        Content {
            parts: vec![Part::Text { text: self.content.clone() }],
            role: Some(self.role.to_string()),
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
        sqlx::query(r#"
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
        "#)
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
    pub async fn save_message(&self, message: &ConversationMessage) -> Result<ConversationMessage, ConversationError> {
        tracing::debug!("💾 Saving message to DB - session: {}, role: {:?}, content_len: {}",
            message.session_id, message.role, message.content.len());

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
    pub async fn get_conversation_history(&self, session_id: &str, limit: Option<i32>) -> Result<Vec<ConversationMessage>, ConversationError> {
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

        let messages = rows.into_iter().map(|(
            id, role, content, metadata, created_at, 
            prompt_tokens, completion_tokens, total_tokens, model, cost_usd
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
        }).collect();

        Ok(messages)
    }

    /// Build Gemini API request with full conversation history
    pub async fn build_chat_request(
        &self,
        session_id: &str,
        system_prompt: &str,
        user_message: &str,
        tools: Option<Vec<Tool>>,
    ) -> Result<GenerateContentRequest, ConversationError> {
        // Get conversation history
        let mut conversation_history = self.get_conversation_history(session_id, Some(20)).await?;

        // Ensure we start with system message (if not already present)
        let has_system_message = conversation_history.iter().any(|msg| matches!(msg.role, MessageRole::System));

        let mut contents = Vec::new();

        if !has_system_message {
            // Add system message at the beginning
            contents.push(Content {
                parts: vec![Part::Text { text: system_prompt.to_string() }],
                role: Some("model".to_string()), // Gemini pattern for system-like messages
            });
        }

        // Add conversation history
        for message in conversation_history {
            contents.push(message.to_gemini_content());
        }

        // Add current user message
        contents.push(Content {
            parts: vec![Part::Text { text: user_message.to_string() }],
            role: Some("user".to_string()),
        });

        Ok(GenerateContentRequest {
            contents,
            tools,
            generation_config: Some(GenerationConfig {
                temperature: 0.3,
                top_k: 40,
                top_p: 0.8,
                max_output_tokens: 2048,
            }),
            tool_config: Some(ToolConfig {
                function_calling_config: FunctionCallingConfig {
                    mode: FunctionCallingMode::Any,
                },
            }),
        })
    }

    /// Process conversation turn and save messages (LangChain pattern)
    pub async fn process_conversation_turn(
        &self,
        session_id: &str,
        user_message: &str,
        ai_response: &str,
        function_calls: Option<Vec<(String, Value, String)>>, // tool_name, args, result
    ) -> Result<Vec<ConversationMessage>, ConversationError> {
        let mut saved_messages = Vec::new();

        // Save user message
        let user_msg = ConversationMessage::new_human(session_id.to_string(), user_message.to_string());
        saved_messages.push(self.save_message(&user_msg).await?);

        // Save function calls if any
        if let Some(calls) = function_calls {
            for (tool_name, args, result) in calls {
                // Save function call
                let call_msg = ConversationMessage::new_function_call(session_id.to_string(), tool_name.clone(), args);
                saved_messages.push(self.save_message(&call_msg).await?);

                // Save function result
                let result_msg = ConversationMessage::new_function_result(session_id.to_string(), tool_name, result);
                saved_messages.push(self.save_message(&result_msg).await?);
            }
        }

        // Save AI response
        let ai_msg = ConversationMessage::new_assistant(session_id.to_string(), ai_response.to_string());
        saved_messages.push(self.save_message(&ai_msg).await?);

        Ok(saved_messages)
    }

    /// Get session database ID
    async fn get_session_db_id(&self, session_uuid: &str) -> Result<i32, ConversationError> {
        let row = sqlx::query_as::<_, (i32,)>(
            "SELECT id FROM chat_sessions WHERE session_uuid = $1"
        )
        .bind(session_uuid)
        .fetch_one(&self.db_pool)
        .await?;

        Ok(row.0)
    }

    /// Migrate from old chat_messages format (optional utility)
    pub async fn migrate_from_old_format(&self) -> Result<(), ConversationError> {
        // Get all old-format messages
        let old_messages = sqlx::query_as::<_, (i32, String, Option<String>, Option<String>, chrono::DateTime<chrono::Utc>)>(
            "SELECT cm.session_id, cs.session_uuid, cm.user_message, cm.ai_message, cm.created_at
             FROM chat_messages cm
             JOIN chat_sessions cs ON cm.session_id = cs.id
             ORDER BY cm.created_at ASC"
        )
        .fetch_all(&self.db_pool)
        .await?;

        for (_, session_uuid, user_msg, ai_msg, created_at) in old_messages {
            if let Some(user_message) = user_msg {
                let msg = ConversationMessage {
                    id: None,
                    session_id: session_uuid.clone(),
                    role: MessageRole::Human,
                    content: user_message,
                    metadata: None,
                    created_at: Some(created_at),
                    prompt_tokens: None,
                    completion_tokens: None,
                    total_tokens: None,
                    model: None,
                    cost_usd: None,
                };
                self.save_message(&msg).await?;
            }

            if let Some(ai_message) = ai_msg {
                let msg = ConversationMessage {
                    id: None,
                    session_id: session_uuid.clone(),
                    role: MessageRole::Assistant,
                    content: ai_message,
                    metadata: None,
                    created_at: Some(created_at),
                    prompt_tokens: None,
                    completion_tokens: None,
                    total_tokens: None,
                    model: None,
                    cost_usd: None,
                };
                self.save_message(&msg).await?;
            }
        }

        Ok(())
    }
}