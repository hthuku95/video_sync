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
use futures::{sink::SinkExt, stream::StreamExt};
use serde::{Deserialize, Serialize};
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
        if mins == 1 {
            "1 minute ago".to_string()
        } else {
            format!("{} minutes ago", mins)
        }
    } else if duration.num_hours() < 24 {
        let hours = duration.num_hours();
        if hours == 1 {
            "1 hour ago".to_string()
        } else {
            format!("{} hours ago", hours)
        }
    } else if duration.num_days() < 30 {
        let days = duration.num_days();
        if days == 1 {
            "1 day ago".to_string()
        } else {
            format!("{} days ago", days)
        }
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
    token: Option<String>,
}

pub fn chat_routes() -> Router {
    let public_routes =
        Router::new()
            .route("/ws", get(websocket_handler))
            .layer(axum::middleware::from_fn(
                ai_operation_rate_limit_middleware,
            ));

    let protected_routes = Router::new()
        .route("/api/chat/history/:session_id", get(get_chat_history))
        .route("/api/chat/recent", get(get_recent_chats))
        .route("/api/chat/all", get(get_all_chats))
        .route(
            "/api/chat/sessions/:session_uuid/jobs",
            get(get_session_jobs),
        )
        // Paywall: trial-expired users hit HTTP 402 when trying to read
        // chat history — they still see /subscribe in upgrade_url.
        .layer(axum::middleware::from_fn(
            crate::middleware::subscription::subscription_middleware,
        ))
        .layer(axum::middleware::from_fn(auth_middleware));

    public_routes.merge(protected_routes)
}

async fn websocket_handler(
    ws: WebSocketUpgrade,
    Query(params): Query<WebSocketQuery>,
    Extension(state): Extension<Arc<AppState>>,
) -> axum::response::Response {
    // Validate JWT before upgrading — WebSocket upgrades are HTTP so we can
    // extract the user_id here and pass it in. The token travels as a query
    // param because WS clients cannot set Authorization headers.
    let user_id: Option<i32> =
        params
            .token
            .as_deref()
            .and_then(|t| match crate::handlers::auth::verify_jwt_token(t) {
                Ok(claims) => claims.sub.parse::<i32>().ok(),
                Err(_) => None,
            });

    // Subscription gate — the WS route doesn't flow through the standard
    // axum middleware stack for the upgrade handshake, so we check here.
    // trial / active / grandfathered / staff / superuser pass; expired
    // users hit 402 with the upgrade link.
    if let Some(uid) = user_id {
        use axum::http::StatusCode;
        match subscription_ok(&state, uid).await {
            Ok(true) => {}
            Ok(false) => {
                return (
                    StatusCode::PAYMENT_REQUIRED,
                    axum::Json(serde_json::json!({
                        "success":     false,
                        "error":       "subscription_required",
                        "message":     "Your free trial has ended. Subscribe for $15/mo USDC to keep the chat running.",
                        "upgrade_url": "/subscribe",
                    })),
                ).into_response();
            }
            Err(e) => {
                tracing::warn!("WS subscription check failed for user {}: {}", uid, e);
                // fail-open rather than blocking legit users on DB hiccup;
                // paywalled compute routes will still gate properly.
            }
        }
    }

    ws.on_upgrade(move |socket| websocket(socket, state, params.session, params.model, user_id))
        .into_response()
}

/// Returns Ok(true) if the user is allowed to use paid compute right now.
/// Mirrors the logic in src/middleware/subscription.rs but called inline
/// because the WS upgrade bypasses standard middleware.
async fn subscription_ok(state: &Arc<AppState>, user_id: i32) -> Result<bool, sqlx::Error> {
    let row = sqlx::query(
        "SELECT is_staff, is_superuser, subscription_status, trial_ends_at, subscription_active_until
         FROM users WHERE id = $1"
    )
    .bind(user_id)
    .fetch_optional(&state.db_pool)
    .await?;

    let Some(row) = row else {
        return Ok(false);
    };
    use sqlx::Row;
    let is_staff: bool = row.try_get("is_staff").unwrap_or(false);
    let is_super: bool = row.try_get("is_superuser").unwrap_or(false);
    if is_staff || is_super {
        return Ok(true);
    }

    let status: String = row
        .try_get::<Option<String>, _>("subscription_status")
        .ok()
        .flatten()
        .unwrap_or_else(|| "grandfathered".to_string());
    let now = chrono::Utc::now();

    Ok(match status.as_str() {
        "grandfathered" => true,
        "trial" => row
            .try_get::<Option<chrono::DateTime<chrono::Utc>>, _>("trial_ends_at")
            .ok()
            .flatten()
            .map(|t| t > now)
            .unwrap_or(false),
        "active" => row
            .try_get::<Option<chrono::DateTime<chrono::Utc>>, _>("subscription_active_until")
            .ok()
            .flatten()
            .map(|t| t > now)
            .unwrap_or(false),
        _ => false,
    })
}

async fn websocket(
    stream: WebSocket,
    state: Arc<AppState>,
    session_uuid: Option<String>,
    _model_preference: Option<String>,
    user_id: Option<i32>,
) {
    let (mut sender, mut receiver) = stream.split();

    // Use provided session UUID or generate a new one
    let session_id = session_uuid.unwrap_or_else(|| uuid::Uuid::new_v4().to_string());
    tracing::info!(
        "🔌 Started new chat session: {} (user_id: {:?})",
        session_id,
        user_id
    );

    // Ensure the session exists in the database, attributed to the authenticated user
    let _ = get_or_create_session(&state, &session_id, user_id).await;

    // 🆕 BACKGROUND JOBS: Create progress channel for this WebSocket connection
    let (progress_tx, mut progress_rx) = tokio::sync::mpsc::unbounded_channel();
    state
        .job_manager
        .register_progress_sender(session_id.clone(), progress_tx)
        .await;
    tracing::info!("📡 Registered progress updates for session: {}", session_id);

    // 🆕 AGENT PROGRESS: Create separate channel for agent thinking/tool calling updates
    let (agent_progress_tx, mut agent_progress_rx) = tokio::sync::mpsc::unbounded_channel();

    // On reconnect: check if there's a running background agent job for this session
    // and immediately inform the user so they know work is still happening
    if let Some(running_msg) = get_running_agent_job_status(&state, &session_id).await {
        let json_response = serde_json::json!({
            "type": "background_job_status",
            "content": running_msg,
            "timestamp": chrono::Utc::now().to_rfc3339(),
        });
        if let Ok(json_str) = serde_json::to_string(&json_response) {
            let _ = sender.send(Message::Text(json_str)).await;
        }
    }

    // Get default model from system settings (admin-configurable)
    let default_model = get_default_model(&state.db_pool).await;
    let use_claude = match default_model.as_str() {
        "claude" => state.claude_client.is_some(),
        "gemini" => false,
        _ => false, // Default to Gemini if unknown
    };

    if use_claude {
        tracing::info!(
            "Using Claude AI (Sonnet 4.5) for session: {} [Admin Default]",
            session_id
        );
    } else {
        tracing::info!(
            "Using Gemini AI (2.5 Flash) for session: {} [Admin Default]",
            session_id
        );
    }

    tracing::info!(
        "✅ Initialized video editing agent for session: {}",
        session_id
    );

    // 🆕 BACKGROUND JOBS: Main event loop - handles both user messages AND progress updates
    tracing::info!(
        "🔄 Entering WebSocket event loop for session: {}",
        session_id
    );

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
                } else if let Some(ref gemini_client) = state.video_gemini_client.as_ref().or(state.gemini_client.as_ref()) {
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
            let _has_context = context.is_some() || !file_context.is_empty();

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

            // 🤖 AI-POWERED ROUTING: Spawn agent as background task so it survives
            // WebSocket disconnects. User can close the page and come back — the task
            // keeps running and the result will be delivered on reconnect via job_manager.
            use crate::agent::stateful_agent::{StatefulClaudeAgent, StatefulGeminiAgent};

            tracing::info!("🤖 Spawning AI agent as background task for session: {}", session_id);

            // Create a DB record for this job so we can replay it on reconnect
            let job_id = create_agent_job(&state, &session_id, &text).await;

            // Clone everything the background task needs (state is Arc, cheap clone)
            let state_bg = state.clone();
            let session_id_bg = session_id.clone();
            let text_bg = text.clone();
            let enhanced_query_bg = enhanced_query.clone();
            let job_manager_bg = state.job_manager.clone();
            let agent_tx_bg = agent_progress_tx.clone();

            tokio::spawn(async move {
                run_agent_background(
                    state_bg,
                    session_id_bg,
                    text_bg,
                    enhanced_query_bg,
                    use_claude,
                    job_id,
                    job_manager_bg,
                    agent_tx_bg,
                ).await;
            });

            // Send immediate ACK so the user knows the agent has started
            let ack = serde_json::json!({
                "type": "thinking",
                "content": "⏳ Working on this in the background. You can close this page and come back — I'll keep working and your result will be here when you return.",
                "timestamp": chrono::Utc::now().to_rfc3339(),
            });
            if let Ok(json_str) = serde_json::to_string(&ack) {
                if sender.send(Message::Text(json_str)).await.is_err() {
                    tracing::error!("Failed to send ACK to WebSocket");
                    break;
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
                        // NOTE: Message is already saved by the job itself (video_job.rs:197)
                        // Saving here would create duplicate messages after page refresh
                        tracing::debug!("📨 Job completed with result (message already saved by job): {}", result.chars().take(100).collect::<String>());

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
                                    Some("general"),
                                ).await {
                                    tracing::warn!("Failed to store in Qdrant (Voyage): {}", e);
                                }
                            } else if let Some(ref gemini_client) = state.video_gemini_client.as_ref().or(state.gemini_client.as_ref()) {
                                if let Err(e) = qdrant_client.store_chat_memory_with_gemini(
                                    &session_id,
                                    None,
                                    &user_message,
                                    result,
                                    files_referenced,
                                    context_data,
                                    gemini_client,
                                    Some("general"),
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
    state
        .job_manager
        .unregister_progress_sender(&session_id)
        .await;
    tracing::info!("🔌 WebSocket handler exiting for session: {}", session_id);
}

// Get uploaded files for the current session
async fn get_session_files(
    session_id: &str,
    state: &AppState,
) -> Result<Vec<crate::models::file::UploadedFile>, sqlx::Error> {
    let files = sqlx::query_as::<_, crate::models::file::UploadedFile>(
        "SELECT uf.* FROM uploaded_files uf 
         JOIN chat_sessions cs ON uf.session_id = cs.id 
         WHERE cs.session_uuid = $1 
         ORDER BY uf.created_at DESC",
    )
    .bind(session_id)
    .fetch_all(&state.db_pool)
    .await?;

    Ok(files)
}

async fn get_session_output_videos(
    session_id: &str,
    state: &AppState,
) -> Result<Vec<crate::models::file::OutputVideo>, sqlx::Error> {
    let output_videos = sqlx::query_as::<_, crate::models::file::OutputVideo>(
        "SELECT ov.* FROM output_videos ov 
         JOIN chat_sessions cs ON ov.session_id = cs.id 
         WHERE cs.session_uuid = $1 AND ov.processing_status = 'completed'
         ORDER BY ov.created_at DESC",
    )
    .bind(session_id)
    .fetch_all(&state.db_pool)
    .await?;

    Ok(output_videos)
}

fn output_file_id_from_path(path: &str) -> String {
    use std::collections::hash_map::DefaultHasher;
    use std::hash::{Hash, Hasher};

    let mut hasher = DefaultHasher::new();
    path.hash(&mut hasher);
    format!("{:x}", hasher.finish())
}

// Build file context string for AI agent
fn build_file_context(
    files: &[crate::models::file::UploadedFile],
    output_videos: &[crate::models::file::OutputVideo],
) -> String {
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
            let file_id = output_file_id_from_path(&video.file_path);
            context.push_str(&format!(
                "{}. \"{}\" - USE THIS PATH: {}\n   - Operation: {} using {}\n   - Size: {:.2} MB\n   - Watch link: /api/outputs/stream/{}\n   - Download link: /api/outputs/download/{}\n   - Created: {}\n\n",
                index + 1,
                video.file_name,
                video.file_path,
                video.operation_type,
                video.tool_used,
                video.file_size as f64 / (1024.0 * 1024.0),
                file_id,
                file_id,
                video.created_at.format("%Y-%m-%d %H:%M:%S UTC")
            ));
        }
    }

    if !files.is_empty() || !output_videos.is_empty() {
        context.push_str("CRITICAL INSTRUCTION: When using ANY video editing tool, you MUST use the PATH shown above (the path after 'USE THIS PATH:'). NEVER use just the filename like 'GothamChess.mp4' - always use the full path like 'uploads/uuid_files.mp4'. The tools will FAIL if you use only the filename!\n");
        context.push_str("CRITICAL USER-FACING INSTRUCTION: Internal server paths are for tool execution only. When telling the user where to watch or download a completed output video, prefer the Watch link or Download link shown above instead of exposing the raw internal path.\n\n");
    }

    context
}

// Get default AI model from system settings (admin-configurable)
// Returns the actual provider identifier ("claude" or "gemini"), not the full model name
async fn get_default_model(pool: &sqlx::PgPool) -> String {
    let result = sqlx::query_scalar::<_, String>(
        "SELECT setting_value FROM system_settings WHERE setting_key = 'default_ai_model'",
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
            tracing::warn!(
                "Failed to fetch default model from settings: {}, using Gemini",
                e
            );
            "gemini".to_string()
        }
    }
}

// Extract file references from user message
async fn extract_file_references(
    _message: &str,
    session_id: &str,
    state: &AppState,
) -> Vec<String> {
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
    let session_owner =
        sqlx::query_scalar::<_, i32>("SELECT user_id FROM chat_sessions WHERE session_uuid = $1")
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
            tracing::warn!(
                "User {} attempted to access session {} owned by another user",
                user_id,
                session_id
            );
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
         LIMIT 200",
    )
    .bind(&session_id)
    .fetch_all(&state.db_pool)
    .await;

    match new_messages {
        Ok(msgs) if !msgs.is_empty() => {
            tracing::info!(
                "Found {} messages in conversation_messages table for session {}",
                msgs.len(),
                session_id
            );

            // Log all messages for debugging
            for (i, (role, content, _)) in msgs.iter().enumerate() {
                tracing::debug!(
                    "Message {}: role='{}', content_length={}",
                    i,
                    role,
                    content.len()
                );
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
                    "model" | "assistant" => {
                        // Handle BOTH "model" (Gemini) and "assistant" (Claude)
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

            let old_messages =
                sqlx::query_as::<_, (String, String, chrono::DateTime<chrono::Utc>)>(
                    "SELECT user_message, ai_message, created_at
                 FROM chat_messages
                 WHERE session_id = (SELECT id FROM chat_sessions WHERE session_uuid = $1)
                 ORDER BY created_at ASC
                 LIMIT 100",
                )
                .bind(&session_id)
                .fetch_all(&state.db_pool)
                .await;

            match old_messages {
                Ok(msgs) if !msgs.is_empty() => {
                    tracing::info!(
                        "Found {} messages in chat_messages table for session {}",
                        msgs.len(),
                        session_id
                    );
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
                    tracing::warn!(
                        "No messages found in chat_messages table either for session {}",
                        session_id
                    );
                    Ok(axum::response::Json(serde_json::json!({
                        "success": true,
                        "session_id": session_id,
                        "history": []
                    })))
                }
            }
        }
        Err(e) => {
            tracing::warn!(
                "Error fetching from conversation_messages: {}. Falling back to chat_messages",
                e
            );

            let old_messages =
                sqlx::query_as::<_, (String, String, chrono::DateTime<chrono::Utc>)>(
                    "SELECT user_message, ai_message, created_at
                 FROM chat_messages
                 WHERE session_id = (SELECT id FROM chat_sessions WHERE session_uuid = $1)
                 ORDER BY created_at ASC
                 LIMIT 100",
                )
                .bind(&session_id)
                .fetch_all(&state.db_pool)
                .await;

            match old_messages {
                Ok(msgs) if !msgs.is_empty() => {
                    tracing::info!(
                        "Found {} messages in chat_messages table for session {}",
                        msgs.len(),
                        session_id
                    );
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
                    tracing::warn!(
                        "No messages found in chat_messages table for session {}",
                        session_id
                    );
                    // No messages in either table, return empty
                    Ok(axum::response::Json(serde_json::json!({
                        "success": true,
                        "session_id": session_id,
                        "history": []
                    })))
                }
                Err(e) => {
                    tracing::error!(
                        "Error fetching from chat_messages table for session {}: {}",
                        session_id,
                        e
                    );
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
    let total_count: (i64,) =
        sqlx::query_as("SELECT COUNT(*) FROM chat_sessions WHERE user_id = $1")
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
         LIMIT $2 OFFSET $3",
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
        let message_count: (i64,) =
            sqlx::query_as("SELECT COUNT(*) FROM conversation_messages WHERE session_id = $1")
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

// ─── Background agent job helpers ────────────────────────────────────────────

/// Create a DB record for a background agent job. Returns the UUID if successful.
async fn create_agent_job(
    state: &Arc<AppState>,
    session_uuid: &str,
    user_message: &str,
) -> Option<uuid::Uuid> {
    sqlx::query_scalar::<_, uuid::Uuid>(
        "INSERT INTO agent_background_jobs (session_uuid, session_id, user_message, status)
         VALUES ($1, (SELECT id FROM chat_sessions WHERE session_uuid = $1 LIMIT 1), $2, 'running')
         RETURNING id",
    )
    .bind(session_uuid)
    .bind(user_message)
    .fetch_optional(&state.db_pool)
    .await
    .ok()
    .flatten()
}

async fn complete_agent_job(state: &Arc<AppState>, job_id: uuid::Uuid, result: &str) {
    let _ = sqlx::query(
        "UPDATE agent_background_jobs SET status = 'completed', result = $2, updated_at = NOW() WHERE id = $1"
    )
    .bind(job_id)
    .bind(result)
    .execute(&state.db_pool)
    .await;
}

async fn fail_agent_job(state: &Arc<AppState>, job_id: uuid::Uuid, error: &str) {
    let _ = sqlx::query(
        "UPDATE agent_background_jobs SET status = 'failed', error = $2, updated_at = NOW() WHERE id = $1"
    )
    .bind(job_id)
    .bind(error)
    .execute(&state.db_pool)
    .await;
}

async fn append_job_progress(state: &Arc<AppState>, job_id: uuid::Uuid, message: &str) {
    let entry = serde_json::json!({
        "ts": chrono::Utc::now().to_rfc3339(),
        "msg": message
    });
    let _ = sqlx::query(
        "UPDATE agent_background_jobs
         SET progress_log = progress_log || $2::jsonb, updated_at = NOW()
         WHERE id = $1",
    )
    .bind(job_id)
    .bind(serde_json::json!([entry]))
    .execute(&state.db_pool)
    .await;
}

/// Returns a human-readable status string if there's a running agent job for this session,
/// or None if there's no in-progress work.
async fn get_running_agent_job_status(state: &Arc<AppState>, session_uuid: &str) -> Option<String> {
    let row = sqlx::query_as::<_, (uuid::Uuid, String, chrono::DateTime<chrono::Utc>)>(
        "SELECT id, user_message, created_at
         FROM agent_background_jobs
         WHERE session_uuid = $1 AND status = 'running'
         ORDER BY created_at DESC LIMIT 1",
    )
    .bind(session_uuid)
    .fetch_optional(&state.db_pool)
    .await
    .ok()
    .flatten();

    row.map(|(_, user_msg, created_at)| {
        let elapsed = chrono::Utc::now().signed_duration_since(created_at);
        let mins = elapsed.num_minutes();
        let elapsed_str = if mins < 1 { "just now".to_string() } else { format!("{} min ago", mins) };
        format!(
            "⏳ Your task is still running in the background (started {}): \"{}\"\n\nI'll send you the result as soon as it's done. You can also check back later.",
            elapsed_str,
            user_msg.chars().take(120).collect::<String>()
        )
    })
}

/// Core background function: runs the agent and routes the result back via job_manager
/// so it's delivered to whichever WebSocket connection is open for this session.
#[allow(unused_imports)]
use crate::agent::stateful_agent::{StatefulClaudeAgent, StatefulGeminiAgent};
async fn run_agent_background(
    state: Arc<AppState>,
    session_id: String,
    text: String,
    enhanced_query: String,
    use_claude: bool,
    job_id: Option<uuid::Uuid>,
    job_manager: std::sync::Arc<crate::jobs::JobManager>,
    agent_progress_tx: tokio::sync::mpsc::UnboundedSender<String>,
) {
    tracing::info!(
        "🚀 Background agent task started for session: {}",
        session_id
    );

    // Proxy for agent progress: forward to WebSocket channel AND save to DB
    let (proxy_tx, mut proxy_rx) = tokio::sync::mpsc::unbounded_channel::<String>();
    let state_prog = state.clone();
    let job_id_prog = job_id;
    // Drain the proxy channel in a separate task, saving each message to DB
    tokio::spawn(async move {
        while let Some(msg) = proxy_rx.recv().await {
            // Forward to WebSocket (best-effort — may fail if WS is closed)
            let _ = agent_progress_tx.send(msg.clone());
            // Persist to DB
            if let Some(jid) = job_id_prog {
                append_job_progress(&state_prog, jid, &msg).await;
            }
        }
    });

    let response = if use_claude {
        if let Some(ref claude_client) = state.claude_client {
            let agent = StatefulClaudeAgent::new(Arc::new(claude_client.clone()));
            match agent
                .chat(
                    &text,
                    &session_id,
                    enhanced_query,
                    state.clone(),
                    job_manager.clone(),
                    Some(proxy_tx),
                )
                .await
            {
                Ok(resp) => resp,
                Err(e) => format!("Sorry, I encountered an error: {}", e),
            }
        } else {
            "Claude client not configured".to_string()
        }
    } else {
        if let Some(gemini_client) = state
            .video_gemini_client
            .as_ref()
            .or(state.gemini_client.as_ref())
        {
            let agent = StatefulGeminiAgent::new(Arc::new(gemini_client.clone()));
            match agent
                .chat(
                    &text,
                    &session_id,
                    enhanced_query,
                    state.clone(),
                    job_manager.clone(),
                    Some(proxy_tx),
                )
                .await
            {
                Ok(resp) => resp,
                Err(e) => format!("Sorry, I encountered an error: {}", e),
            }
        } else {
            "Gemini client not configured".to_string()
        }
    };

    tracing::info!(
        "✅ Background agent task completed for session: {}",
        session_id
    );

    // Mark job as completed in DB (also saves result so polling works after reconnect)
    if let Some(jid) = job_id {
        complete_agent_job(&state, jid, &response).await;
    }

    // Route result back to the WebSocket via job_manager — works even if the user
    // reconnected with a new WebSocket after navigating away.
    // The WebSocket loop handles JobStatus::Completed by sending `result` as a chat message.
    if !response.is_empty() {
        let update = crate::jobs::ProgressUpdate::new(
            job_id
                .map(|j| j.to_string())
                .unwrap_or_else(|| session_id.clone()),
            "Agent task complete".to_string(),
            crate::jobs::JobStatus::Completed {
                result: response,
                output_files: vec![],
                duration_seconds: 0.0,
            },
        );
        job_manager.send_progress(&session_id, update).await;
    }
}

/// REST endpoint: list all agent background jobs for a session (for frontend polling)
async fn get_session_jobs(
    axum::extract::Path(session_uuid): axum::extract::Path<String>,
    Extension(state): Extension<Arc<AppState>>,
    Extension(_claims): Extension<crate::models::auth::Claims>,
) -> Result<axum::response::Json<serde_json::Value>, axum::http::StatusCode> {
    // Security model: the session UUID is itself an unguessable token (UUID v4).
    // WebSocket sessions are always created with user_id = 1 (anonymous default in
    // get_or_create_session) so a strict owner == JWT user_id check always fails.
    // Any authenticated user who holds the correct UUID can poll it — this matches
    // how most UUID-keyed APIs work (UUID is the unforgeable capability).
    let exists: bool =
        sqlx::query_scalar("SELECT EXISTS(SELECT 1 FROM chat_sessions WHERE session_uuid = $1)")
            .bind(&session_uuid)
            .fetch_one(&state.db_pool)
            .await
            .map_err(|_| axum::http::StatusCode::INTERNAL_SERVER_ERROR)?;

    if !exists {
        return Err(axum::http::StatusCode::NOT_FOUND);
    }

    let rows = sqlx::query_as::<
        _,
        (
            uuid::Uuid,
            String,
            String,
            Option<String>,
            Option<String>,
            serde_json::Value,
            chrono::DateTime<chrono::Utc>,
            chrono::DateTime<chrono::Utc>,
        ),
    >(
        "SELECT id, user_message, status, result, error, progress_log, created_at, updated_at
         FROM agent_background_jobs
         WHERE session_uuid = $1
         ORDER BY created_at DESC
         LIMIT 20",
    )
    .bind(&session_uuid)
    .fetch_all(&state.db_pool)
    .await
    .map_err(|_| axum::http::StatusCode::INTERNAL_SERVER_ERROR)?;

    let jobs: Vec<serde_json::Value> = rows
        .into_iter()
        .map(
            |(id, msg, status, result, error, progress_log, created_at, updated_at)| {
                serde_json::json!({
                    "id": id,
                    "user_message": msg,
                    "status": status,
                    "result": result,
                    "error": error,
                    "progress_log": progress_log,
                    "created_at": created_at.to_rfc3339(),
                    "updated_at": updated_at.to_rfc3339(),
                })
            },
        )
        .collect();

    Ok(axum::response::Json(serde_json::json!({
        "success": true,
        "session_uuid": session_uuid,
        "jobs": jobs
    })))
}
