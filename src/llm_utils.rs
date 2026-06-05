/// LLM utility functions — multi-provider text generation with automatic fallback.
///
/// Priority order for text-only tasks:
///   1. Ollama (self-hosted Gemma 4 12B, encoder-free multimodal, vision+audio on t3.xlarge)
///   2. NVIDIA NIM (Gemma 4 31B, 40 RPM, own quota pool)
///   3. Gemma 4 via Gemini API (own quota pool, separate from Gemini Flash)
///   4. Primary Gemini client (last resort)
///   5. DeepSeek V4 (OpenAI-compatible, 1M context, cheap fallback)
use crate::deepseek_client::DeepSeekClient;
use crate::gemini_client::GeminiClient;
use crate::nvidia_nim_client::NvidiaNimClient;
use crate::ollama_client::OllamaClient;
use std::time::Duration;

const PROVIDER_TIMEOUT: Duration = Duration::from_secs(30);

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

/// Fast text generation — tries Ollama first, then DeepSeek, then Gemini.
/// Use for bulk/scoring tasks where speed matters.
pub async fn generate_text_fast(
    ollama: Option<&OllamaClient>,
    gemini: Option<&GeminiClient>,
    deepseek: Option<&DeepSeekClient>,
    prompt: &str,
) -> Result<String, Box<dyn std::error::Error + Send + Sync>> {
    try_provider!(ollama, prompt, "Ollama (Gemma 4B, fast path)", "trying DeepSeek");
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
    try_provider!(ollama, prompt, "Ollama (Gemma 4B)", "trying NVIDIA NIM fallback");
    try_provider!(nvidia, prompt, "NVIDIA NIM (Gemma 4)", "trying Gemma fallback");
    try_provider!(gemma, prompt, "Gemma 4 (Gemini API)", "trying Gemini Flash fallback");
    try_provider!(gemini, prompt, "Gemini Flash", "trying DeepSeek fallback");
    try_provider!(deepseek, prompt, "DeepSeek V4", "no more fallbacks");

    Err("No LLM client configured for text generation".into())
}
