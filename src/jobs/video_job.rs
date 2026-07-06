// src/jobs/video_job.rs
//! Video editing job executor - runs AI agents in background with progress updates
//! Now with LangGraph-style ReAct pattern: Thought → Action → Observation → Reflection

use super::{Job, JobControl, JobId, JobManager, JobStatus, ProgressUpdate};
use crate::agent::conversation_manager::{ConversationManager, ConversationMessage};
use crate::agent::stateful_agent::StatefulGeminiAgent;
use crate::AppState;
use serde_json::json;
use std::sync::atomic::{AtomicUsize, Ordering};
use std::sync::{Arc, Mutex};
use tokio::sync::mpsc;
use tokio::time::Duration;

/// Type of AI model to use for the job
#[derive(Debug, Clone)]
pub enum AgentType {
    Claude,
}

/// Video editing job that runs in background
pub struct VideoEditingJob {
    job: Job,
    agent_type: AgentType,
    app_state: Arc<AppState>,
    job_manager: Arc<JobManager>,
}

impl VideoEditingJob {
    fn request_expects_generated_artifact(job_type: &str, raw_input: &str, augmented_input: &str) -> bool {
        let combined = format!("{} {}", raw_input, augmented_input).to_lowercase();

        let generation_intent = [
            "create",
            "generate",
            "render",
            "produce",
            "make",
            "build",
            "edit",
        ]
        .iter()
        .any(|needle| combined.contains(needle));

        let media_intent = [
            "video",
            "thumbnail",
            "clip",
            "scene",
            "animation",
            "advert",
            "ad ",
            "demo",
            "landing page",
            "hero video",
            "narration",
            "youtube",
        ]
        .iter()
        .any(|needle| combined.contains(needle));

        job_type == "video_editing" || (generation_intent && media_intent)
    }

    fn extract_output_links(response: &str) -> Vec<String> {
        response
            .lines()
            .flat_map(|line| {
                ["/api/outputs/stream/", "/api/outputs/download/", "/delivery/"]
                    .into_iter()
                    .filter_map(move |needle| line.find(needle).map(|idx| line[idx..].trim().to_string()))
            })
            .collect::<std::collections::BTreeSet<_>>()
            .into_iter()
            .collect()
    }

    pub fn new(
        job: Job,
        agent_type: AgentType,
        app_state: Arc<AppState>,
        job_manager: Arc<JobManager>,
    ) -> Self {
        Self {
            job,
            agent_type,
            app_state,
            job_manager,
        }
    }

    /// Execute the video editing job in background
    pub async fn execute(self) -> Result<String, String> {
        let job_id = self.job.id.clone();
        let session_id = self.job.session_id.clone();

        tracing::info!(
            "🎬 Starting video editing job: {} (agent: {:?})",
            job_id,
            self.agent_type
        );

        // Initialize ConversationManager for history persistence
        let conversation_manager = ConversationManager::new(self.app_state.db_pool.clone());

        // Ensure schema exists
        if let Err(e) = conversation_manager.initialize_schema().await {
            tracing::warn!("Failed to initialize conversation schema: {}", e);
        }

        // Create control channel for this job
        let (control_tx, mut control_rx) = mpsc::unbounded_channel::<JobControl>();
        self.job_manager
            .register_control_channel(job_id.clone(), control_tx)
            .await;

        // Extract inputs from job data
        let raw_input = self
            .job
            .input_data
            .get("raw_input")
            .and_then(|v| v.as_str())
            .ok_or("Missing raw_input in job data")?
            .to_string();

        let augmented_input = self
            .job
            .input_data
            .get("augmented_input")
            .and_then(|v| v.as_str())
            .unwrap_or(&raw_input) // Fallback to raw input if augmented not found
            .to_string();

        let artifact_expected = Self::request_expects_generated_artifact(
            &self.job.job_type,
            &raw_input,
            &augmented_input,
        );

        let workflow_runtime = crate::services::WorkflowRuntime::new(self.app_state.db_pool.clone());
        let workflow_id = workflow_runtime
            .create_workflow(crate::services::NewWorkflow {
                idempotency_key: Some(format!("background-video-job:{job_id}")),
                workflow_type: "background_video_editing_job".to_string(),
                status: crate::services::WorkflowStatus::Planning,
                session_uuid: Some(session_id.clone()),
                user_id: self.job.user_id.as_deref().and_then(|value| value.parse::<i32>().ok()),
                source_table: Some("job_manager".to_string()),
                source_record_id: uuid::Uuid::parse_str(&job_id).ok(),
                request_summary: raw_input.chars().take(200).collect::<String>(),
                current_step: Some("job_initialized".to_string()),
                metadata: json!({
                    "job_id": job_id,
                    "agent_type": "claude",
                    "job_type": self.job.job_type.clone(),
                }),
                artifact_requirements: json!([]),
            })
            .await
            .ok();

        if let Some(workflow_id) = workflow_id {
            let _ = workflow_runtime
                .append_event(
                    workflow_id,
                    "queued",
                    Some("job_initialized"),
                    "Background video workflow created and waiting for the agent runtime to begin.",
                    json!({
                        "job_id": job_id,
                    }),
                )
                .await;
            ensure_video_editing_workflow_plan(
                &workflow_runtime,
                workflow_id,
                &job_id,
                &raw_input,
                artifact_expected,
            )
            .await;
            complete_video_workflow_node(
                &workflow_runtime,
                workflow_id,
                "plan_request",
                json!({
                    "job_id": job_id,
                    "artifact_expected": artifact_expected,
                    "agent_type": "claude",
                }),
                "Video editing request planned and durable workflow nodes initialized.",
            )
            .await;
        }

        // Persist User Message to Database
        let user_msg = ConversationMessage::new_human(session_id.clone(), raw_input.clone());
        if let Err(e) = conversation_manager.save_message(&user_msg).await {
            tracing::error!("Failed to save user message to DB: {}", e);
        }

        // Fetch recent conversation history (Short-term memory)
        // We fetch slightly more to ensure we have context, but exclude the message we just saved
        // to avoid duplication in the prompt if we were to iterate blindly,
        // but get_conversation_history returns chronological.
        // The message we just saved IS in the DB now.
        // We want the history *before* the current request to provide context.
        // But `augmented_input` already contains the current user request at the end.
        // So we should fetch history, exclude the very last message (current request), and format the rest.

        let history_text = match conversation_manager
            .get_conversation_history(&session_id, Some(10))
            .await
        {
            Ok(messages) => {
                let mut text = String::new();
                if !messages.is_empty() {
                    text.push_str("\n[RECENT CHAT HISTORY]\n");
                    // Filter out the last message if it matches our current raw_input (it's the one we just saved)
                    // OR just take everything except the last one.
                    // Since we just saved `user_msg`, it is likely the last one.
                    let len = messages.len();
                    for (i, msg) in messages.iter().enumerate() {
                        // Skip the last message as it is likely the current request which is already in augmented_input
                        if i == len - 1
                            && msg.role == crate::agent::conversation_manager::MessageRole::Human
                            && msg.content == raw_input
                        {
                            continue;
                        }

                        let role = match msg.role {
                            crate::agent::conversation_manager::MessageRole::Human => "User",
                            crate::agent::conversation_manager::MessageRole::Assistant => {
                                "Assistant"
                            }
                            crate::agent::conversation_manager::MessageRole::System => "System",
                            crate::agent::conversation_manager::MessageRole::Function => {
                                "System (Tool Output)"
                            }
                            crate::agent::conversation_manager::MessageRole::ToolCall => {
                                "Tool Call"
                            }
                            crate::agent::conversation_manager::MessageRole::ToolResult => {
                                "Tool Result"
                            }
                        };
                        text.push_str(&format!("{}: {}\n", role, msg.content));
                    }
                    text.push_str("\n");
                }
                text
            }
            Err(e) => {
                tracing::warn!("Failed to fetch conversation history: {}", e);
                String::new()
            }
        };

        // Construct Final Prompt: History + Augmented Input (Context + Request)
        let final_prompt = format!("{}{}", history_text, augmented_input);

        // Don't send generic "starting" message - let the agent respond naturally

        // Create progress callback that sends updates via WebSocket
        let job_manager_clone = self.job_manager.clone();
        let job_id_clone = job_id.clone();
        let session_id_clone = session_id.clone();
        let workflow_id_clone = workflow_id.clone();
        let workflow_pool = self.app_state.db_pool.clone();
        let active_tool_node = Arc::new(Mutex::new(None::<String>));
        let tool_step_counter = Arc::new(AtomicUsize::new(0));

        let progress_callback = Arc::new(move |progress: f32, message: &str| {
            let job_manager = job_manager_clone.clone();
            let job_id = job_id_clone.clone();
            let session_id = session_id_clone.clone();
            let msg = message.to_string();
            let workflow_id = workflow_id_clone.clone();
            let workflow_pool = workflow_pool.clone();
            let active_tool_node = active_tool_node.clone();
            let tool_step_counter = tool_step_counter.clone();

            // Spawn async task to send progress
            tokio::spawn(async move {
                let status = JobStatus::Running {
                    current_step: msg.clone(),
                    progress_percent: Some((progress * 100.0) as f64),
                    steps_completed: 0,
                    total_steps: 0,
                    completed_actions: None,
                    current_action_detail: None,
                };

                let update = ProgressUpdate::new(job_id.clone(), msg.clone(), status.clone());
                job_manager.update_job_status(&job_id, status).await;
                job_manager.send_progress(&session_id, update).await;

                if let Some(workflow_id) = workflow_id {
                    let workflow_runtime = crate::services::WorkflowRuntime::new(workflow_pool);
                    if let Some(tool_name) = extract_agent_tool_name(&msg) {
                        let tool_policy =
                            crate::tool_registry::ToolRegistry::durable_policy_for_tool(&tool_name);
                        let node_key = format!(
                            "tool_{:03}_{}",
                            tool_step_counter.fetch_add(1, Ordering::SeqCst) + 1,
                            sanitize_workflow_node_key(&tool_name)
                        );
                        let previous_node = active_tool_node
                            .lock()
                            .ok()
                            .and_then(|mut guard| guard.replace(node_key.clone()));

                        if let Some(previous_node) = previous_node {
                            let _ = workflow_runtime
                                .complete_node(
                                    workflow_id,
                                    &previous_node,
                                    json!({
                                        "next_tool": tool_name,
                                        "handoff": "agent_started_next_tool",
                                    }),
                                    "Agent moved to the next tool call.",
                                )
                                .await;
                        }

                        let _ = workflow_runtime
                            .ensure_node(
                                workflow_id,
                                &node_key,
                                if tool_policy.requires_durable_node() {
                                    "durable_agent_tool_call"
                                } else {
                                    "agent_tool_call"
                                },
                                json!({
                                    "job_id": job_id,
                                    "tool_name": tool_name,
                                    "progress_message": msg,
                                    "durable_policy": tool_policy.as_str(),
                                    "requires_durable_node": tool_policy.requires_durable_node(),
                                    "timeout_hint_seconds": tool_policy.timeout_hint_seconds(),
                                }),
                                tool_policy.max_attempts(),
                            )
                            .await;
                        let _ = workflow_runtime
                            .start_node(
                                workflow_id,
                                &node_key,
                                &format!("Agent is executing tool '{}'.", tool_name),
                                json!({
                                    "job_id": job_id,
                                    "tool_name": tool_name,
                                    "durable_policy": tool_policy.as_str(),
                                    "requires_durable_node": tool_policy.requires_durable_node(),
                                }),
                            )
                            .await;
                    } else if is_agent_completion_message(&msg) {
                        let completed_node = active_tool_node
                            .lock()
                            .ok()
                            .and_then(|mut guard| guard.take());
                        if let Some(completed_node) = completed_node {
                            let _ = workflow_runtime
                                .complete_node(
                                    workflow_id,
                                    &completed_node,
                                    json!({
                                        "progress_message": msg,
                                        "handoff": "agent_reported_completion",
                                    }),
                                    "Agent reported the current tool/action completed.",
                                )
                                .await;
                        }
                    }

                    let _ = workflow_runtime
                        .heartbeat(
                            workflow_id,
                            crate::services::WorkflowStatus::Running,
                            Some("agent_execution"),
                            &msg,
                            json!({
                                "job_id": job_id,
                                "progress_percent": (progress * 100.0) as f64,
                            }),
                        )
                        .await;
                }
            });
        });

        if let Some(workflow_id) = workflow_id {
            start_video_workflow_node(
                &workflow_runtime,
                workflow_id,
                "agent_execution",
                "Running the AI editing/generation agent with canonical tool access.",
                json!({
                    "job_id": job_id,
                    "session_id": session_id,
                    "artifact_expected": artifact_expected,
                }),
            )
            .await;
        }

        // Execute based on agent type using the FINAL PROMPT
        let result = match &self.agent_type {
            AgentType::Claude => {
                self.execute_with_claude(
                    &final_prompt,
                    &session_id,
                    progress_callback,
                    &mut control_rx,
                    workflow_id,
                )
                .await
            }
        };

        // Update final status and save response
        match result {
            Ok(response) => {
                let output_links = Self::extract_output_links(&response);
                if let Some(workflow_id) = workflow_id {
                    complete_video_workflow_node(
                        &workflow_runtime,
                        workflow_id,
                        "agent_execution",
                        json!({
                            "output_link_count": output_links.len(),
                            "response_preview": response.chars().take(240).collect::<String>(),
                        }),
                        "AI agent execution completed and returned a response.",
                    )
                    .await;
                    start_video_workflow_node(
                        &workflow_runtime,
                        workflow_id,
                        "artifact_verification",
                        "Verifying generated delivery/output links before accepting the job as complete.",
                        json!({
                            "artifact_expected": artifact_expected,
                            "output_links": output_links,
                        }),
                    )
                    .await;
                }
                if artifact_expected && output_links.is_empty() {
                    let error = "The workflow returned assistant text without a delivery or output link, so it was not accepted as a completed generation result.".to_string();
                    if let Some(workflow_id) = workflow_id {
                        fail_video_workflow_node(
                            &workflow_runtime,
                            workflow_id,
                            "artifact_verification",
                            &error,
                            json!({
                                "artifact_expected": artifact_expected,
                                "output_links": output_links,
                            }),
                        )
                        .await;
                        let _ = workflow_runtime
                            .mark_failed(
                                workflow_id,
                                Some("artifact_verification"),
                                &error,
                                None,
                            )
                            .await;
                    }
                    self.send_progress(
                        "The workflow stopped because it did not return a playable delivery or output artifact.",
                        JobStatus::Failed {
                            error: error.clone(),
                            failed_at_step: "artifact_verification".to_string(),
                        },
                    )
                    .await;
                    return Err(error);
                }

                let artifact_verification = if artifact_expected {
                    crate::services::ArtifactVerifier::verify_links(
                        &self.app_state.db_pool,
                        &output_links,
                    )
                    .await
                } else {
                    crate::services::ArtifactVerificationResult {
                        verified: true,
                        details: json!({
                            "verified": true,
                            "reason": "No generated artifact was required for this workflow",
                            "links": [],
                        }),
                    }
                };

                if artifact_expected && !artifact_verification.verified {
                    let error = "The workflow returned delivery/output links, but the linked artifacts could not be verified from storage or the database.".to_string();
                    if let Some(workflow_id) = workflow_id {
                        fail_video_workflow_node(
                            &workflow_runtime,
                            workflow_id,
                            "artifact_verification",
                            &error,
                            artifact_verification.details.clone(),
                        )
                        .await;
                        let _ = workflow_runtime
                            .mark_failed(
                                workflow_id,
                                Some("artifact_verification"),
                                &error,
                                None,
                            )
                            .await;
                    }
                    self.send_progress(
                        "The workflow stopped because the returned delivery/output links could not be verified.",
                        JobStatus::Failed {
                            error: error.clone(),
                            failed_at_step: "artifact_verification".to_string(),
                        },
                    )
                    .await;
                    return Err(error);
                }

                if let Some(workflow_id) = workflow_id {
                    if artifact_expected {
                        complete_video_workflow_node(
                            &workflow_runtime,
                            workflow_id,
                            "artifact_verification",
                            artifact_verification.details.clone(),
                            "Generated artifacts were verified.",
                        )
                        .await;
                    } else {
                        skip_video_workflow_node(
                            &workflow_runtime,
                            workflow_id,
                            "artifact_verification",
                            "No generated artifact was required for this editing response.",
                        )
                        .await;
                    }
                    start_video_workflow_node(
                        &workflow_runtime,
                        workflow_id,
                        "persist_response",
                        "Persisting assistant response, output links, and usage metadata.",
                        json!({
                            "job_id": job_id,
                            "output_links": output_links,
                        }),
                    )
                    .await;
                }

                // Fetch pricing from database
                let pricing = self.fetch_pricing_from_db().await;

                let model_name = "claude-sonnet-4-5";

                let prompt_tokens = Self::estimate_tokens(&final_prompt);
                let completion_tokens = Self::estimate_tokens(&response);
                let total_tokens = prompt_tokens + completion_tokens;

                let cost_usd =
                    Self::calculate_cost(model_name, prompt_tokens, completion_tokens, &pricing);

                tracing::info!(
                    "💰 Estimated Usage: {} prompt + {} completion = {} total tokens. Cost: ${:.6}",
                    prompt_tokens,
                    completion_tokens,
                    total_tokens,
                    cost_usd
                );

                // Persist Assistant Response to Database with Usage Metrics
                let mut assistant_msg =
                    ConversationMessage::new_assistant(session_id.clone(), response.clone());
                assistant_msg.prompt_tokens = Some(prompt_tokens);
                assistant_msg.completion_tokens = Some(completion_tokens);
                assistant_msg.total_tokens = Some(total_tokens);
                assistant_msg.model = Some(model_name.to_string());
                assistant_msg.cost_usd = Some(cost_usd);

                if let Err(e) = conversation_manager.save_message(&assistant_msg).await {
                    tracing::error!("Failed to save assistant message to DB: {}", e);
                }

                if let Some(workflow_id) = workflow_id {
                    complete_video_workflow_node(
                        &workflow_runtime,
                        workflow_id,
                        "persist_response",
                        json!({
                            "prompt_tokens": prompt_tokens,
                            "completion_tokens": completion_tokens,
                            "total_tokens": total_tokens,
                            "estimated_cost_usd": cost_usd.to_string(),
                            "output_links": output_links,
                        }),
                        "Assistant response and usage metadata persisted.",
                    )
                    .await;
                    start_video_workflow_node(
                        &workflow_runtime,
                        workflow_id,
                        "vectorize_memory",
                        "Storing the editing conversation in long-term memory when vector storage is available.",
                        json!({
                            "session_id": session_id,
                            "provider": if self.app_state.video_gemini_client.is_some()
                                || self.app_state.gemini_client.is_some()
                            {
                                "gemini_embedding2"
                            } else if self.app_state.voyage_embeddings.is_some() {
                                "voyage"
                            } else {
                                "not_configured"
                            },
                        }),
                    )
                    .await;
                }

                // Store in Vector Database (Qdrant) for Long-Term Memory
                if let Some(ref qdrant_client) = self.app_state.qdrant_client {
                    let files_referenced = vec![];
                    let context_data = std::collections::HashMap::new();

                    if let Err(e) = qdrant_client
                        .store_chat_memory(
                            &session_id,
                            None,
                            &raw_input,
                            &response,
                            files_referenced,
                            context_data,
                            self.app_state.voyage_embeddings.as_ref(),
                            self.app_state.video_gemini_client.as_ref().or(self.app_state.gemini_client.as_ref()),
                            Some("video_editing"),
                        )
                        .await
                    {
                        tracing::warn!(
                            "Failed to store conversation in Qdrant: {}",
                            e
                        );
                    }
                }

                if let Some(workflow_id) = workflow_id {
                    complete_video_workflow_node(
                        &workflow_runtime,
                        workflow_id,
                        "vectorize_memory",
                        json!({
                            "session_id": session_id,
                            "attempted": self.app_state.qdrant_client.is_some(),
                        }),
                        "Long-term memory storage step completed.",
                    )
                    .await;
                }

                // Send the AI's response directly (no generic "video editing completed" message)
                self.send_progress(
                    &response,
                    JobStatus::Completed {
                        result: response.clone(),
                        output_files: output_links.clone(),
                        duration_seconds: 0.0,
                    },
                )
                .await;

                if let Some(workflow_id) = workflow_id {
                    let _ = workflow_runtime
                        .mark_completed(
                            workflow_id,
                            Some("assistant_response_ready"),
                            "Background video workflow completed and returned a terminal assistant response.",
                            json!({
                                "artifact_verification": artifact_verification.details,
                                "output_links": output_links,
                                "response_preview": response.chars().take(240).collect::<String>(),
                            }),
                        )
                        .await;
                }
                Ok(response)
            }
            Err(error) => {
                let was_cancelled = error == "Job cancelled by user";
                if let Some(workflow_id) = workflow_id {
                    if was_cancelled {
                        skip_video_workflow_node(
                            &workflow_runtime,
                            workflow_id,
                            "agent_execution",
                            "The user cancelled the job while the agent was running.",
                        )
                        .await;
                    } else {
                        fail_video_workflow_node(
                            &workflow_runtime,
                            workflow_id,
                            "agent_execution",
                            &error,
                            json!({ "job_id": job_id }),
                        )
                        .await;
                    }
                    let _ = if was_cancelled {
                        workflow_runtime
                            .mark_cancelled(
                                workflow_id,
                                Some("cancelled"),
                                "The background video workflow was cancelled by the user.",
                            )
                            .await
                    } else {
                        workflow_runtime
                            .mark_failed(
                                workflow_id,
                                Some("background_video_job"),
                                &error,
                                None,
                        )
                        .await
                    };
                }
                let progress_message = if was_cancelled {
                    "The background video workflow was cancelled by the user.".to_string()
                } else {
                    format!("The background video workflow failed: {}", error)
                };

                self.send_progress(
                    &progress_message,
                    if was_cancelled {
                        JobStatus::Cancelled {
                            cancelled_at_step: "User requested cancellation".to_string(),
                        }
                    } else {
                        JobStatus::Failed {
                            error: error.clone(),
                            failed_at_step: "Processing".to_string(),
                        }
                    },
                )
                .await;
                Err(error)
            }
        }
    }

    /// Fetch model pricing from system_settings table
    async fn fetch_pricing_from_db(
        &self,
    ) -> std::collections::HashMap<String, rust_decimal::Decimal> {
        use rust_decimal::prelude::*;
        use std::str::FromStr;

        let mut pricing = std::collections::HashMap::new();

        // Default fallback values
        pricing.insert(
            "claude_input".to_string(),
            Decimal::from_str("3.00").unwrap(),
        );
        pricing.insert(
            "claude_output".to_string(),
            Decimal::from_str("15.00").unwrap(),
        );
        pricing.insert(
            "gemini_input".to_string(),
            Decimal::from_str("3.50").unwrap(),
        );
        pricing.insert(
            "gemini_output".to_string(),
            Decimal::from_str("10.50").unwrap(),
        );

        // Query DB for overrides
        // Note: We handle potential DB errors gracefully by falling back to defaults
        let query = "SELECT setting_key, setting_value FROM system_settings WHERE setting_key LIKE 'model_pricing.%'";

        if let Ok(rows) = sqlx::query_as::<_, (String, String)>(query)
            .fetch_all(&self.app_state.db_pool)
            .await
        {
            for (key, value) in rows {
                if let Ok(decimal_val) = Decimal::from_str(&value) {
                    match key.as_str() {
                        "model_pricing.claude-sonnet-4-5.input" => {
                            pricing.insert("claude_input".to_string(), decimal_val);
                        }
                        "model_pricing.claude-sonnet-4-5.output" => {
                            pricing.insert("claude_output".to_string(), decimal_val);
                        }
                        "model_pricing.gemini-2.5-flash.input" => {
                            pricing.insert("gemini_input".to_string(), decimal_val);
                        }
                        "model_pricing.gemini-2.5-flash.output" => {
                            pricing.insert("gemini_output".to_string(), decimal_val);
                        }
                        _ => {}
                    }
                }
            }
        } else {
            tracing::warn!("Failed to fetch pricing from DB, using defaults");
        }

        pricing
    }

    /// Estimate token count (approx 4 characters per token)
    fn estimate_tokens(text: &str) -> i32 {
        (text.len() as f32 / 4.0).ceil() as i32
    }

    /// Calculate estimated cost in USD
    fn calculate_cost(
        model: &str,
        prompt_tokens: i32,
        completion_tokens: i32,
        pricing: &std::collections::HashMap<String, rust_decimal::Decimal>,
    ) -> rust_decimal::Decimal {
        use rust_decimal::prelude::*;

        let one_million = Decimal::from(1_000_000);

        let (input_price, output_price) = match model {
            "claude-sonnet-4-5" => (
                pricing
                    .get("claude_input")
                    .cloned()
                    .unwrap_or(Decimal::from_str("3.00").unwrap()),
                pricing
                    .get("claude_output")
                    .cloned()
                    .unwrap_or(Decimal::from_str("15.00").unwrap()),
            ),
            "gemini-2.5-flash" => (
                pricing
                    .get("gemini_input")
                    .cloned()
                    .unwrap_or(Decimal::from_str("3.50").unwrap()),
                pricing
                    .get("gemini_output")
                    .cloned()
                    .unwrap_or(Decimal::from_str("10.50").unwrap()),
            ),
            _ => (Decimal::from(1), Decimal::from(3)),
        };

        let input_cost = (Decimal::from(prompt_tokens) / one_million) * input_price;
        let output_cost = (Decimal::from(completion_tokens) / one_million) * output_price;

        input_cost + output_cost
    }

    /// Execute using StatefulGeminiAgent with ReAct pattern and user interruption support
    async fn execute_with_claude(
        &self,
        user_input: &str,
        session_id: &str,
        progress_callback: Arc<dyn Fn(f32, &str) + Send + Sync>,
        control_rx: &mut mpsc::UnboundedReceiver<JobControl>,
        workflow_id: Option<uuid::Uuid>,
    ) -> Result<String, String> {
        let gemini_client = self
            .app_state
            .video_gemini_client
            .as_ref()
            .or(self.app_state.gemini_client.as_ref())
            .ok_or("Gemini client not configured")?;
        let agent = StatefulGeminiAgent::new_with_nvidia(
            Arc::new(gemini_client.clone()),
            self.app_state.bedrock_client.clone(),
            self.app_state.nvidia_nim_client.clone().map(Arc::new),
            self.app_state.ollama_client.clone().map(Arc::new),
        );

        let (progress_tx, mut progress_rx) = tokio::sync::mpsc::unbounded_channel::<String>();

        let ctx = if self.job.job_type == "video_editing" {
            let raw = self.job.input_data.get("raw_input")
                .and_then(|v| v.as_str())
                .unwrap_or("");
            let augmented = self.job.input_data.get("augmented_input")
                .and_then(|v| v.as_str())
                .unwrap_or("");
            format!("Raw input: {}\nAugmented context: {}", raw, augmented)
        } else {
            String::new()
        };

        let user_input_owned = user_input.to_string();
        let session_id_owned = session_id.to_string();
        let app_state_clone = self.app_state.clone();
        let job_manager_clone = self.job_manager.clone();

        let mut agent_handle = tokio::spawn(async move {
            agent
                .chat(
                    &user_input_owned,
                    &session_id_owned,
                    ctx,
                    app_state_clone,
                    job_manager_clone,
                    Some(progress_tx),
                    workflow_id,
                    None,
                    None,
                )
                .await
        });

        // Forward progress updates
        let progress_cb = progress_callback.clone();
        let mut prog_handle = tokio::spawn(async move {
            while let Some(msg) = progress_rx.recv().await {
                progress_cb(0.0, &msg);
            }
        });

        let timeout_secs = std::env::var("VIDEO_JOB_AGENT_TIMEOUT_SECONDS")
            .ok()
            .and_then(|value| value.parse::<u64>().ok())
            .unwrap_or(1200);
        let timeout = tokio::time::sleep(Duration::from_secs(timeout_secs));
        tokio::pin!(timeout);

        // Poll for control commands
        loop {
            tokio::select! {
                result = &mut agent_handle => {
                    prog_handle.abort();
                    return result.map_err(|e| format!("Agent task failed: {}", e))?;
                }
                _ = &mut timeout => {
                    tracing::error!("⏱️ Video job timed out after {} seconds", timeout_secs);
                    progress_callback(1.0, "Video job hit its execution limit and was stopped.");
                    agent_handle.abort();
                    return Err(format!("Video job exceeded the {} second execution limit", timeout_secs));
                }
                control = control_rx.recv() => {
                    if let Some(JobControl::Cancel) = control {
                        tracing::info!("🛑 Job cancelled by user");
                        agent_handle.abort();
                        return Err("Job cancelled by user".to_string());
                    }
                }
            }
        }
    }

    /// Execute a future with support for pause/resume/cancel
    #[allow(dead_code)]
    async fn execute_with_interruption_support<F, Fut>(
        &self,
        executor: F,
        control_rx: &mut mpsc::UnboundedReceiver<JobControl>,
    ) -> Result<String, String>
    where
        F: FnOnce() -> Fut + Send + 'static,
        Fut: std::future::Future<Output = Result<String, String>> + Send + 'static,
    {
        let job_id = self.job.id.clone();
        let _session_id = self.job.session_id.clone();

        // Spawn the actual execution
        let mut execution_handle = tokio::spawn(executor());

        // Monitor for control commands
        loop {
            tokio::select! {
                // Check if execution completed
                result = &mut execution_handle => {
                    match result {
                        Ok(Ok(response)) => return Ok(response),
                        Ok(Err(e)) => return Err(e),
                        Err(e) => return Err(format!("Job execution panicked: {}", e)),
                    }
                }

                // Check for control commands
                control = control_rx.recv() => {
                    if let Some(cmd) = control {
                        match cmd {
                            JobControl::Pause => {
                                tracing::info!("⏸️ Pausing job: {}", job_id);
                                self.send_progress(
                                    "The workflow is paused because the user requested a pause.",
                                    JobStatus::Paused {
                                        paused_at_step: "User requested pause".to_string(),
                                        progress_percent: 50.0,
                                    },
                                ).await;
                                // Note: Full pause implementation would require cooperative agent support
                            }
                            JobControl::Cancel => {
                                tracing::info!("🛑 Cancelling job: {}", job_id);
                                self.send_progress(
                                    "The workflow was cancelled because the user requested cancellation.",
                                    JobStatus::Cancelled {
                                        cancelled_at_step: "User requested cancellation".to_string(),
                                    },
                                ).await;
                                return Err("Job cancelled by user".to_string());
                            }
                            JobControl::Resume => {
                                tracing::info!("▶️ Resuming job: {}", job_id);
                                self.send_progress(
                                    "The workflow is resuming after the user requested continuation.",
                                    JobStatus::Running {
                                        current_step: "Resumed".to_string(),
                                        progress_percent: Some(50.0),
                                        steps_completed: 0,
                                        total_steps: 0,
                                        completed_actions: None,
                                        current_action_detail: None,
                                    },
                                ).await;
                            }
                            JobControl::UpdateInput(new_input) => {
                                tracing::info!("🔄 User provided new input for job: {}", job_id);
                                self.send_progress(
                                    &format!("The workflow received updated user input: {:?}", new_input),
                                    JobStatus::Running {
                                        current_step: "Updated by user".to_string(),
                                        progress_percent: Some(50.0),
                                        steps_completed: 0,
                                        total_steps: 0,
                                        completed_actions: None,
                                        current_action_detail: None,
                                    },
                                ).await;
                                // Note: Full dynamic update would require agent cooperation
                            }
                        }
                    }
                }
            }
        }
    }

    /// Helper to send progress update
    async fn send_progress(&self, message: &str, status: JobStatus) {
        let mut update =
            ProgressUpdate::new(self.job.id.clone(), message.to_string(), status.clone());

        // Include original user message in details for proper DB storage
        let user_message = self
            .job
            .input_data
            .get("raw_input")
            .and_then(|v| v.as_str())
            .unwrap_or("")
            .to_string();

        update.details = Some(serde_json::json!({
            "user_message": user_message,
        }));

        self.job_manager
            .update_job_status(&self.job.id, status)
            .await;
        self.job_manager
            .send_progress(&self.job.session_id, update)
            .await;
    }
}

async fn ensure_video_editing_workflow_plan(
    workflow_runtime: &crate::services::WorkflowRuntime,
    workflow_id: uuid::Uuid,
    job_id: &str,
    raw_input: &str,
    artifact_expected: bool,
) {
    let node_specs = [
        (
            "plan_request",
            "video_request_planning",
            json!({
                "job_id": job_id,
                "request_preview": raw_input.chars().take(240).collect::<String>(),
                "artifact_expected": artifact_expected,
            }),
        ),
        (
            "agent_execution",
            "canonical_tool_agent_execution",
            json!({
                "job_id": job_id,
                "tool_access": "canonical_video_editing_generation_tools",
                "supports_long_running_outputs": true,
            }),
        ),
        (
            "artifact_verification",
            "generated_artifact_verification",
            json!({
                "job_id": job_id,
                "required": artifact_expected,
            }),
        ),
        (
            "persist_response",
            "conversation_and_usage_persistence",
            json!({
                "job_id": job_id,
            }),
        ),
        (
            "vectorize_memory",
            "long_term_memory_vectorization",
            json!({
                "job_id": job_id,
            }),
        ),
    ];

    for (node_key, node_type, input) in node_specs {
        let _ = workflow_runtime
            .ensure_node(workflow_id, node_key, node_type, input, 3)
            .await;
    }
}

async fn start_video_workflow_node(
    workflow_runtime: &crate::services::WorkflowRuntime,
    workflow_id: uuid::Uuid,
    node_key: &str,
    message: &str,
    details: serde_json::Value,
) {
    let _ = workflow_runtime
        .start_node(workflow_id, node_key, message, details)
        .await;
}

async fn complete_video_workflow_node(
    workflow_runtime: &crate::services::WorkflowRuntime,
    workflow_id: uuid::Uuid,
    node_key: &str,
    output: serde_json::Value,
    message: &str,
) {
    let _ = workflow_runtime
        .complete_node(workflow_id, node_key, output, message)
        .await;
}

async fn fail_video_workflow_node(
    workflow_runtime: &crate::services::WorkflowRuntime,
    workflow_id: uuid::Uuid,
    node_key: &str,
    error: &str,
    details: serde_json::Value,
) {
    let _ = workflow_runtime
        .fail_node(workflow_id, node_key, error, details)
        .await;
}

async fn skip_video_workflow_node(
    workflow_runtime: &crate::services::WorkflowRuntime,
    workflow_id: uuid::Uuid,
    node_key: &str,
    reason: &str,
) {
    let _ = workflow_runtime
        .skip_node(workflow_id, node_key, reason, json!({ "reason": reason }))
        .await;
}

fn extract_agent_tool_name(message: &str) -> Option<String> {
    let trimmed = message.trim();
    let without_prefix = trimmed
        .strip_prefix("🔧")
        .or_else(|| trimmed.strip_prefix("Executing"))
        .or_else(|| trimmed.strip_prefix("Detected tool call:"))?;
    let candidate = without_prefix
        .trim()
        .trim_start_matches(':')
        .trim()
        .trim_end_matches("...")
        .trim();

    if candidate.is_empty() || candidate.len() > 120 {
        return None;
    }

    Some(candidate.to_string())
}

fn is_agent_completion_message(message: &str) -> bool {
    let lower = message.to_ascii_lowercase();
    lower.contains("task completed")
        || lower.contains(" completed")
        || lower.contains("output ready")
        || lower.contains("delivery ready")
}

fn sanitize_workflow_node_key(value: &str) -> String {
    let mut sanitized = value
        .chars()
        .map(|ch| {
            if ch.is_ascii_alphanumeric() {
                ch.to_ascii_lowercase()
            } else {
                '_'
            }
        })
        .collect::<String>();
    while sanitized.contains("__") {
        sanitized = sanitized.replace("__", "_");
    }
    sanitized = sanitized.trim_matches('_').to_string();
    if sanitized.is_empty() {
        "tool".to_string()
    } else {
        sanitized.chars().take(64).collect()
    }
}

/// Spawn a video editing job in background
pub async fn spawn_video_editing_job(
    raw_input: String,
    augmented_input: String,
    session_id: String,
    agent_type: AgentType,
    app_state: Arc<AppState>,
    job_manager: Arc<JobManager>,
) -> Result<JobId, String> {
    // Create job
    let job_data = json!({
        "raw_input": raw_input,
        "augmented_input": augmented_input,
        "agent_type": format!("{:?}", agent_type),
    });

    let job = Job::new(session_id.clone(), "video_editing".to_string(), job_data);
    let job_id = job.id.clone();

    // Store job in manager
    let job_id_stored = job_manager.create_job(job.clone()).await;

    // Spawn background execution
    let video_job = VideoEditingJob::new(job, agent_type, app_state, job_manager.clone());
    let job_id_for_spawn = job_id.clone();

    tokio::spawn(async move {
        tracing::info!("🔥 INSIDE tokio::spawn for job: {}", job_id_for_spawn);
        match video_job.execute().await {
            Ok(result) => {
                tracing::info!("✅ Video editing job completed: {}", job_id_for_spawn);
                tracing::debug!("Result: {}", result);
            }
            Err(e) => {
                tracing::error!(
                    "❌ Video editing job failed: {} - Error: {}",
                    job_id_for_spawn,
                    e
                );
            }
        }
        tracing::info!("🔥 EXITING tokio::spawn for job: {}", job_id_for_spawn);
    });

    tracing::info!(
        "🚀 Spawned video editing job: {} for session: {}",
        job_id_stored,
        session_id
    );
    Ok(job_id_stored)
}
