// Stateful agent that manages conversations and decides when to spawn background jobs
// The AI has a special tool to start background jobs for complex video editing tasks

use crate::agent::conversation_manager::{ConversationManager, ConversationMessage};
use crate::claude_client::{
    ClaudeClient, ClaudeContent, ClaudeMessage, ClaudeTool, InputSchema, PropertyDefinition,
};

use crate::AppState;
use serde_json::json;
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
        _user_id: Option<i32>,
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
- Use the broader creative stack when helpful: clipping, education, landing pages, explainers, animated infographics, algorithm visualizations, investor pitches, and delivery pages.
- When a job is running, you remain available for conversation and can check its status
- Only start new jobs for new work requests, not for status inquiries about existing work
- If the user's task is clearly defined, call `set_chat_title` early with a concise descriptive title
- If tools created output files, finish with `submit_final_answer` so the user gets delivery/output links instead of internal file paths

## CRITICAL: Only Use Declared Tools
You MUST only call tools that are explicitly listed in the `tools` array of this request. Do NOT call tools like `imagen`, `imagen_generate`, `remove_background`, `expand_image`, `search_web`, `google_search`, `web_search`, `read_website`, `extract_content`, or `fetch_url` — these tools do NOT exist in this system. If a tool name isn't in the catalog, don't guess — pick the closest declared tool instead.

IMPORTANT: For fetching website content, use `browserbase_fetch_url(url)` — it renders JavaScript, solves CAPTCHAs, and returns clean markdown. Use `read_website_content(url)` as a fallback."#;

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
                            send_progress("🚀 Background job tool called — handling inline...");
                            tracing::info!("🚀 AI called start_background_job — handling inline");

                            let task_description = input
                                .get("task_description")
                                .and_then(|v| v.as_str())
                                .unwrap_or(user_input);

                            let tool_result = format!("The background job is being handled within the current agent session. Task: {}. Background job dispatch was removed — the agent handles all tasks inline.", task_description);

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
    bedrock_client: Option<Arc<crate::bedrock_client::BedrockClient>>,
    nvidia_nim_client: Option<Arc<crate::nvidia_nim_client::NvidiaNimClient>>,
    ollama_client: Option<Arc<crate::ollama_client::OllamaClient>>,
}

impl StatefulGeminiAgent {
    pub fn new(client: Arc<crate::gemini_client::GeminiClient>) -> Self {
        Self {
            client,
            bedrock_client: None,
            nvidia_nim_client: None,
            ollama_client: None,
        }
    }

    pub fn new_with_nvidia(
        client: Arc<crate::gemini_client::GeminiClient>,
        bedrock_client: Option<Arc<crate::bedrock_client::BedrockClient>>,
        nvidia_nim_client: Option<Arc<crate::nvidia_nim_client::NvidiaNimClient>>,
        ollama_client: Option<Arc<crate::ollama_client::OllamaClient>>,
    ) -> Self {
        Self {
            client,
            bedrock_client,
            nvidia_nim_client,
            ollama_client,
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
        workflow_id: Option<uuid::Uuid>,
        mut user_message_rx: Option<tokio::sync::mpsc::UnboundedReceiver<String>>,
        user_id: Option<i32>,
    ) -> Result<String, String> {
        // Helper to send progress updates
        let send_progress = |msg: &str| {
            if let Some(ref tx) = progress_tx {
                let _ = tx.send(msg.to_string());
            }
            tracing::info!("{}", msg);
        };

        let mut last_tool_result_with_output: Option<String> = None;

        send_progress("🧠 Loading the full production toolbelt for your request...");
        // Full-toolbelt mode: agents receive the full allowed video tool catalog by
        // default and choose the right tools themselves at runtime.
        let selected_video_tools = crate::ai_tool_selector::select_tools_for_request(
            user_input,
            app_state.ollama_client.as_ref(),
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

## ⚠️ CRITICAL TOOL RESTRICTION: NEVER use generate_long_form_video
The tool `generate_long_form_video` exists in your catalog but is a DELEGATION wrapper that starts a new agent — it WILL produce the wrong output. When a DFY service brief tells you to create a specific type of video (landing page, education clip, manim explainer, whiteboard animation, kinetic typography, animated infographic, algorithm viz, investor pitch, year in review, isometric explainer, etc.), you MUST call the actual rendering tools directly yourself: blender_generate_scene_type for 3D/visual content, manim_execute_script for diagrams/animations, and the editing tools for clip processing. NEVER delegate to generate_long_form_video.

## Your Full Tool Catalog
You have access to ALL tools defined in the `tools` array. Use them to edit, generate, and produce professional video content of any length.
- **Media generation**: text-to-image, text-to-video, text-to-audio, thumbnail creation, script writing
- **Video editing**: trim, merge, split, crop, rotate, resize, stabilize, speed change, transitions
- **Visual effects**: text overlays, filters, color adjustment, picture-in-picture, chroma key, split screen, subtitles
- **Audio**: add, extract, adjust volume, fade, mix, text-to-speech, sound effects, music generation
- **Stock media**: Pexels search and download for photos and videos
- **3D & animation**: BlenderMCP scenes (Blender 2D/3D), procedural animations, 3D models, product scenes, lower thirds, data visuals, cinematic loops
- **Educational/technical**: Manim animations (math/STEM explanations), LaTeX diagram rendering, formula visualization
- **Voice & narration**: VibeVoice narration with multiple voice profiles, audio cleanup, audio visualization, summaries
- **UI/product mockups**: UI mockup scenes, browser mockups, device frames, product demos
- **Long-form assembly**: multi-segment video assembly, chaptered content, complex pipelines
- **Analysis**: video inspection, frame extraction, quality review, duration check
- **Session**: chat title, background jobs, final answer submission

**You must scan this full catalog and intelligently choose which tools to call.** Do not limit yourself to the tools mentioned in examples below — those are just illustrations of the loop pattern.

## Tool-Loop Workflow

You operate in a loop: call a tool → read the result → decide the next tool → call it → repeat. Do NOT try to do everything in one tool call. Work one step at a time:

1. READ the request
2. Scan the available tools for one that can take the next step
3. Call it with the appropriate parameters
4. READ the result
5. Decide what's needed next based on what was returned and call another tool
6. Repeat until the deliverable is complete
7. Call `submit_final_answer` with all output file paths

### Example loop pattern (tool names are placeholders — choose the right tools for YOUR task):
- Step 1: Call a generation tool that works from text → get an output file
- Step 2: Call a search/download tool to get supporting media → get more files
- Step 3: Call an editing/effects tool using the generated files as inputs → get a combined result
- Step 4: Call an audio/narration tool if the deliverable needs voiceover → get an audio file
- Step 5: Call a final assembly tool to merge everything → get the finished file(s)
- Step 6: Call submit_final_answer with all output paths

Every step depends on the previous result. You decide the sequence dynamically based on what each tool returns. The right tool choice depends on the specific request:
- blender_generate_scene_type — for ALL 3D Blender content: scenes, thumbnails, title cards, UI mockups, lower thirds, logos, abstract backgrounds, particle effects, product mockups
- manim_execute_script — for ALL Manim/animated diagram content: math/STEM explanations, data visualizations, LaTeX equations, code animations, timelines, network graphs, geometry proofs, vector fields, matrix transforms
- Both can be used in one video — render clips separately, then merge with merge_videos
- For a math explainer you might use Manim tools; for a 3D product showcase you might use Blender tools; for a voiceover you might use VibeVoice tools. Read each tool's description carefully.

## Bootstrapping When No Source Files Exist

If the user has not uploaded any files, start by calling tools that work from text alone. Many tools in your catalog accept no input-file parameters — read their descriptions to find them. Common patterns:

- **Text-to-image tools**: generate images from descriptions (thumbnails, graphics, title cards, backgrounds)
- **Text-to-video tools**: orchestrate full videos from a topic
- **Text-to-audio tools**: generate narration, music, sound effects
- **Search tools**: find stock footage/photos by keyword
- **Script tools**: write video scripts from a topic

For tools that DO require input files (trim_video, merge_videos, add_text_overlay, etc.), first produce the inputs using bootstrap tools, then pass those output paths as inputs.

## How to Talk
- Answer the user's question directly. Don't restate the task or what you're doing.
- No preambles. No "I'm already on it!"
- Be brief. Be natural. Sound human.
- After you finish generating, call submit_final_answer with the summary and output file paths.

## Reference: Tool Categories (not exhaustive — read the actual tool definitions in the request)
Your tool catalog includes: image generation, video generation, audio generation, video editing, visual effects, audio editing, stock media search/download, BlenderMCP 3D scenes, Manim animations, LaTeX rendering, VibeVoice narration, UI mockups, long-form assembly, script writing, video analysis, frame extraction, session management. Read each tool's description and parameters before calling it.

## Important Principles
- You work in a tool loop: call → read result → decide next → call again. Do NOT try to plan everything upfront.
- Direct tool execution is FASTER than background jobs — use tools directly, call them one at a time
- Use start_background_job only for multi-step workflows spanning 5+ operations
- Call `set_chat_title` early with a concise descriptive title
- When you create output files, end with `submit_final_answer` and include every generated output file path
- Do NOT stop at a plan — actually generate the deliverable the user asked for

## CRITICAL: Only Use Declared Tools
You MUST only call tools that are explicitly listed in the `tools` array of this request. Do NOT call tools like `imagen`, `imagen_generate`, `remove_background`, `expand_image`, `search_web`, `google_search`, `web_search`, `read_website`, `extract_content`, or `fetch_url` — these tools do NOT exist in this system. If a tool name isn't in the catalog, don't guess — pick the closest declared tool instead.

IMPORTANT: For fetching website content, use `browserbase_fetch_url(url)` — it renders JavaScript, solves CAPTCHAs, and returns clean markdown. Use `read_website_content(url)` as a fallback."#;

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
            if is_first_call {
                send_progress("🤖 Processing your message...");
                is_first_call = false;
            }

            // 🆕 INTERACTIVE: Check for user follow-up messages between tool calls
            if let Some(ref mut rx) = user_message_rx {
                while let Ok(followup) = rx.try_recv() {
                    // 🛑 CANCELLATION: If the user sends __CANCEL__, stop immediately
                    if followup == "__CANCEL__" {
                        tracing::info!("🛑 Agent cancelled by user: session={}", session_id);
                        send_progress("🛑 Agent cancelled by user.");
                        final_response = "Cancelled by user.".to_string();
                        return Ok(final_response);
                    }

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

            let request = crate::gemini_client::GenerateContentRequest {
                contents: conversation_contents.clone(),
                tools: Some(vec![crate::gemini_client::Tool {
                    function_declarations: all_tools.clone(),
                }]),
                generation_config: Some(crate::gemini_client::GenerationConfig {
                    temperature: 0.5,
                    top_k: 40,
                    top_p: 0.9,
                    max_output_tokens: 8192,
                }),
                tool_config: Some(crate::gemini_client::ToolConfig {
                    function_calling_config: crate::gemini_client::FunctionCallingConfig {
                        mode: crate::gemini_client::FunctionCallingMode::Auto,
                    },
                }),
                system_instruction: Some(system_instruction_content.clone()),
            };

            // ── Provider Fallback Chain ──
            // Order: NVIDIA NIM (fastest, GPU) → AWS Bedrock → Ollama (self-hosted) → Gemini → DeepSeek

            // Helper to build OpenAI-format messages for OpenAI-compatible APIs (NVIDIA, Ollama, DeepSeek)
            let build_messages = |sys_inst: &str| -> Vec<serde_json::Value> {
                let mut msgs: Vec<serde_json::Value> = vec![
                    serde_json::json!({"role": "system", "content": sys_inst})
                ];
                for msg in &conversation_history {
                    let role = match msg.role {
                        crate::agent::conversation_manager::MessageRole::Human => "user",
                        crate::agent::conversation_manager::MessageRole::Assistant => "assistant",
                        _ => continue,
                    };
                    msgs.push(serde_json::json!({"role": role, "content": msg.content}));
                }
                let current_msg = if !context.is_empty() {
                    context.to_string()
                } else {
                    user_input.to_string()
                };
                msgs.push(serde_json::json!({"role": "user", "content": current_msg}));
                msgs
            };

            // 1. Try NVIDIA NIM (GPU, fastest path)
            if let Some(ref nim) = self.nvidia_nim_client {
                let mut nim_messages = build_messages(system_instruction);
                let exec_context = crate::agent::tool_executor::ToolExecutionContext {
                    session_id: session_id.to_string(),
                    user_id,
                    app_state: app_state.clone(),
                    workflow_id,
                };
                match run_nvidia_tool_loop(
                    nim,
                    &mut nim_messages,
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
                        tracing::info!("✅ NVIDIA NIM completed task for session {}", session_id);
                        return Ok(clean);
                    }
                    Err(nim_err) => {
                        tracing::warn!("⚠️ NVIDIA NIM failed, trying Bedrock: {}", nim_err);
                    }
                }
            }

            // 2. Try AWS Bedrock
            if let Some(ref bedrock) = self.bedrock_client {
                let mut bd_messages: Vec<aws_sdk_bedrockruntime::types::Message> = Vec::new();
                for msg in &conversation_history {
                    let role = match msg.role {
                        crate::agent::conversation_manager::MessageRole::Human => aws_sdk_bedrockruntime::types::ConversationRole::User,
                        crate::agent::conversation_manager::MessageRole::Assistant => aws_sdk_bedrockruntime::types::ConversationRole::Assistant,
                        _ => continue,
                    };
                    if let Ok(m) = bedrock_text_message(role, &msg.content) {
                        bd_messages.push(m);
                    }
                }
                let current_msg = if !context.is_empty() {
                    context.to_string()
                } else {
                    user_input.to_string()
                };
                if let Ok(m) = bedrock_text_message(
                    aws_sdk_bedrockruntime::types::ConversationRole::User,
                    &format!("{}\n\n{}", system_instruction, current_msg),
                ) {
                    bd_messages.push(m);
                }

                let exec_context = crate::agent::tool_executor::ToolExecutionContext {
                    session_id: session_id.to_string(),
                    user_id,
                    app_state: app_state.clone(),
                    workflow_id,
                };
                match run_bedrock_tool_loop(
                    bedrock,
                    &mut bd_messages,
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
                        tracing::info!("✅ Bedrock completed task for session {}", session_id);
                        return Ok(clean);
                    }
                    Err(bedrock_err) => {
                        tracing::warn!("⚠️ Bedrock failed, trying Ollama: {}", bedrock_err);
                    }
                }
            }

            // 3. Try Ollama (self-hosted, free)
            let ollama_client = self.ollama_client.as_ref()
                .map(|arc| arc.as_ref())
                .or_else(|| app_state.ollama_client.as_ref());
            let response = match ollama_client {
                Some(ollama) => {
                    let mut oa_messages = build_messages(system_instruction);

                    send_progress("🤖 Processing your request...");

                    let exec_context = crate::agent::tool_executor::ToolExecutionContext {
                        session_id: session_id.to_string(),
                        user_id,
                        app_state: app_state.clone(),
                        workflow_id,
                    };

                    match run_ollama_tool_loop(
                        ollama,
                        &mut oa_messages,
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
                                "✅ Ollama completed task for session {}",
                                session_id
                            );
                            return Ok(clean);
                        }
                        Err(ollama_err) => {
                            tracing::warn!("⚠️ Ollama failed, trying Gemini: {}", ollama_err);
                            self.client.generate_content(request).await
                        }
                    }
                }
                None => self.client.generate_content(request).await,
            };

            // 4. Gemini — if all prior providers failed
            // 5. DeepSeek (last resort) — if Gemini also fails
            let response = match response {
                Ok(r) => r,
                Err(gemini_err) => {
                    tracing::warn!("⚠️ Gemini failed, trying DeepSeek fallback: {}", gemini_err);
                    let mut ds_messages = build_messages(system_instruction);

                    if let Some(ref ds_client) = app_state.deepseek_client {
                        send_progress("🤖 Processing your request...");

                        let exec_context = crate::agent::tool_executor::ToolExecutionContext {
                            session_id: session_id.to_string(),
                            user_id,
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
                                    && function_name != "list_campaigns"
                                    && function_name != "get_campaign_status"
                                    && function_name != "control_campaign"
                                    && function_name != "search_campaign_knowledge"
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
                                            user_id,
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

                                    if function_name == "submit_final_answer"
                                        || function_name == "generate_image"
                                        || function_name == "blender_generate_scene_type"
                                        || function_name == "manim_execute_script"
                                        || function_name == "create_thumbnail"
                                        || function_name == "create_thumbnail_hd"
                                        || function_name == "merge_videos"
                                    {
                                        last_tool_result_with_output =
                                            Some(tool_result.clone());
                                    }

                                    function_results.push((
                                        function_name.clone(),
                                        result_value,
                                        function_call.thought_signature.clone(),
                                    ));
                                } else if function_name == "start_background_job" {
                                    send_progress("🚀 Background job tool called — handling inline...");
                                    tracing::info!("🚀 Gemini called start_background_job — handling inline");

                                    let task_description = function_call
                                        .args
                                        .get("task_description")
                                        .and_then(|v| v.as_str())
                                        .unwrap_or(user_input);

                                    let tool_result = json!({
                                        "result": format!("The background job is being handled within the current agent session. Task: {}", task_description),
                                        "note": "Background job dispatch was removed. The agent handles all tasks inline."
                                    });

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
                                } else if function_name == "list_campaigns" {
                                    send_progress("📋 Listing your campaigns...");
                                    let user_id_i32 = user_id.unwrap_or(0);
                                    let rows = sqlx::query_as::<_, (uuid::Uuid, String, String, String, i32, i32, i32)>(
                                        "SELECT id, name, service_type, status, posts_per_day, \
                                                total_posts_planned, total_posts_published \
                                         FROM campaigns WHERE user_id = $1 ORDER BY created_at DESC LIMIT 50",
                                    )
                                    .bind(user_id_i32)
                                    .fetch_all(&app_state.db_pool)
                                    .await
                                    .unwrap_or_default();

                                    let campaigns_list: Vec<serde_json::Value> = rows.into_iter().map(|(cid, cname, stype, cstatus, ppd, planned, published)| {
                                        serde_json::json!({
                                            "id": cid.to_string(),
                                            "name": cname,
                                            "service_type": stype,
                                            "status": cstatus,
                                            "posts_per_day": ppd,
                                            "total_posts_planned": planned,
                                            "total_posts_published": published,
                                        })
                                    }).collect();

                                    let tool_result = serde_json::json!({
                                        "campaigns": campaigns_list,
                                        "total": campaigns_list.len(),
                                        "note": "You can ask about a specific campaign by name or ID for more details."
                                    });

                                    function_results.push((
                                        function_name.clone(),
                                        tool_result,
                                        function_call.thought_signature.clone(),
                                    ));
                                } else if function_name == "get_campaign_status" {
                                    send_progress("📊 Fetching campaign details...");
                                    let user_id_i32 = user_id.unwrap_or(0);
                                    let campaign_id = function_call.args.get("campaign_id").and_then(|v| v.as_str());
                                    let campaign_name = function_call.args.get("name").and_then(|v| v.as_str());

                                    let tool_result = if let Some(cid_str) = campaign_id {
                                        if let Ok(cid) = uuid::Uuid::parse_str(cid_str) {
                                            let row = sqlx::query_as::<_, (uuid::Uuid, String, String, String, String, f64, i32, i32, i32, chrono::DateTime<chrono::Utc>, chrono::DateTime<chrono::Utc>, String)>(
                                                "SELECT id, name, service_type, brief, style, duration, posts_per_day, \
                                                        total_posts_planned, total_posts_published, start_date, end_date, status \
                                                 FROM campaigns WHERE id = $1 AND user_id = $2",
                                            )
                                            .bind(cid)
                                            .bind(user_id_i32)
                                            .fetch_optional(&app_state.db_pool)
                                            .await;

                                            match row {
                                                Ok(Some((cid2, cname, stype, brief, style, duration, ppd, planned, published, start, end, cstatus))) => {
                                                    let recent_posts = sqlx::query_as::<_, (String, String, Option<String>)>(
                                                        "SELECT scheduled_at::text, status, media_r2_url \
                                                         FROM campaign_posts WHERE campaign_id = $1 \
                                                         ORDER BY scheduled_at DESC LIMIT 10",
                                                    )
                                                    .bind(cid2)
                                                    .fetch_all(&app_state.db_pool)
                                                    .await
                                                    .unwrap_or_default();

                                                    let posts_json: Vec<serde_json::Value> = recent_posts.into_iter().map(|(sa, ps, url)| {
                                                        serde_json::json!({
                                                            "scheduled_at": sa,
                                                            "status": ps,
                                                            "has_media": url.is_some(),
                                                        })
                                                    }).collect();

                                                    serde_json::json!({
                                                        "found": true,
                                                        "campaign": {
                                                            "id": cid2.to_string(),
                                                            "name": cname,
                                                            "service_type": stype,
                                                            "brief": brief,
                                                            "style": style,
                                                            "duration_seconds": duration,
                                                            "posts_per_day": ppd,
                                                            "total_planned": planned,
                                                            "total_published": published,
                                                            "start_date": start.to_rfc3339(),
                                                            "end_date": end.to_rfc3339(),
                                                            "status": cstatus,
                                                        },
                                                        "recent_posts": posts_json,
                                                    })
                                                }
                                                Ok(None) => serde_json::json!({"found": false, "error": "Campaign not found or access denied."}),
                                                Err(e) => serde_json::json!({"error": format!("Database error: {e}")}),
                                            }
                                        } else {
                                            serde_json::json!({"error": "Invalid campaign ID format."})
                                        }
                                    } else if let Some(cname) = campaign_name {
                                        let row = sqlx::query_as::<_, (uuid::Uuid, String)>(
                                            "SELECT id, name FROM campaigns WHERE user_id = $1 AND name ILIKE $2 LIMIT 1",
                                        )
                                        .bind(user_id_i32)
                                        .bind(format!("%{}%", cname))
                                        .fetch_optional(&app_state.db_pool)
                                        .await;

                                        match row {
                                            Ok(Some((found_id, found_name))) => {
                                                // Recurse with the found ID
                                                // We use the same logic — re-call with the ID found
                                                serde_json::json!({
                                                    "found_campaign": found_name,
                                                    "campaign_id": found_id.to_string(),
                                                    "note": "Use get_campaign_status with this campaign_id to get full details."
                                                })
                                            }
                                            Ok(None) => serde_json::json!({"found": false, "error": format!("No campaign found with name matching '{cname}'.")}),
                                            Err(e) => serde_json::json!({"error": format!("Database error: {e}")}),
                                        }
                                    } else {
                                        serde_json::json!({"error": "Provide either campaign_id or name."})
                                    };

                                    function_results.push((
                                        function_name.clone(),
                                        tool_result,
                                        function_call.thought_signature.clone(),
                                    ));
                                } else if function_name == "control_campaign" {
                                    send_progress("🔄 Controlling campaign...");
                                    let user_id_i32 = user_id.unwrap_or(0);
                                    let campaign_id = function_call.args.get("campaign_id").and_then(|v| v.as_str());
                                    let action = function_call.args.get("action").and_then(|v| v.as_str()).unwrap_or("");

                                    let valid_actions = ["pause", "resume", "cancel"];
                                    if !valid_actions.contains(&action) {
                                        let tool_result = serde_json::json!({
                                            "error": format!("Invalid action '{action}'. Must be one of: pause, resume, cancel.")
                                        });
                                        function_results.push((function_name.clone(), tool_result, function_call.thought_signature.clone()));
                                    } else if let Some(cid_str) = campaign_id {
                                        if let Ok(cid) = uuid::Uuid::parse_str(cid_str) {
                                            let status = match action {
                                                "pause" => "paused",
                                                "resume" => "active",
                                                "cancel" => "cancelled",
                                                _ => "paused",
                                            };
                                            let result = sqlx::query(
                                                "UPDATE campaigns SET status = $1, updated_at = NOW() WHERE id = $2 AND user_id = $3"
                                            )
                                            .bind(status)
                                            .bind(cid)
                                            .bind(user_id_i32)
                                            .execute(&app_state.db_pool)
                                            .await;

                                            let tool_result = match result {
                                                Ok(r) if r.rows_affected() > 0 => serde_json::json!({
                                                    "success": true,
                                                    "campaign_id": cid_str,
                                                    "new_status": status,
                                                    "message": format!("Campaign {}d successfully.", action)
                                                }),
                                                Ok(_) => serde_json::json!({"error": "Campaign not found or access denied."}),
                                                Err(e) => serde_json::json!({"error": format!("Database error: {e}")}),
                                            };
                                            function_results.push((function_name.clone(), tool_result, function_call.thought_signature.clone()));
                                        } else {
                                            let tool_result = serde_json::json!({"error": "Invalid campaign ID format."});
                                            function_results.push((function_name.clone(), tool_result, function_call.thought_signature.clone()));
                                        }
                                    } else {
                                        let tool_result = serde_json::json!({"error": "campaign_id is required."});
                                        function_results.push((function_name.clone(), tool_result, function_call.thought_signature.clone()));
                                    }
                                } else if function_name == "search_campaign_knowledge" {
                                    send_progress("🔍 Searching campaign knowledge...");
                                    let query = function_call
                                        .args
                                        .get("query")
                                        .and_then(|v| v.as_str())
                                        .unwrap_or("");
                                    let service_type_filter = function_call
                                        .args
                                        .get("service_type")
                                        .and_then(|v| v.as_str());
                                    let limit = function_call
                                        .args
                                        .get("limit")
                                        .and_then(|v| v.as_u64())
                                        .unwrap_or(10) as usize;

                                    let tool_result = if let Some(ref qdrant_client) = app_state.qdrant_client {
                                        let gemini = app_state.video_gemini_client.as_ref()
                                            .or(app_state.gemini_client.as_ref());
                                        if let Some(g) = gemini {
                                            // Embed the query
                                            match g.embed_content_with_model(query, "models/gemini-embedding-2", Some(1536)).await {
                                                Ok(embedding) => {
                                                    // Build filter: feature=campaign_post + user_id + optional service_type
                                                    let mut must_conditions = vec![
                                                        serde_json::json!({"key": "feature", "match": {"value": "campaign_post"}}),
                                                    ];
                                                    if let Some(uid) = user_id {
                                                        must_conditions.push(serde_json::json!({"key": "user_id", "match": {"value": uid.to_string()}}));
                                                    }
                                                    if let Some(st) = service_type_filter {
                                                        must_conditions.push(serde_json::json!({"key": "context.service_type", "match": {"value": st}}));
                                                    }
                                                    let filter = serde_json::json!({"must": must_conditions});

                                                    match qdrant_client.search_points(
                                                        &embedding,
                                                        limit,
                                                        Some(&filter),
                                                        qdrant_client::EmbeddingProvider::GeminiEmbedding2,
                                                    ).await {
                                                        Ok(results) => {
                                                            let posts: Vec<serde_json::Value> = results.into_iter().map(|p| {
                                                                serde_json::json!({
                                                                    "campaign": p.get("context.campaign_name").or(p.get("campaign_name")).and_then(|v| v.as_str()),
                                                                    "service_type": p.get("context.service_type").or(p.get("service_type")).and_then(|v| v.as_str()),
                                                                    "variation": p.get("user_message").and_then(|v| v.as_str()),
                                                                    "caption": p.get("agent_response").and_then(|v| v.as_str()),
                                                                    "output_url": p.get("context.output_url").or(p.get("output_url")).and_then(|v| v.as_str()),
                                                                    "score": p.get("score").and_then(|v| v.as_f64()),
                                                                })
                                                            }).collect();
                                                            serde_json::json!({
                                                                "found": true,
                                                                "posts": posts,
                                                                "total": posts.len(),
                                                                "note": "These are past campaign posts matching your query. You can ask follow-up questions about any result."
                                                            })
                                                        }
                                                        Err(e) => serde_json::json!({"error": format!("Search failed: {e}")}),
                                                    }
                                                }
                                                Err(e) => serde_json::json!({"error": format!("Embedding failed: {e}")}),
                                            }
                                        } else {
                                            serde_json::json!({"error": "Campaign knowledge search unavailable - no embedding client"})
                                        }
                                    } else {
                                        serde_json::json!({"error": "Campaign knowledge search unavailable - Qdrant not configured"})
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

        // If the last media-generation tool produced output links but
        // final_response has none, override so response_output_links can find them
        if let Some(tool_result) = &last_tool_result_with_output {
            let has_download = tool_result.contains("/api/outputs/download/");
            let final_has_download = final_response.contains("/api/outputs/download/")
                || final_response.contains("/api/outputs/stream/")
                || final_response.contains("/delivery/");
            tracing::info!(
                "final_response_override: tool_result_len={}, final_response_len={}, tool_has_download={}, final_has_download={}, last_tool_result_first_100={}",
                tool_result.len(),
                final_response.len(),
                has_download,
                final_has_download,
                &tool_result[..std::cmp::min(100, tool_result.len())]
            );
            if !tool_result.is_empty() && (final_response.is_empty() || !final_has_download) {
                final_response = tool_result.clone();
                tracing::info!("✅ Overrode final_response with tool result");
            }
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
        // Start with the 3 control tools + campaign management tools
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
            crate::gemini_client::FunctionDeclaration {
                name: "list_campaigns".to_string(),
                description: "List all content campaigns for the current user. Returns campaign names, status, service type, and post counts. Use this when the user asks 'what campaigns do I have?', 'show me my campaigns', or 'list my content schedules'.".to_string(),
                parameters: crate::gemini_client::Parameters {
                    param_type: "object".to_string(),
                    properties: HashMap::from([]),
                    required: vec![],
                },
            },
            crate::gemini_client::FunctionDeclaration {
                name: "get_campaign_status".to_string(),
                description: "Get detailed status of a specific campaign by its ID or name. Returns schedule, post calendar, recent publish stats, and current status. Use this when the user asks 'how is my campaign doing?', 'show me campaign X', or 'what happened to my education campaign?'.".to_string(),
                parameters: crate::gemini_client::Parameters {
                    param_type: "object".to_string(),
                    properties: HashMap::from([
                        ("campaign_id".to_string(), crate::gemini_client::PropertyDefinition {
                            prop_type: "string".to_string(),
                            description: "The UUID of the campaign to check. If omitted, tries to find by name.".to_string(),
                            items: None,
                        }),
                        ("name".to_string(), crate::gemini_client::PropertyDefinition {
                            prop_type: "string".to_string(),
                            description: "Optional campaign name to search for if no ID provided.".to_string(),
                            items: None,
                        }),
                    ]),
                    required: vec![],
                },
            },
            crate::gemini_client::FunctionDeclaration {
                name: "control_campaign".to_string(),
                description: "Pause, resume, or cancel a campaign. Use this when the user wants to stop, start, or delete a campaign.".to_string(),
                parameters: crate::gemini_client::Parameters {
                    param_type: "object".to_string(),
                    properties: HashMap::from([
                        ("campaign_id".to_string(), crate::gemini_client::PropertyDefinition {
                            prop_type: "string".to_string(),
                            description: "The UUID of the campaign to control.".to_string(),
                            items: None,
                        }),
                        ("action".to_string(), crate::gemini_client::PropertyDefinition {
                            prop_type: "string".to_string(),
                            description: "One of: 'pause', 'resume', 'cancel'.".to_string(),
                            items: None,
                        }),
                    ]),
                    required: vec!["campaign_id".to_string(), "action".to_string()],
                },
            },
            crate::gemini_client::FunctionDeclaration {
                name: "search_campaign_knowledge".to_string(),
                description: "Search across all past campaign posts to find what content performed well, what topics were covered, and what outputs were generated. Use this when the user asks about past campaign results, wants to learn from previous content, or asks 'what worked well' or 'show me similar outputs'.".to_string(),
                parameters: crate::gemini_client::Parameters {
                    param_type: "object".to_string(),
                    properties: HashMap::from([
                        ("query".to_string(), crate::gemini_client::PropertyDefinition {
                            prop_type: "string".to_string(),
                            description: "What to search for in past campaign content (e.g., 'best performing posts', 'education campaign results', 'videos about calculus')".to_string(),
                            items: None,
                        }),
                        ("service_type".to_string(), crate::gemini_client::PropertyDefinition {
                            prop_type: "string".to_string(),
                            description: "Optional: filter by service type (e.g., 'clipping', 'education', 'kick_auto_clipper')".to_string(),
                            items: None,
                        }),
                        ("limit".to_string(), crate::gemini_client::PropertyDefinition {
                            prop_type: "number".to_string(),
                            description: "Maximum number of results to return (default: 10)".to_string(),
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
/// Multi-turn tool loop for Ollama (self-hosted Gemma 4).
/// Same contract as `run_deepseek_tool_loop` — feeds tool results back to the model
/// and keeps calling until a text answer is returned (or MAX_TURNS exhausted).
async fn run_ollama_tool_loop<F>(
    ollama_client: &crate::ollama_client::OllamaClient,
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
        let response = ollama_client
            .generate_single(messages, tools)
            .await
            .map_err(|e| format!("Ollama API error: {}", e))?;

        match response {
            crate::ollama_client::OllamaResponse::Text(text) => {
                tracing::info!("✅ Ollama final answer after {} turns", turn + 1);
                return Ok(text);
            }

            crate::ollama_client::OllamaResponse::ToolCalls(tool_calls) => {
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
                    send_progress(&format!("🔧 Ollama calling: {}", tc.name));
                    tracing::info!("🎬 Ollama tool call: {}", tc.name);

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

    Err(format!("Ollama exceeded max turns ({})", MAX_TURNS))
}

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

/// Multi-turn tool loop for NVIDIA NIM (Gemma 4 31B on GPU).
/// OpenAI-compatible message format. Same contract as run_ollama_tool_loop.
async fn run_nvidia_tool_loop<F>(
    nim_client: &crate::nvidia_nim_client::NvidiaNimClient,
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
        let response = nim_client
            .generate_single(messages, tools)
            .await
            .map_err(|e| format!("NVIDIA NIM API error: {}", e))?;

        match response {
            crate::nvidia_nim_client::NimResponse::Text(text) => {
                tracing::info!("✅ NVIDIA NIM final answer after {} turns", turn + 1);
                return Ok(text);
            }

            crate::nvidia_nim_client::NimResponse::ToolCalls(tool_calls) => {
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
                    send_progress(&format!("🔧 NVIDIA NIM calling: {}", tc.name));
                    tracing::info!("🎬 NVIDIA NIM tool call: {}", tc.name);

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

    Err(format!("NVIDIA NIM exceeded max turns ({})", MAX_TURNS))
}

/// Multi-turn tool loop for AWS Bedrock (Meta Llama 4 Maverick 17B).
/// Uses AWS SDK Message types. Same contract as run_ollama_tool_loop.
async fn run_bedrock_tool_loop<F>(
    bedrock_client: &crate::bedrock_client::BedrockClient,
    messages: &mut Vec<aws_sdk_bedrockruntime::types::Message>,
    tools: &[crate::gemini_client::FunctionDeclaration],
    exec_context: &crate::agent::tool_executor::ToolExecutionContext,
    send_progress: &F,
) -> Result<String, String>
where
    F: Fn(&str),
{
    const MAX_TURNS: usize = 10;

    for turn in 0..MAX_TURNS {
        let response = bedrock_client
            .generate_single("", messages, tools)
            .await
            .map_err(|e| format!("Bedrock API error: {}", e))?;

        match response {
            crate::bedrock_client::BedrockResponse::Text(text) => {
                tracing::info!("✅ Bedrock final answer after {} turns", turn + 1);
                return Ok(text);
            }

            crate::bedrock_client::BedrockResponse::ToolCalls(tool_calls) => {
                let mut content_blocks = Vec::new();
                for tc in &tool_calls {
                    content_blocks.push(
                        crate::bedrock_client::tool_call_to_content_block(tc),
                    );
                }
                messages.push(
                    aws_sdk_bedrockruntime::types::Message::builder()
                        .role(aws_sdk_bedrockruntime::types::ConversationRole::Assistant)
                        .set_content(Some(content_blocks))
                        .build()
                        .map_err(|e| format!("Bedrock build error: {e}"))?,
                );

                let mut result_blocks = Vec::new();
                for tc in &tool_calls {
                    send_progress(&format!("🔧 Bedrock calling: {}", tc.name));
                    tracing::info!("🎬 Bedrock tool call: {}", tc.name);

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

                    let result_str = serde_json::to_string(&result).unwrap_or_default();
                    result_blocks.push(
                        crate::bedrock_client::tool_result_to_content_block(tc, &result_str),
                    );
                }
                messages.push(
                    aws_sdk_bedrockruntime::types::Message::builder()
                        .role(aws_sdk_bedrockruntime::types::ConversationRole::User)
                        .set_content(Some(result_blocks))
                        .build()
                        .map_err(|e| format!("Bedrock build error: {e}"))?,
                );
            }
        }
    }

    Err(format!("Bedrock exceeded max turns ({})", MAX_TURNS))
}

fn bedrock_text_message(
    role: aws_sdk_bedrockruntime::types::ConversationRole,
    text: &str,
) -> Result<aws_sdk_bedrockruntime::types::Message, String> {
    aws_sdk_bedrockruntime::types::Message::builder()
        .role(role)
        .content(aws_sdk_bedrockruntime::types::ContentBlock::Text(text.to_string()))
        .build()
        .map_err(|e| format!("Bedrock build message error: {e}"))
}
