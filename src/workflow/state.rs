// WorkflowState - Persistent state with reducers (LangGraph-inspired)
use serde::{Deserialize, Serialize};
use std::collections::HashMap;
use chrono::{DateTime, Utc};

/// State reducer strategy for merging state updates
#[derive(Debug, Clone, Serialize, Deserialize)]
pub enum ReducerStrategy {
    /// Replace old value with new value (default)
    Replace,
    /// Append new value to list (for arrays)
    Append,
    /// Merge maps/objects
    Merge,
    /// Keep first value (ignore updates)
    KeepFirst,
    /// Custom reducer function name
    Custom(String),
}

/// WorkflowState - The core state object passed between nodes
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct WorkflowState {
    /// Unique workflow execution ID
    pub workflow_id: String,

    /// Thread ID for checkpoint persistence (tenant:user:session)
    pub thread_id: String,

    /// Current node in the graph
    pub current_node: String,

    /// Conversation messages (appended, not replaced)
    pub messages: Vec<StateMessage>,

    /// Tool execution history
    pub tool_calls: Vec<ToolCall>,

    /// Intermediate results from nodes
    pub node_outputs: HashMap<String, serde_json::Value>,

    /// File references (input/output videos, images, etc.)
    pub files: HashMap<String, FileReference>,

    /// Error tracking for retry logic
    pub errors: Vec<WorkflowError>,
    pub error_count: usize,

    /// Metadata (user_id, session_id, etc.)
    pub metadata: HashMap<String, String>,

    /// Timestamps
    pub created_at: DateTime<Utc>,
    pub updated_at: DateTime<Utc>,

    /// Status tracking
    pub status: WorkflowStatus,

    /// Next action (for conditional routing)
    pub next: Option<String>,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub enum WorkflowStatus {
    Initializing,
    Running,
    AwaitingInput,  // Human-in-the-loop
    Retrying,
    Completed,
    Failed,
    Cancelled,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct StateMessage {
    pub role: String,  // user, assistant, system
    pub content: String,
    pub timestamp: DateTime<Utc>,
    pub metadata: HashMap<String, String>,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct ToolCall {
    pub tool_name: String,
    pub arguments: serde_json::Value,
    pub result: Option<String>,
    pub success: bool,
    pub timestamp: DateTime<Utc>,
    pub duration_ms: u64,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct FileReference {
    pub file_id: String,
    pub file_path: String,
    pub file_type: String,
    pub size_bytes: Option<u64>,
    pub created_by_node: String,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct WorkflowError {
    pub node: String,
    pub error_type: String,
    pub message: String,
    pub timestamp: DateTime<Utc>,
    pub recoverable: bool,
}

impl WorkflowState {
    /// Create new workflow state
    pub fn new(workflow_id: String, thread_id: String, user_input: String) -> Self {
        let now = Utc::now();
        Self {
            workflow_id,
            thread_id,
            current_node: "start".to_string(),
            messages: vec![StateMessage {
                role: "user".to_string(),
                content: user_input,
                timestamp: now,
                metadata: HashMap::new(),
            }],
            tool_calls: Vec::new(),
            node_outputs: HashMap::new(),
            files: HashMap::new(),
            errors: Vec::new(),
            error_count: 0,
            metadata: HashMap::new(),
            created_at: now,
            updated_at: now,
            status: WorkflowStatus::Initializing,
            next: None,
        }
    }

    /// Apply state update with reducer strategy
    pub fn apply_update(&mut self, updates: StateUpdate) {
        self.updated_at = Utc::now();

        // Update messages (always append)
        if let Some(new_messages) = updates.messages {
            self.messages.extend(new_messages);
        }

        // Update tool calls (always append)
        if let Some(new_calls) = updates.tool_calls {
            self.tool_calls.extend(new_calls);
        }

        // Update node outputs (merge strategy)
        if let Some(new_outputs) = updates.node_outputs {
            for (key, value) in new_outputs {
                self.node_outputs.insert(key, value);
            }
        }

        // Update files (merge strategy)
        if let Some(new_files) = updates.files {
            for (key, file) in new_files {
                self.files.insert(key, file);
            }
        }

        // Update errors (append)
        if let Some(new_errors) = updates.errors {
            self.error_count += new_errors.len();
            self.errors.extend(new_errors);
        }

        // Update metadata (merge)
        if let Some(new_metadata) = updates.metadata {
            for (key, value) in new_metadata {
                self.metadata.insert(key, value);
            }
        }

        // Update current node
        if let Some(node) = updates.current_node {
            self.current_node = node;
        }

        // Update status
        if let Some(status) = updates.status {
            self.status = status;
        }

        // Update next action
        if let Some(next) = updates.next {
            self.next = Some(next);
        }
    }

    /// Check if workflow should retry
    pub fn should_retry(&self, max_retries: usize) -> bool {
        self.error_count > 0 && self.error_count < max_retries
    }

    /// Get last user message
    pub fn get_last_user_message(&self) -> Option<&StateMessage> {
        self.messages.iter().rev().find(|m| m.role == "user")
    }

    /// Get conversation history
    pub fn get_conversation_history(&self, limit: usize) -> Vec<&StateMessage> {
        self.messages.iter().rev().take(limit).collect()
    }

    /// Get current node name
    pub fn get_current_node(&self) -> Option<String> {
        Some(self.current_node.clone())
    }

    /// Set current node
    pub fn set_current_node(&mut self, node: &str) {
        self.current_node = node.to_string();
        self.updated_at = Utc::now();
    }

    /// Check if workflow is completed
    pub fn is_completed(&self) -> bool {
        matches!(self.status, WorkflowStatus::Completed | WorkflowStatus::Failed | WorkflowStatus::Cancelled)
    }
}

/// State update payload
#[derive(Debug, Clone, Default, Serialize, Deserialize)]
pub struct StateUpdate {
    pub messages: Option<Vec<StateMessage>>,
    pub tool_calls: Option<Vec<ToolCall>>,
    pub node_outputs: Option<HashMap<String, serde_json::Value>>,
    pub files: Option<HashMap<String, FileReference>>,
    pub errors: Option<Vec<WorkflowError>>,
    pub metadata: Option<HashMap<String, String>>,
    pub current_node: Option<String>,
    pub status: Option<WorkflowStatus>,
    pub next: Option<String>,
}

impl StateUpdate {
    pub fn new() -> Self {
        Self::default()
    }

    pub fn with_message(mut self, role: &str, content: String) -> Self {
        let msg = StateMessage {
            role: role.to_string(),
            content,
            timestamp: Utc::now(),
            metadata: HashMap::new(),
        };
        self.messages = Some(vec![msg]);
        self
    }

    pub fn with_tool_call(mut self, call: ToolCall) -> Self {
        self.tool_calls = Some(vec![call]);
        self
    }

    pub fn with_node_output(mut self, node: String, output: serde_json::Value) -> Self {
        let mut outputs = HashMap::new();
        outputs.insert(node, output);
        self.node_outputs = Some(outputs);
        self
    }

    pub fn with_error(mut self, error: WorkflowError) -> Self {
        self.errors = Some(vec![error]);
        self
    }

    pub fn with_next(mut self, next: String) -> Self {
        self.next = Some(next);
        self
    }

    pub fn with_status(mut self, status: WorkflowStatus) -> Self {
        self.status = Some(status);
        self
    }
}
