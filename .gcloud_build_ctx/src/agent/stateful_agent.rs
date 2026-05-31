// Stateful agent that manages conversations and decides when to spawn background jobs
// The AI has a special tool to start background jobs for complex video editing tasks

use crate::agent::conversation_manager::{ConversationManager, ConversationMessage};
use crate::claude_client::{
    ClaudeClient, ClaudeContent, ClaudeMessage, ClaudeTool, InputSchema, PropertyDefinition,
};
use crate::jobs::video_job;
use crate::AppState;
use std::collections::HashMap;
use std::sync::Arc;

pub struct StatefulClaudeAgent {
    client: Arc<ClaudeClient>,
}

impl StatefulClaudeAgent {
    pub fn new(client: Arc<ClaudeClient>) -> Self {
        Self { client }
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
        _workflow_id: Option<uuid::Uuid>,
        _user_message_rx: Option<tokio::sync::mpsc::UnboundedReceiver<String>>,
    ) -> Result<String, String> {
        // Helper to send progress updates
        let send_progress = |msg: &str| {
            if let Some(ref tx) = progress_tx {
                let _ = tx.send(msg.to_string());
            }
            tracing::info!("{}", msg);
        };

        send_progress("🔧 Initializing Claude agent with durable workflow control and the full VideoSync creative toolbelt...");
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
                    crate::agent::conversation_manager::MessageRole::Assistant => {
                        "assistant".to_string()
                    }
                    _ => continue, // Skip system and function messages
                },
                content: ClaudeContent::Text(msg.content.clone()),
            });
        }

        // Add current user message with context (context already contains user_request, avoid duplication)
        let current_message = if !context.is_empty() {
            context.to_string()
        } else {
            user_input.to_string()
        };

        messages.push(ClaudeMessage {
            role: "user".to_string(),
            content: ClaudeContent::Text(current_message.clone()),
        });

        let system_prompt = r#"You are a media generation engine. Your single job is to call tools to produce actual media files. Never give a plan, never confirm you understand — just call the tool.

## How to Talk
- Answer the user's question directly. Don't restate the task, the workflow, or what you're doing unless it's genuinely relevant.
- No preambles like "I'm already on it!" or "Regarding your request..." or "The task is currently being processed."
- If the user asks "how's it going?" just say "Almost done, just rendering the final frame" — not "The sample generation workflow is currently in the rendering phase."
- Be brief. Be natural. Sound human.

## Available Tools

### start_background_job
Launches a dedicated video editing agent with the full VideoSync tool registry: FFmpeg editing tools, long-form generation, Blender/Manim/LaTeX rendering, VibeVoice/audio tools, thumbnails, QA/review, uploads, and delivery packaging. Use this when the user requests video processing or generation work, especially long-running tasks.

### check_job_status
Queries the status of background jobs. Use this when the user asks about progress, completion, or wants updates on running tasks. Can check specific jobs by ID or list all jobs in the current session.

## Decision-Making Guidelines

Trust your understanding of natural language to determine user intent:

**Start background jobs for:** Video editing requests, file processing tasks, multi-step operations

**Check job status for:** Progress inquiries, completion questions, status requests

**Respond conversationally for:** Greetings, general questions, clarifications, feedback, discussions about capabilities, weather, or any non-task conversation

## Important Principles

- You can chat naturally while background jobs execute - these are parallel operations
- Long videos are allowed. The background system can break long-form work into durable, resumable workflow nodes and report progress while it renders.
- Use the broader creative stack when helpful: clipping/enhancement, thumbnails, product mockups, education visuals, 3D scenes, voice/audio, mixed bundles, and delivery pages.
- When a job is running, you remain available for conversation and can check its status
- Only start new jobs for new work requests, not for status inquiries about existing work
- If the user's task is clearly defined, call `set_chat_title` early with a concise descriptive title
- If tools created output files, finish with `submit_final_answer` so the user gets delivery/output links instead of internal file paths"#;

        // Save user message to conversation history
        let user_msg =
            ConversationMessage::new_human(session_id.to_string(), user_input.to_string());
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

            let response = self
                .client
                .generate_content(
                    conversation_messages.clone(),
                    Some(control_tools.clone()),
                    Some(system_prompt.to_string()),
                )
                .await
                .map_err(|e| format!("Claude API Error: {}", e))?;

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

                            let task_description = input
                                .get("task_description")
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
                            )
                            .await;

                            let tool_result = match job_result {
                                Ok(job_id) => {
                                    send_progress(&format!(
                                        "✅ Background job started: {}",
                                        job_id
                                    ));
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
                            let job_id = input
                                .get("job_id")
                                .and_then(|v| v.as_str())
                                .filter(|s| !s.trim().is_empty());

                            let tool_result = if let Some(jid) = job_id {
                                // Check specific job with enhanced details
                                if let Some(job) = job_manager.get_job(jid).await {
                                    let elapsed = if let Some(started) = job.started_at {
                                        let duration =
                                            chrono::Utc::now().signed_duration_since(started);
                                        format!(
                                            "{}m {}s",
                                            duration.num_minutes(),
                                            duration.num_seconds() % 60
                                        )
                                    } else {
                                        "Not started yet".to_string()
                                    };

                                    let status_detail = match &job.status {
                                        crate::jobs::JobStatus::Running {
                                            current_step,
                                            progress_percent,
                                            steps_completed,
                                            total_steps,
                                            completed_actions,
                                            current_action_detail,
                                        } => {
                                            let mut status_str = format!(
                                                "RUNNING\n  Current Step: {}\n  Steps: {}/{}",
                                                current_step, steps_completed, total_steps
                                            );

                                            if let Some(pct) = progress_percent {
                                                status_str.push_str(&format!(
                                                    "\n  Progress: {:.1}%",
                                                    pct
                                                ));
                                            }

                                            if let Some(actions) = completed_actions {
                                                if !actions.is_empty() {
                                                    status_str.push_str(&format!(
                                                        "\n\n  Completed Actions:\n"
                                                    ));
                                                    for action in actions {
                                                        status_str.push_str(&format!(
                                                            "    ✅ {}\n",
                                                            action
                                                        ));
                                                    }
                                                }
                                            }

                                            if let Some(detail) = current_action_detail {
                                                status_str
                                                    .push_str(&format!("\n  Detail: {}", detail));
                                            }

                                            status_str
                                        }
                                        crate::jobs::JobStatus::Completed {
                                            result,
                                            output_files,
                                            duration_seconds,
                                        } => {
                                            format!(
                                                "COMPLETED\n  Duration: {:.1}s\n  Files: {}\n  Result: {}",
                                                duration_seconds, output_files.len(), result
                                            )
                                        }
                                        crate::jobs::JobStatus::Failed {
                                            error,
                                            failed_at_step,
                                        } => {
                                            format!(
                                                "FAILED\n  Failed at: {}\n  Error: {}",
                                                failed_at_step, error
                                            )
                                        }
                                        crate::jobs::JobStatus::Queued { position } => {
                                            format!("QUEUED (position: {})", position)
                                        }
                                        _ => format!("{:?}", job.status),
                                    };

                                    // Check if job appears stuck
                                    let stuck_warning = if job.is_possibly_stuck() {
                                        "\n\n⚠️ WARNING: This job hasn't reported progress in over 10 minutes. It may be stuck.\n\
                                        Possible causes:\n\
                                        - FFmpeg process hung due to system sleep/wake\n\
                                        - Network download timeout\n\
                                        - External process not responding\n\n\
                                        Consider canceling and restarting the job."
                                    } else {
                                        ""
                                    };

                                    format!(
                                        "📊 JOB STATUS REPORT\n\n\
                                        Job ID: {}\n\
                                        Status: {}\n\
                                        Time Elapsed: {}\n\
                                        Created: {}{}",
                                        jid,
                                        status_detail,
                                        elapsed,
                                        job.created_at.format("%H:%M:%S"),
                                        stuck_warning
                                    )
                                } else {
                                    format!("❌ Job {} not found. It may have completed and been cleaned up, or the ID is incorrect.", jid)
                                }
                            } else {
                                // Get all jobs for this session
                                let session_jobs = job_manager.get_session_jobs(session_id).await;
                                let jobs_data: Vec<_> = session_jobs
                                    .iter()
                                    .map(|job| {
                                        serde_json::json!({
                                            "job_id": job.id,
                                            "status": format!("{:?}", job.status),
                                            "created_at": job.created_at.to_rfc3339()
                                        })
                                    })
                                    .collect();

                                serde_json::to_string_pretty(&serde_json::json!({
                                    "jobs": jobs_data,
                                    "total_count": jobs_data.len()
                                }))
                                .unwrap_or_else(|_| "Error formatting jobs".to_string())
                            };

                            tool_results.push((tool_use_id.clone(), tool_result));
                        } else if name == "search_memory" {
                            send_progress("🔍 Searching memory for relevant context...");
                            let query = input.get("query").and_then(|v| v.as_str()).unwrap_or("");

                            let tool_result = if let Some(ref qdrant_client) =
                                app_state.qdrant_client
                            {
                                match qdrant_client
                                    .build_context_for_query(
                                        query,
                                        session_id,
                                        app_state.voyage_embeddings.as_ref(),
                                        app_state.video_gemini_client.as_ref().or(app_state.gemini_client.as_ref()),
                                    )
                                    .await
                                {
                                    Ok(Some(context)) => {
                                        context
                                    }
                                    Ok(None) => {
                                        "Memory search unavailable - no embedding client".to_string()
                                    }
                                    Err(e) => format!("Error searching memory: {}", e),
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
            let content_blocks: Vec<crate::claude_client::ContentBlock> = response
                .content
                .iter()
                .map(|rc| match rc {
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
                })
                .collect();

            conversation_messages.push(ClaudeMessage {
                role: "assistant".to_string(),
                content: ClaudeContent::Blocks(content_blocks),
            });

            // Add tool results
            let tool_result_blocks: Vec<_> = tool_results
                .iter()
                .map(
                    |(id, result)| crate::claude_client::ContentBlock::ToolResult {
                        tool_use_id: id.clone(),
                        content: result.clone(),
                        is_error: None,
                    },
                )
                .collect();

            conversation_messages.push(ClaudeMessage {
                role: "user".to_string(),
                content: ClaudeContent::Blocks(tool_result_blocks),
            });

            // Continue loop - AI will process tool results and respond naturally
        }

        // Save assistant's final conversational response to history
        if !final_response.is_empty() {
            tracing::info!(
                "💾 Attempting to save assistant response (length: {}) for session {}",
                final_response.len(),
                session_id
            );
            let assistant_msg =
                ConversationMessage::new_assistant(session_id.to_string(), final_response.clone());
            match conversation_manager.save_message(&assistant_msg).await {
                Ok(_) => tracing::info!(
                    "✅ Successfully saved assistant message to DB for session {}",
                    session_id
                ),
                Err(e) => tracing::error!("❌ Failed to save assistant message: {}", e),
            }
        } else {
            tracing::warn!(
                "⚠️ final_response is empty, not saving assistant message for session {}",
                session_id
            );
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
}

impl StatefulGeminiAgent {
    pub fn new(client: Arc<crate::gemini_client::GeminiClient>) -> Self {
        Self { client }
    }

    pub async fn chat(
        &self,
        user_input: &str,
        session_id: &str,
        context: String,
        app_state: Arc<AppState>,
        job_manager: Arc<crate::jobs::JobManager>,
        progress_tx: Option<tokio::sync::mpsc::UnboundedSender<String>>,
        workflow_id: Option<uuid::Uuid>,
        mut user_message_rx: Option<tokio::sync::mpsc::UnboundedReceiver<String>>,
    ) -> Result<String, String> {
        // Helper to send progress updates
        let send_progress = |msg: &str| {
            if let Some(ref tx) = progress_tx {
                let _ = tx.send(msg.to_string());
            }
            tracing::info!("{}", msg);
        };

        send_progress("🧠 Loading the full production toolbelt for your request...");
        // Full-toolbelt mode: agents receive the full allowed video tool catalog by
        // default and choose the right tools themselves at runtime.
        let selected_video_tools = crate::ai_tool_selector::select_tools_for_request(
            user_input,
            app_state.nvidia_nim_client.as_ref(),
            app_state
                .video_gemini_client
                .as_ref()
                .or(app_state.gemini_client.as_ref()),
        )
        .await;
        // Prepend the 3 control tools (always needed for agent self-management)
        let mut all_tools = Self::create_control_tools_gemini();
        all_tools.extend(selected_video_tools);
        tracing::info!(
            "🔧 Agent loaded {} tools for session {}",
            all_tools.len(),
            session_id
        );

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

        let system_instruction = r#"You are a media generation engine. Your single job is to call tools to produce actual media files. Never give a plan, never confirm you understand — just call the tool.

## CRITICAL RULE: CALL TOOLS IMMEDIATELY
When the user asks you to CREATE, GENERATE, PRODUCE, MAKE, BUILD, RENDER, or EDIT media (video, thumbnail, clip, demo, image, ad, animation, scene, sample, narration), you MUST call the appropriate generation tool on your very first response. Do NOT respond with text saying what you will do. Call the tool NOW.

Wrong: "I'll generate that video for you right away!"
Right: Call auto_generate_video() directly

Wrong: "Let me create a thumbnail for your video."
Right: Call create_thumbnail() directly

Wrong: "Here's my plan for the sample..."
Right: Call generate_image(), create_thumbnail(), or auto_generate_video() directly

## How to Talk
- Answer the user's question directly. Don't restate the task or what you're doing.
- No preambles. No "I'm already on it!" No "Regarding your request..."
- Be brief. Be natural. Sound human.
- After you finish generating, call submit_final_answer with the summary and output file paths.

## Available Tools Summary

**Generation (call these FIRST):** create_thumbnail, generate_image, auto_generate_video, generate_long_form_video, generate_text_to_speech, generate_sound_effect, generate_music, generate_video_script

**Editing:** trim_video, merge_videos, split_video, crop_video, rotate_video, flip_video, resize_video, scale_video, stabilize_video

**Visual Effects:** add_text_overlay, add_overlay, apply_filter, adjust_color, picture_in_picture, chroma_key, split_screen, add_subtitles

**Audio:** add_audio, extract_audio, adjust_volume, fade_audio

**Stock Media:** pexels_search, pexels_download_video, pexels_download_photo

**Analysis:** view_video, analyze_video, review_video, extract_frames

**Session:** set_chat_title, submit_final_answer

## Workflow
1. READ the request carefully
2. CALL the appropriate generation/editing tool immediately
3. After the tool returns, check the result
4. If more work is needed, call another tool
5. When all work is complete, call submit_final_answer with the output files

## Important Principles
- Direct tool execution is FASTER than background jobs — use tools directly
- Use start_background_job only for multi-step workflows spanning 5+ operations
- Call `set_chat_title` early with a concise descriptive title
- When you create output files, end with `submit_final_answer` and include every generated output file path
- Do NOT stop at a plan — actually generate the deliverable the user asked for"#;

        // Build contents array with conversation history
        let mut contents = Vec::new();

        // System instruction is passed via the system_instruction field in the request (not as a content message)
        let system_instruction_content = crate::gemini_client::Content {
            parts: vec![crate::gemini_client::Part::Text {
                text: system_instruction.to_string(),
            }],
            role: None,
        };

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

        // Add current user message with context (context already contains user_request, avoid duplication)
        let current_message = if !context.is_empty() {
            context.to_string()
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
        let user_msg =
            ConversationMessage::new_human(session_id.to_string(), user_input.to_string());
        match conversation_manager.save_message(&user_msg).await {
            Ok(_) => tracing::debug!("✅ Saved user message to DB for session {}", session_id),
            Err(e) => tracing::error!("❌ Failed to save user message: {}", e),
        }

        // ── Gemini primary agent ─────────────────────────────────────────────

        let mut final_response = String::new();
        let mut conversation_contents = contents;

        // Tool calling loop - continue until AI returns text (not function calls)
        let mut is_first_call = true;
        loop {
            // Determine tool calling mode BEFORE setting is_first_call = false
            let expect_generation = is_first_call && (
                user_input.to_lowercase().contains("generate")
                || user_input.to_lowercase().contains("create")
                || user_input.to_lowercase().contains("sample")
                || user_input.to_lowercase().contains("thumbnail")
                || user_input.to_lowercase().contains("produce")
                || user_input.to_lowercase().contains("make")
                || user_input.to_lowercase().contains("render")
                || user_input.to_lowercase().contains("build")
                || user_input.to_lowercase().contains("edit")
                || user_input.to_lowercase().contains("video")
                || user_input.to_lowercase().contains("deliver")
                || user_input.to_lowercase().contains("demo")
            );

            if is_first_call {
                send_progress("🤖 Processing your message...");
                is_first_call = false;
            }

            // 🆕 INTERACTIVE: Check for user follow-up messages between tool calls
            if let Some(ref mut rx) = user_message_rx {
                while let Ok(followup) = rx.try_recv() {
                    tracing::info!("📨 Agent received follow-up message mid-work: session={}", session_id);
                    send_progress("💬 Received your follow-up while working...");

                    // Save user message to conversation history
                    let user_msg = ConversationMessage::new_human(
                        session_id.to_string(),
                        followup.clone(),
                    );
                    let _ = conversation_manager.save_message(&user_msg).await;

                    // Append to conversation history for the LLM
                    conversation_contents.push(crate::gemini_client::Content {
                        parts: vec![crate::gemini_client::Part::Text {
                            text: followup,
                        }],
                        role: Some("user".to_string()),
                    });

                    // Call Gemini for a quick conversational response
                    let quick_req = crate::gemini_client::GenerateContentRequest {
                        contents: conversation_contents.clone(),
                        tools: Some(vec![crate::gemini_client::Tool {
                            function_declarations: all_tools.clone(),
                        }]),
                        generation_config: Some(crate::gemini_client::GenerationConfig {
                            temperature: 0.7,
                            top_k: 40,
                            top_p: 0.9,
                            max_output_tokens: 1024,
                        }),
                        tool_config: None,
                        system_instruction: Some(system_instruction_content.clone()),
                    };
                    let response = self.client.generate_content(quick_req).await;

                    if let Ok(response) = response {
                        if let Some(candidate) = response.candidates.first() {
                            if let Some(content) = &candidate.content {
                                if let Some(text) = content.parts.first() {
                                    if let crate::gemini_client::Part::Text { text } = text {
                                        let reply = text.clone();
                                        // Save assistant response
                                        let assistant_msg = ConversationMessage::new_assistant(
                                            session_id.to_string(),
                                            reply.clone(),
                                        );
                                        let _ = conversation_manager.save_message(&assistant_msg).await;
                                        // Push to conversation history
                                        conversation_contents.push(crate::gemini_client::Content {
                                            parts: vec![crate::gemini_client::Part::Text {
                                                text: reply.clone(),
                                            }],
                                            role: Some("model".to_string()),
                                        });
                                        // Send to WebSocket via progress channel
                                        send_progress(&format!("💬 {}", reply));
                                    }
                                }
                            }
                        }
                    }
                }
            }

            // When forcing Any mode on the first call, exclude completion tools
            // so Gemini doesn't shortcut by calling submit_final_answer immediately
            let first_call_tools: Vec<crate::gemini_client::FunctionDeclaration> = if expect_generation {
                all_tools.iter()
                    .filter(|t| t.name != "submit_final_answer" && t.name != "set_chat_title")
                    .cloned()
                    .collect()
            } else {
                all_tools.clone()
            };

            let tools_for_request = if expect_generation {
                first_call_tools.as_slice()
            } else {
                all_tools.as_slice()
            };

            let request = crate::gemini_client::GenerateContentRequest {
                contents: conversation_contents.clone(),
                tools: Some(vec![crate::gemini_client::Tool {
                    function_declarations: tools_for_request.to_vec(),
                }]),
                generation_config: Some(crate::gemini_client::GenerationConfig {
                    temperature: 0.5,
                    top_k: 40,
                    top_p: 0.9,
                    max_output_tokens: 8192,
                }),
                tool_config: Some(crate::gemini_client::ToolConfig {
                    function_calling_config: crate::gemini_client::FunctionCallingConfig {
                        mode: if expect_generation {
                            tracing::warn!("🔧 FORCING FunctionCallingMode::Any for generation request");
                            crate::gemini_client::FunctionCallingMode::Any
                        } else {
                            crate::gemini_client::FunctionCallingMode::Auto
                        },
                    },
                }),
                system_instruction: Some(system_instruction_content.clone()),
            };

            let response = match self.client.generate_content(request).await {
                Ok(r) => r,
                Err(gemini_err) => {
                    tracing::warn!("⚠️ Gemini failed, trying DeepSeek fallback: {}", gemini_err);
                    // Build OpenAI-format messages from conversation history
                    let mut ds_messages: Vec<serde_json::Value> = vec![
                        serde_json::json!({"role": "system", "content": system_instruction})
                    ];
                    for msg in &conversation_history {
                        let role = match msg.role {
                            crate::agent::conversation_manager::MessageRole::Human => "user",
                            crate::agent::conversation_manager::MessageRole::Assistant => "assistant",
                            _ => continue,
                        };
                        ds_messages.push(serde_json::json!({"role": role, "content": msg.content}));
                    }
                    let current_msg = if !context.is_empty() {
                        context.to_string()
                    } else {
                        user_input.to_string()
                    };
                    ds_messages.push(serde_json::json!({"role": "user", "content": current_msg}));

                    if let Some(ref ds_client) = app_state.deepseek_client {
                        send_progress("🤖 Gemini unavailable, trying DeepSeek V4...");

                        let exec_context = crate::agent::tool_executor::ToolExecutionContext {
                            session_id: session_id.to_string(),
                            user_id: None,
                            app_state: app_state.clone(),
                            workflow_id,
                        };

                        match run_deepseek_tool_loop(
                            ds_client,
                            &mut ds_messages,
                            &all_tools,
                            &exec_context,
                            &send_progress,
                        )
                        .await
                        {
                            Ok(response) => {
                                let clean = response.trim().to_string();
                                let msg = ConversationMessage::new_assistant(
                                    session_id.to_string(),
                                    clean.clone(),
                                );
                                let _ = conversation_manager.save_message(&msg).await;
                                tracing::info!(
                                    "✅ DeepSeek V4 completed task for session {}",
                                    session_id
                                );
                                return Ok(clean);
                            }
                            Err(ds_err) => {
                                return Err(format!(
                                    "Gemini + DeepSeek both failed. Gemini: {} | DeepSeek: {}",
                                    gemini_err, ds_err
                                ));
                            }
                        }
                    }
                    return Err(format!("Gemini API Error: {}", gemini_err));
                }
            };

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

                                // NEW: Check if this is a video editing tool (not a control tool)
                                if function_name != "start_background_job"
                                    && function_name != "check_job_status"
                                    && function_name != "search_memory"
                                {
                                    // Execute video editing tool directly using tool_executor
                                    send_progress(&format!(
                                        "🎬 Executing {} directly...",
                                        function_name
                                    ));
                                    tracing::info!(
                                        "🎬 Executing video tool directly: {}",
                                        function_name
                                    );

                                    let exec_context =
                                        crate::agent::tool_executor::ToolExecutionContext {
                                            session_id: session_id.to_string(),
                                            user_id: None, // StatefulAgent doesn't have user_id
                                            app_state: app_state.clone(),
                                            workflow_id,
                                        };

                                    let tool_result = crate::agent::tool_executor::execute_tool_gemini_with_context(
                                        &function_name,
                                        &function_call.args,
                                        &exec_context
                                    ).await;

                                    send_progress(&format!("✅ {} completed", function_name));

                                    // Parse the result string as JSON
                                    let result_value =
                                        serde_json::from_str::<serde_json::Value>(&tool_result)
                                            .unwrap_or_else(
                                                |_| serde_json::json!({"result": tool_result}),
                                            );

                                    function_results.push((
                                        function_name.clone(),
                                        result_value,
                                        function_call.thought_signature.clone(),
                                    ));
                                } else if function_name == "start_background_job" {
                                    send_progress("🚀 Starting background video editing job...");
                                    tracing::info!("🚀 Gemini decided to start background job");

                                    let task_description = function_call
                                        .args
                                        .get("task_description")
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
                                    )
                                    .await;

                                    let tool_result = match job_result {
                                        Ok(job_id) => {
                                            send_progress(&format!(
                                                "✅ Background job started: {}",
                                                job_id
                                            ));
                                            tracing::info!("✅ Background job started: {}", job_id);
                                            serde_json::json!({
                                                "success": true,
                                                "job_id": job_id,
                                                "message": format!("Successfully started background video editing job with ID: {}. The job is now processing in the background and will send progress updates.", job_id)
                                            })
                                        }
                                        Err(e) => {
                                            send_progress(&format!(
                                                "❌ Failed to start job: {}",
                                                e
                                            ));
                                            serde_json::json!({
                                                "success": false,
                                                "error": format!("Failed to start background job: {}", e)
                                            })
                                        }
                                    };

                                    function_results.push((
                                        function_name.clone(),
                                        tool_result,
                                        function_call.thought_signature.clone(),
                                    ));
                                } else if function_name == "check_job_status" {
                                    send_progress("📊 Checking job status...");
                                    let job_id = function_call
                                        .args
                                        .get("job_id")
                                        .and_then(|v| v.as_str())
                                        .filter(|s| !s.trim().is_empty());

                                    let tool_result = if let Some(jid) = job_id {
                                        // Check specific job with enhanced details
                                        if let Some(job) = job_manager.get_job(jid).await {
                                            let elapsed = if let Some(started) = job.started_at {
                                                let duration = chrono::Utc::now()
                                                    .signed_duration_since(started);
                                                format!(
                                                    "{}m {}s",
                                                    duration.num_minutes(),
                                                    duration.num_seconds() % 60
                                                )
                                            } else {
                                                "Not started yet".to_string()
                                            };

                                            let status_detail = match &job.status {
                                                crate::jobs::JobStatus::Running {
                                                    current_step,
                                                    progress_percent,
                                                    steps_completed,
                                                    total_steps,
                                                    completed_actions,
                                                    current_action_detail,
                                                } => {
                                                    let mut status_str = format!("RUNNING\n  Current Step: {}\n  Steps: {}/{}", current_step, steps_completed, total_steps);

                                                    if let Some(pct) = progress_percent {
                                                        status_str.push_str(&format!(
                                                            "\n  Progress: {:.1}%",
                                                            pct
                                                        ));
                                                    }

                                                    if let Some(actions) = completed_actions {
                                                        if !actions.is_empty() {
                                                            status_str.push_str(&format!(
                                                                "\n\n  Completed Actions:\n"
                                                            ));
                                                            for action in actions {
                                                                status_str.push_str(&format!(
                                                                    "    ✅ {}\n",
                                                                    action
                                                                ));
                                                            }
                                                        }
                                                    }

                                                    if let Some(detail) = current_action_detail {
                                                        status_str.push_str(&format!(
                                                            "\n  Detail: {}",
                                                            detail
                                                        ));
                                                    }

                                                    status_str
                                                }
                                                crate::jobs::JobStatus::Completed {
                                                    result,
                                                    output_files,
                                                    duration_seconds,
                                                } => {
                                                    format!(
                                                        "COMPLETED\n  Duration: {:.1}s\n  Files: {}\n  Result: {}",
                                                        duration_seconds, output_files.len(), result
                                                    )
                                                }
                                                crate::jobs::JobStatus::Failed {
                                                    error,
                                                    failed_at_step,
                                                } => {
                                                    format!(
                                                        "FAILED\n  Failed at: {}\n  Error: {}",
                                                        failed_at_step, error
                                                    )
                                                }
                                                crate::jobs::JobStatus::Queued { position } => {
                                                    format!("QUEUED (position: {})", position)
                                                }
                                                _ => format!("{:?}", job.status),
                                            };

                                            // Check if job appears stuck
                                            let stuck_warning = if job.is_possibly_stuck() {
                                                "\n\n⚠️ WARNING: This job hasn't reported progress in over 10 minutes. It may be stuck.\n\
                                                Possible causes:\n\
                                                - FFmpeg process hung due to system sleep/wake\n\
                                                - Network download timeout\n\
                                                - External process not responding\n\n\
                                                Consider canceling and restarting the job."
                                            } else {
                                                ""
                                            };

                                            let report = format!(
                                                "📊 JOB STATUS REPORT\n\n\
                                                Job ID: {}\n\
                                                Status: {}\n\
                                                Time Elapsed: {}\n\
                                                Created: {}{}",
                                                jid,
                                                status_detail,
                                                elapsed,
                                                job.created_at.format("%H:%M:%S"),
                                                stuck_warning
                                            );
                                            serde_json::json!({ "report": report })
                                        } else {
                                            serde_json::json!({
                                                "error": format!("❌ Job {} not found. It may have completed and been cleaned up, or the ID is incorrect.", jid)
                                            })
                                        }
                                    } else {
                                        // Get all jobs for this session
                                        let session_jobs =
                                            job_manager.get_session_jobs(session_id).await;
                                        let jobs_data: Vec<_> = session_jobs
                                            .iter()
                                            .map(|job| {
                                                serde_json::json!({
                                                    "job_id": job.id,
                                                    "status": format!("{:?}", job.status),
                                                    "created_at": job.created_at.to_rfc3339()
                                                })
                                            })
                                            .collect();

                                        serde_json::json!({
                                            "jobs": jobs_data,
                                            "total_count": jobs_data.len()
                                        })
                                    };

                                    function_results.push((
                                        function_name.clone(),
                                        tool_result,
                                        function_call.thought_signature.clone(),
                                    ));
                                } else if function_name == "search_memory" {
                                    send_progress("🔍 Searching memory for relevant context...");
                                    let query = function_call
                                        .args
                                        .get("query")
                                        .and_then(|v| v.as_str())
                                        .unwrap_or("");

                                    let tool_result = if let Some(ref qdrant_client) =
                                        app_state.qdrant_client
                                    {
                                        match qdrant_client
                                            .build_context_for_query(
                                                query,
                                                session_id,
                                                app_state.voyage_embeddings.as_ref(),
                                                app_state.video_gemini_client.as_ref().or(app_state.gemini_client.as_ref()),
                                            )
                                            .await
                                        {
                                            Ok(Some(context)) => {
                                                serde_json::json!({
                                                    "found": true,
                                                    "context": context
                                                })
                                            }
                                            Ok(None) => {
                                                serde_json::json!({
                                                    "error": "Memory search unavailable - no embedding client"
                                                })
                                            }
                                            Err(e) => serde_json::json!({
                                                "error": format!("Error searching memory: {}", e)
                                            }),
                                        }
                                    } else {
                                        serde_json::json!({
                                            "error": "Memory search unavailable - Qdrant not configured"
                                        })
                                    };

                                    function_results.push((
                                        function_name.clone(),
                                        tool_result,
                                        function_call.thought_signature.clone(),
                                    ));
                                }
                            }
                            _ => {}
                        }
                    }
                } else {
                    // Content was blocked or missing
                    if let Some(block_reason) = response
                        .prompt_feedback
                        .as_ref()
                        .and_then(|f| f.block_reason.as_ref())
                    {
                        tracing::warn!("Gemini content blocked: {}", block_reason);
                        final_response = format!(
                            "I cannot process this request due to content safety filters: {}",
                            block_reason
                        );
                    } else if let Some(finish_reason) = &candidate.finish_reason {
                        tracing::warn!("Gemini response finished with reason: {}", finish_reason);
                        final_response =
                            format!("Response could not be generated: {}", finish_reason);
                    } else {
                        tracing::warn!("Gemini response has no content");
                        final_response =
                            "I apologize, but I couldn't generate a response for that request."
                                .to_string();
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
            let function_response_parts: Vec<_> = function_results
                .iter()
                .map(|(name, result, thought_sig)| {
                    let mut response_map = HashMap::new();
                    response_map.insert("result".to_string(), result.clone());

                    crate::gemini_client::Part::FunctionResponse {
                        function_response: crate::gemini_client::FunctionResponse {
                            name: name.clone(),
                            response: response_map,
                            thought_signature: thought_sig.clone(),
                        },
                    }
                })
                .collect();

            conversation_contents.push(crate::gemini_client::Content {
                parts: function_response_parts,
                role: Some("function".to_string()),
            });

            // Continue loop - AI will process function results and respond naturally
        }

        // Save assistant's final conversational response to history
        if !final_response.is_empty() {
            tracing::info!(
                "💾 Attempting to save assistant response (length: {}) for session {}",
                final_response.len(),
                session_id
            );
            let assistant_msg =
                ConversationMessage::new_assistant(session_id.to_string(), final_response.clone());
            match conversation_manager.save_message(&assistant_msg).await {
                Ok(_) => tracing::info!(
                    "✅ Successfully saved assistant message to DB for session {}",
                    session_id
                ),
                Err(e) => tracing::error!("❌ Failed to save assistant message: {}", e),
            }
        } else {
            tracing::warn!(
                "⚠️ final_response is empty, not saving assistant message for session {}",
                session_id
            );
        }

        Ok(final_response)
    }

    fn create_all_tools(_user_input: &str) -> Vec<crate::gemini_client::FunctionDeclaration> {
        // Start with the 3 control tools
        let all_tools = vec![
            crate::gemini_client::FunctionDeclaration {
                name: "start_background_job".to_string(),
                description: "Start a background video editing job for complex multi-step workflows that benefit from parallel execution. Use this when the user requests complex operations like 'create a full ad from scratch with 5 scenes' or 'process all videos in this folder'. For simple single operations, use the direct tools instead.".to_string(),
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
        ];

        // NOTE: video tools are no longer added here — the caller now uses
        // ai_tool_selector::select_tools_for_request() for intelligent AI-driven
        // selection. This method only returns the 3 control tools.
        all_tools
    }

    /// Returns the 3 agent control tool declarations.
    /// Video tools are selected separately by ai_tool_selector::select_tools_for_request().
    fn create_control_tools_gemini() -> Vec<crate::gemini_client::FunctionDeclaration> {
        // Only the 3 control tools — video tools come from AI-driven selection
        // This is intentionally a subset of create_all_tools()
        let all = Self::create_all_tools("__control_tools_only__");
        // create_all_tools builds the 3 control tools first then extends with video tools.
        // Filter to only the control tool names.
        all.into_iter()
            .filter(|t| {
                matches!(
                    t.name.as_str(),
                    "start_background_job" | "check_job_status" | "search_memory"
                )
            })
            .collect()
    }
}

// ── Gemma 4 / NVIDIA NIM tool calling loop ────────────────────────────────────
//
// Runs the same multi-turn tool loop as the Gemini agent but using NVIDIA NIM's
// OpenAI-compatible API. Gemma 4 has NATIVE function calling (special tokens,
// not prompt engineering), so this is a first-class supported path.
//
// Conversation format: OpenAI messages array (system / user / assistant / tool).
// Tool results are added as { role: "tool", tool_call_id, content } messages.
// Loop exits when `finish_reason` is not "tool_calls".

/// Multi-turn tool loop for DeepSeek V4.
/// Same contract as `run_nim_tool_loop` — feeds tool results back to the model
/// and keeps calling until a text answer is returned (or MAX_TURNS exhausted).
async fn run_deepseek_tool_loop<F>(
    ds_client: &crate::deepseek_client::DeepSeekClient,
    messages: &mut Vec<serde_json::Value>,
    tools: &[crate::gemini_client::FunctionDeclaration],
    exec_context: &crate::agent::tool_executor::ToolExecutionContext,
    send_progress: &F,
) -> Result<String, String>
where
    F: Fn(&str),
{
    const MAX_TURNS: usize = 10;

    for turn in 0..MAX_TURNS {
        let response = ds_client
            .generate_single(messages, tools)
            .await
            .map_err(|e| format!("DeepSeek API error: {}", e))?;

        match response {
            crate::deepseek_client::DeepSeekResponse::Text(text) => {
                tracing::info!("✅ DeepSeek V4 final answer after {} turns", turn + 1);
                return Ok(text);
            }

            crate::deepseek_client::DeepSeekResponse::ToolCalls(tool_calls) => {
                let assistant_tool_calls: Vec<serde_json::Value> = tool_calls
                    .iter()
                    .map(|tc| {
                        serde_json::json!({
                            "id": tc.id,
                            "type": "function",
                            "function": {
                                "name": tc.name,
                                "arguments": tc.arguments.to_string(),
                            }
                        })
                    })
                    .collect();

                messages.push(serde_json::json!({
                    "role": "assistant",
                    "content": null,
                    "tool_calls": assistant_tool_calls,
                }));

                for tc in &tool_calls {
                    send_progress(&format!("🔧 DeepSeek calling: {}", tc.name));
                    tracing::info!("🎬 DeepSeek V4 tool call: {}", tc.name);

                    let args_map: std::collections::HashMap<String, serde_json::Value> = tc
                        .arguments
                        .as_object()
                        .map(|o| o.iter().map(|(k, v)| (k.clone(), v.clone())).collect())
                        .unwrap_or_default();

                    let result = crate::agent::tool_executor::execute_tool_gemini_with_context(
                        &tc.name,
                        &args_map,
                        exec_context,
                    )
                    .await;

                    send_progress(&format!("✅ {} done", tc.name));

                    messages.push(serde_json::json!({
                        "role": "tool",
                        "tool_call_id": tc.id,
                        "content": result,
                    }));
                }
            }
        }
    }

    Err(format!("DeepSeek V4 exceeded max turns ({})", MAX_TURNS))
}
