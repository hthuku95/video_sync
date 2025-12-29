// Executor - Runs the workflow graph with checkpointing and retries
use super::state::{WorkflowState, StateUpdate, WorkflowStatus, WorkflowError};
use super::graph::{StateGraph, NodeType};
use super::checkpoint::WorkflowCheckpointer;
use tokio::time::{timeout, Duration};
use tracing::{info, warn, error};
use futures::future::join_all;
use chrono::Utc;

/// Workflow executor config
pub struct ExecutorConfig {
    pub max_iterations: usize,
    pub checkpoint_every_n_steps: usize,
    pub enable_parallel: bool,
    pub timeout_seconds: u64,
}

impl Default for ExecutorConfig {
    fn default() -> Self {
        Self {
            max_iterations: 100,  // Much higher than 15!
            checkpoint_every_n_steps: 5,
            enable_parallel: true,
            timeout_seconds: 600,
        }
    }
}

/// Workflow executor
pub struct WorkflowExecutor {
    graph: StateGraph,
    checkpointer: Option<WorkflowCheckpointer>,
    config: ExecutorConfig,
}

impl WorkflowExecutor {
    pub fn new(
        graph: StateGraph,
        checkpointer: Option<WorkflowCheckpointer>,
        config: ExecutorConfig,
    ) -> Self {
        if !graph.is_compiled() {
            panic!("Cannot create executor with uncompiled graph");
        }

        Self {
            graph,
            checkpointer,
            config,
        }
    }

    /// Run workflow to completion
    pub async fn run(&self, mut state: WorkflowState) -> Result<WorkflowState, String> {
        info!("🚀 Starting workflow execution: {}", state.workflow_id);
        state.status = WorkflowStatus::Running;

        // Start from entry point or resume from checkpoint
        let mut current_node = if state.current_node == "start" {
            self.graph.get_entry_point()
                .ok_or("No entry point")?
                .clone()
        } else {
            state.current_node.clone()
        };

        let mut iteration = 0;

        loop {
            iteration += 1;

            // Check iteration limit
            if iteration > self.config.max_iterations {
                warn!("⚠️ Workflow hit iteration limit: {}", self.config.max_iterations);
                state.status = WorkflowStatus::Failed;
                state.errors.push(WorkflowError {
                    node: current_node.clone(),
                    error_type: "IterationLimitExceeded".to_string(),
                    message: format!("Exceeded max iterations: {}", self.config.max_iterations),
                    timestamp: Utc::now(),
                    recoverable: false,
                });
                break;
            }

            info!("📍 Step {}: Executing node '{}'", iteration, current_node);

            // Get node
            let node = self.graph.get_node(&current_node)
                .ok_or(format!("Node '{}' not found", current_node))?;

            // Update state with current node
            state.current_node = current_node.clone();

            // Execute node with timeout and retry
            let update = match self.execute_node_with_retry(node, &state).await {
                Ok(update) => update,
                Err(e) => {
                    error!("❌ Node '{}' failed: {}", current_node, e);
                    state.errors.push(WorkflowError {
                        node: current_node.clone(),
                        error_type: "NodeExecutionError".to_string(),
                        message: e.clone(),
                        timestamp: Utc::now(),
                        recoverable: true,
                    });
                    state.error_count += 1;

                    // Check if should retry
                    if state.should_retry(node.max_retries) {
                        state.status = WorkflowStatus::Retrying;
                        warn!("🔄 Retrying node '{}' (attempt {}/{})",
                            current_node, state.error_count, node.max_retries);
                        continue;
                    } else {
                        state.status = WorkflowStatus::Failed;
                        break;
                    }
                }
            };

            // Apply state update
            state.apply_update(update);

            // Checkpoint periodically
            if iteration % self.config.checkpoint_every_n_steps == 0 {
                if let Some(ref checkpointer) = self.checkpointer {
                    match checkpointer.save(&state.workflow_id, &state.thread_id, &state).await {
                        Ok(checkpoint_id) => {
                            info!("💾 Checkpoint saved at step {}: {}", iteration, checkpoint_id);
                        }
                        Err(e) => {
                            warn!("⚠️ Failed to save checkpoint: {}", e);
                        }
                    }
                }
            }

            // Check for terminal states
            match state.status {
                WorkflowStatus::Completed => {
                    info!("✅ Workflow completed successfully");
                    break;
                }
                WorkflowStatus::Failed => {
                    error!("❌ Workflow failed");
                    break;
                }
                WorkflowStatus::AwaitingInput => {
                    info!("⏸️ Workflow paused for human input");
                    break;
                }
                _ => {}
            }

            // Get next node(s)
            let next_nodes = self.graph.get_next_nodes(&current_node, &state);

            if next_nodes.is_empty() {
                info!("🏁 Reached end node");
                state.status = WorkflowStatus::Completed;
                break;
            }

            // Handle parallel execution
            if next_nodes.len() > 1 && self.config.enable_parallel {
                state = self.execute_parallel_nodes(&next_nodes, state).await?;
                // After parallel, move to merge node (if exists)
                if let Some(merge_node) = state.next.clone() {
                    current_node = merge_node;
                } else {
                    state.status = WorkflowStatus::Completed;
                    break;
                }
            } else {
                current_node = next_nodes[0].clone();
            }
        }

        // Final checkpoint
        if let Some(ref checkpointer) = self.checkpointer {
            let _ = checkpointer.save(&state.workflow_id, &state.thread_id, &state).await;
        }

        info!("🎬 Workflow execution finished: {} (iterations: {})",
            state.workflow_id, iteration);

        Ok(state)
    }

    /// Resume workflow from checkpoint
    pub async fn resume_from_checkpoint(
        &self,
        checkpoint_id: &str,
    ) -> Result<WorkflowState, String> {
        let checkpointer = self.checkpointer.as_ref()
            .ok_or("No checkpointer configured")?;

        let checkpoint = checkpointer.load_by_id(checkpoint_id).await?
            .ok_or(format!("Checkpoint '{}' not found", checkpoint_id))?;

        info!("🔄 Resuming workflow from checkpoint: {}", checkpoint_id);
        self.run(checkpoint.state).await
    }

    /// Execute node with retry logic
    async fn execute_node_with_retry(
        &self,
        node: &super::graph::Node,
        state: &WorkflowState,
    ) -> Result<StateUpdate, String> {
        let node_timeout = Duration::from_secs(node.timeout_seconds);

        match timeout(node_timeout, node.function.execute(state)).await {
            Ok(Ok(update)) => Ok(update),
            Ok(Err(e)) => Err(e),
            Err(_) => Err(format!("Node '{}' timed out after {}s",
                node.id, node.timeout_seconds)),
        }
    }

    /// Execute multiple nodes in parallel
    /// Note: For production use, parallel execution requires careful state management
    /// Currently processes sequentially for safety and simplicity
    async fn execute_parallel_nodes(
        &self,
        node_ids: &[String],
        mut state: WorkflowState,
    ) -> Result<WorkflowState, String> {
        info!("⚡ Executing {} nodes sequentially (parallel disabled for safety)", node_ids.len());

        // Execute sequentially for now - proper parallel execution requires
        // more complex lifetime and state cloning management
        for node_id in node_ids {
            let node = self.graph.get_node(node_id)
                .ok_or(format!("Parallel node '{}' not found", node_id))?;

            match node.function.execute(&state).await {
                Ok(update) => {
                    state.apply_update(update);
                }
                Err(e) => {
                    warn!("⚠️ Node {} failed: {}", node_id, e);
                    state.errors.push(WorkflowError {
                        node: node_id.clone(),
                        error_type: "NodeExecutionError".to_string(),
                        message: e,
                        timestamp: Utc::now(),
                        recoverable: false,
                    });
                }
            }
        }

        Ok(state)
    }

    /// Stream workflow execution (for real-time updates via WebSocket)
    /// This is a placeholder for future WebSocket integration
    pub async fn run_streaming<F>(
        &self,
        state: WorkflowState,
        mut callback: F,
    ) -> Result<WorkflowState, String>
    where
        F: FnMut(&WorkflowState, &str) + Send,
    {
        // Run workflow with streaming updates
        let mut current_state = state.clone();

        loop {
            // Get next node to execute
            let current_node_name = current_state.get_current_node()
                .unwrap_or_else(|| self.graph.get_entry_point().map(|s| s.clone()).unwrap_or_default());

            let node = match self.graph.get_node(&current_node_name) {
                Some(n) => n,
                None => {
                    callback(&current_state, "No more nodes to execute");
                    break;
                }
            };

            // Stream update
            callback(&current_state, &format!("Executing node: {}", current_node_name));

            // Execute node
            let result = node.function.execute(&current_state).await;

            match result {
                Ok(update) => {
                    current_state.apply_update(update);

                    // Check if workflow completed
                    if current_state.is_completed() {
                        callback(&current_state, "Workflow completed");
                        break;
                    }

                    // Get next node
                    if let Some(next_node) = self.graph.get_next_node(&current_node_name, &current_state) {
                        current_state.set_current_node(&next_node);
                    } else {
                        callback(&current_state, "No next node found");
                        break;
                    }
                }
                Err(e) => {
                    callback(&current_state, &format!("Error: {}", e));
                    return Err(e);
                }
            }

            // Checkpoint if enabled
            if let Some(ref checkpointer) = self.checkpointer {
                if let Err(e) = checkpointer.save_checkpoint(&current_state).await {
                    tracing::warn!("Failed to save checkpoint: {}", e);
                }
            }
        }

        Ok(current_state)
    }
}

/// Builder for workflow executor
pub struct ExecutorBuilder {
    graph: Option<StateGraph>,
    checkpointer: Option<WorkflowCheckpointer>,
    config: ExecutorConfig,
}

impl ExecutorBuilder {
    pub fn new() -> Self {
        Self {
            graph: None,
            checkpointer: None,
            config: ExecutorConfig::default(),
        }
    }

    pub fn with_graph(mut self, graph: StateGraph) -> Self {
        self.graph = Some(graph);
        self
    }

    pub fn with_checkpointer(mut self, checkpointer: WorkflowCheckpointer) -> Self {
        self.checkpointer = Some(checkpointer);
        self
    }

    pub fn with_config(mut self, config: ExecutorConfig) -> Self {
        self.config = config;
        self
    }

    pub fn max_iterations(mut self, max: usize) -> Self {
        self.config.max_iterations = max;
        self
    }

    pub fn checkpoint_every(mut self, n: usize) -> Self {
        self.config.checkpoint_every_n_steps = n;
        self
    }

    pub fn build(self) -> Result<WorkflowExecutor, String> {
        let graph = self.graph.ok_or("Graph not set")?;
        Ok(WorkflowExecutor::new(graph, self.checkpointer, self.config))
    }
}

impl Default for ExecutorBuilder {
    fn default() -> Self {
        Self::new()
    }
}
