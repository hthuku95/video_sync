//! Token usage primitives shared across LLM clients and the pipeline runner.
//!
//! Every provider client extracts whatever usage metadata its API returns
//! (Ollama: prompt_eval_count/eval_count; OpenAI-compatible DeepSeek/NIM:
//! usage.prompt_tokens/completion_tokens) into this uniform struct so the
//! agent tool loops can accumulate a per-run cost ledger.

use serde::{Deserialize, Serialize};

#[derive(Debug, Clone, Copy, Default, Serialize, Deserialize, PartialEq)]
pub struct UsageInfo {
    pub prompt_tokens: u64,
    pub completion_tokens: u64,
}

impl UsageInfo {
    pub fn total_tokens(&self) -> u64 {
        self.prompt_tokens.saturating_add(self.completion_tokens)
    }

    /// Parse OpenAI-compatible `usage` object (DeepSeek, NVIDIA NIM).
    pub fn from_openai(json: &serde_json::Value) -> Self {
        Self {
            prompt_tokens: json["usage"]["prompt_tokens"].as_u64().unwrap_or(0),
            completion_tokens: json["usage"]["completion_tokens"].as_u64().unwrap_or(0),
        }
    }

    /// Parse Ollama's `/api/chat` eval counters.
    pub fn from_ollama(json: &serde_json::Value) -> Self {
        Self {
            prompt_tokens: json["prompt_eval_count"].as_u64().unwrap_or(0),
            completion_tokens: json["eval_count"].as_u64().unwrap_or(0),
        }
    }
}
