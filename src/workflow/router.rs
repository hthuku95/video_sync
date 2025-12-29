// Router - Conditional routing and decision logic
use super::state::{WorkflowState, WorkflowStatus};
use std::sync::Arc;

/// Common routing functions for workflows

/// Route to retry node if errors exist and under retry limit
pub fn retry_router(max_retries: usize) -> Arc<dyn Fn(&WorkflowState) -> Option<String> + Send + Sync> {
    Arc::new(move |state: &WorkflowState| {
        if state.error_count > 0 && state.error_count < max_retries {
            Some("retry".to_string())
        } else if state.error_count >= max_retries {
            Some("error_handler".to_string())
        } else {
            Some("continue".to_string())
        }
    })
}

/// Route based on workflow status
pub fn status_router() -> Arc<dyn Fn(&WorkflowState) -> Option<String> + Send + Sync> {
    Arc::new(|state: &WorkflowState| {
        match state.status {
            WorkflowStatus::Completed => Some("end".to_string()),
            WorkflowStatus::Failed => Some("error_handler".to_string()),
            WorkflowStatus::AwaitingInput => Some("human_input".to_string()),
            WorkflowStatus::Retrying => Some("retry".to_string()),
            _ => Some("continue".to_string()),
        }
    })
}

/// Route based on tool execution success
pub fn tool_success_router() -> Arc<dyn Fn(&WorkflowState) -> Option<String> + Send + Sync> {
    Arc::new(|state: &WorkflowState| {
        if let Some(last_call) = state.tool_calls.last() {
            if last_call.success {
                Some("success".to_string())
            } else {
                Some("tool_error".to_string())
            }
        } else {
            Some("no_tools".to_string())
        }
    })
}

/// Route based on next field in state (agent decision)
pub fn agent_decision_router() -> Arc<dyn Fn(&WorkflowState) -> Option<String> + Send + Sync> {
    Arc::new(|state: &WorkflowState| {
        state.next.clone()
    })
}

/// Custom router builder
pub struct RouterBuilder {
    conditions: Vec<(Box<dyn Fn(&WorkflowState) -> bool + Send + Sync>, String)>,
    default: Option<String>,
}

impl RouterBuilder {
    pub fn new() -> Self {
        Self {
            conditions: Vec::new(),
            default: None,
        }
    }

    /// Add condition with target node
    pub fn when<F>(mut self, condition: F, target: &str) -> Self
    where
        F: Fn(&WorkflowState) -> bool + Send + Sync + 'static,
    {
        self.conditions.push((Box::new(condition), target.to_string()));
        self
    }

    /// Set default target if no conditions match
    pub fn otherwise(mut self, target: &str) -> Self {
        self.default = Some(target.to_string());
        self
    }

    /// Build router function
    pub fn build(self) -> Arc<dyn Fn(&WorkflowState) -> Option<String> + Send + Sync> {
        Arc::new(move |state: &WorkflowState| {
            for (condition, target) in &self.conditions {
                if condition(state) {
                    return Some(target.clone());
                }
            }
            self.default.clone()
        })
    }
}

impl Default for RouterBuilder {
    fn default() -> Self {
        Self::new()
    }
}

/// Predefined condition helpers
pub mod conditions {
    use super::*;

    pub fn has_errors() -> Box<dyn Fn(&WorkflowState) -> bool + Send + Sync> {
        Box::new(|state: &WorkflowState| state.error_count > 0)
    }

    pub fn under_retry_limit(max: usize) -> Box<dyn Fn(&WorkflowState) -> bool + Send + Sync> {
        Box::new(move |state: &WorkflowState| state.error_count < max)
    }

    pub fn has_file_type(file_type: String) -> Box<dyn Fn(&WorkflowState) -> bool + Send + Sync> {
        Box::new(move |state: &WorkflowState| {
            state.files.values().any(|f| f.file_type == file_type)
        })
    }

    pub fn message_contains(keyword: String) -> Box<dyn Fn(&WorkflowState) -> bool + Send + Sync> {
        Box::new(move |state: &WorkflowState| {
            state.messages.iter().any(|m| m.content.contains(&keyword))
        })
    }

    pub fn tool_succeeded(tool_name: String) -> Box<dyn Fn(&WorkflowState) -> bool + Send + Sync> {
        Box::new(move |state: &WorkflowState| {
            state.tool_calls.iter().any(|tc| tc.tool_name == tool_name && tc.success)
        })
    }
}
