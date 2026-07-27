/// LLM utility functions — multi-provider text generation with automatic fallback.
///
/// Priority order for ALL text tasks:
///   1. Ollama (Gemma 4 12B — self-hosted, free, auto-scaled GPU cluster via NLB)
///   2. NVIDIA NIM (Gemma 4 31B, 40 RPM, own quota pool — best-effort only)
///   3. Gemini 2.5 Flash (last resort fallback, quota-limited)
///   4. DeepSeek V4 (OpenAI-compatible, 1M context — cheapest cloud fallback)
///
/// ⚠️ Ollama (gemma4:12b) MUST be the first attempt for EVERY LLM call.
///    No text-only models are permitted. The qwen3:4b model was removed.
use crate::deepseek_client::DeepSeekClient;
use crate::gemini_client::GeminiClient;
use crate::nvidia_nim_client::NvidiaNimClient;
use crate::ollama_client::OllamaClient;
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

/// Fast text generation — tries Ollama (gemma4:12b) first, then DeepSeek, then Gemini.
/// Use for bulk/scoring tasks where speed matters.
pub async fn generate_text_fast(
    ollama: Option<&OllamaClient>,
    deepseek: Option<&DeepSeekClient>,
    gemini: Option<&GeminiClient>,
    prompt: &str,
) -> Result<String, Box<dyn std::error::Error + Send + Sync>> {
    try_provider!(ollama, prompt, "Ollama (Gemma 4 12B, fast path)", "trying DeepSeek");
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
    ollama: Option<&OllamaClient>,
    nvidia: Option<&NvidiaNimClient>,
    gemma: Option<&GeminiClient>,
    gemini: Option<&GeminiClient>,
    deepseek: Option<&DeepSeekClient>,
    prompt: &str,
) -> Result<String, Box<dyn std::error::Error + Send + Sync>> {
    try_provider!(ollama, prompt, "Ollama (Gemma 4 12B)", "trying NVIDIA NIM fallback");
    try_provider!(nvidia, prompt, "NVIDIA NIM (Gemma 4)", "trying Gemma via AI Studio fallback");
    try_provider!(gemma, prompt, "Gemma 4 (Google AI Studio)", "trying Gemini Flash fallback");
    try_provider!(gemini, prompt, "Gemini Flash", "trying DeepSeek fallback");
    try_provider!(deepseek, prompt, "DeepSeek V4", "no more fallbacks");

    Err("No LLM client configured for text generation".into())
}
