// ReAct Agent State Management (LangGraph-inspired)
// Implements Thought → Action → Observation → Reflection cycle

use serde::{Serialize, Deserialize};
use chrono::{DateTime, Utc};
use std::collections::HashMap;

/// Agent execution state following ReAct pattern
#[derive(Debug, Clone, Serialize, Deserialize, PartialEq)]
#[serde(tag = "phase", rename_all = "lowercase")]
pub enum AgentState {
    /// Initial planning phase - analyzing user request
    Planning {
        user_request: String,
        analysis: String,
        identified_steps: Vec<String>,
    },
    /// Reasoning phase - thinking about next action
    Thinking {
        thought: String,
        reasoning: String,
        step_number: usize,
        total_steps: usize,
    },
    /// Action phase - executing a tool
    Executing {
        action: String,
        tool_name: String,
        tool_args: serde_json::Value,
        step_number: usize,
    },
    /// Observation phase - processing tool results
    Observing {
        observation: String,
        tool_output: String,
        step_number: usize,
    },
    /// Reflection phase - evaluating progress and deciding next step
    Reflecting {
        reflection: String,
        progress_evaluation: String,
        next_action_decision: String,
        step_number: usize,
    },
    /// User interrupted with new instruction
    Interrupted {
        original_state: Box<AgentState>,
        user_message: String,
        timestamp: DateTime<Utc>,
    },
    /// Final completion phase
    Completed {
        summary: String,
        outputs: Vec<String>,
        total_steps_executed: usize,
    },
    /// Error state
    Failed {
        error: String,
        last_state: Box<AgentState>,
    },
}

impl AgentState {
    pub fn initial_planning(user_request: String) -> Self {
        AgentState::Planning {
            user_request,
            analysis: String::new(),
            identified_steps: vec![],
        }
    }

    pub fn to_user_message(&self) -> String {
        match self {
            AgentState::Planning { user_request, analysis, identified_steps } => {
                let mut msg = format!("🧠 **Planning Phase**\n\nAnalyzing: \"{}\"\n\n", user_request);
                if !analysis.is_empty() {
                    msg.push_str(&format!("**Analysis**: {}\n\n", analysis));
                }
                if !identified_steps.is_empty() {
                    msg.push_str("**Identified Steps**:\n");
                    for (i, step) in identified_steps.iter().enumerate() {
                        msg.push_str(&format!("{}. {}\n", i + 1, step));
                    }
                }
                msg
            }
            AgentState::Thinking { thought, reasoning, step_number, total_steps } => {
                format!(
                    "💭 **Thinking** (Step {}/{})\n\n**Thought**: {}\n\n**Reasoning**: {}",
                    step_number, total_steps, thought, reasoning
                )
            }
            AgentState::Executing { action, tool_name, tool_args, step_number } => {
                format!(
                    "🔧 **Executing Action** (Step {})\n\n**Action**: {}\n**Tool**: {}\n**Parameters**: {}",
                    step_number, action, tool_name, serde_json::to_string_pretty(tool_args).unwrap_or_default()
                )
            }
            AgentState::Observing { observation, tool_output, step_number } => {
                format!(
                    "👁️ **Observation** (Step {})\n\n**Result**: {}\n\n**Output**: {}",
                    step_number, observation, tool_output
                )
            }
            AgentState::Reflecting { reflection, progress_evaluation, next_action_decision, step_number } => {
                format!(
                    "🤔 **Reflection** (Step {})\n\n**Evaluation**: {}\n\n**Progress**: {}\n\n**Next**: {}",
                    step_number, reflection, progress_evaluation, next_action_decision
                )
            }
            AgentState::Interrupted { user_message, timestamp, .. } => {
                format!(
                    "⏸️ **Interrupted by User** at {}\n\n**New Instruction**: {}",
                    timestamp.format("%H:%M:%S"), user_message
                )
            }
            AgentState::Completed { summary, outputs, total_steps_executed } => {
                let mut msg = format!("✅ **Completed** ({} steps executed)\n\n{}\n\n", total_steps_executed, summary);
                if !outputs.is_empty() {
                    msg.push_str("**Output Files**:\n");
                    for output in outputs {
                        msg.push_str(&format!("📁 {}\n", output));
                    }
                }
                msg
            }
            AgentState::Failed { error, .. } => {
                format!("❌ **Failed**\n\n**Error**: {}", error)
            }
        }
    }
}

/// Agent context - shared state across execution
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct AgentContext {
    pub session_id: String,
    pub current_state: AgentState,
    pub state_history: Vec<AgentState>,
    pub conversation_context: String,
    pub uploaded_files: Vec<FileInfo>,
    pub output_files: Vec<String>,
    pub metadata: HashMap<String, serde_json::Value>,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct FileInfo {
    pub name: String,
    pub path: String,
    pub file_type: String,
}

impl AgentContext {
    pub fn new(session_id: String, user_request: String) -> Self {
        Self {
            session_id,
            current_state: AgentState::initial_planning(user_request),
            state_history: vec![],
            conversation_context: String::new(),
            uploaded_files: vec![],
            output_files: vec![],
            metadata: HashMap::new(),
        }
    }

    pub fn transition_to(&mut self, new_state: AgentState) {
        let old_state = std::mem::replace(&mut self.current_state, new_state.clone());
        self.state_history.push(old_state);
    }

    pub fn get_current_step(&self) -> usize {
        match &self.current_state {
            AgentState::Thinking { step_number, .. } |
            AgentState::Executing { step_number, .. } |
            AgentState::Observing { step_number, .. } |
            AgentState::Reflecting { step_number, .. } => *step_number,
            _ => 0,
        }
    }

    pub fn add_output_file(&mut self, file_path: String) {
        self.output_files.push(file_path);
    }
}

/// User control commands during agent execution
#[derive(Debug, Clone, Serialize, Deserialize)]
pub enum UserCommand {
    /// User asks a question about current state
    Question(String),
    /// User provides new instruction/modification
    NewInstruction(String),
    /// User requests current status
    GetStatus,
    /// User requests pause
    Pause,
    /// User requests resume
    Resume,
    /// User requests cancellation
    Cancel,
}
