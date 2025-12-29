// src/jobs/video_job.rs
//! Video editing job executor - runs AI agents in background with progress updates
//! Now with LangGraph-style ReAct pattern: Thought → Action → Observation → Reflection

use super::{Job, JobControl, JobId, JobManager, JobStatus, ProgressUpdate};
use crate::agent::simple_claude_agent::SimpleClaudeAgent;
use crate::agent::simple_gemini_agent::SimpleGeminiAgent;
use crate::agent::react_agent::{ReActClaudeAgent, ReActGeminiAgent};
use crate::agent::react_state::{AgentState, UserCommand};
use crate::agent::conversation_manager::{ConversationManager, ConversationMessage};
use crate::AppState;
use std::sync::Arc;
use tokio::sync::mpsc;
use serde_json::json;

/// Type of AI model to use for the job
#[derive(Debug, Clone)]
pub enum AgentType {
    Claude,
    Gemini,
}

/// Video editing job that runs in background
pub struct VideoEditingJob {
    job: Job,
    agent_type: AgentType,
    app_state: Arc<AppState>,
    job_manager: Arc<JobManager>,
}

impl VideoEditingJob {
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

        tracing::info!("🎬 Starting video editing job: {} (agent: {:?})", job_id, self.agent_type);

        // Initialize ConversationManager for history persistence
        let conversation_manager = ConversationManager::new(self.app_state.db_pool.clone());
        
        // Ensure schema exists
        if let Err(e) = conversation_manager.initialize_schema().await {
            tracing::warn!("Failed to initialize conversation schema: {}", e);
        }

        // Create control channel for this job
        let (control_tx, mut control_rx) = mpsc::unbounded_channel::<JobControl>();
        self.job_manager.register_control_channel(job_id.clone(), control_tx).await;

        // Extract inputs from job data
        let raw_input = self.job.input_data.get("raw_input")
            .and_then(|v| v.as_str())
            .ok_or("Missing raw_input in job data")?
            .to_string();
            
        let augmented_input = self.job.input_data.get("augmented_input")
            .and_then(|v| v.as_str())
            .unwrap_or(&raw_input) // Fallback to raw input if augmented not found
            .to_string();

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
        
        let history_text = match conversation_manager.get_conversation_history(&session_id, Some(10)).await {
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
                        if i == len - 1 && msg.role == crate::agent::conversation_manager::MessageRole::Human && msg.content == raw_input {
                            continue;
                        }
                        
                        let role = match msg.role {
                            crate::agent::conversation_manager::MessageRole::Human => "User",
                            crate::agent::conversation_manager::MessageRole::Assistant => "Assistant",
                            crate::agent::conversation_manager::MessageRole::System => "System",
                            crate::agent::conversation_manager::MessageRole::Function => "System (Tool Output)",
                        };
                        text.push_str(&format!("{}: {}\n", role, msg.content));
                    }
                    text.push_str("\n");
                }
                text
            },
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

        let progress_callback = Arc::new(move |progress: f32, message: &str| {
            let job_manager = job_manager_clone.clone();
            let job_id = job_id_clone.clone();
            let session_id = session_id_clone.clone();
            let msg = message.to_string();

            // Spawn async task to send progress
            tokio::spawn(async move {
                let status = JobStatus::Running {
                    current_step: msg.clone(),
                    progress_percent: (progress * 100.0) as f64,
                    steps_completed: 0,
                    total_steps: 0,
                };

                let update = ProgressUpdate::new(job_id.clone(), msg, status.clone());
                job_manager.update_job_status(&job_id, status).await;
                job_manager.send_progress(&session_id, update).await;
            });
        });

        // Execute based on agent type using the FINAL PROMPT
        let result = match self.agent_type {
            AgentType::Claude => {
                self.execute_with_claude(&final_prompt, &session_id, progress_callback, &mut control_rx).await
            }
            AgentType::Gemini => {
                self.execute_with_gemini(&final_prompt, &session_id, progress_callback, &mut control_rx).await
            }
        };

        // Update final status and save response
        match result {
            Ok(response) => {
                // Fetch pricing from database
                let pricing = self.fetch_pricing_from_db().await;
                
                let model_name = match self.agent_type {
                    AgentType::Claude => "claude-sonnet-4-5",
                    AgentType::Gemini => "gemini-3-pro-preview",
                };
                
                let prompt_tokens = Self::estimate_tokens(&final_prompt);
                let completion_tokens = Self::estimate_tokens(&response);
                let total_tokens = prompt_tokens + completion_tokens;
                
                let cost_usd = Self::calculate_cost(model_name, prompt_tokens, completion_tokens, &pricing);
                
                tracing::info!(
                    "💰 Estimated Usage: {} prompt + {} completion = {} total tokens. Cost: ${:.6}", 
                    prompt_tokens, completion_tokens, total_tokens, cost_usd
                );

                // Persist Assistant Response to Database with Usage Metrics
                let mut assistant_msg = ConversationMessage::new_assistant(session_id.clone(), response.clone());
                assistant_msg.prompt_tokens = Some(prompt_tokens);
                assistant_msg.completion_tokens = Some(completion_tokens);
                assistant_msg.total_tokens = Some(total_tokens);
                assistant_msg.model = Some(model_name.to_string());
                assistant_msg.cost_usd = Some(cost_usd);

                if let Err(e) = conversation_manager.save_message(&assistant_msg).await {
                    tracing::error!("Failed to save assistant message to DB: {}", e);
                }

                // Store in Vector Database (Qdrant) for Long-Term Memory
                if let Some(ref qdrant_client) = self.app_state.qdrant_client {
                    let files_referenced = vec![]; 
                    let context_data = std::collections::HashMap::new();
                    
                    if let Some(ref voyage_embeddings) = self.app_state.voyage_embeddings {
                        if let Err(e) = qdrant_client.store_chat_memory_with_voyage(
                            &session_id,
                            None, 
                            &raw_input,
                            &response,
                            files_referenced,
                            context_data,
                            voyage_embeddings,
                        ).await {
                            tracing::warn!("Failed to store conversation in Qdrant (Voyage): {}", e);
                        }
                    } else if let Some(ref gemini_client) = self.app_state.gemini_client {
                        if let Err(e) = qdrant_client.store_chat_memory_with_gemini(
                            &session_id,
                            None,
                            &raw_input,
                            &response,
                            files_referenced,
                            context_data,
                            gemini_client,
                        ).await {
                            tracing::warn!("Failed to store conversation in Qdrant (Gemini): {}", e);
                        }
                    }
                }

                // Send the AI's response directly (no generic "video editing completed" message)
                self.send_progress(
                    &response,
                    JobStatus::Completed {
                        result: response.clone(),
                        output_files: vec![],
                        duration_seconds: 0.0,
                    },
                ).await;
                Ok(response)
            }
            Err(error) => {
                self.send_progress(
                    &format!("❌ Video editing failed: {}", error),
                    JobStatus::Failed {
                        error: error.clone(),
                        failed_at_step: "Processing".to_string(),
                    },
                ).await;
                Err(error)
            }
        }
    }

    /// Fetch model pricing from system_settings table
    async fn fetch_pricing_from_db(&self) -> std::collections::HashMap<String, rust_decimal::Decimal> {
        use rust_decimal::prelude::*;
        use std::str::FromStr;
        
        let mut pricing = std::collections::HashMap::new();
        
        // Default fallback values
        pricing.insert("claude_input".to_string(), Decimal::from_str("3.00").unwrap());
        pricing.insert("claude_output".to_string(), Decimal::from_str("15.00").unwrap());
        pricing.insert("gemini_input".to_string(), Decimal::from_str("3.50").unwrap());
        pricing.insert("gemini_output".to_string(), Decimal::from_str("10.50").unwrap());
        
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
                        "model_pricing.claude-sonnet-4-5.input" => { pricing.insert("claude_input".to_string(), decimal_val); },
                        "model_pricing.claude-sonnet-4-5.output" => { pricing.insert("claude_output".to_string(), decimal_val); },
                        "model_pricing.gemini-2.5-flash.input" => { pricing.insert("gemini_input".to_string(), decimal_val); },
                        "model_pricing.gemini-2.5-flash.output" => { pricing.insert("gemini_output".to_string(), decimal_val); },
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
        pricing: &std::collections::HashMap<String, rust_decimal::Decimal>
    ) -> rust_decimal::Decimal {
        use rust_decimal::prelude::*;
        
        let one_million = Decimal::from(1_000_000);
        
        let (input_price, output_price) = match model {
            "claude-sonnet-4-5" => (
                pricing.get("claude_input").cloned().unwrap_or(Decimal::from_str("3.00").unwrap()),
                pricing.get("claude_output").cloned().unwrap_or(Decimal::from_str("15.00").unwrap())
            ),
            "gemini-2.5-flash" => (
                pricing.get("gemini_input").cloned().unwrap_or(Decimal::from_str("3.50").unwrap()),
                pricing.get("gemini_output").cloned().unwrap_or(Decimal::from_str("10.50").unwrap())
            ),
            _ => (Decimal::from(1), Decimal::from(3)),
        };

        let input_cost = (Decimal::from(prompt_tokens) / one_million) * input_price;
        let output_cost = (Decimal::from(completion_tokens) / one_million) * output_price;
        
        input_cost + output_cost
    }

    /// Execute using Claude agent with ReAct pattern and user interruption support
    async fn execute_with_claude(
        &self,
        user_input: &str,
        session_id: &str,
        progress_callback: Arc<dyn Fn(f32, &str) + Send + Sync>,
        control_rx: &mut mpsc::UnboundedReceiver<JobControl>,
    ) -> Result<String, String> {
        // Use SimpleClaudeAgent with all 38 tools
        let claude_client_ref = self.app_state.claude_client.as_ref()
            .ok_or("Claude client not configured")?;
        let claude_client = Arc::new(claude_client_ref.clone());

        let agent = SimpleClaudeAgent::new(claude_client);

        // Send initial progress
        progress_callback(0.1, "🎬 Starting video editing agent...");

        // Execute agent (handles tool calling internally with up to 15 iterations)
        let user_input_clone = user_input.to_string();
        let session_id_clone = session_id.to_string();
        let app_state_clone = self.app_state.clone();
        let progress_callback_clone = progress_callback.clone();
        let mut agent_handle = tokio::spawn(async move {
            agent.execute(&user_input_clone, &session_id_clone, None, app_state_clone, Some(progress_callback_clone)).await
        });

        // Poll for control commands
        loop {
            tokio::select! {
                result = &mut agent_handle => {
                    return result.map_err(|e| format!("Agent task failed: {}", e))?;
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

    /// Execute using Gemini agent with simple tool execution
    async fn execute_with_gemini(
        &self,
        user_input: &str,
        session_id: &str,
        progress_callback: Arc<dyn Fn(f32, &str) + Send + Sync>,
        control_rx: &mut mpsc::UnboundedReceiver<JobControl>,
    ) -> Result<String, String> {
        // Use SimpleGeminiAgent with all 38 tools
        let gemini_client_ref = self.app_state.gemini_client.as_ref()
            .ok_or("Gemini client not configured")?;
        let gemini_client = Arc::new(gemini_client_ref.clone());

        let agent = SimpleGeminiAgent::new(gemini_client);

        // Send initial progress
        progress_callback(0.1, "🎬 Starting video editing agent...");

        // Execute agent (handles tool calling internally with up to 15 iterations)
        let user_input_clone = user_input.to_string();
        let session_id_clone = session_id.to_string();
        let app_state_clone = self.app_state.clone();
        let progress_callback_clone = progress_callback.clone();
        let mut agent_handle = tokio::spawn(async move {
            agent.execute(&user_input_clone, &session_id_clone, None, app_state_clone, Some(progress_callback_clone)).await
        });

        // Poll for control commands
        loop {
            tokio::select! {
                result = &mut agent_handle => {
                    return result.map_err(|e| format!("Agent task failed: {}", e))?;
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
        let session_id = self.job.session_id.clone();

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
                                    "⏸️ Job paused by user",
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
                                    "🛑 Job cancelled by user",
                                    JobStatus::Cancelled {
                                        cancelled_at_step: "User requested cancellation".to_string(),
                                    },
                                ).await;
                                return Err("Job cancelled by user".to_string());
                            }
                            JobControl::Resume => {
                                tracing::info!("▶️ Resuming job: {}", job_id);
                                self.send_progress(
                                    "▶️ Resuming job...",
                                    JobStatus::Running {
                                        current_step: "Resumed".to_string(),
                                        progress_percent: 50.0,
                                        steps_completed: 0,
                                        total_steps: 0,
                                    },
                                ).await;
                            }
                            JobControl::UpdateInput(new_input) => {
                                tracing::info!("🔄 User provided new input for job: {}", job_id);
                                self.send_progress(
                                    &format!("🔄 Updated task: {:?}", new_input),
                                    JobStatus::Running {
                                        current_step: "Updated by user".to_string(),
                                        progress_percent: 50.0,
                                        steps_completed: 0,
                                        total_steps: 0,
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
        let mut update = ProgressUpdate::new(self.job.id.clone(), message.to_string(), status.clone());

        // Include original user message in details for proper DB storage
        let user_message = self.job.input_data.get("raw_input")
            .and_then(|v| v.as_str())
            .unwrap_or("")
            .to_string();

        update.details = Some(serde_json::json!({
            "user_message": user_message,
        }));

        self.job_manager.update_job_status(&self.job.id, status).await;
        self.job_manager.send_progress(&self.job.session_id, update).await;
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
                tracing::error!("❌ Video editing job failed: {} - Error: {}", job_id_for_spawn, e);
            }
        }
        tracing::info!("🔥 EXITING tokio::spawn for job: {}", job_id_for_spawn);
    });

    tracing::info!("🚀 Spawned video editing job: {} for session: {}", job_id_stored, session_id);
    Ok(job_id_stored)
}
