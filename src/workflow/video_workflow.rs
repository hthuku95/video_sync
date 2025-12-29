// Video editing workflow - Simplified example using existing agent system
// This demonstrates how to use LangGraph-style workflow orchestration

use super::state::{WorkflowState, StateUpdate, WorkflowStatus};
use super::graph::{StateGraph, NodeType, NodeFunction, StateGraphBuilder};
use super::router::agent_decision_router;
use super::executor::{WorkflowExecutor, ExecutorBuilder, ExecutorConfig};
use super::checkpoint::WorkflowCheckpointer;
use crate::claude_client::ClaudeClient;
use crate::agent::simple_claude_agent::SimpleClaudeAgent;
use async_trait::async_trait;
use std::sync::Arc;

/// Agent node - Wraps existing SimpleClaudeAgent
#[derive(Clone)]
pub struct AgentNode {
    agent: Arc<SimpleClaudeAgent>,
    app_state: Arc<crate::AppState>,
}

impl AgentNode {
    pub fn new(claude_client: Arc<ClaudeClient>, app_state: Arc<crate::AppState>) -> Self {
        Self {
            agent: Arc::new(SimpleClaudeAgent::new(claude_client)),
            app_state,
        }
    }
}

#[async_trait]
impl NodeFunction for AgentNode {
    async fn execute(&self, state: &WorkflowState) -> Result<StateUpdate, String> {
        tracing::info!("🤖 Executing agent node...");

        // Get last user message
        let user_input = state.get_last_user_message()
            .map(|m| m.content.clone())
            .unwrap_or_else(|| "Continue processing".to_string());

        // Execute agent (this handles tool calling internally)
        match self.agent.execute(&user_input, "workflow-session", None, self.app_state.clone(), None).await {
            Ok(result) => {
                let mut update = StateUpdate::new()
                    .with_message("assistant", result.clone());

                // Check if workflow complete
                if result.contains("✅") || result.contains("submit_final_answer") {
                    update = update.with_status(WorkflowStatus::Completed);
                }

                Ok(update)
            }
            Err(e) => Err(format!("Agent execution failed: {}", e)),
        }
    }
}

/// Simple completion node
#[derive(Clone)]
pub struct CompleteNode;

#[async_trait]
impl NodeFunction for CompleteNode {
    async fn execute(&self, _state: &WorkflowState) -> Result<StateUpdate, String> {
        Ok(StateUpdate::new().with_status(WorkflowStatus::Completed))
    }
}

/// Build video editing workflow graph
pub fn build_video_workflow(
    claude_client: Arc<ClaudeClient>,
    app_state: Arc<crate::AppState>,
) -> Result<StateGraph, String> {
    let agent_node = Arc::new(AgentNode::new(claude_client, app_state));
    let complete_node = Arc::new(CompleteNode);

    let graph = StateGraphBuilder::new()
        .add_node(
            "agent",
            NodeType::Agent,
            agent_node,
            "AI agent for video editing reasoning"
        )
        .add_node(
            "complete",
            NodeType::End,
            complete_node,
            "Complete the workflow"
        )
        .set_entry_point("agent")
        // Agent continues until workflow marked as complete
        .add_edge("agent", "complete")
        .build()?;

    Ok(graph)
}

/// Create workflow executor with checkpointing
pub fn create_video_workflow_executor(
    claude_client: Arc<ClaudeClient>,
    app_state: Arc<crate::AppState>,
    checkpointer: Option<WorkflowCheckpointer>,
    max_iterations: usize,
) -> Result<WorkflowExecutor, String> {
    let graph = build_video_workflow(claude_client, app_state)?;

    let config = ExecutorConfig {
        max_iterations,
        checkpoint_every_n_steps: 3,
        enable_parallel: false,
        timeout_seconds: 600,
    };

    let executor = if let Some(cp) = checkpointer {
        ExecutorBuilder::new()
            .with_graph(graph)
            .with_checkpointer(cp)
            .with_config(config)
            .build()?
    } else {
        return Err("Checkpointer required for video workflow".to_string());
    };

    Ok(executor)
}

// =============================================================================
// USAGE EXAMPLES
// =============================================================================
//
// Example 1: Create and run a workflow
// ```rust,ignore
// let checkpointer = WorkflowCheckpointer::new(db_pool.clone());
// let executor = create_video_workflow_executor(
//     claude_client.clone(),
//     Some(checkpointer),
//     100  // max iterations - much higher than the old 15 limit!
// )?;
//
// let workflow_id = Uuid::new_v4().to_string();
// let thread_id = format!("user-{}:session-{}", user_id, session_id);
// let state = WorkflowState::new(workflow_id, thread_id, user_input);
//
// let final_state = executor.run(state).await?;
// ```
//
// Example 2: Resume from checkpoint
// ```rust,ignore
// let checkpoint_id = "workflow_123::1732893600000";
// let resumed_state = executor.resume_from_checkpoint(checkpoint_id).await?;
// ```
