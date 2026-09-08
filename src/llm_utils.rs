/// LLM utility functions — multi-provider text generation with automatic fallback.
///
/// Priority order for ALL text tasks (§52, Sep 8 2026 — Qwen is the DEFAULT,
/// Ollama retired):
///   1. Qwen (qwen3.7-plus via DashScope compatible-mode — multimodal, default)
///   2. DeepSeek V4 (OpenAI-compatible, 1M context — cheapest cloud fallback)
///   3. Gemini (last-resort fallback, quota-limited)
///
/// ⚠️ Qwen MUST be the first attempt for EVERY LLM call (owner directive §52.2).
///    No text-only models are permitted. Ollama (gemma4:12b) is RETIRED — it
///    does not run on the Alibaba Cloud box, do not re-enable.
use crate::deepseek_client::DeepSeekClient;
use crate::gemini_client::GeminiClient;
use crate::nvidia_nim_client::NvidiaNimClient;
use crate::qwen_client::QwenClient;
use std::time::Duration;

const PROVIDER_TIMEOUT: Duration = Duration::from_secs(120);

macro_rules! try_provider {
    ($client:expr, $prompt:expr, $name:expr, $fallback_label:expr) => {{
        if let Some(client) = $client {
            match tokio::time::timeout(PROVIDER_TIMEOUT, client.generate_text($prompt)).await {
                Ok(Ok(result)) => {
                    tracing::debug!("✅ Text generated via {}", $name);
                    return Ok(result);
                }
                Ok(Err(e)) => {
                    tracing::warn!(
                        "⚠️ {} failed ({}), {}",
                        $name,
                        e,
                        $fallback_label
                    );
                }
                Err(_) => {
                    tracing::warn!(
                        "⚠️ {} timed out after {:?}, {}",
                        $name,
                        PROVIDER_TIMEOUT,
                        $fallback_label
                    );
                }
            }
        }
    }};
}

/// Fast text generation — tries Qwen (qwen3.7-plus) first, then DeepSeek, then Gemini.
/// Use for bulk/scoring tasks where speed matters.
pub async fn generate_text_fast(
    qwen: Option<&QwenClient>,
    deepseek: Option<&DeepSeekClient>,
    gemini: Option<&GeminiClient>,
    prompt: &str,
) -> Result<String, Box<dyn std::error::Error + Send + Sync>> {
    try_provider!(qwen, prompt, "Qwen (qwen3.7-plus, fast path)", "trying DeepSeek");
    try_provider!(deepseek, prompt, "DeepSeek V4 (fast path)", "trying Gemini");
    try_provider!(gemini, prompt, "Gemini Flash (fast path)", "no more fallbacks");

    Err("No LLM client available for fast text generation".into())
}

/// Generate text using the best available LLM, with automatic fallback.
///
/// Use this for all text-only tasks (DM scripts, prospect scoring, outreach messages,
/// code generation) to avoid hitting Gemini Flash quota limits.
/// Do NOT use this for video analysis — call GeminiClient::analyze_video_from_url directly.
pub async fn generate_text_best_effort(
    qwen: Option<&QwenClient>,
    nvidia: Option<&NvidiaNimClient>,
    gemma: Option<&GeminiClient>,
    gemini: Option<&GeminiClient>,
    deepseek: Option<&DeepSeekClient>,
    prompt: &str,
) -> Result<String, Box<dyn std::error::Error + Send + Sync>> {
    try_provider!(qwen, prompt, "Qwen (qwen3.7-plus)", "trying NVIDIA NIM fallback");
    try_provider!(nvidia, prompt, "NVIDIA NIM (Gemma 4)", "trying Gemma via AI Studio fallback");
    try_provider!(gemma, prompt, "Gemma 4 (Google AI Studio)", "trying Gemini Flash fallback");
    try_provider!(gemini, prompt, "Gemini Flash", "trying DeepSeek fallback");
    try_provider!(deepseek, prompt, "DeepSeek V4", "no more fallbacks");

    Err("No LLM client configured for text generation".into())
}