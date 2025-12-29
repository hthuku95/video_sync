// Stateful agent that manages conversations and decides when to spawn background jobs
// The AI has a special tool to start background jobs for complex video editing tasks

use crate::claude_client::{ClaudeClient, ClaudeMessage, ClaudeContent, ClaudeTool, InputSchema, PropertyDefinition};
use crate::agent::video_workflow_state::VideoWorkflowManager;
use crate::agent::conversation_manager::{ConversationManager, ConversationMessage};
use crate::jobs::video_job;
use crate::AppState;
use std::sync::Arc;
use std::collections::HashMap;

pub struct StatefulClaudeAgent {
    client: Arc<ClaudeClient>,
    workflow_manager: Arc<VideoWorkflowManager>,
}

impl StatefulClaudeAgent {
    pub fn new(client: Arc<ClaudeClient>) -> Self {
        Self {
            client,
            workflow_manager: Arc::new(VideoWorkflowManager::new()),
        }
    }

    /// Main conversational interface - AI decides when to use background jobs
    pub async fn chat(
        &self,
        user_input: &str,
        session_id: &str,
        context: String,
        app_state: Arc<AppState>,
        job_manager: Arc<crate::jobs::JobManager>,
        progress_tx: Option<tokio::sync::mpsc::UnboundedSender<String>>,
    ) -> Result<String, String> {
        // Helper to send progress updates
        let send_progress = |msg: &str| {
            if let Some(ref tx) = progress_tx {
                let _ = tx.send(msg.to_string());
            }
            tracing::info!("{}", msg);
        };

        send_progress("🔧 Initializing Claude agent (3 control tools + 40+ video editing tools in background job system)...");
        let control_tools = Self::create_control_tools();

        // Initialize ConversationManager to retrieve and save conversation history
        let conversation_manager = ConversationManager::new(app_state.db_pool.clone());

        // Ensure schema exists
        if let Err(e) = conversation_manager.initialize_schema().await {
            tracing::warn!("Failed to initialize conversation schema: {}", e);
        }

        // Retrieve conversation history (last 20 messages)
        let conversation_history = conversation_manager
            .get_conversation_history(session_id, Some(20))
            .await
            .unwrap_or_default();

        // Build messages array with conversation history
        let mut messages = Vec::new();

        // Add conversation history
        for msg in &conversation_history {
            messages.push(ClaudeMessage {
                role: match msg.role {
                    crate::agent::conversation_manager::MessageRole::Human => "user".to_string(),
                    crate::agent::conversation_manager::MessageRole::Assistant => "assistant".to_string(),
                    _ => continue, // Skip system and function messages
                },
                content: ClaudeContent::Text(msg.content.clone()),
            });
        }

        // Add current user message with context
        let current_message = if !context.is_empty() {
            format!("{}\n\n{}", context, user_input)
        } else {
            user_input.to_string()
        };

        messages.push(ClaudeMessage {
            role: "user".to_string(),
            content: ClaudeContent::Text(current_message.clone()),
        });

        let system_prompt = r#"You are an intelligent video editing assistant with the ability to manage background processing workflows.

## Your Role
You engage in natural conversation with users while coordinating video editing tasks. You have access to a background job system that handles complex video processing operations in parallel while you continue chatting.

## Available Tools

### start_background_job
Launches a dedicated video editing agent with 39 specialized tools (trim, merge, filter, overlay, color adjustment, audio processing, etc.) that executes in the background. Use this when the user requests video processing work.

### check_job_status
Queries the status of background jobs. Use this when the user asks about progress, completion, or wants updates on running tasks. Can check specific jobs by ID or list all jobs in the current session.

## Decision-Making Guidelines

Trust your understanding of natural language to determine user intent:

**Start background jobs for:** Video editing requests, file processing tasks, multi-step operations

**Check job status for:** Progress inquiries, completion questions, status requests

**Respond conversationally for:** Greetings, general questions, clarifications, feedback, discussions about capabilities, weather, or any non-task conversation

## Important Principles

- You can chat naturally while background jobs execute - these are parallel operations
- When a job is running, you remain available for conversation and can check its status
- Only start new jobs for new work requests, not for status inquiries about existing work
- Be helpful, conversational, and context-aware in all interactions"#;

        // Save user message to conversation history
        let user_msg = ConversationMessage::new_human(session_id.to_string(), user_input.to_string());
        match conversation_manager.save_message(&user_msg).await {
            Ok(_) => tracing::debug!("✅ Saved user message to DB for session {}", session_id),
            Err(e) => tracing::error!("❌ Failed to save user message: {}", e),
        }

        let mut final_response = String::new();
        let mut conversation_messages = messages;

        // Tool calling loop - continue until AI returns text (not tool calls)
        // CRITICAL FIX: Don't force tool calling for conversational queries
        // Let Claude decide when tools are needed (ToolChoice::Auto is set in claude_client.rs)
        let mut is_first_call = true;
        loop {
            if is_first_call {
                send_progress("🤖 Processing your message...");
                is_first_call = false;
            }

            let response = self.client.generate_content(
                conversation_messages.clone(),
                Some(control_tools.clone()),
                Some(system_prompt.to_string()),
            ).await.map_err(|e| format!("Claude API Error: {}", e))?;

            let mut has_tool_calls = false;
            let mut tool_results = Vec::new();

            // Process AI's response
            for content in &response.content {
                match content {
                    crate::claude_client::ResponseContent::Text { text } => {
                        final_response = text.clone();
                    }
                    crate::claude_client::ResponseContent::ToolUse { id, name, input } => {
                        has_tool_calls = true;
                        let tool_use_id = id.clone();
                        send_progress(&format!("🔧 Detected tool call: {}", name));
                        if name == "start_background_job" {
                            send_progress("🚀 Starting background video editing job...");
                            tracing::info!("🚀 AI decided to start background job");

                            let task_description = input.get("task_description")
                                .and_then(|v| v.as_str())
                                .unwrap_or(user_input);

                            // Spawn background job
                            let agent_type = video_job::AgentType::Claude;
                            let job_result = video_job::spawn_video_editing_job(
                                user_input.to_string(),
                                task_description.to_string(),
                                session_id.to_string(),
                                agent_type,
                                app_state.clone(),
                                job_manager.clone(),
                            ).await;

                            let tool_result = match job_result {
                                Ok(job_id) => {
                                    send_progress(&format!("✅ Background job started: {}", job_id));
                                    tracing::info!("✅ Background job started: {}", job_id);
                                    format!("Successfully started background video editing job with ID: {}. The job is now processing in the background and will send progress updates.", job_id)
                                }
                                Err(e) => {
                                    send_progress(&format!("❌ Failed to start job: {}", e));
                                    format!("Failed to start background job: {}", e)
                                }
                            };

                            tool_results.push((tool_use_id.clone(), tool_result));
                        } else if name == "check_job_status" {
                            send_progress("📊 Checking job status...");
                            let job_id = input.get("job_id")
                                .and_then(|v| v.as_str())
                                .filter(|s| !s.trim().is_empty());

                            let tool_result = if let Some(jid) = job_id {
                                // Check specific job
                                if let Some(status) = job_manager.get_job_status(jid).await {
                                    serde_json::to_string_pretty(&serde_json::json!({
                                        "job_id": jid,
                                        "status": format!("{:?}", status),
                                        "found": true
                                    })).unwrap_or_else(|_| "Error formatting job status".to_string())
                                } else {
                                    serde_json::to_string_pretty(&serde_json::json!({
                                        "job_id": jid,
                                        "found": false,
                                        "message": "Job not found"
                                    })).unwrap_or_else(|_| "Job not found".to_string())
                                }
                            } else {
                                // Get all jobs for this session
                                let session_jobs = job_manager.get_session_jobs(session_id).await;
                                let jobs_data: Vec<_> = session_jobs.iter().map(|job| {
                                    serde_json::json!({
                                        "job_id": job.id,
                                        "status": format!("{:?}", job.status),
                                        "created_at": job.created_at.to_rfc3339()
                                    })
                                }).collect();

                                serde_json::to_string_pretty(&serde_json::json!({
                                    "jobs": jobs_data,
                                    "total_count": jobs_data.len()
                                })).unwrap_or_else(|_| "Error formatting jobs".to_string())
                            };

                            tool_results.push((tool_use_id.clone(), tool_result));
                        } else if name == "search_memory" {
                            send_progress("🔍 Searching memory for relevant context...");
                            let query = input.get("query")
                                .and_then(|v| v.as_str())
                                .unwrap_or("");

                            let tool_result = if let Some(ref qdrant_client) = app_state.qdrant_client {
                                if let Some(ref voyage_embeddings) = app_state.voyage_embeddings {
                                    match qdrant_client.build_context_for_query_with_voyage(query, session_id, voyage_embeddings).await {
                                        Ok(context) => {
                                            if context.is_empty() {
                                                "No relevant memories found".to_string()
                                            } else {
                                                context
                                            }
                                        }
                                        Err(e) => format!("Error searching memory: {}", e)
                                    }
                                } else if let Some(ref gemini_client) = app_state.gemini_client {
                                    match qdrant_client.build_context_for_query_with_gemini(query, session_id, gemini_client).await {
                                        Ok(context) => {
                                            if context.is_empty() {
                                                "No relevant memories found".to_string()
                                            } else {
                                                context
                                            }
                                        }
                                        Err(e) => format!("Error searching memory: {}", e)
                                    }
                                } else {
                                    "Memory search unavailable - no embedding client".to_string()
                                }
                            } else {
                                "Memory search unavailable - Qdrant not configured".to_string()
                            };

                            tool_results.push((tool_use_id.clone(), tool_result));
                        }
                    }
                }
            }

            // If no tool calls, we have the final response
            if !has_tool_calls {
                break;
            }

            // Add assistant's tool uses and tool results to conversation
            // Convert ResponseContent to ContentBlock
            let content_blocks: Vec<crate::claude_client::ContentBlock> = response.content.iter().map(|rc| {
                match rc {
                    crate::claude_client::ResponseContent::Text { text } => {
                        crate::claude_client::ContentBlock::Text { text: text.clone() }
                    }
                    crate::claude_client::ResponseContent::ToolUse { id, name, input } => {
                        crate::claude_client::ContentBlock::ToolUse {
                            id: id.clone(),
                            name: name.clone(),
                            input: input.clone(),
                        }
                    }
                }
            }).collect();

            conversation_messages.push(ClaudeMessage {
                role: "assistant".to_string(),
                content: ClaudeContent::Blocks(content_blocks),
            });

            // Add tool results
            let tool_result_blocks: Vec<_> = tool_results.iter().map(|(id, result)| {
                crate::claude_client::ContentBlock::ToolResult {
                    tool_use_id: id.clone(),
                    content: result.clone(),
                    is_error: None,
                }
            }).collect();

            conversation_messages.push(ClaudeMessage {
                role: "user".to_string(),
                content: ClaudeContent::Blocks(tool_result_blocks),
            });

            // Continue loop - AI will process tool results and respond naturally
        }

        // Save assistant's final conversational response to history
        if !final_response.is_empty() {
            tracing::info!("💾 Attempting to save assistant response (length: {}) for session {}", final_response.len(), session_id);
            let assistant_msg = ConversationMessage::new_assistant(session_id.to_string(), final_response.clone());
            match conversation_manager.save_message(&assistant_msg).await {
                Ok(_) => tracing::info!("✅ Successfully saved assistant message to DB for session {}", session_id),
                Err(e) => tracing::error!("❌ Failed to save assistant message: {}", e),
            }
        } else {
            tracing::warn!("⚠️ final_response is empty, not saving assistant message for session {}", session_id);
        }

        Ok(final_response)
    }

    /// Create control tools for the AI to manage workflows
    fn create_control_tools() -> Vec<ClaudeTool> {
        vec![
            ClaudeTool {
                name: "start_background_job".to_string(),
                description: "Start a background video editing job to process videos. Use this ONLY when the user gives you a COMMAND or INSTRUCTION to perform video editing work (e.g., 'make it black and white', 'trim from 0-10 seconds', 'add text overlay'). DO NOT use this for questions like 'can you help', 'what can you do', 'are you able to', or status inquiries. The background job spawns a specialized agent with 39 video editing tools that executes the requested operations and sends progress updates.".to_string(),
                input_schema: InputSchema {
                    schema_type: "object".to_string(),
                    properties: HashMap::from([
                        ("task_description".to_string(), PropertyDefinition {
                            prop_type: "string".to_string(),
                            description: "Detailed description of what needs to be done (the user's request)".to_string(),
                            items: None,
                        }),
                        ("complexity".to_string(), PropertyDefinition {
                            prop_type: "string".to_string(),
                            description: "Task complexity: 'simple' (1-2 steps), 'medium' (3-5 steps), 'complex' (6+ steps)".to_string(),
                            items: None,
                        }),
                        ("estimated_steps".to_string(), PropertyDefinition {
                            prop_type: "number".to_string(),
                            description: "Estimated number of video editing steps needed".to_string(),
                            items: None,
                        }),
                    ]),
                    required: vec!["task_description".to_string()],
                },
            },
            ClaudeTool {
                name: "check_job_status".to_string(),
                description: "Check the status of background video editing jobs. Use this when the user asks about progress or wants updates. IMPORTANT: Leave job_id empty to get ALL jobs in the session. Only specify job_id if the user provides a specific job ID.".to_string(),
                input_schema: InputSchema {
                    schema_type: "object".to_string(),
                    properties: HashMap::from([
                        ("job_id".to_string(), PropertyDefinition {
                            prop_type: "string".to_string(),
                            description: "Optional: specific job ID to check. If not provided, returns ALL jobs in this session.".to_string(),
                            items: None,
                        }),
                    ]),
                    required: vec![],
                },
            },
            ClaudeTool {
                name: "search_memory".to_string(),
                description: "Search your conversation memory to find relevant past discussions, previous video editing tasks, or information from earlier in the conversation. Use this when the user asks about something from the past, wants to recall previous work, or when you need context from earlier conversations.".to_string(),
                input_schema: InputSchema {
                    schema_type: "object".to_string(),
                    properties: HashMap::from([
                        ("query".to_string(), PropertyDefinition {
                            prop_type: "string".to_string(),
                            description: "What to search for in past conversations".to_string(),
                            items: None,
                        }),
                        ("limit".to_string(), PropertyDefinition {
                            prop_type: "number".to_string(),
                            description: "Maximum number of results to return (default: 5)".to_string(),
                            items: None,
                        }),
                    ]),
                    required: vec!["query".to_string()],
                },
            },
        ]
    }
}

/// Gemini version of stateful agent
pub struct StatefulGeminiAgent {
    client: Arc<crate::gemini_client::GeminiClient>,
    workflow_manager: Arc<VideoWorkflowManager>,
}

impl StatefulGeminiAgent {
    pub fn new(client: Arc<crate::gemini_client::GeminiClient>) -> Self {
        Self {
            client,
            workflow_manager: Arc::new(VideoWorkflowManager::new()),
        }
    }

    pub async fn chat(
        &self,
        user_input: &str,
        session_id: &str,
        context: String,
        app_state: Arc<AppState>,
        job_manager: Arc<crate::jobs::JobManager>,
        progress_tx: Option<tokio::sync::mpsc::UnboundedSender<String>>,
    ) -> Result<String, String> {
        // Helper to send progress updates
        let send_progress = |msg: &str| {
            if let Some(ref tx) = progress_tx {
                let _ = tx.send(msg.to_string());
            }
            tracing::info!("{}", msg);
        };

        send_progress("🔧 Initializing Gemini agent (3 control tools + 40+ video editing tools in background job system)...");
        let control_tools = Self::create_control_tools();

        // Initialize ConversationManager to retrieve and save conversation history
        let conversation_manager = ConversationManager::new(app_state.db_pool.clone());

        // Ensure schema exists
        if let Err(e) = conversation_manager.initialize_schema().await {
            tracing::warn!("Failed to initialize conversation schema: {}", e);
        }

        // Retrieve conversation history (last 20 messages)
        let conversation_history = conversation_manager
            .get_conversation_history(session_id, Some(20))
            .await
            .unwrap_or_default();

        let system_instruction = r#"You are an intelligent video editing assistant with the ability to manage background processing workflows.

## Your Role
You engage in natural conversation with users while coordinating video editing tasks. You have access to a background job system that handles complex video processing operations in parallel while you continue chatting.

## Available Tools

### start_background_job
Launches a dedicated video editing agent with 39 specialized tools (trim, merge, filter, overlay, color adjustment, audio processing, etc.) that executes in the background. Use this when the user requests video processing work.

### check_job_status
Queries the status of background jobs. Use this when the user asks about progress, completion, or wants updates on running tasks. Can check specific jobs by ID or list all jobs in the current session.

## Decision-Making Guidelines

Trust your understanding of natural language to determine user intent:

**Start background jobs for:** Video editing requests, file processing tasks, multi-step operations

**Check job status for:** Progress inquiries, completion questions, status requests

**Respond conversationally for:** Greetings, general questions, clarifications, feedback, discussions about capabilities, weather, or any non-task conversation

## Important Principles

- You can chat naturally while background jobs execute - these are parallel operations
- When a job is running, you remain available for conversation and can check its status
- Only start new jobs for new work requests, not for status inquiries about existing work
- Be helpful, conversational, and context-aware in all interactions"#;

        // Build contents array with conversation history
        let mut contents = Vec::new();

        // Add system instruction as first model message (Gemini pattern)
        contents.push(crate::gemini_client::Content {
            parts: vec![crate::gemini_client::Part::Text {
                text: system_instruction.to_string(),
            }],
            role: Some("model".to_string()),
        });

        // Add conversation history
        for msg in &conversation_history {
            let role = match msg.role {
                crate::agent::conversation_manager::MessageRole::Human => "user",
                crate::agent::conversation_manager::MessageRole::Assistant => "model",
                _ => continue, // Skip system and function messages
            };

            contents.push(crate::gemini_client::Content {
                parts: vec![crate::gemini_client::Part::Text {
                    text: msg.content.clone(),
                }],
                role: Some(role.to_string()),
            });
        }

        // Add current user message with context
        let current_message = if !context.is_empty() {
            format!("{}\n\n{}", context, user_input)
        } else {
            user_input.to_string()
        };

        contents.push(crate::gemini_client::Content {
            parts: vec![crate::gemini_client::Part::Text {
                text: current_message,
            }],
            role: Some("user".to_string()),
        });

        // Save user message to conversation history
        let user_msg = ConversationMessage::new_human(session_id.to_string(), user_input.to_string());
        match conversation_manager.save_message(&user_msg).await {
            Ok(_) => tracing::debug!("✅ Saved user message to DB for session {}", session_id),
            Err(e) => tracing::error!("❌ Failed to save user message: {}", e),
        }

        let mut final_response = String::new();
        let mut conversation_contents = contents;

        // Tool calling loop - continue until AI returns text (not function calls)
        let mut is_first_call = true;
        loop {
            if is_first_call {
                send_progress("🤖 Processing your message...");
                is_first_call = false;
            }

            let request = crate::gemini_client::GenerateContentRequest {
                contents: conversation_contents.clone(),
                tools: Some(vec![crate::gemini_client::Tool {
                    function_declarations: control_tools.clone(),
                }]),
                generation_config: Some(crate::gemini_client::GenerationConfig {
                    temperature: 0.5,
                    top_k: 40,
                    top_p: 0.9,
                    max_output_tokens: 2048,
                }),
                tool_config: Some(crate::gemini_client::ToolConfig {
                    function_calling_config: crate::gemini_client::FunctionCallingConfig {
                        mode: crate::gemini_client::FunctionCallingMode::Auto,  // Auto: Let Gemini decide - respond naturally OR call tools
                    },
                }),
            };

            let response = self.client.generate_content(request).await
                .map_err(|e| format!("Gemini API Error: {}", e))?;

            let mut has_function_calls = false;
            let mut function_results: Vec<(String, serde_json::Value, Option<String>)> = Vec::new(); // (name, result, thought_signature)

            if let Some(candidate) = response.candidates.first() {
                // Handle optional content field (may be None if blocked by safety filters)
                if let Some(ref content) = candidate.content {
                    for part in &content.parts {
                        match part {
                            crate::gemini_client::Part::Text { text } => {
                                final_response = text.clone();
                            }
                            crate::gemini_client::Part::FunctionCall { function_call } => {
                                has_function_calls = true;
                                let function_name = function_call.name.clone();

                                send_progress(&format!("🔧 Detected tool call: {}", function_name));

                                if function_name == "start_background_job" {
                                    send_progress("🚀 Starting background video editing job...");
                                    tracing::info!("🚀 Gemini decided to start background job");

                                    let task_description = function_call.args.get("task_description")
                                        .and_then(|v| v.as_str())
                                        .unwrap_or(user_input);

                                    let agent_type = video_job::AgentType::Gemini;

                                    let job_result = video_job::spawn_video_editing_job(
                                        user_input.to_string(),
                                        task_description.to_string(),
                                        session_id.to_string(),
                                        agent_type,
                                        app_state.clone(),
                                        job_manager.clone(),
                                    ).await;

                                    let tool_result = match job_result {
                                        Ok(job_id) => {
                                            send_progress(&format!("✅ Background job started: {}", job_id));
                                            tracing::info!("✅ Background job started: {}", job_id);
                                            serde_json::json!({
                                                "success": true,
                                                "job_id": job_id,
                                                "message": format!("Successfully started background video editing job with ID: {}. The job is now processing in the background and will send progress updates.", job_id)
                                            })
                                        }
                                        Err(e) => {
                                            send_progress(&format!("❌ Failed to start job: {}", e));
                                            serde_json::json!({
                                                "success": false,
                                                "error": format!("Failed to start background job: {}", e)
                                            })
                                        }
                                    };

                                    function_results.push((function_name.clone(), tool_result, function_call.thought_signature.clone()));
                                } else if function_name == "check_job_status" {
                                    send_progress("📊 Checking job status...");
                                    let job_id = function_call.args.get("job_id")
                                        .and_then(|v| v.as_str())
                                        .filter(|s| !s.trim().is_empty());

                                    let tool_result = if let Some(jid) = job_id {
                                        // Check specific job
                                        if let Some(status) = job_manager.get_job_status(jid).await {
                                            serde_json::json!({
                                                "job_id": jid,
                                                "status": format!("{:?}", status),
                                                "found": true
                                            })
                                        } else {
                                            serde_json::json!({
                                                "job_id": jid,
                                                "found": false,
                                                "message": "Job not found"
                                            })
                                        }
                                    } else {
                                        // Get all jobs for this session
                                        let session_jobs = job_manager.get_session_jobs(session_id).await;
                                        let jobs_data: Vec<_> = session_jobs.iter().map(|job| {
                                            serde_json::json!({
                                                "job_id": job.id,
                                                "status": format!("{:?}", job.status),
                                                "created_at": job.created_at.to_rfc3339()
                                            })
                                        }).collect();

                                        serde_json::json!({
                                            "jobs": jobs_data,
                                            "total_count": jobs_data.len()
                                        })
                                    };

                                    function_results.push((function_name.clone(), tool_result, function_call.thought_signature.clone()));
                                } else if function_name == "search_memory" {
                                    send_progress("🔍 Searching memory for relevant context...");
                                    let query = function_call.args.get("query")
                                        .and_then(|v| v.as_str())
                                        .unwrap_or("");

                                    let tool_result = if let Some(ref qdrant_client) = app_state.qdrant_client {
                                        if let Some(ref voyage_embeddings) = app_state.voyage_embeddings {
                                            match qdrant_client.build_context_for_query_with_voyage(query, session_id, voyage_embeddings).await {
                                                Ok(context) => {
                                                    if context.is_empty() {
                                                        serde_json::json!({
                                                            "found": false,
                                                            "message": "No relevant memories found"
                                                        })
                                                    } else {
                                                        serde_json::json!({
                                                            "found": true,
                                                            "context": context
                                                        })
                                                    }
                                                }
                                                Err(e) => serde_json::json!({
                                                    "error": format!("Error searching memory: {}", e)
                                                })
                                            }
                                        } else if let Some(ref gemini_client) = app_state.gemini_client {
                                            match qdrant_client.build_context_for_query_with_gemini(query, session_id, gemini_client).await {
                                                Ok(context) => {
                                                    if context.is_empty() {
                                                        serde_json::json!({
                                                            "found": false,
                                                            "message": "No relevant memories found"
                                                        })
                                                    } else {
                                                        serde_json::json!({
                                                            "found": true,
                                                            "context": context
                                                        })
                                                    }
                                                }
                                                Err(e) => serde_json::json!({
                                                    "error": format!("Error searching memory: {}", e)
                                                })
                                            }
                                        } else {
                                            serde_json::json!({
                                                "error": "Memory search unavailable - no embedding client"
                                            })
                                        }
                                    } else {
                                        serde_json::json!({
                                            "error": "Memory search unavailable - Qdrant not configured"
                                        })
                                    };

                                    function_results.push((function_name.clone(), tool_result, function_call.thought_signature.clone()));
                                }
                            }
                            _ => {}
                        }
                    }
                } else {
                    // Content was blocked or missing
                    if let Some(block_reason) = response.prompt_feedback.as_ref().and_then(|f| f.block_reason.as_ref()) {
                        tracing::warn!("Gemini content blocked: {}", block_reason);
                        final_response = format!("I cannot process this request due to content safety filters: {}", block_reason);
                    } else if let Some(finish_reason) = &candidate.finish_reason {
                        tracing::warn!("Gemini response finished with reason: {}", finish_reason);
                        final_response = format!("Response could not be generated: {}", finish_reason);
                    } else {
                        tracing::warn!("Gemini response has no content");
                        final_response = "I apologize, but I couldn't generate a response for that request.".to_string();
                    }
                    break;
                }
            }

            // If no function calls, we have the final response
            if !has_function_calls {
                break;
            }

            // Add model's function calls to conversation
            if let Some(candidate) = response.candidates.first() {
                if let Some(ref content) = candidate.content {
                    conversation_contents.push(crate::gemini_client::Content {
                        parts: content.parts.clone(),
                        role: Some("model".to_string()),
                    });
                }
            }

            // Add function responses to conversation (with thought signatures)
            let function_response_parts: Vec<_> = function_results.iter().map(|(name, result, thought_sig)| {
                let mut response_map = HashMap::new();
                response_map.insert("result".to_string(), result.clone());

                crate::gemini_client::Part::FunctionResponse {
                    function_response: crate::gemini_client::FunctionResponse {
                        name: name.clone(),
                        response: response_map,
                        thought_signature: thought_sig.clone(),
                    }
                }
            }).collect();

            conversation_contents.push(crate::gemini_client::Content {
                parts: function_response_parts,
                role: Some("function".to_string()),
            });

            // Continue loop - AI will process function results and respond naturally
        }

        // Save assistant's final conversational response to history
        if !final_response.is_empty() {
            tracing::info!("💾 Attempting to save assistant response (length: {}) for session {}", final_response.len(), session_id);
            let assistant_msg = ConversationMessage::new_assistant(session_id.to_string(), final_response.clone());
            match conversation_manager.save_message(&assistant_msg).await {
                Ok(_) => tracing::info!("✅ Successfully saved assistant message to DB for session {}", session_id),
                Err(e) => tracing::error!("❌ Failed to save assistant message: {}", e),
            }
        } else {
            tracing::warn!("⚠️ final_response is empty, not saving assistant message for session {}", session_id);
        }

        Ok(final_response)
    }

    fn create_control_tools() -> Vec<crate::gemini_client::FunctionDeclaration> {
        vec![
            crate::gemini_client::FunctionDeclaration {
                name: "start_background_job".to_string(),
                description: "Start a background video editing job to process videos. Use this ONLY when the user gives you a COMMAND or INSTRUCTION to perform video editing work (e.g., 'make it black and white', 'trim from 0-10 seconds', 'add text overlay'). DO NOT use this for questions like 'can you help', 'what can you do', 'are you able to', or status inquiries. The background job spawns a specialized agent with 39 video editing tools that executes the requested operations and sends progress updates.".to_string(),
                parameters: crate::gemini_client::Parameters {
                    param_type: "object".to_string(),
                    properties: HashMap::from([
                        ("task_description".to_string(), crate::gemini_client::PropertyDefinition {
                            prop_type: "string".to_string(),
                            description: "What needs to be done".to_string(),
                            items: None,
                        }),
                        ("complexity".to_string(), crate::gemini_client::PropertyDefinition {
                            prop_type: "string".to_string(),
                            description: "simple, medium, or complex".to_string(),
                            items: None,
                        }),
                    ]),
                    required: vec!["task_description".to_string()],
                },
            },
            crate::gemini_client::FunctionDeclaration {
                name: "check_job_status".to_string(),
                description: "Check the status of background video editing jobs. Use this when the user asks about progress, wants updates, or inquires about task completion. Can check a specific job by ID or list all jobs in the current session if no ID is provided.".to_string(),
                parameters: crate::gemini_client::Parameters {
                    param_type: "object".to_string(),
                    properties: HashMap::from([
                        ("job_id".to_string(), crate::gemini_client::PropertyDefinition {
                            prop_type: "string".to_string(),
                            description: "Optional: Specific job ID to check. If omitted, lists all session jobs.".to_string(),
                            items: None,
                        }),
                    ]),
                    required: vec![],
                },
            },
            crate::gemini_client::FunctionDeclaration {
                name: "search_memory".to_string(),
                description: "Search your conversation memory to find relevant past discussions, previous video editing tasks, or information from earlier in the conversation. Use this when the user asks about something from the past, wants to recall previous work, or when you need context from earlier conversations.".to_string(),
                parameters: crate::gemini_client::Parameters {
                    param_type: "object".to_string(),
                    properties: HashMap::from([
                        ("query".to_string(), crate::gemini_client::PropertyDefinition {
                            prop_type: "string".to_string(),
                            description: "What to search for in past conversations".to_string(),
                            items: None,
                        }),
                        ("limit".to_string(), crate::gemini_client::PropertyDefinition {
                            prop_type: "number".to_string(),
                            description: "Maximum number of results to return (default: 5)".to_string(),
                            items: None,
                        }),
                    ]),
                    required: vec!["query".to_string()],
                },
            },
        ]
    }
}
