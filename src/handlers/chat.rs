// src/handlers/chat.rs
use crate::handlers::upload::get_or_create_session;
use crate::middleware::auth::auth_middleware;
use crate::middleware::frontend_rate_limit::ai_operation_rate_limit_middleware;
use crate::AppState;
use axum::{
    extract::{
        ws::{Message, WebSocket, WebSocketUpgrade},
        Extension, Query,
    },
    response::IntoResponse,
    routing::get,
    Router,
};
use serde::{Deserialize, Serialize};
use futures::{sink::SinkExt, stream::StreamExt};
use std::sync::Arc;
use uuid;

// Helper function to format timestamps in a human-readable relative format
fn format_relative_time(timestamp: &chrono::DateTime<chrono::Utc>) -> String {
    let now = chrono::Utc::now();
    let duration = now.signed_duration_since(*timestamp);

    if duration.num_seconds() < 60 {
        "just now".to_string()
    } else if duration.num_minutes() < 60 {
        let mins = duration.num_minutes();
        if mins == 1 { "1 minute ago".to_string() } else { format!("{} minutes ago", mins) }
    } else if duration.num_hours() < 24 {
        let hours = duration.num_hours();
        if hours == 1 { "1 hour ago".to_string() } else { format!("{} hours ago", hours) }
    } else if duration.num_days() < 30 {
        let days = duration.num_days();
        if days == 1 { "1 day ago".to_string() } else { format!("{} days ago", days) }
    } else {
        timestamp.format("%B %d, %Y").to_string()
    }
}

#[derive(Serialize)]
#[serde(tag = "type")]
enum WebSocketMessage {
    #[serde(rename = "progress")]
    Progress {
        percentage: f32,
        message: String,
        operation_id: String,
    },
    #[serde(rename = "result")]  
    Result {
        content: String,
        operation_id: String,
    },
    #[serde(rename = "error")]
    Error {
        message: String,
        operation_id: String,
    },
}

#[derive(Deserialize)]
struct WebSocketQuery {
    session: Option<String>,
    model: Option<String>,
}

pub fn chat_routes() -> Router {
    let public_routes = Router::new()
        .route("/ws", get(websocket_handler))
        .layer(axum::middleware::from_fn(ai_operation_rate_limit_middleware));

    let protected_routes = Router::new()
        .route("/api/chat/history/:session_id", get(get_chat_history))
        .route("/api/chat/recent", get(get_recent_chats))
        .route("/api/chat/all", get(get_all_chats))
        .layer(axum::middleware::from_fn(auth_middleware));

    public_routes.merge(protected_routes)
}

async fn websocket_handler(
    ws: WebSocketUpgrade,
    Query(params): Query<WebSocketQuery>,
    Extension(state): Extension<Arc<AppState>>,
) -> impl IntoResponse {
    ws.on_upgrade(|socket| websocket(socket, state, params.session, params.model))
}

async fn websocket(stream: WebSocket, state: Arc<AppState>, session_uuid: Option<String>, _model_preference: Option<String>) {
    let (mut sender, mut receiver) = stream.split();

    // Use provided session UUID or generate a new one
    let session_id = session_uuid.unwrap_or_else(|| uuid::Uuid::new_v4().to_string());
    tracing::info!("🔌 Started new chat session: {}", session_id);

    // Ensure the session exists in the database
    let _ = get_or_create_session(&state, &session_id).await;

    // 🆕 BACKGROUND JOBS: Create progress channel for this WebSocket connection
    let (progress_tx, mut progress_rx) = tokio::sync::mpsc::unbounded_channel();
    state.job_manager.register_progress_sender(session_id.clone(), progress_tx).await;
    tracing::info!("📡 Registered progress updates for session: {}", session_id);

    // 🆕 AGENT PROGRESS: Create separate channel for agent thinking/tool calling updates
    let (agent_progress_tx, mut agent_progress_rx) = tokio::sync::mpsc::unbounded_channel();

    // Get default model from system settings (admin-configurable)
    let default_model = get_default_model(&state.db_pool).await;
    let use_claude = match default_model.as_str() {
        "claude" => state.claude_client.is_some(),
        "gemini" => false,
        _ => false, // Default to Gemini if unknown
    };

    if use_claude {
        tracing::info!("Using Claude AI (Sonnet 4.5) for session: {} [Admin Default]", session_id);
    } else {
        tracing::info!("Using Gemini AI (2.5 Flash) for session: {} [Admin Default]", session_id);
    }

    tracing::info!("✅ Initialized video editing agent for session: {}", session_id);

    // 🆕 BACKGROUND JOBS: Main event loop - handles both user messages AND progress updates
    tracing::info!("🔄 Entering WebSocket event loop for session: {}", session_id);

    loop {
        tokio::select! {
            // Handle incoming messages from user
            Some(Ok(message)) = receiver.next() => {
                tracing::debug!("📥 Received WebSocket message in session: {}", session_id);
                if let Message::Text(text) = message {
                    tracing::info!("💬 Got message in session {}: {}", session_id, text);

            // Build context from vector database if available (prefer Qdrant over AstraDB)
            let context = if let Some(ref qdrant_client) = state.qdrant_client {
                // Prefer Voyage embeddings for Claude, fallback to Gemini
                if let Some(ref voyage_embeddings) = state.voyage_embeddings {
                    match qdrant_client.build_context_for_query_with_voyage(&text, &session_id, voyage_embeddings).await {
                        Ok(ctx) => {
                            if !ctx.is_empty() {
                                tracing::debug!("Built context from Qdrant with Voyage AI: {} chars", ctx.len());
                                Some(ctx)
                            } else {
                                None
                            }
                        }
                        Err(e) => {
                            tracing::warn!("Failed to build context from Qdrant with Voyage: {}", e);
                            None
                        }
                    }
                } else if let Some(ref gemini_client) = state.gemini_client {
                    match qdrant_client.build_context_for_query_with_gemini(&text, &session_id, gemini_client).await {
                        Ok(ctx) => {
                            if !ctx.is_empty() {
                                tracing::debug!("Built context from Qdrant with Gemini: {} chars", ctx.len());
                                Some(ctx)
                            } else {
                                None
                            }
                        }
                        Err(e) => {
                            tracing::warn!("Failed to build context from Qdrant: {}", e);
                            None
                        }
                    }
                } else {
                    tracing::warn!("No embedding client available for Qdrant");
                    None
                }
            } else if let Some(ref vector_db) = state.vector_db {
                // Fallback to AstraDB
                if let Some(ref gemini_client) = state.gemini_client {
                    match vector_db.build_context_for_query_with_gemini(&text, &session_id, gemini_client).await {
                        Ok(ctx) => {
                            if !ctx.is_empty() {
                                tracing::debug!("Built context from AstraDB with Gemini: {} chars", ctx.len());
                                Some(ctx)
                            } else {
                                None
                            }
                        }
                        Err(e) => {
                            tracing::warn!("Failed to build context from AstraDB with Gemini: {}", e);
                            None
                        }
                    }
                } else {
                    match vector_db.build_context_for_query(&text, &session_id).await {
                        Ok(ctx) => {
                            if !ctx.is_empty() {
                                tracing::debug!("Built context from AstraDB (fallback): {} chars", ctx.len());
                                Some(ctx)
                            } else {
                                None
                            }
                        }
                        Err(e) => {
                            tracing::warn!("Failed to build context from AstraDB: {}", e);
                            None
                        }
                    }
                }
            } else {
                None
            };

            // Get uploaded files and output videos for this session
            let session_files = get_session_files(&session_id, &state).await.unwrap_or_default();
            let output_videos = get_session_output_videos(&session_id, &state).await.unwrap_or_default();
            let file_context = build_file_context(&session_files, &output_videos);
            
            if !session_files.is_empty() || !output_videos.is_empty() {
                tracing::info!("Including {} uploaded file(s) and {} output video(s) in AI context for session {}",
                              session_files.len(), output_videos.len(), session_id);
                tracing::debug!("Files: {:?}", session_files.iter().map(|f| &f.original_name).collect::<Vec<_>>());
                tracing::debug!("Output videos: {:?}", output_videos.iter().map(|v| &v.file_name).collect::<Vec<_>>());
            }

            // Check if we have context (before moving it)
            let has_context = context.is_some() || !file_context.is_empty();

            // Create enhanced query with file context and conversation context
            let enhanced_query = {
                let mut query_parts = Vec::new();

                // Add file context first (most important for the user's immediate request)
                if !file_context.is_empty() {
                    query_parts.push(file_context.clone());
                }

                // Add conversation context if available
                if let Some(ctx) = context {
                    query_parts.push(format!("PREVIOUS CONVERSATIONS CONTEXT:\n{}", ctx));
                }

                // Add the current user request
                query_parts.push(format!("USER REQUEST:\n{}", text));

                query_parts.join("\n\n")
            };

            // 🤖 AI-POWERED ROUTING: Let the AI decide when to start background jobs
            // The AI has a special tool called "start_background_job" that it can call
            use crate::agent::stateful_agent::{StatefulClaudeAgent, StatefulGeminiAgent};

            tracing::info!("🤖 Processing message with AI-powered routing");

            let response = if use_claude {
                if let Some(ref claude_client) = state.claude_client {
                    let agent = StatefulClaudeAgent::new(Arc::new(claude_client.clone()));

                    match agent.chat(
                        &text,
                        &session_id,
                        enhanced_query.clone(),
                        state.clone(),
                        state.job_manager.clone(),
                        Some(agent_progress_tx.clone()),
                    ).await {
                        Ok(resp) => resp,
                        Err(e) => format!("Sorry, I encountered an error: {}", e),
                    }
                } else {
                    "Claude client not configured".to_string()
                }
            } else {
                if let Some(ref gemini_client) = state.gemini_client {
                    let agent = StatefulGeminiAgent::new(Arc::new(gemini_client.clone()));

                    match agent.chat(
                        &text,
                        &session_id,
                        enhanced_query.clone(),
                        state.clone(),
                        state.job_manager.clone(),
                        Some(agent_progress_tx.clone()),
                    ).await {
                        Ok(resp) => resp,
                        Err(e) => format!("Sorry, I encountered an error: {}", e),
                    }
                } else {
                    "Gemini client not configured".to_string()
                }
            };

            // NOTE: Message saving is handled by the stateful agent (stateful_agent.rs)
            // to avoid duplicate database entries. The agent saves:
            // - User message before processing
            // - AI response after completion
            // - Qdrant vectorization for both

            // Send AI's response
            if !response.is_empty() {

                let json_response = serde_json::json!({
                    "type": "message",
                    "content": response,
                    "timestamp": chrono::Utc::now().to_rfc3339(),
                });

                if let Ok(json_str) = serde_json::to_string(&json_response) {
                    if sender.send(Message::Text(json_str)).await.is_err() {
                        tracing::error!("Failed to send message to WebSocket");
                        break;
                    }
                }
            }
                }
            }

            // 🆕 AGENT PROGRESS: Handle thinking/tool calling updates from agent
            Some(agent_msg) = agent_progress_rx.recv() => {
                tracing::debug!("🤖 Agent progress: {}", agent_msg);

                let json_response = serde_json::json!({
                    "type": "thinking",
                    "content": agent_msg,
                    "timestamp": chrono::Utc::now().to_rfc3339(),
                });

                if let Ok(json_str) = serde_json::to_string(&json_response) {
                    if sender.send(Message::Text(json_str)).await.is_err() {
                        tracing::error!("Failed to send agent progress update to WebSocket");
                        break;
                    }
                }
            }

            // 🆕 BACKGROUND JOBS: Handle progress updates from background jobs
            Some(progress_update) = progress_rx.recv() => {
                tracing::debug!("📡 Received progress update: {}", progress_update.message);

                // 💾 Save completed job results to conversation history (PostgreSQL + Qdrant)
                if let crate::jobs::JobStatus::Completed { ref result, .. } = progress_update.status {
                    if !result.is_empty() {
                        tracing::info!("💾 Saving completed job result to PostgreSQL for session: {}", session_id);
                        let conversation_manager = crate::agent::conversation_manager::ConversationManager::new(state.db_pool.clone());
                        let assistant_msg = crate::agent::conversation_manager::ConversationMessage::new_assistant(
                            session_id.clone(),
                            result.clone()
                        );
                        match conversation_manager.save_message(&assistant_msg).await {
                            Ok(_) => tracing::info!("✅ Saved completed job result to PostgreSQL"),
                            Err(e) => tracing::error!("❌ Failed to save job result to PostgreSQL: {}", e),
                        }

                        // 🔮 Also save to Qdrant vector database for enhanced context retrieval
                        if let Some(ref qdrant_client) = state.qdrant_client {
                            tracing::debug!("💾 Saving to Qdrant vector database for session: {}", session_id);
                            let files_referenced = vec![];
                            let context_data = std::collections::HashMap::new();

                            // Get the original user message from progress_update details if available
                            let user_message = if let Some(details) = &progress_update.details {
                                details.get("user_message")
                                    .and_then(|v| v.as_str())
                                    .unwrap_or("")
                                    .to_string()
                            } else {
                                String::new()
                            };

                            if let Some(ref voyage_embeddings) = state.voyage_embeddings {
                                if let Err(e) = qdrant_client.store_chat_memory_with_voyage(
                                    &session_id,
                                    None,
                                    &user_message,
                                    result,
                                    files_referenced.clone(),
                                    context_data.clone(),
                                    voyage_embeddings,
                                ).await {
                                    tracing::warn!("Failed to store in Qdrant (Voyage): {}", e);
                                }
                            } else if let Some(ref gemini_client) = state.gemini_client {
                                if let Err(e) = qdrant_client.store_chat_memory_with_gemini(
                                    &session_id,
                                    None,
                                    &user_message,
                                    result,
                                    files_referenced,
                                    context_data,
                                    gemini_client,
                                ).await {
                                    tracing::warn!("Failed to store in Qdrant (Gemini): {}", e);
                                }
                            }
                        }

                        // 🎯 Send the final result to the user as a regular message
                        let json_response = serde_json::json!({
                            "type": "message",
                            "content": result.clone(),
                            "timestamp": chrono::Utc::now().to_rfc3339(),
                        });

                        if let Ok(json_str) = serde_json::to_string(&json_response) {
                            if sender.send(Message::Text(json_str)).await.is_err() {
                                tracing::error!("Failed to send final result to WebSocket");
                                break;
                            }
                        }
                    }
                } else if let crate::jobs::JobStatus::Failed { ref error, .. } = progress_update.status {
                    // Send error messages to the user as regular message
                    let json_response = serde_json::json!({
                        "type": "message",
                        "content": format!("❌ {}", error),
                        "timestamp": chrono::Utc::now().to_rfc3339(),
                    });

                    if let Ok(json_str) = serde_json::to_string(&json_response) {
                        if sender.send(Message::Text(json_str)).await.is_err() {
                            tracing::error!("Failed to send error message to WebSocket");
                            break;
                        }
                    }
                } else {
                    // 💭 Send intermediate progress updates (not as chat messages)
                    let json_response = serde_json::json!({
                        "type": "progress",
                        "content": progress_update.message,
                        "timestamp": chrono::Utc::now().to_rfc3339(),
                    });

                    if let Ok(json_str) = serde_json::to_string(&json_response) {
                        if sender.send(Message::Text(json_str)).await.is_err() {
                            tracing::error!("Failed to send progress indicator to WebSocket");
                            break;
                        }
                    }
                }
            }

            // WebSocket closed - both streams ended
            else => {
                tracing::warn!("❌ WebSocket event loop ended (both streams closed) for session: {}", session_id);
                tracing::warn!("This should NOT happen unless client disconnected or channels closed");
                break;
            }
        }
    }

    // Cleanup: Unregister progress sender when WebSocket disconnects
    state.job_manager.unregister_progress_sender(&session_id).await;
    tracing::info!("🔌 WebSocket handler exiting for session: {}", session_id);
}


// Get uploaded files for the current session
async fn get_session_files(session_id: &str, state: &AppState) -> Result<Vec<crate::models::file::UploadedFile>, sqlx::Error> {
    let files = sqlx::query_as::<_, crate::models::file::UploadedFile>(
        "SELECT uf.* FROM uploaded_files uf 
         JOIN chat_sessions cs ON uf.session_id = cs.id 
         WHERE cs.session_uuid = $1 
         ORDER BY uf.created_at DESC"
    )
    .bind(session_id)
    .fetch_all(&state.db_pool)
    .await?;
    
    Ok(files)
}

async fn get_session_output_videos(session_id: &str, state: &AppState) -> Result<Vec<crate::models::file::OutputVideo>, sqlx::Error> {
    let output_videos = sqlx::query_as::<_, crate::models::file::OutputVideo>(
        "SELECT ov.* FROM output_videos ov 
         JOIN chat_sessions cs ON ov.session_id = cs.id 
         WHERE cs.session_uuid = $1 AND ov.processing_status = 'completed'
         ORDER BY ov.created_at DESC"
    )
    .bind(session_id)
    .fetch_all(&state.db_pool)
    .await?;
    
    Ok(output_videos)
}

// Build file context string for AI agent
fn build_file_context(files: &[crate::models::file::UploadedFile], output_videos: &[crate::models::file::OutputVideo]) -> String {
    let mut context = String::new();
    
    // Add uploaded files section
    if !files.is_empty() {
        context.push_str("UPLOADED FILES IN THIS CHAT SESSION:\n");
        
        for (index, file) in files.iter().enumerate() {
            context.push_str(&format!(
                "{}. \"{}\" - USE THIS PATH: {}\n   - Type: {} ({})\n   - Size: {:.2} MB\n   - Uploaded: {}\n\n",
                index + 1,
                file.original_name,
                file.file_path,
                file.file_type,
                file.mime_type.as_deref().unwrap_or("unknown"),
                file.file_size as f64 / (1024.0 * 1024.0),
                file.created_at.format("%Y-%m-%d %H:%M:%S UTC")
            ));
        }
    }
    
    // Add output videos section
    if !output_videos.is_empty() {
        context.push_str("PREVIOUSLY GENERATED OUTPUT VIDEOS IN THIS SESSION:\n");
        
        for (index, video) in output_videos.iter().enumerate() {
            context.push_str(&format!(
                "{}. \"{}\" - USE THIS PATH: {}\n   - Operation: {} using {}\n   - Size: {:.2} MB\n   - Created: {}\n\n",
                index + 1,
                video.file_name,
                video.file_path,
                video.operation_type,
                video.tool_used,
                video.file_size as f64 / (1024.0 * 1024.0),
                video.created_at.format("%Y-%m-%d %H:%M:%S UTC")
            ));
        }
    }

    if !files.is_empty() || !output_videos.is_empty() {
        context.push_str("CRITICAL INSTRUCTION: When using ANY video editing tool, you MUST use the PATH shown above (the path after 'USE THIS PATH:'). NEVER use just the filename like 'GothamChess.mp4' - always use the full path like 'uploads/uuid_files.mp4'. The tools will FAIL if you use only the filename!\n\n");
    }
    
    context
}

// Get default AI model from system settings (admin-configurable)
// Returns the actual provider identifier ("claude" or "gemini"), not the full model name
async fn get_default_model(pool: &sqlx::PgPool) -> String {
    let result = sqlx::query_scalar::<_, String>(
        "SELECT setting_value FROM system_settings WHERE setting_key = 'default_ai_model'"
    )
    .fetch_optional(pool)
    .await;

    match result {
        Ok(Some(model)) => {
            tracing::debug!("Using admin-configured default model provider: {}", model);
            model
        }
        Ok(None) => {
            tracing::info!("No default model configured, using Gemini as fallback");
            "gemini".to_string()
        }
        Err(e) => {
            tracing::warn!("Failed to fetch default model from settings: {}, using Gemini", e);
            "gemini".to_string()
        }
    }
}

// Extract file references from user message
async fn extract_file_references(_message: &str, session_id: &str, state: &AppState) -> Vec<String> {
    let mut file_references = Vec::new();
    
    // Get all files for this session
    if let Ok(files) = sqlx::query_scalar::<_, String>(
        "SELECT uf.id FROM uploaded_files uf JOIN chat_sessions cs ON uf.session_id = cs.id WHERE cs.session_uuid = $1"
    )
    .bind(session_id)
    .fetch_all(&state.db_pool)
    .await
    {
        // For now, just return the file IDs as potential references
        // In a full implementation, we'd check if the message mentions specific files
        file_references = files;
    }
    
    file_references
}

async fn get_chat_history(
    axum::extract::Path(session_id): axum::extract::Path<String>,
    Extension(state): Extension<Arc<AppState>>,
    Extension(claims): Extension<crate::models::auth::Claims>,
) -> Result<axum::response::Json<serde_json::Value>, axum::http::StatusCode> {
    // CRITICAL: Verify that the session belongs to the authenticated user
    let user_id = claims.sub.parse::<i32>().unwrap_or(0);
    
    // Check if the session belongs to the user
    let session_owner = sqlx::query_scalar::<_, i32>(
        "SELECT user_id FROM chat_sessions WHERE session_uuid = $1"
    )
    .bind(&session_id)
    .fetch_optional(&state.db_pool)
    .await
    .map_err(|e| {
        tracing::error!("Failed to verify session ownership: {}", e);
        axum::http::StatusCode::INTERNAL_SERVER_ERROR
    })?;
    
    // If session doesn't exist or doesn't belong to the user, return forbidden
    match session_owner {
        Some(owner_id) if owner_id == user_id => {
            // User owns this session, proceed with fetching history
        }
        Some(_) => {
            // Session exists but belongs to another user
            tracing::warn!("User {} attempted to access session {} owned by another user", user_id, session_id);
            return Ok(axum::response::Json(serde_json::json!({
                "success": false,
                "message": "Access denied: You don't have permission to view this chat session",
                "history": []
            })));
        }
        None => {
            // Session doesn't exist
            return Ok(axum::response::Json(serde_json::json!({
                "success": false,
                "message": "Chat session not found",
                "history": []
            })));
        }
    }
    
    // Fetch history from PostgreSQL - try conversation_messages first (new schema)
    tracing::debug!("Fetching conversation history for session: {}", session_id);

    let new_messages = sqlx::query_as::<_, (String, String, chrono::DateTime<chrono::Utc>)>(
        "SELECT role, content, created_at
         FROM conversation_messages
         WHERE session_id = (SELECT id FROM chat_sessions WHERE session_uuid = $1)
         ORDER BY created_at ASC
         LIMIT 200"
    )
    .bind(&session_id)
    .fetch_all(&state.db_pool)
    .await;

    match new_messages {
        Ok(msgs) if !msgs.is_empty() => {
            tracing::info!("Found {} messages in conversation_messages table for session {}", msgs.len(), session_id);

            // Log all messages for debugging
            for (i, (role, content, _)) in msgs.iter().enumerate() {
                tracing::debug!("Message {}: role='{}', content_length={}", i, role, content.len());
            }

            // Reconstruct conversation from role-based messages
            let mut formatted_history: Vec<serde_json::Value> = Vec::new();
            let mut current_user_msg: Option<(String, chrono::DateTime<chrono::Utc>)> = None;

            for (role, content, timestamp) in msgs {
                match role.as_str() {
                    "user" => {
                        // If there's a pending user message, add it as standalone
                        if let Some((pending_user, pending_ts)) = current_user_msg.take() {
                            formatted_history.push(serde_json::json!({
                                "timestamp": pending_ts.format("%Y-%m-%d %H:%M:%S%.3f UTC").to_string(),
                                "timestamp_relative": format_relative_time(&pending_ts),
                                "user_message": pending_user,
                                "agent_response": ""
                            }));
                        }
                        current_user_msg = Some((content, timestamp));
                    }
                    "model" | "assistant" => {  // Handle BOTH "model" (Gemini) and "assistant" (Claude)
                        if let Some((user_msg, user_ts)) = current_user_msg.take() {
                            // Pair with previous user message
                            formatted_history.push(serde_json::json!({
                                "timestamp": user_ts.format("%Y-%m-%d %H:%M:%S%.3f UTC").to_string(),
                                "timestamp_relative": format_relative_time(&user_ts),
                                "user_message": user_msg,
                                "agent_response": content
                            }));
                        } else {
                            // Standalone assistant message (no preceding user message)
                            formatted_history.push(serde_json::json!({
                                "timestamp": timestamp.format("%Y-%m-%d %H:%M:%S%.3f UTC").to_string(),
                                "timestamp_relative": format_relative_time(&timestamp),
                                "user_message": "",
                                "agent_response": content
                            }));
                        }
                    }
                    _ => {} // Skip system, function messages for now
                }
            }

            // Handle any remaining unpaired user message
            if let Some((pending_user, pending_ts)) = current_user_msg.take() {
                formatted_history.push(serde_json::json!({
                    "timestamp": pending_ts.format("%Y-%m-%d %H:%M:%S%.3f UTC").to_string(),
                    "timestamp_relative": format_relative_time(&pending_ts),
                    "user_message": pending_user,
                    "agent_response": ""
                }));
            }

            Ok(axum::response::Json(serde_json::json!({
                "success": true,
                "session_id": session_id,
                "history": formatted_history
            })))
        }
        Ok(_) => {
            tracing::info!("No messages found in conversation_messages table for session {}. Falling back to chat_messages", session_id);

            let old_messages = sqlx::query_as::<_, (String, String, chrono::DateTime<chrono::Utc>)>(
                "SELECT user_message, ai_message, created_at
                 FROM chat_messages
                 WHERE session_id = (SELECT id FROM chat_sessions WHERE session_uuid = $1)
                 ORDER BY created_at ASC
                 LIMIT 100"
            )
            .bind(&session_id)
            .fetch_all(&state.db_pool)
            .await;

            match old_messages {
                Ok(msgs) if !msgs.is_empty() => {
                    tracing::info!("Found {} messages in chat_messages table for session {}", msgs.len(), session_id);
                    let formatted_history: Vec<serde_json::Value> = msgs
                        .into_iter()
                        .map(|(user_msg, assistant_msg, timestamp)| {
                            serde_json::json!({
                                "timestamp": timestamp.format("%Y-%m-%d %H:%M:%S%.3f UTC").to_string(),
                                "timestamp_relative": format_relative_time(&timestamp),
                                "user_message": user_msg,
                                "agent_response": assistant_msg
                            })
                        })
                        .collect();

                    Ok(axum::response::Json(serde_json::json!({
                        "success": true,
                        "session_id": session_id,
                        "history": formatted_history
                    })))
                }
                _ => {
                    tracing::warn!("No messages found in chat_messages table either for session {}", session_id);
                    Ok(axum::response::Json(serde_json::json!({
                        "success": true,
                        "session_id": session_id,
                        "history": []
                    })))
                }
            }
        }
        Err(e) => {
            tracing::warn!("Error fetching from conversation_messages: {}. Falling back to chat_messages", e);

            let old_messages = sqlx::query_as::<_, (String, String, chrono::DateTime<chrono::Utc>)>(
                "SELECT user_message, ai_message, created_at
                 FROM chat_messages
                 WHERE session_id = (SELECT id FROM chat_sessions WHERE session_uuid = $1)
                 ORDER BY created_at ASC
                 LIMIT 100"
            )
            .bind(&session_id)
            .fetch_all(&state.db_pool)
            .await;

            match old_messages {
                Ok(msgs) if !msgs.is_empty() => {
                    tracing::info!("Found {} messages in chat_messages table for session {}", msgs.len(), session_id);
                    let formatted_history: Vec<serde_json::Value> = msgs
                        .into_iter()
                        .map(|(user_msg, assistant_msg, timestamp)| {
                            serde_json::json!({
                                "timestamp": timestamp.format("%Y-%m-%d %H:%M:%S%.3f UTC").to_string(),
                                "timestamp_relative": format_relative_time(&timestamp),
                                "user_message": user_msg,
                                "agent_response": assistant_msg
                            })
                        })
                        .collect();

                    Ok(axum::response::Json(serde_json::json!({
                        "success": true,
                        "session_id": session_id,
                        "history": formatted_history
                    })))
                }
                Ok(_) => {
                    tracing::warn!("No messages found in chat_messages table for session {}", session_id);
                    // No messages in either table, return empty
                    Ok(axum::response::Json(serde_json::json!({
                        "success": true,
                        "session_id": session_id,
                        "history": []
                    })))
                }
                Err(e) => {
                    tracing::error!("Error fetching from chat_messages table for session {}: {}", session_id, e);
                    // No messages in either table, return empty
                    Ok(axum::response::Json(serde_json::json!({
                        "success": true,
                        "session_id": session_id,
                        "history": []
                    })))
                }
            }
        }
    }
}

async fn get_recent_chats(
    Extension(state): Extension<Arc<AppState>>,
    Extension(claims): Extension<crate::models::auth::Claims>,
) -> Result<axum::response::Json<serde_json::Value>, axum::http::StatusCode> {
    // Get recent chat sessions for the user from the database
    match sqlx::query_as::<_, (i32, String, String, chrono::DateTime<chrono::Utc>)>(
        "SELECT id, session_uuid, title, created_at FROM chat_sessions WHERE user_id = $1 ORDER BY created_at DESC LIMIT 10"
    )
    .bind(claims.sub.parse::<i32>().unwrap_or(0))
    .fetch_all(&state.db_pool)
    .await
    {
        Ok(rows) => {
            let chats: Vec<serde_json::Value> = rows
                .into_iter()
                .map(|(id, session_uuid, title, created_at)| {
                    serde_json::json!({
                        "id": id,
                        "session_id": session_uuid,
                        "title": title,
                        "created_at": created_at.format("%Y-%m-%d %H:%M:%S").to_string()
                    })
                })
                .collect();

            Ok(axum::response::Json(serde_json::json!({
                "success": true,
                "chats": chats
            })))
        }
        Err(e) => {
            tracing::error!("Failed to get recent chats: {}", e);
            Err(axum::http::StatusCode::INTERNAL_SERVER_ERROR)
        }
    }
}

#[derive(serde::Deserialize)]
struct AllChatsQuery {
    page: Option<i64>,
    limit: Option<i64>,
}

async fn get_all_chats(
    Query(params): Query<AllChatsQuery>,
    Extension(state): Extension<Arc<AppState>>,
    Extension(claims): Extension<crate::models::auth::Claims>,
) -> Result<axum::response::Json<serde_json::Value>, axum::http::StatusCode> {
    let page = params.page.unwrap_or(1).max(1);
    let limit = params.limit.unwrap_or(20).max(1).min(100);
    let offset = (page - 1) * limit;

    let user_id = claims.sub.parse::<i32>().unwrap_or(0);

    // Get total count
    let total_count: (i64,) = sqlx::query_as(
        "SELECT COUNT(*) FROM chat_sessions WHERE user_id = $1"
    )
    .bind(user_id)
    .fetch_one(&state.db_pool)
    .await
    .map_err(|e| {
        tracing::error!("Failed to get chat count: {}", e);
        axum::http::StatusCode::INTERNAL_SERVER_ERROR
    })?;

    // Get paginated chats
    let rows = sqlx::query_as::<_, (i32, String, String, chrono::DateTime<chrono::Utc>)>(
        "SELECT cs.id, cs.session_uuid, cs.title, cs.created_at
         FROM chat_sessions cs
         WHERE cs.user_id = $1
         ORDER BY cs.created_at DESC
         LIMIT $2 OFFSET $3"
    )
    .bind(user_id)
    .bind(limit)
    .bind(offset)
    .fetch_all(&state.db_pool)
    .await
    .map_err(|e| {
        tracing::error!("Failed to get all chats: {}", e);
        axum::http::StatusCode::INTERNAL_SERVER_ERROR
    })?;

    let mut chats = Vec::new();
    for (id, session_uuid, title, created_at) in rows {
        // Get message count for this session
        let message_count: (i64,) = sqlx::query_as(
            "SELECT COUNT(*) FROM conversation_messages WHERE session_id = $1"
        )
        .bind(id)
        .fetch_one(&state.db_pool)
        .await
        .unwrap_or((0,));

        chats.push(serde_json::json!({
            "id": id,
            "session_id": session_uuid,
            "title": title,
            "created_at": created_at.format("%Y-%m-%d %H:%M:%S").to_string(),
            "message_count": message_count.0
        }));
    }

    Ok(axum::response::Json(serde_json::json!({
        "success": true,
        "chats": chats,
        "pagination": {
            "page": page,
            "limit": limit,
            "total": total_count.0,
            "total_pages": (total_count.0 + limit - 1) / limit
        }
    })))
}
