// src/handlers/chat.rs
use crate::handlers::upload::get_or_create_session;
use crate::middleware::auth::auth_middleware;
use crate::middleware::frontend_rate_limit::ai_operation_rate_limit_middleware;
use crate::AppState;
use axum::{
    extract::{
        ws::{Message, WebSocket, WebSocketUpgrade},
        Extension, Path, Query,
    },
    response::IntoResponse,
    routing::{get, post},
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

#[derive(Deserialize)]
struct WebSocketQuery {
    session: Option<String>,
    model: Option<String>,
    token: Option<String>,
    workflow_id: Option<String>,
}

#[derive(Debug, Serialize)]
struct ActiveWorkflowSnapshot {
    workflow_id: String,
    workflow_type: String,
    status: String,
    current_step: Option<String>,
    request_summary: String,
    created_at: String,
    started_ago: String,
    last_heartbeat_at: String,
    last_heartbeat_ago: String,
    completed_at: Option<String>,
    completed_ago: Option<String>,
    latest_progress_message: Option<String>,
    user_message: Option<String>,
    error_message: Option<String>,
    result_summary: Option<String>,
    artifact_status: serde_json::Value,
    retry_count: i32,
    recent_events: Vec<serde_json::Value>,
}

#[derive(Debug, Deserialize)]
struct WorkflowFollowupDecision {
    mode: String,
    reply: Option<String>,
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
            "/api/chat/sessions/:session_uuid/title",
            post(update_chat_title),
        )
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
    let target_workflow_id = match params
        .workflow_id
        .as_deref()
        .filter(|value| !value.trim().is_empty())
    {
        Some(raw_workflow_id) => match uuid::Uuid::parse_str(raw_workflow_id.trim()) {
            Ok(parsed) => Some(parsed),
            Err(_) => {
                return (
                    axum::http::StatusCode::BAD_REQUEST,
                    axum::Json(serde_json::json!({
                        "success": false,
                        "error": "invalid_workflow_id",
                        "message": "The requested workflow id is not valid.",
                    })),
                )
                    .into_response();
            }
        },
        None => None,
    };

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

    let Some(user_id) = user_id else {
        return (
            axum::http::StatusCode::UNAUTHORIZED,
            axum::Json(serde_json::json!({
                "success": false,
                "error": "authentication_required",
                "message": "Chat websocket requires a valid signed-in user session.",
            })),
        )
            .into_response();
    };

    if let Some(session_uuid) = params.session.as_deref() {
        let owner = sqlx::query_scalar::<_, i32>(
            "SELECT user_id FROM chat_sessions WHERE session_uuid = $1",
        )
        .bind(session_uuid)
        .fetch_optional(&state.db_pool)
        .await;

        match owner {
            Ok(Some(existing_owner)) if existing_owner != user_id && existing_owner != 1 => {
                tracing::warn!(
                    "Rejecting websocket session {} for user {} because it belongs to user {}",
                    session_uuid,
                    user_id,
                    existing_owner
                );
                return (
                    axum::http::StatusCode::FORBIDDEN,
                    axum::Json(serde_json::json!({
                        "success": false,
                        "error": "forbidden_session",
                        "message": "This chat session belongs to another user.",
                    })),
                )
                    .into_response();
            }
            Ok(_) => {}
            Err(error) => {
                tracing::error!(
                    "Failed to verify websocket session ownership for {}: {}",
                    session_uuid,
                    error
                );
                return (
                    axum::http::StatusCode::INTERNAL_SERVER_ERROR,
                    axum::Json(serde_json::json!({
                        "success": false,
                        "error": "session_verification_failed",
                        "message": "Unable to verify chat session ownership right now.",
                    })),
                )
                    .into_response();
            }
        }
    }

    if let Some(workflow_id) = target_workflow_id {
        match workflow_is_accessible_to_user(&state, workflow_id, user_id).await {
            Ok(true) => {}
            Ok(false) => {
                return (
                    axum::http::StatusCode::FORBIDDEN,
                    axum::Json(serde_json::json!({
                        "success": false,
                        "error": "forbidden_workflow",
                        "message": "This workflow does not belong to the signed-in user.",
                    })),
                )
                    .into_response();
            }
            Err(error) => {
                tracing::error!(
                    "Failed to verify websocket workflow ownership for {}: {}",
                    workflow_id,
                    error
                );
                return (
                    axum::http::StatusCode::INTERNAL_SERVER_ERROR,
                    axum::Json(serde_json::json!({
                        "success": false,
                        "error": "workflow_verification_failed",
                        "message": "Unable to verify workflow ownership right now.",
                    })),
                )
                    .into_response();
            }
        }
    }

    // Subscription gate — the WS route doesn't flow through the standard
    // axum middleware stack for the upgrade handshake, so we check here.
    // trial / active / grandfathered / staff / superuser pass; expired
    // users hit 402 with the upgrade link.
    use axum::http::StatusCode;
    match subscription_ok(&state, user_id).await {
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
            tracing::warn!("WS subscription check failed for user {}: {}", user_id, e);
            // fail-open rather than blocking legit users on DB hiccup;
            // paywalled compute routes will still gate properly.
        }
    }

    ws.on_upgrade(move |socket| {
        websocket(
            socket,
            state,
            params.session,
            params.model,
            Some(user_id),
            target_workflow_id,
        )
    })
    .into_response()
}

/// Returns Ok(true) if the user is allowed to use paid compute right now.
/// Mirrors the logic in src/middleware/subscription.rs but called inline
/// because the WS upgrade bypasses standard middleware.
pub async fn subscription_ok(state: &Arc<AppState>, user_id: i32) -> Result<bool, sqlx::Error> {
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
    target_workflow_id: Option<uuid::Uuid>,
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

    // 🆕 BACKGROUND JOBS: Subscribe to progress updates.
    // Two delivery paths converge into one mpsc channel:
    //   1. In-memory (JobManager -> register_progress_sender) — same-instance
    //   2. Redis pub/sub (PubSubBus -> subscribe) — cross-instance Fargate
    let (progress_tx, mut progress_rx) = tokio::sync::mpsc::unbounded_channel::<String>();

    // Path 1: In-memory bridge — JobManager sends ProgressUpdate, we serialize to String
    {
        let (pg_tx, mut pg_rx) = tokio::sync::mpsc::unbounded_channel::<crate::jobs::ProgressUpdate>();
        state
            .job_manager
            .register_progress_sender(session_id.clone(), pg_tx)
            .await;
        let prog_tx = progress_tx.clone();
        tokio::spawn(async move {
            while let Some(update) = pg_rx.recv().await {
                if let Ok(json) = serde_json::to_string(&update) {
                    if prog_tx.send(json).is_err() {
                        break;
                    }
                }
            }
        });
    }

    // Path 2: Redis pub/sub bridge — for cross-instance delivery
    if let Some(ref bus) = state.pubsub_bus {
        if let Ok(mut redis_rx) = bus.subscribe(&format!("progress:{}", session_id)).await {
            tracing::info!("📡 Subscribed to Redis progress channel for session: {}", session_id);
            let prog_tx = progress_tx.clone();
            tokio::spawn(async move {
                while let Some(msg) = redis_rx.recv().await {
                    if prog_tx.send(msg).is_err() {
                        break;
                    }
                }
            });
        } else {
            tracing::warn!("Failed to subscribe to Redis progress channel");
        }
    }

    // 🆕 AGENT PROGRESS: Create separate channel for agent thinking/tool calling updates
    let (agent_progress_tx, mut agent_progress_rx) = tokio::sync::mpsc::unbounded_channel();

    // On reconnect: check if there's a running background agent job for this session
    // and immediately inform the user so they know work is still happening
    let reconnect_message = if let Some(workflow_id) = target_workflow_id.clone() {
        get_workflow_reconnect_status(&state, workflow_id).await
    } else {
        get_running_agent_job_status(&state, &session_id).await
    };

    if let Some(running_msg) = reconnect_message {
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
                match qdrant_client.build_context_for_query(
                    &text,
                    &session_id,
                    state.voyage_embeddings.as_ref(),
                    state.video_gemini_client.as_ref().or(state.gemini_client.as_ref()),
                ).await {
                    Ok(Some(ctx)) => {
                        tracing::debug!("Built context from Qdrant: {} chars", ctx.len());
                        Some(ctx)
                    }
                    Ok(None) => {
                        tracing::warn!("No embedding client available for Qdrant");
                        None
                    }
                    Err(e) => {
                        tracing::warn!("Qdrant context retrieval error: {}", e);
                        None
                    }
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

            // 🆕 INTERACTIVE AGENT: If there's an active background agent for this
            // session, forward the user's message to it via Redis pub/sub.
            {
                let forwarded = if let Some(ref bus) = state.pubsub_bus {
                    let channel = format!("feedback:{}", session_id);
                    let enhanced_query = {
                        let mut query_parts = Vec::new();
                        if !file_context.is_empty() {
                            query_parts.push(file_context.clone());
                        }
                        if let Some(ref ctx) = context {
                            query_parts.push(format!("PREVIOUS CONVERSATIONS CONTEXT:\n{}", ctx));
                        }
                        query_parts.push(format!("USER REQUEST:\n{}", text));
                        query_parts.join("\n\n")
                    };
                    let subs = bus.publish(&channel, &enhanced_query).await.unwrap_or(0);
                    if subs > 0 {
                        tracing::info!("📨 Forwarded message to running agent for session: {} via Redis", session_id);
                        true
                    } else {
                        false
                    }
                } else {
                    false
                };
                if forwarded {
                    continue;
                }
            }

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

            let workflow_snapshot = if let Some(workflow_id) = target_workflow_id.clone() {
                get_workflow_snapshot_by_id(&state, workflow_id).await
            } else {
                get_active_workflow_snapshot(&state, &session_id).await
            };

            if let Some(active_workflow_snapshot) = workflow_snapshot
            {
                // 🚀 Planning workflows haven't started yet — always route to the
                // agent instead of asking the LLM whether to answer or continue.
                if active_workflow_snapshot.status != "planning" {
                    if let Some(followup_reply) = try_answer_active_workflow_followup(
                        &state,
                        &session_id,
                        &text,
                        &active_workflow_snapshot,
                        use_claude,
                    )
                    .await
                    {
                        let json_response = serde_json::json!({
                            "type": "result",
                            "content": followup_reply.clone(),
                            "timestamp": chrono::Utc::now().to_rfc3339(),
                        });

                        if let Ok(json_str) = serde_json::to_string(&json_response) {
                            if sender.send(Message::Text(json_str)).await.is_err() {
                                tracing::error!("Failed to send active-workflow follow-up response");
                                break;
                            }
                        }
                        continue;
                    }
                }
            }

            // 🤖 AI-POWERED ROUTING: Spawn agent as background task so it survives
            // WebSocket disconnects. User can close the page and come back — the task
            // keeps running and the result will be delivered on reconnect via job_manager.

            tracing::info!("🤖 Spawning AI agent as background task for session: {}", session_id);

            // Create a DB record for this job so we can replay it on reconnect
            let job_id =
                create_agent_job(&state, &session_id, &text, target_workflow_id.clone()).await;

            // Clone everything the background task needs (state is Arc, cheap clone)
            let state_bg = state.clone();
            let session_id_bg = session_id.clone();
            let text_bg = text.clone();
            let enhanced_query_bg = enhanced_query.clone();
            let job_manager_bg = state.job_manager.clone();
            let agent_tx_bg = agent_progress_tx.clone();
            let user_id_bg = user_id; // Some(i32) from JWT

            // 🆕 Subscribe to feedback channel so running agent receives
            // user follow-up messages via Redis pub/sub (cross-instance).
            let feedback_rx = if let Some(ref bus) = state.pubsub_bus {
                match bus.subscribe(&format!("feedback:{}", session_id)).await {
                    Ok(rx) => {
                        tracing::info!("📡 Subscribed to feedback channel for session: {}", session_id);
                        Some(rx)
                    }
                    Err(e) => {
                        tracing::warn!("Failed to subscribe to feedback channel: {}", e);
                        let (_tx, rx) = tokio::sync::mpsc::unbounded_channel();
                        Some(rx)
                    }
                }
            } else {
                let (_tx, rx) = tokio::sync::mpsc::unbounded_channel();
                Some(rx)
            };

            // Drop the session_id_clone variable — feedback channel cleanup
            // is automatic when the receiver is dropped inside run_agent_background.
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
                    feedback_rx,
                    user_id_bg,
                )
                .await;
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
            // (receives serialized ProgressUpdate JSON — parses from Redis pub/sub string)
            progress_msg = progress_rx.recv() => {
                let Some(msg) = progress_msg else { break; };
                let progress_update: crate::jobs::ProgressUpdate = match serde_json::from_str(&msg) {
                    Ok(p) => p,
                    Err(_) => {
                        tracing::warn!("Failed to parse progress update JSON");
                        continue;
                    }
                };
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

                            if let Err(e) = qdrant_client.store_chat_memory(
                                &session_id,
                                None,
                                &user_message,
                                result,
                                files_referenced.clone(),
                                context_data.clone(),
                                state.voyage_embeddings.as_ref(),
                                state.video_gemini_client.as_ref().or(state.gemini_client.as_ref()),
                                Some("general"),
                            ).await {
                                tracing::warn!("Failed to store in Qdrant: {}", e);
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
                "{}. \"{}\" - USE THIS PATH: {}\n   - Operation: {} using {}\n   - Size: {:.2} MB\n   - Cloud URL: {}\n   - Watch link: /api/outputs/stream/{}\n   - Download link: /api/outputs/download/{}\n   - Created: {}\n\n",
                index + 1,
                video.file_name,
                video.file_path,
                video.operation_type,
                video.tool_used,
                video.file_size as f64 / (1024.0 * 1024.0),
                video.r2_url.as_deref().unwrap_or("(none)"),
                file_id,
                file_id,
                video.created_at.format("%Y-%m-%d %H:%M:%S UTC")
            ));
        }
    }

    if !files.is_empty() || !output_videos.is_empty() {
        context.push_str("CRITICAL INSTRUCTION: When using ANY video editing tool, you MUST use the PATH shown above (the path after 'USE THIS PATH:'). NEVER use just the filename like 'GothamChess.mp4' — always use the full path. R2 cloud URLs (starting with https://) work natively with all editing tools — FFmpeg reads them over HTTP. Local paths like 'outputs/...' also work. The tools will FAIL if you use only the filename!\n");
        context.push_str("CRITICAL USER-FACING INSTRUCTION: Use the Cloud URL shown above for delivery to the user. Never expose internal paths like 'outputs/...' in user-facing responses.\n\n");
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
        "SELECT cs.id, cs.session_uuid, cs.title, cs.created_at
         FROM chat_sessions cs
         WHERE cs.user_id = $1
           AND EXISTS (
             SELECT 1
             FROM conversation_messages cm
             WHERE cm.session_id = cs.id
               AND cm.role IN ('user', 'human', 'assistant', 'model')
               AND LENGTH(BTRIM(COALESCE(cm.content, ''))) > 0
           )
         ORDER BY cs.created_at DESC
         LIMIT 10",
    )
    .bind(claims.sub.parse::<i32>().unwrap_or(0))
    .fetch_all(&state.db_pool)
    .await
    {
        Ok(rows) => {
            let mut chats = Vec::new();
            for (id, session_uuid, title, created_at) in rows {
                let display_title = resolve_chat_display_title(&state.db_pool, id, &title).await;
                chats.push(serde_json::json!({
                    "id": id,
                    "session_id": session_uuid,
                    "title": display_title,
                    "created_at": created_at.format("%Y-%m-%d %H:%M:%S").to_string()
                }));
            }

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
        "SELECT COUNT(*)
             FROM chat_sessions cs
             WHERE cs.user_id = $1
               AND EXISTS (
                 SELECT 1
                 FROM conversation_messages cm
                 WHERE cm.session_id = cs.id
                   AND cm.role IN ('user', 'human', 'assistant', 'model')
                   AND LENGTH(BTRIM(COALESCE(cm.content, ''))) > 0
               )",
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
           AND EXISTS (
             SELECT 1
             FROM conversation_messages cm
             WHERE cm.session_id = cs.id
               AND cm.role IN ('user', 'human', 'assistant', 'model')
               AND LENGTH(BTRIM(COALESCE(cm.content, ''))) > 0
           )
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

        let display_title = resolve_chat_display_title(&state.db_pool, id, &title).await;

        chats.push(serde_json::json!({
            "id": id,
            "session_id": session_uuid,
            "title": display_title,
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

#[derive(Deserialize)]
struct UpdateChatTitleRequest {
    title: String,
}

async fn update_chat_title(
    Path(session_uuid): Path<String>,
    Extension(state): Extension<Arc<AppState>>,
    Extension(claims): Extension<crate::models::auth::Claims>,
    axum::Json(payload): axum::Json<UpdateChatTitleRequest>,
) -> Result<axum::response::Json<serde_json::Value>, axum::http::StatusCode> {
    let user_id = claims.sub.parse::<i32>().unwrap_or(0);
    let title = payload.title.trim();

    if title.is_empty() {
        return Ok(axum::response::Json(serde_json::json!({
            "success": false,
            "message": "Title cannot be empty."
        })));
    }

    if title.len() > 100 {
        return Ok(axum::response::Json(serde_json::json!({
            "success": false,
            "message": "Title must be 100 characters or less."
        })));
    }

    let updated = sqlx::query(
        "UPDATE chat_sessions
         SET title = $1, updated_at = NOW()
         WHERE session_uuid = $2 AND user_id = $3",
    )
    .bind(title)
    .bind(&session_uuid)
    .bind(user_id)
    .execute(&state.db_pool)
    .await
    .map_err(|e| {
        tracing::error!("Failed to update chat title for {}: {}", session_uuid, e);
        axum::http::StatusCode::INTERNAL_SERVER_ERROR
    })?;

    if updated.rows_affected() == 0 {
        return Err(axum::http::StatusCode::NOT_FOUND);
    }

    Ok(axum::response::Json(serde_json::json!({
        "success": true,
        "title": title
    })))
}

async fn resolve_chat_display_title(
    pool: &sqlx::PgPool,
    session_id: i32,
    current_title: &str,
) -> String {
    let trimmed = current_title.trim();
    if !trimmed.is_empty() && trimmed != "New Chat Session" {
        return trimmed.to_string();
    }

    let first_user_message = sqlx::query_scalar::<_, String>(
        "SELECT content
         FROM conversation_messages
         WHERE session_id = $1
           AND role IN ('user', 'human')
           AND LENGTH(BTRIM(COALESCE(content, ''))) > 0
         ORDER BY created_at ASC
         LIMIT 1",
    )
    .bind(session_id)
    .fetch_optional(pool)
    .await
    .ok()
    .flatten();

    if let Some(message) = first_user_message {
        let generated = generate_chat_title_from_message(&message);
        let _ = sqlx::query(
            "UPDATE chat_sessions
             SET title = $1, updated_at = NOW()
             WHERE id = $2 AND (title IS NULL OR title = '' OR title = 'New Chat Session')",
        )
        .bind(&generated)
        .bind(session_id)
        .execute(pool)
        .await;
        return generated;
    }

    let first_assistant_message = sqlx::query_scalar::<_, String>(
        "SELECT content
         FROM conversation_messages
         WHERE session_id = $1
           AND role IN ('assistant', 'model')
           AND LENGTH(BTRIM(COALESCE(content, ''))) > 0
         ORDER BY created_at ASC
         LIMIT 1",
    )
    .bind(session_id)
    .fetch_optional(pool)
    .await
    .ok()
    .flatten();

    if let Some(message) = first_assistant_message {
        let generated = generate_chat_title_from_message(&message);
        let _ = sqlx::query(
            "UPDATE chat_sessions
             SET title = $1, updated_at = NOW()
             WHERE id = $2 AND (title IS NULL OR title = '' OR title = 'New Chat Session')",
        )
        .bind(&generated)
        .bind(session_id)
        .execute(pool)
        .await;
        return generated;
    }

    "New Chat Session".to_string()
}

fn generate_chat_title_from_message(message: &str) -> String {
    let normalized = message.split_whitespace().collect::<Vec<_>>().join(" ");
    if normalized.is_empty() {
        return "New Chat Session".to_string();
    }

    if let Some(service_line) = normalized
        .strip_prefix("Service page sample request for ")
        .or_else(|| normalized.strip_prefix("Service page request for "))
        .and_then(|rest| rest.split('.').next())
    {
        if let Some(brief_idx) = message.find("Sample brief:") {
            let brief = message[brief_idx + "Sample brief:".len()..]
                .lines()
                .next()
                .unwrap_or("")
                .trim();
            if !brief.is_empty() {
                return truncate_title(&format!("{service_line}: {brief}"), 90);
            }
        }
        return truncate_title(service_line, 90);
    }

    truncate_title(&normalized, 90)
}

fn truncate_title(input: &str, max_chars: usize) -> String {
    let trimmed = input.trim();
    if trimmed.chars().count() <= max_chars {
        return trimmed.to_string();
    }

    let shortened: String = trimmed.chars().take(max_chars).collect();
    match shortened.rfind(' ') {
        Some(idx) if idx > 24 => format!("{}...", shortened[..idx].trim_end()),
        _ => format!("{}...", shortened.trim_end()),
    }
}

// ─── Background agent job helpers ────────────────────────────────────────────

fn request_expects_generated_artifact(user_message: &str) -> bool {
    let normalized = user_message.to_lowercase();
    let generation_intent = [
        "create", "generate", "render", "produce", "make", "build", "edit", "deliver",
    ]
    .iter()
    .any(|needle| normalized.contains(needle));

    let media_intent = [
        "video",
        "thumbnail",
        "clip",
        "scene",
        "animation",
        "ad ",
        "advert",
        "demo",
        "landing page",
        "hero video",
        "narration",
        "youtube",
        "sample",
        "delivery",
    ]
    .iter()
    .any(|needle| normalized.contains(needle));

    generation_intent && media_intent
}

fn response_output_links(response: &str) -> Vec<String> {
    fn extract_link(line: &str, needle: &str) -> Option<String> {
        let idx = line.find(needle)?;
        let candidate = &line[idx..];
        let token = candidate.split_whitespace().next().unwrap_or(candidate);
        let cleaned = token
            .trim_matches(|ch: char| matches!(ch, '`' | '"' | '\'' | ')' | ']' | '}' | ',' | '.'));
        if cleaned.starts_with(needle) {
            Some(cleaned.to_string())
        } else {
            None
        }
    }

    response
        .lines()
        .flat_map(|line| {
            [
                "/api/outputs/stream/",
                "/api/outputs/download/",
                "/delivery/",
            ]
            .into_iter()
            .filter_map(move |needle| extract_link(line, needle))
        })
        .collect::<std::collections::BTreeSet<_>>()
        .into_iter()
        .collect()
}

fn background_workflow_idempotency_key(
    session_uuid: &str,
    user_message: &str,
    existing_workflow_id: Option<uuid::Uuid>,
) -> String {
    let message_hash = crate::services::GeneratedArtifactService::legacy_file_id(user_message);
    match existing_workflow_id {
        Some(workflow_id) => {
            format!("background-agent:{session_uuid}:{workflow_id}:{message_hash}")
        }
        None => format!("background-agent:{session_uuid}:{message_hash}"),
    }
}

/// Create a DB record for a background agent job. Returns the UUID if successful.
async fn create_agent_job(
    state: &Arc<AppState>,
    session_uuid: &str,
    user_message: &str,
    existing_workflow_id: Option<uuid::Uuid>,
) -> Option<uuid::Uuid> {
    let session_context = sqlx::query_as::<_, (Option<i32>, Option<i32>)>(
        "SELECT id, user_id FROM chat_sessions WHERE session_uuid = $1 LIMIT 1",
    )
    .bind(session_uuid)
    .fetch_optional(&state.db_pool)
    .await
    .ok()
    .flatten();

    let (session_id, user_id) = match session_context {
        Some((session_id, user_id)) => (session_id, user_id),
        None => (None, None),
    };

    let is_service_sample_request = user_message.starts_with("Service page request for")
        || user_message.starts_with("Service page sample request for");
    let expects_generated_artifact = request_expects_generated_artifact(user_message);
    let workflow_type = if is_service_sample_request {
        "service_sample_generation"
    } else {
        "background_agent_generation"
    };
    let artifact_requirements = if expects_generated_artifact {
        serde_json::json!([
            {
                "kind": if is_service_sample_request { "buyer_facing_sample" } else { "generated_media_response" },
                "required": true,
                "must_include_delivery_or_output_link": true
            }
        ])
    } else {
        serde_json::json!([])
    };

    let workflow_runtime = crate::services::WorkflowRuntime::new(state.db_pool.clone());
    let workflow_id = if let Some(existing_workflow_id) = existing_workflow_id {
        let _ = workflow_runtime
            .heartbeat(
                existing_workflow_id,
                crate::services::WorkflowStatus::Queued,
                Some("job_created"),
                "Background agent job attached to the existing service-page workflow.",
                serde_json::json!({
                    "session_uuid": session_uuid,
                    "user_message": user_message,
                    "service_sample_request": is_service_sample_request,
                    "expects_generated_artifact": expects_generated_artifact,
                }),
            )
            .await;
        existing_workflow_id
    } else {
        let workflow_id = workflow_runtime
            .create_or_reuse_workflow(crate::services::NewWorkflow {
                idempotency_key: Some(background_workflow_idempotency_key(
                    session_uuid,
                    user_message,
                    existing_workflow_id,
                )),
                workflow_type: workflow_type.to_string(),
                status: crate::services::WorkflowStatus::Queued,
                session_uuid: Some(session_uuid.to_string()),
                user_id,
                source_table: Some("agent_background_jobs".to_string()),
                source_record_id: None,
                request_summary: user_message.chars().take(200).collect::<String>(),
                current_step: Some("job_created".to_string()),
                metadata: serde_json::json!({
                    "session_uuid": session_uuid,
                    "user_message": user_message,
                    "service_sample_request": is_service_sample_request,
                    "expects_generated_artifact": expects_generated_artifact,
                }),
                artifact_requirements,
            })
            .await
            .ok()?;

        let _ = workflow_runtime
            .append_event(
                workflow_id,
                "queued",
                Some("job_created"),
                "Background agent job created and waiting for the agent runtime to begin execution.",
                serde_json::json!({
                    "session_uuid": session_uuid,
                }),
            )
            .await;
        workflow_id
    };

    let job_id = sqlx::query_scalar::<_, uuid::Uuid>(
        "INSERT INTO agent_background_jobs (session_uuid, session_id, user_message, status, workflow_id)
         VALUES ($1, $2, $3, 'running', $4)
         RETURNING id",
    )
    .bind(session_uuid)
    .bind(session_id)
    .bind(user_message)
    .bind(workflow_id)
    .fetch_optional(&state.db_pool)
    .await
    .ok()
    .flatten()?;

    let _ = sqlx::query(
        "UPDATE app_workflows
         SET source_record_id = $2, updated_at = NOW()
         WHERE id = $1",
    )
    .bind(workflow_id)
    .bind(job_id)
    .execute(&state.db_pool)
    .await;

    Some(job_id)
}

async fn complete_agent_job(state: &Arc<AppState>, job_id: uuid::Uuid, result: &str) -> bool {
    let workflow_row = sqlx::query_as::<_, (Option<uuid::Uuid>, String)>(
        "SELECT workflow_id, user_message FROM agent_background_jobs WHERE id = $1",
    )
    .bind(job_id)
    .fetch_optional(&state.db_pool)
    .await
    .ok()
    .flatten();

    let (workflow_id, user_message) = match workflow_row {
        Some((workflow_id, user_message)) => (workflow_id, user_message),
        None => (None, String::new()),
    };

    let is_service_sample_request = user_message.starts_with("Service page request for")
        || user_message.starts_with("Service page sample request for");
    let expects_generated_artifact = request_expects_generated_artifact(&user_message);
    let output_links = response_output_links(result);
    let has_delivery_or_output_link = !output_links.is_empty();

    if expects_generated_artifact && !has_delivery_or_output_link {
        fail_agent_job(
            state,
            job_id,
            if is_service_sample_request {
                "The sample workflow finished without returning a delivery or output link, so it was not accepted as a valid buyer-facing sample."
            } else {
                "The workflow finished without returning a delivery or output link, so it was not accepted as a completed generated-media result."
            },
        )
        .await;
        return false;
    }

    let artifact_verification = if expects_generated_artifact {
        crate::services::ArtifactVerifier::verify_links(&state.db_pool, &output_links).await
    } else {
        crate::services::ArtifactVerificationResult {
            verified: true,
            details: serde_json::json!({
                "verified": true,
                "reason": "No generated artifact was required for this workflow",
                "links": [],
            }),
        }
    };

    if expects_generated_artifact && !artifact_verification.verified {
        fail_agent_job(
            state,
            job_id,
            "The workflow returned delivery/output links, but the linked artifacts could not be verified from storage or the database.",
        )
        .await;
        return false;
    }

    let _ = sqlx::query(
        "UPDATE agent_background_jobs SET status = 'completed', result = $2, updated_at = NOW() WHERE id = $1"
    )
    .bind(job_id)
    .bind(result)
    .execute(&state.db_pool)
    .await;

    if let Some(workflow_id) = workflow_id {
        let workflow_runtime = crate::services::WorkflowRuntime::new(state.db_pool.clone());
        let _ = workflow_runtime
            .mark_completed(
                workflow_id,
                Some("response_delivered"),
                "Background agent execution completed and returned a terminal assistant response.",
                serde_json::json!({
                    "artifact_verification": artifact_verification.details,
                    "service_sample_request": is_service_sample_request,
                    "expects_generated_artifact": expects_generated_artifact,
                    "output_links": output_links,
                    "response_preview": result.chars().take(240).collect::<String>(),
                }),
            )
            .await;
    }

    true
}

async fn fail_agent_job(state: &Arc<AppState>, job_id: uuid::Uuid, error: &str) {
    let workflow_id = sqlx::query_scalar::<_, Option<uuid::Uuid>>(
        "SELECT workflow_id FROM agent_background_jobs WHERE id = $1",
    )
    .bind(job_id)
    .fetch_optional(&state.db_pool)
    .await
    .ok()
    .flatten()
    .flatten();

    let _ = sqlx::query(
        "UPDATE agent_background_jobs SET status = 'failed', error = $2, updated_at = NOW() WHERE id = $1"
    )
    .bind(job_id)
    .bind(error)
    .execute(&state.db_pool)
    .await;

    if let Some(workflow_id) = workflow_id {
        let workflow_runtime = crate::services::WorkflowRuntime::new(state.db_pool.clone());
        let _ = workflow_runtime
            .mark_failed(workflow_id, Some("background_agent"), error, None)
            .await;
    }
}

async fn append_job_progress(state: &Arc<AppState>, job_id: uuid::Uuid, message: &str) {
    let workflow_id = sqlx::query_scalar::<_, Option<uuid::Uuid>>(
        "SELECT workflow_id FROM agent_background_jobs WHERE id = $1",
    )
    .bind(job_id)
    .fetch_optional(&state.db_pool)
    .await
    .ok()
    .flatten()
    .flatten();

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

    if let Some(workflow_id) = workflow_id {
        let workflow_runtime = crate::services::WorkflowRuntime::new(state.db_pool.clone());
        let _ = workflow_runtime
            .heartbeat(
                workflow_id,
                crate::services::WorkflowStatus::Running,
                Some("agent_progress"),
                message,
                serde_json::json!({
                    "job_id": job_id,
                    "ts": chrono::Utc::now().to_rfc3339(),
                }),
            )
            .await;
    }
}

async fn get_agent_job_workflow_id(
    state: &Arc<AppState>,
    job_id: uuid::Uuid,
) -> Option<uuid::Uuid> {
    sqlx::query_scalar::<_, Option<uuid::Uuid>>(
        "SELECT workflow_id FROM agent_background_jobs WHERE id = $1",
    )
    .bind(job_id)
    .fetch_optional(&state.db_pool)
    .await
    .ok()
    .flatten()
    .flatten()
}

fn latest_progress_message(progress_log: &serde_json::Value) -> Option<String> {
    progress_log
        .as_array()
        .and_then(|entries| entries.last())
        .and_then(|entry| entry.get("msg"))
        .and_then(|value| value.as_str())
        .map(str::trim)
        .filter(|msg| !msg.is_empty())
        .map(str::to_string)
}

fn parse_json_object<T: serde::de::DeserializeOwned>(raw: &str) -> Option<T> {
    serde_json::from_str(raw).ok().or_else(|| {
        let trimmed = raw.trim();
        let fenced = trimmed
            .strip_prefix("```json")
            .or_else(|| trimmed.strip_prefix("```"))
            .and_then(|value| value.strip_suffix("```"))
            .map(str::trim)?;
        serde_json::from_str(fenced).ok()
    })
}

async fn workflow_is_accessible_to_user(
    state: &Arc<AppState>,
    workflow_id: uuid::Uuid,
    user_id: i32,
) -> Result<bool, sqlx::Error> {
    let row = sqlx::query_as::<_, (Option<i32>, Option<String>)>(
        "SELECT user_id, session_uuid
         FROM app_workflows
         WHERE id = $1",
    )
    .bind(workflow_id)
    .fetch_optional(&state.db_pool)
    .await?;

    let Some((workflow_user_id, workflow_session_uuid)) = row else {
        return Ok(false);
    };

    if user_id == 1 {
        return Ok(true);
    }

    if let Some(owner_id) = workflow_user_id {
        return Ok(owner_id == user_id || owner_id == 1);
    }

    if let Some(session_uuid) = workflow_session_uuid {
        let session_owner = sqlx::query_scalar::<_, i32>(
            "SELECT user_id FROM chat_sessions WHERE session_uuid = $1",
        )
        .bind(session_uuid)
        .fetch_optional(&state.db_pool)
        .await?;

        return Ok(matches!(
            session_owner,
            Some(existing_owner) if existing_owner == user_id || existing_owner == 1
        ));
    }

    Ok(false)
}

fn build_workflow_snapshot(
    workflow_id: uuid::Uuid,
    workflow_type: String,
    status: String,
    current_step: Option<String>,
    request_summary: String,
    created_at: chrono::DateTime<chrono::Utc>,
    last_heartbeat_at: chrono::DateTime<chrono::Utc>,
    completed_at: Option<chrono::DateTime<chrono::Utc>>,
    progress_log: Option<serde_json::Value>,
    user_message: Option<String>,
    error_message: Option<String>,
    result_summary: Option<String>,
    artifact_status: Option<serde_json::Value>,
    retry_count: i32,
    recent_events: Vec<serde_json::Value>,
) -> ActiveWorkflowSnapshot {
    let progress_log = progress_log.unwrap_or_else(|| serde_json::json!([]));

    ActiveWorkflowSnapshot {
        workflow_id: workflow_id.to_string(),
        workflow_type,
        status,
        current_step,
        request_summary,
        created_at: created_at.to_rfc3339(),
        started_ago: format_relative_time(&created_at),
        last_heartbeat_at: last_heartbeat_at.to_rfc3339(),
        last_heartbeat_ago: format_relative_time(&last_heartbeat_at),
        completed_at: completed_at.map(|value| value.to_rfc3339()),
        completed_ago: completed_at.as_ref().map(format_relative_time),
        latest_progress_message: latest_progress_message(&progress_log),
        user_message,
        error_message,
        result_summary,
        artifact_status: artifact_status.unwrap_or_else(|| serde_json::json!({})),
        retry_count,
        recent_events,
    }
}

async fn get_active_workflow_snapshot(
    state: &Arc<AppState>,
    session_uuid: &str,
) -> Option<ActiveWorkflowSnapshot> {
    let row = sqlx::query_as::<
        _,
        (
            uuid::Uuid,
            String,
            String,
            Option<String>,
            String,
            chrono::DateTime<chrono::Utc>,
            chrono::DateTime<chrono::Utc>,
            Option<chrono::DateTime<chrono::Utc>>,
            Option<serde_json::Value>,
            Option<String>,
            Option<String>,
            Option<String>,
            Option<serde_json::Value>,
            i32,
        ),
    >(
        "SELECT aw.id, aw.workflow_type, aw.status, aw.current_step, aw.request_summary,
                aw.created_at, aw.last_heartbeat_at, aw.completed_at, abj.progress_log,
                abj.user_message, aw.error_message, aw.result_summary, aw.artifact_status,
                aw.retry_count
         FROM app_workflows aw
         LEFT JOIN agent_background_jobs abj ON abj.workflow_id = aw.id
         WHERE aw.session_uuid = $1
           AND aw.status IN ('queued','planning','running','waiting_for_input','waiting_for_external_service','retrying')
         ORDER BY aw.created_at DESC
         LIMIT 1",
    )
    .bind(session_uuid)
    .fetch_optional(&state.db_pool)
    .await
    .ok()
    .flatten()?;

    let (
        workflow_id,
        workflow_type,
        status,
        current_step,
        request_summary,
        created_at,
        last_heartbeat_at,
        completed_at,
        progress_log,
        user_message,
        error_message,
        result_summary,
        artifact_status,
        retry_count,
    ) = row;

    let recent_events = workflow_recent_events(state, workflow_id).await;

    Some(build_workflow_snapshot(
        workflow_id,
        workflow_type,
        status,
        current_step,
        request_summary,
        created_at,
        last_heartbeat_at,
        completed_at,
        progress_log,
        user_message,
        error_message,
        result_summary,
        artifact_status,
        retry_count,
        recent_events,
    ))
}

async fn workflow_recent_events(
    state: &Arc<AppState>,
    workflow_id: uuid::Uuid,
) -> Vec<serde_json::Value> {
    sqlx::query_as::<
        _,
        (
            String,
            Option<String>,
            String,
            chrono::DateTime<chrono::Utc>,
        ),
    >(
        "SELECT event_type, node_name, message, created_at
         FROM app_workflow_events
         WHERE workflow_id = $1
         ORDER BY created_at DESC
         LIMIT 8",
    )
    .bind(workflow_id)
    .fetch_all(&state.db_pool)
    .await
    .ok()
    .unwrap_or_default()
    .into_iter()
    .map(|(event_type, node_name, message, created_at)| {
        serde_json::json!({
            "event_type": event_type,
            "node_name": node_name,
            "message": message,
            "created_at": created_at.to_rfc3339(),
            "created_ago": format_relative_time(&created_at),
        })
    })
    .collect::<Vec<_>>()
}

async fn get_workflow_snapshot_by_id(
    state: &Arc<AppState>,
    workflow_id: uuid::Uuid,
) -> Option<ActiveWorkflowSnapshot> {
    let row = sqlx::query_as::<
        _,
        (
            uuid::Uuid,
            String,
            String,
            Option<String>,
            String,
            chrono::DateTime<chrono::Utc>,
            chrono::DateTime<chrono::Utc>,
            Option<chrono::DateTime<chrono::Utc>>,
            Option<serde_json::Value>,
            Option<String>,
            Option<String>,
            Option<String>,
            Option<serde_json::Value>,
            i32,
        ),
    >(
        "SELECT aw.id, aw.workflow_type, aw.status, aw.current_step, aw.request_summary,
                aw.created_at, aw.last_heartbeat_at, aw.completed_at, abj.progress_log,
                abj.user_message, aw.error_message, aw.result_summary, aw.artifact_status,
                aw.retry_count
         FROM app_workflows aw
         LEFT JOIN agent_background_jobs abj ON abj.workflow_id = aw.id
         WHERE aw.id = $1
         LIMIT 1",
    )
    .bind(workflow_id)
    .fetch_optional(&state.db_pool)
    .await
    .ok()
    .flatten()?;

    let (
        workflow_id,
        workflow_type,
        status,
        current_step,
        request_summary,
        created_at,
        last_heartbeat_at,
        completed_at,
        progress_log,
        user_message,
        error_message,
        result_summary,
        artifact_status,
        retry_count,
    ) = row;

    let recent_events = workflow_recent_events(state, workflow_id).await;

    Some(build_workflow_snapshot(
        workflow_id,
        workflow_type,
        status,
        current_step,
        request_summary,
        created_at,
        last_heartbeat_at,
        completed_at,
        progress_log,
        user_message,
        error_message,
        result_summary,
        artifact_status,
        retry_count,
        recent_events,
    ))
}

async fn get_workflow_reconnect_status(
    state: &Arc<AppState>,
    workflow_id: uuid::Uuid,
) -> Option<String> {
    let snapshot = get_workflow_snapshot_by_id(state, workflow_id).await?;

    snapshot
        .latest_progress_message
        .clone()
        .or_else(|| {
            snapshot
                .recent_events
                .first()
                .and_then(|event| event.get("message"))
                .and_then(|value| value.as_str())
                .map(str::trim)
                .filter(|value| !value.is_empty())
                .map(str::to_string)
        })
        .or_else(|| snapshot.current_step.clone())
        .or_else(|| snapshot.result_summary.clone())
        .or_else(|| snapshot.error_message.clone())
}

async fn persist_direct_followup_exchange(
    state: &Arc<AppState>,
    session_id: &str,
    user_message: &str,
    assistant_reply: &str,
) {
    let conversation_manager =
        crate::agent::conversation_manager::ConversationManager::new(state.db_pool.clone());
    let _ = conversation_manager.initialize_schema().await;

    let user_msg = crate::agent::conversation_manager::ConversationMessage::new_human(
        session_id.to_string(),
        user_message.to_string(),
    );
    let assistant_msg = crate::agent::conversation_manager::ConversationMessage::new_assistant(
        session_id.to_string(),
        assistant_reply.to_string(),
    );

    let _ = conversation_manager.save_message(&user_msg).await;
    let _ = conversation_manager.save_message(&assistant_msg).await;
}

async fn try_answer_active_workflow_followup(
    state: &Arc<AppState>,
    session_id: &str,
    user_message: &str,
    snapshot: &ActiveWorkflowSnapshot,
    prefer_claude: bool,
) -> Option<String> {
    let prompt = format!(
        r#"You are VideoSync's AI assistant.

The user is referencing a workflow. Decide whether to reply conversationally or let the agent handle it.

Rules:
- Reply conversationally if the user is just asking about status/progress/timing. Be brief and natural — no boilerplate, no "the request has been stored." Just say what's happening.
- Return continue_background_task if the user is making a new request, asking for changes, or giving instructions for the agent to execute.
- Reply JSON only.

JSON schema:
{{"mode":"reply"|"continue_background_task","reply":"string"}}

Workflow:
{}
User:
{}
"#,
        serde_json::to_string_pretty(snapshot).ok()?,
        serde_json::to_string(user_message).ok()?
    );

    let raw = if prefer_claude {
        if let Some(ref claude_client) = state.claude_client {
            claude_client.generate_text(&prompt).await.ok()
        } else if let Some(gemini_client) = state
            .video_gemini_client
            .as_ref()
            .or(state.gemini_client.as_ref())
        {
            gemini_client.generate_text(&prompt).await.ok()
        } else {
            None
        }
    } else if let Some(gemini_client) = state
        .video_gemini_client
        .as_ref()
        .or(state.gemini_client.as_ref())
    {
        gemini_client.generate_text(&prompt).await.ok()
    } else if let Some(ref claude_client) = state.claude_client {
        claude_client.generate_text(&prompt).await.ok()
    } else {
        None
    }?;

    let decision: WorkflowFollowupDecision = parse_json_object(&raw)?;
    if decision.mode != "reply" {
        return None;
    }

    let reply = decision.reply?.trim().to_string();
    if reply.is_empty() {
        return None;
    }

    persist_direct_followup_exchange(state, session_id, user_message, &reply).await;
    Some(reply)
}

/// Returns the latest persisted progress step for a running agent job in this session,
/// or None if there's no in-progress work or no persisted step yet.
async fn get_running_agent_job_status(state: &Arc<AppState>, session_uuid: &str) -> Option<String> {
    let row = sqlx::query_as::<
        _,
        (
            uuid::Uuid,
            String,
            chrono::DateTime<chrono::Utc>,
            serde_json::Value,
            Option<String>,
            Option<String>,
        ),
    >(
        "SELECT abj.id, abj.user_message, abj.created_at, abj.progress_log, aw.current_step,
                (
                    SELECT awe.message
                    FROM app_workflow_events awe
                    WHERE awe.workflow_id = aw.id
                    ORDER BY awe.created_at DESC
                    LIMIT 1
                ) AS latest_event_message
         FROM agent_background_jobs abj
         LEFT JOIN app_workflows aw ON aw.id = abj.workflow_id
         WHERE abj.session_uuid = $1 AND abj.status = 'running'
         ORDER BY abj.created_at DESC LIMIT 1",
    )
    .bind(session_uuid)
    .fetch_optional(&state.db_pool)
    .await
    .ok()
    .flatten();

    row.and_then(
        |(_, _user_msg, _created_at, progress_log, workflow_current_step, latest_event_message)| {
            latest_progress_message(&progress_log)
                .or(latest_event_message)
                .or(workflow_current_step)
        },
    )
}

fn workflow_status_payload(
    workflow_id: Option<uuid::Uuid>,
    workflow_current_step: Option<String>,
    workflow_last_heartbeat_at: Option<chrono::DateTime<chrono::Utc>>,
    workflow_latest_event_message: Option<String>,
) -> serde_json::Value {
    serde_json::json!({
        "id": workflow_id,
        "current_step": workflow_current_step,
        "last_heartbeat_at": workflow_last_heartbeat_at.map(|ts| ts.to_rfc3339()),
        "latest_event_message": workflow_latest_event_message,
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
    user_message_rx: Option<tokio::sync::mpsc::UnboundedReceiver<String>>,
    user_id: Option<i32>,
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

    let agent_workflow_id = if let Some(jid) = job_id {
        get_agent_job_workflow_id(&state, jid).await
    } else {
        None
    };
    if let (Some(jid), Some(workflow_id)) = (job_id, agent_workflow_id) {
        let workflow_runtime = crate::services::WorkflowRuntime::new(state.db_pool.clone());
        let _ = workflow_runtime
            .heartbeat(
                workflow_id,
                crate::services::WorkflowStatus::Planning,
                Some("agent_runtime_started"),
                "The background agent runtime has started and is preparing the first execution step.",
                serde_json::json!({
                    "session_id": session_id,
                    "job_id": jid,
                }),
            )
            .await;
    }

    let timeout_secs = std::env::var("AGENT_BACKGROUND_TIMEOUT_SECONDS")
        .ok()
        .and_then(|value| value.parse::<u64>().ok())
        .unwrap_or(1800);

    let response = tokio::time::timeout(std::time::Duration::from_secs(timeout_secs), async {
        if use_claude {
            if let Some(ref claude_client) = state.claude_client {
                let agent = StatefulClaudeAgent::new(Arc::new(claude_client.clone()));
                agent
                    .chat(
                        &text,
                        &session_id,
                        enhanced_query,
                        state.clone(),
                        job_manager.clone(),
                        Some(proxy_tx),
                        agent_workflow_id,
                        None, // Claude agent doesn't support interactivity yet
                        user_id,
                    )
                    .await
            } else {
                Err("Claude client not configured".to_string())
            }
        } else if let Some(gemini_client) = state
            .video_gemini_client
            .as_ref()
            .or(state.gemini_client.as_ref())
        {
            let agent = StatefulGeminiAgent::new(Arc::new(gemini_client.clone()));
            agent
                .chat(
                    &text,
                    &session_id,
                    enhanced_query,
                    state.clone(),
                    job_manager.clone(),
                    Some(proxy_tx),
                    agent_workflow_id,
                    user_message_rx,
                    user_id,
                )
                .await
        } else {
            Err("Gemini client not configured".to_string())
        }
    })
    .await;

    match response {
        Ok(Ok(response)) => {
            tracing::info!(
                "✅ Background agent task completed for session: {}",
                session_id
            );

            let output_links = response_output_links(&response);

            let completion_accepted = if let Some(jid) = job_id {
                complete_agent_job(&state, jid, &response).await
            } else {
                true
            };

            if !completion_accepted {
                let update = crate::jobs::ProgressUpdate::new(
                    job_id
                        .map(|j| j.to_string())
                        .unwrap_or_else(|| session_id.clone()),
                    "The background workflow was rejected because it did not return a required output artifact.".to_string(),
                    crate::jobs::JobStatus::Failed {
                        error: "The workflow finished without returning a required delivery or output link.".to_string(),
                        failed_at_step: "artifact_verification".to_string(),
                    },
                );
                job_manager.send_progress(&session_id, update).await;
                return;
            }

            // Detect corrections in general chat and create skills
            let _user_msg_for_correction = text.clone();
            let _agent_resp_for_correction = response.clone();
            let _state_for_correction = state.clone();
            let _uid_for_correction = user_id;
            tokio::spawn(async move {
                crate::services::skills::detect_and_store_correction(
                    _state_for_correction.db_pool.clone(),
                    _state_for_correction.qdrant_client.clone(),
                    _state_for_correction.gemini_client.clone().map(std::sync::Arc::new),
                    _state_for_correction.ollama_client.as_ref(),
                    _state_for_correction.deepseek_client.as_ref(),
                    _state_for_correction.gemini_client.as_ref(),
                    _uid_for_correction,
                    None, // service_type — unknown in general chat
                    None, // campaign_id — unknown in general chat
                    _user_msg_for_correction,
                    _agent_resp_for_correction,
                ).await;
            });

            if !response.is_empty() {
                let update = crate::jobs::ProgressUpdate::new(
                    job_id
                        .map(|j| j.to_string())
                        .unwrap_or_else(|| session_id.clone()),
                    "The background workflow reached a completed state.".to_string(),
                    crate::jobs::JobStatus::Completed {
                        result: response,
                        output_files: output_links,
                        duration_seconds: 0.0,
                    },
                );
                job_manager.send_progress(&session_id, update).await;
            }
        }
        Ok(Err(error)) => {
            tracing::error!(
                "❌ Background agent task failed for session {}: {}",
                session_id,
                error
            );

            if let Some(jid) = job_id {
                fail_agent_job(&state, jid, &error).await;
            }

            let update = crate::jobs::ProgressUpdate::new(
                job_id
                    .map(|j| j.to_string())
                    .unwrap_or_else(|| session_id.clone()),
                "The background workflow reached a failed state.".to_string(),
                crate::jobs::JobStatus::Failed {
                    error,
                    failed_at_step: "background_agent".to_string(),
                },
            );
            job_manager.send_progress(&session_id, update).await;
        }
        Err(_) => {
            let error = format!(
                "The background agent exceeded the {} second execution limit and was stopped.",
                timeout_secs
            );
            tracing::error!(
                "⏱️ Background agent task timed out for session {}",
                session_id
            );

            if let Some(jid) = job_id {
                fail_agent_job(&state, jid, &error).await;
            }

            let update = crate::jobs::ProgressUpdate::new(
                job_id
                    .map(|j| j.to_string())
                    .unwrap_or_else(|| session_id.clone()),
                "The background workflow reached a failed timeout state.".to_string(),
                crate::jobs::JobStatus::Failed {
                    error,
                    failed_at_step: "background_agent_timeout".to_string(),
                },
            );
            job_manager.send_progress(&session_id, update).await;
        }
    }
}

/// REST endpoint: list all agent background jobs for a session (for frontend polling)
async fn get_session_jobs(
    axum::extract::Path(session_uuid): axum::extract::Path<String>,
    Extension(state): Extension<Arc<AppState>>,
    Extension(claims): Extension<crate::models::auth::Claims>,
) -> Result<axum::response::Json<serde_json::Value>, axum::http::StatusCode> {
    let user_id = claims.sub.parse::<i32>().unwrap_or(0);
    let session_row = sqlx::query_as::<_, (i32, i32)>(
        "SELECT id, user_id FROM chat_sessions WHERE session_uuid = $1",
    )
    .bind(&session_uuid)
    .fetch_optional(&state.db_pool)
    .await
    .map_err(|_| axum::http::StatusCode::INTERNAL_SERVER_ERROR)?;

    let Some((session_id, session_owner)) = session_row else {
        return Ok(axum::response::Json(serde_json::json!({
            "success": true,
            "session_uuid": session_uuid,
            "jobs": []
        })));
    };

    if session_owner == 1 {
        let _ = sqlx::query("UPDATE chat_sessions SET user_id = $1 WHERE id = $2")
            .bind(user_id)
            .bind(session_id)
            .execute(&state.db_pool)
            .await;
    } else if session_owner != user_id {
        tracing::warn!(
            "User {} attempted to read jobs for session {} owned by {}",
            user_id,
            session_uuid,
            session_owner
        );
        return Err(axum::http::StatusCode::FORBIDDEN);
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
            Option<uuid::Uuid>,
            Option<String>,
            Option<chrono::DateTime<chrono::Utc>>,
            Option<String>,
        ),
    >(
        "SELECT abj.id, abj.user_message, abj.status, abj.result, abj.error, abj.progress_log, abj.created_at, abj.updated_at,
                aw.id AS workflow_id, aw.current_step, aw.last_heartbeat_at,
                (
                    SELECT awe.message
                    FROM app_workflow_events awe
                    WHERE awe.workflow_id = aw.id
                    ORDER BY awe.created_at DESC
                    LIMIT 1
                ) AS latest_event_message
         FROM agent_background_jobs abj
         LEFT JOIN app_workflows aw ON aw.id = abj.workflow_id
         WHERE abj.session_uuid = $1
         ORDER BY abj.created_at DESC
         LIMIT 20",
    )
    .bind(&session_uuid)
    .fetch_all(&state.db_pool)
    .await
    .map_err(|_| axum::http::StatusCode::INTERNAL_SERVER_ERROR)?;

    let jobs: Vec<serde_json::Value> = rows
        .into_iter()
        .map(
            |(
                id,
                msg,
                status,
                result,
                error,
                progress_log,
                created_at,
                updated_at,
                workflow_id,
                workflow_current_step,
                workflow_last_heartbeat_at,
                workflow_latest_event_message,
            )| {
                serde_json::json!({
                    "id": id,
                    "user_message": msg,
                    "status": status,
                    "result": result,
                    "error": error,
                    "progress_log": progress_log,
                    "created_at": created_at.to_rfc3339(),
                    "updated_at": updated_at.to_rfc3339(),
                    "workflow": workflow_status_payload(
                        workflow_id,
                        workflow_current_step,
                        workflow_last_heartbeat_at,
                        workflow_latest_event_message,
                    )
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
