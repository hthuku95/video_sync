// ReAct Agent Implementation - Thought → Action → Observation → Reflection
// Supports user interruption and real-time reasoning updates

use super::react_state::{AgentState, AgentContext, UserCommand};
use super::tool_executor::{execute_tool_claude, execute_tool_gemini};
use crate::claude_client::{ClaudeClient, ClaudeMessage, ClaudeContent, ContentBlock};
use crate::gemini_client::{GeminiClient, GenerateContentRequest, Content, Part, Tool, GenerationConfig, ToolConfig, FunctionCallingConfig, FunctionCallingMode};
use std::sync::Arc;
use tokio::sync::mpsc;
use serde_json::json;

pub struct ReActClaudeAgent {
    client: Arc<ClaudeClient>,
    context: AgentContext,
}

impl ReActClaudeAgent {
    pub fn new(client: Arc<ClaudeClient>, session_id: String, user_request: String) -> Self {
        Self {
            client,
            context: AgentContext::new(session_id, user_request),
        }
    }

    pub async fn execute_with_state(
        &mut self,
        progress_tx: mpsc::UnboundedSender<AgentState>,
        mut user_command_rx: mpsc::UnboundedReceiver<UserCommand>,
    ) -> Result<String, String> {
        let tools = crate::claude_client::ClaudeClient::create_video_editing_tools();
        let mut messages: Vec<ClaudeMessage> = vec![];

        // PHASE 1: PLANNING
        self.context.transition_to(AgentState::Planning {
            user_request: "Analyzing request...".to_string(),
            analysis: String::new(),
            identified_steps: vec![],
        });
        let _ = progress_tx.send(self.context.current_state.clone());

        let planning_prompt = format!(
            "Analyze this video editing request and break it down into specific steps:\n\n{}\n\nList the exact tools and sequence needed.",
            self.get_user_request()
        );

        messages.push(ClaudeMessage {
            role: "user".to_string(),
            content: ClaudeContent::Text(planning_prompt),
        });

        let planning_response = self.client.generate_content(
            messages.clone(),
            None, // No tools yet, just planning
            Some(self.get_system_prompt()),
        ).await.map_err(|e| format!("Planning failed: {}", e))?;

        let plan = self.extract_text_from_response(&planning_response.content);
        let steps = self.extract_steps_from_plan(&plan);

        self.context.transition_to(AgentState::Planning {
            user_request: self.get_user_request(),
            analysis: plan.clone(),
            identified_steps: steps.clone(),
        });
        let _ = progress_tx.send(self.context.current_state.clone());

        // PHASE 2: EXECUTION LOOP (ReAct Pattern)
        let total_steps = steps.len().max(1);
        let mut current_step = 1;
        let max_iterations = 15;
        let mut iterations = 0;

        // Add actual user request for execution
        messages.clear();
        messages.push(ClaudeMessage {
            role: "user".to_string(),
            content: ClaudeContent::Text(self.get_user_request()),
        });

        let mut final_text = String::new();

        while iterations < max_iterations {
            iterations += 1;

            // Check for user interruption
            if let Ok(command) = user_command_rx.try_recv() {
                match command {
                    UserCommand::Question(q) => {
                        // Send current state as response
                        let state_msg = self.context.current_state.to_user_message();
                        let _ = progress_tx.send(AgentState::Interrupted {
                            original_state: Box::new(self.context.current_state.clone()),
                            user_message: format!("User asked: {}\n\nCurrent status: {}", q, state_msg),
                            timestamp: chrono::Utc::now(),
                        });
                        continue;
                    }
                    UserCommand::NewInstruction(new_inst) => {
                        self.context.transition_to(AgentState::Interrupted {
                            original_state: Box::new(self.context.current_state.clone()),
                            user_message: new_inst.clone(),
                            timestamp: chrono::Utc::now(),
                        });
                        let _ = progress_tx.send(self.context.current_state.clone());

                        // Add new instruction to conversation
                        messages.push(ClaudeMessage {
                            role: "user".to_string(),
                            content: ClaudeContent::Text(format!("NEW INSTRUCTION: {}", new_inst)),
                        });
                        continue;
                    }
                    UserCommand::Cancel => {
                        return Err("Cancelled by user".to_string());
                    }
                    _ => {}
                }
            }

            // THINKING PHASE
            self.context.transition_to(AgentState::Thinking {
                thought: format!("Considering next action for step {}/{}", current_step, total_steps),
                reasoning: "Analyzing current situation and determining best tool to use...".to_string(),
                step_number: current_step,
                total_steps,
            });
            let _ = progress_tx.send(self.context.current_state.clone());

            // ACTION PHASE
            let response = self.client.generate_content(
                messages.clone(),
                Some(tools.clone()),
                Some(self.get_system_prompt()),
            ).await.map_err(|e| format!("Claude API Error: {}", e))?;

            let mut has_tool_calls = false;
            let mut tool_results = vec![];
            let mut assistant_blocks = vec![];

            for content in &response.content {
                match content {
                    crate::claude_client::ResponseContent::Text { text } => {
                        final_text = text.clone();
                        assistant_blocks.push(ContentBlock::Text { text: text.clone() });
                    }
                    crate::claude_client::ResponseContent::ToolUse { id, name, input } => {
                        has_tool_calls = true;

                        // EXECUTING PHASE
                        self.context.transition_to(AgentState::Executing {
                            action: format!("Calling {} tool", name),
                            tool_name: name.clone(),
                            tool_args: input.clone(),
                            step_number: current_step,
                        });
                        let _ = progress_tx.send(self.context.current_state.clone());

                        assistant_blocks.push(ContentBlock::ToolUse {
                            id: id.clone(),
                            name: name.clone(),
                            input: input.clone(),
                        });

                        let result = execute_tool_claude(name, input).await;

                        // OBSERVATION PHASE
                        self.context.transition_to(AgentState::Observing {
                            observation: format!("Tool {} returned results", name),
                            tool_output: result.clone(),
                            step_number: current_step,
                        });
                        let _ = progress_tx.send(self.context.current_state.clone());

                        tool_results.push(ContentBlock::ToolResult {
                            tool_use_id: id.clone(),
                            content: result.clone(),
                            is_error: None,
                        });

                        // Track output files
                        if result.contains("✅") && result.contains("outputs/") {
                            if let Some(path) = self.extract_output_path(&result) {
                                self.context.add_output_file(path);
                            }
                        }
                    }
                }
            }

            messages.push(ClaudeMessage {
                role: "assistant".to_string(),
                content: ClaudeContent::Blocks(assistant_blocks),
            });

            if !has_tool_calls {
                // FINAL REFLECTION
                self.context.transition_to(AgentState::Reflecting {
                    reflection: "All tasks completed".to_string(),
                    progress_evaluation: format!("Successfully completed {} steps", current_step - 1),
                    next_action_decision: "Ready to present final answer".to_string(),
                    step_number: current_step,
                });
                let _ = progress_tx.send(self.context.current_state.clone());
                break;
            }

            // Add tool results
            if !tool_results.is_empty() {
                messages.push(ClaudeMessage {
                    role: "user".to_string(),
                    content: ClaudeContent::Blocks(tool_results),
                });
            }

            // REFLECTION PHASE
            self.context.transition_to(AgentState::Reflecting {
                reflection: format!("Completed step {}, evaluating progress", current_step),
                progress_evaluation: format!("{} of {} estimated steps done", current_step, total_steps),
                next_action_decision: if current_step < total_steps {
                    "Proceeding to next step".to_string()
                } else {
                    "Approaching completion".to_string()
                },
                step_number: current_step,
            });
            let _ = progress_tx.send(self.context.current_state.clone());

            current_step += 1;
        }

        // COMPLETION
        self.context.transition_to(AgentState::Completed {
            summary: final_text.clone(),
            outputs: self.context.output_files.clone(),
            total_steps_executed: current_step - 1,
        });
        let _ = progress_tx.send(self.context.current_state.clone());

        Ok(final_text)
    }

    fn get_system_prompt(&self) -> String {
        format!(
            "You are a professional video editing AI agent using the ReAct (Reasoning + Action) pattern.

For each step, you should:
1. **Think** about what needs to be done
2. **Act** by calling the appropriate tool
3. **Observe** the result
4. **Reflect** on whether you're making progress

You have access to {} video editing tools. Use them to complete multi-step requests.

Current session files: {}

Always explain your reasoning before acting. Be transparent about your decision-making process.",
            38,
            self.format_uploaded_files()
        )
    }

    fn get_user_request(&self) -> String {
        match &self.context.current_state {
            AgentState::Planning { user_request, .. } => user_request.clone(),
            _ => self.context.state_history.first()
                .and_then(|s| match s {
                    AgentState::Planning { user_request, .. } => Some(user_request.clone()),
                    _ => None,
                })
                .unwrap_or_default(),
        }
    }

    fn extract_text_from_response(&self, content: &[crate::claude_client::ResponseContent]) -> String {
        content.iter()
            .filter_map(|c| match c {
                crate::claude_client::ResponseContent::Text { text } => Some(text.clone()),
                _ => None,
            })
            .collect::<Vec<_>>()
            .join("\n")
    }

    fn extract_steps_from_plan(&self, plan: &str) -> Vec<String> {
        plan.lines()
            .filter(|line| line.trim().starts_with(|c: char| c.is_numeric()))
            .map(|line| line.trim().to_string())
            .collect()
    }

    fn extract_output_path(&self, result: &str) -> Option<String> {
        result.split("outputs/")
            .nth(1)
            .and_then(|s| s.split_whitespace().next())
            .map(|s| format!("outputs/{}", s))
    }

    fn format_uploaded_files(&self) -> String {
        if self.context.uploaded_files.is_empty() {
            "None".to_string()
        } else {
            self.context.uploaded_files.iter()
                .map(|f| format!("{} ({})", f.name, f.path))
                .collect::<Vec<_>>()
                .join(", ")
        }
    }
}

pub struct ReActGeminiAgent {
    client: Arc<GeminiClient>,
    context: AgentContext,
}

impl ReActGeminiAgent {
    pub fn new(client: Arc<GeminiClient>, session_id: String, user_request: String) -> Self {
        Self {
            client,
            context: AgentContext::new(session_id, user_request),
        }
    }

    pub async fn execute_with_state(
        &mut self,
        progress_tx: mpsc::UnboundedSender<AgentState>,
        mut user_command_rx: mpsc::UnboundedReceiver<UserCommand>,
    ) -> Result<String, String> {
        let tools = crate::gemini_client::GeminiClient::create_video_editing_tools();
        let mut conversation: Vec<Content> = vec![];

        // PHASE 1: PLANNING
        self.context.transition_to(AgentState::Planning {
            user_request: self.get_user_request(),
            analysis: "Breaking down the request...".to_string(),
            identified_steps: vec![],
        });
        let _ = progress_tx.send(self.context.current_state.clone());

        conversation.push(Content {
            parts: vec![Part::Text { text: self.get_user_request() }],
            role: Some("user".to_string()),
        });

        let mut iterations = 0;
        let max_iterations = 15;
        let mut current_step = 1;
        let total_steps = 10; // Estimated
        let mut final_text = String::new();

        while iterations < max_iterations {
            iterations += 1;

            // Check for user interruption
            if let Ok(command) = user_command_rx.try_recv() {
                match command {
                    UserCommand::Question(q) => {
                        let state_msg = self.context.current_state.to_user_message();
                        let _ = progress_tx.send(AgentState::Interrupted {
                            original_state: Box::new(self.context.current_state.clone()),
                            user_message: format!("User asked: {}\n\nCurrent: {}", q, state_msg),
                            timestamp: chrono::Utc::now(),
                        });
                        continue;
                    }
                    UserCommand::NewInstruction(new_inst) => {
                        self.context.transition_to(AgentState::Interrupted {
                            original_state: Box::new(self.context.current_state.clone()),
                            user_message: new_inst.clone(),
                            timestamp: chrono::Utc::now(),
                        });
                        let _ = progress_tx.send(self.context.current_state.clone());

                        conversation.push(Content {
                            parts: vec![Part::Text { text: format!("NEW INSTRUCTION: {}", new_inst) }],
                            role: Some("user".to_string()),
                        });
                        continue;
                    }
                    UserCommand::Cancel => return Err("Cancelled by user".to_string()),
                    _ => {}
                }
            }

            // THINKING PHASE
            self.context.transition_to(AgentState::Thinking {
                thought: format!("Analyzing what to do next..."),
                reasoning: "Determining appropriate tool for current objective".to_string(),
                step_number: current_step,
                total_steps,
            });
            let _ = progress_tx.send(self.context.current_state.clone());

            // ACTION PHASE
            let request = GenerateContentRequest {
                contents: conversation.clone(),
                tools: Some(vec![Tool { function_declarations: tools.iter().cloned().collect() }]),
                generation_config: Some(GenerationConfig {
                    temperature: 0.3,
                    top_k: 40,
                    top_p: 0.9,
                    max_output_tokens: 4096,
                }),
                tool_config: Some(ToolConfig {
                    function_calling_config: FunctionCallingConfig {
                        mode: FunctionCallingMode::Any,  // CRITICAL FIX: Force tool calling like Claude does
                    },
                }),
            };

            let response = self.client.generate_content(request).await
                .map_err(|e| format!("Gemini API Error: {}", e))?;

            if let Some(candidate) = response.candidates.first() {
                if let Some(ref content) = candidate.content {
                    let mut has_tool_calls = false;
                    let mut tool_results = vec![];

                    for part in &content.parts {
                    match part {
                        Part::Text { text } => {
                            final_text = text.clone();
                        }
                        Part::FunctionCall { function_call } => {
                            has_tool_calls = true;

                            // EXECUTING PHASE
                            self.context.transition_to(AgentState::Executing {
                                action: format!("Executing {}", function_call.name),
                                tool_name: function_call.name.clone(),
                                tool_args: json!(function_call.args),
                                step_number: current_step,
                            });
                            let _ = progress_tx.send(self.context.current_state.clone());

                            let result = execute_tool_gemini(&function_call.name, &function_call.args).await;

                            // OBSERVATION PHASE
                            self.context.transition_to(AgentState::Observing {
                                observation: format!("Received results from {}", function_call.name),
                                tool_output: result.clone(),
                                step_number: current_step,
                            });
                            let _ = progress_tx.send(self.context.current_state.clone());

                            tool_results.push(Part::FunctionResponse {
                                function_response: crate::gemini_client::FunctionResponse {
                                    name: function_call.name.clone(),
                                    response: {
                                        let mut map = std::collections::HashMap::new();
                                        map.insert("result".to_string(), serde_json::Value::String(result));
                                        map
                                    },
                                    thought_signature: function_call.thought_signature.clone(),
                                },
                            });

                            current_step += 1;
                        }
                        _ => {}
                    }
                }

                conversation.push(content.clone());

                if !has_tool_calls {
                    break;
                }

                if !tool_results.is_empty() {
                    conversation.push(Content {
                        parts: tool_results,
                        role: Some("user".to_string()),
                    });

                    // REFLECTION PHASE
                    self.context.transition_to(AgentState::Reflecting {
                        reflection: "Evaluating progress...".to_string(),
                        progress_evaluation: format!("Completed {} actions so far", current_step - 1),
                        next_action_decision: "Determining if more work needed".to_string(),
                        step_number: current_step,
                    });
                    let _ = progress_tx.send(self.context.current_state.clone());
                }
                } else {
                    // No content in response
                    tracing::warn!("Gemini response has no content");
                    break;
                }
            }
        }

        // COMPLETION
        self.context.transition_to(AgentState::Completed {
            summary: final_text.clone(),
            outputs: self.context.output_files.clone(),
            total_steps_executed: current_step - 1,
        });
        let _ = progress_tx.send(self.context.current_state.clone());

        Ok(final_text)
    }

    fn get_user_request(&self) -> String {
        match &self.context.current_state {
            AgentState::Planning { user_request, .. } => user_request.clone(),
            _ => "Video editing task".to_string(),
        }
    }
}
