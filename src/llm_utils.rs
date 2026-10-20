/// LLM utility functions — multi-provider text generation with automatic fallback.
///
/// Priority order for ALL text tasks (owner directive Oct 20 2026 — NVIDIA NIM
/// is the DEFAULT; Qwen/DeepSeek are unpaid, Gemini is quota-walled):
///   1. NVIDIA NIM (free endpoints, tool calling — the only provider that works)
///   2. Qwen (qwen3.7-plus via DashScope — currently in arrearage)
///   3. DeepSeek V4 (currently 402 Insufficient Balance)
///   4. Gemini / Gemma (free tier: 5 requests/minute, last resort)
///
/// ⚠️ NIM MUST be the first attempt for EVERY LLM call.
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

/// Fast text generation — tries NVIDIA NIM first, then Qwen, then DeepSeek,
/// then Gemini.
///
/// NIM is built from `NVIDIA_API_KEY` here rather than threaded through the
/// signature so the 21 existing call sites (twitch_mapper, kick_mapper,
/// campaign_engine, skills) get the working provider without churn.
/// Use for bulk/scoring tasks where speed matters.
pub async fn generate_text_fast(
    qwen: Option<&QwenClient>,
    deepseek: Option<&DeepSeekClient>,
    gemini: Option<&GeminiClient>,
    prompt: &str,
) -> Result<String, Box<dyn std::error::Error + Send + Sync>> {
    let nim = std::env::var("NVIDIA_API_KEY")
        .ok()
        .filter(|k| !k.trim().is_empty())
        .map(NvidiaNimClient::new);

    try_provider!(nim.as_ref(), prompt, "NVIDIA NIM (fast path)", "trying Qwen");
    try_provider!(qwen, prompt, "Qwen (qwen3.7-plus, fast path)", "trying DeepSeek");
    try_provider!(deepseek, prompt, "DeepSeek V4 (fast path)", "trying Gemini");
    try_provider!(gemini, prompt, "Gemini Flash (fast path)", "no more fallbacks");

    Err("No LLM client available for fast text generation".into())
}

/// Generate text using the best available LLM, with automatic fallback.
///
/// Use this for all text-only tasks (DM scripts, prospect scoring, outreach messages,
/// code generation). NVIDIA NIM is the default and only working provider.
/// Do NOT use this for video analysis — call GeminiClient::analyze_video_from_url directly.
pub async fn generate_text_best_effort(
    qwen: Option<&QwenClient>,
    nvidia: Option<&NvidiaNimClient>,
    gemma: Option<&GeminiClient>,
    gemini: Option<&GeminiClient>,
    deepseek: Option<&DeepSeekClient>,
    prompt: &str,
) -> Result<String, Box<dyn std::error::Error + Send + Sync>> {
    try_provider!(nvidia, prompt, "NVIDIA NIM", "trying Qwen fallback");
    try_provider!(qwen, prompt, "Qwen (qwen3.7-plus)", "trying Gemma via AI Studio fallback");
    try_provider!(gemma, prompt, "Gemma 4 (Google AI Studio)", "trying Gemini Flash fallback");
    try_provider!(gemini, prompt, "Gemini Flash", "trying DeepSeek fallback");
    try_provider!(deepseek, prompt, "DeepSeek V4", "no more fallbacks");

    Err("No LLM client configured for text generation".into())
}

/// Video-script generation with provider fallback (Oct 19 2026).
///
/// The `generate_video_script` agent tool used to hit Gemini directly, so a
/// single Gemini 429 (quota) stalled the whole education/clipping pipeline for
/// ~1h per attempt. This walks the working text providers first
/// (Qwen → NVIDIA NIM → DeepSeek); the caller keeps the Gemini script-tuned
/// path as a last resort. Clients are built from env so the tool handler —
/// which has no `AppState` — can use them.
pub async fn generate_video_script_best_effort(
    prompt: &str,
) -> Result<String, Box<dyn std::error::Error + Send + Sync>> {
    let qwen = std::env::var("DASHSCOPE_API_KEY")
        .ok()
        .filter(|k| !k.trim().is_empty())
        .map(QwenClient::new);
    let nvidia = std::env::var("NVIDIA_API_KEY")
        .ok()
        .filter(|k| !k.trim().is_empty())
        .map(NvidiaNimClient::new);
    let deepseek = std::env::var("DEEPSEEK_API_KEY")
        .ok()
        .filter(|k| !k.trim().is_empty())
        .map(DeepSeekClient::new);

    try_provider!(nvidia.as_ref(), prompt, "NVIDIA NIM (script)", "trying Qwen");
    try_provider!(qwen.as_ref(), prompt, "Qwen (script)", "trying DeepSeek");
    try_provider!(
        deepseek.as_ref(),
        prompt,
        "DeepSeek V4 (script)",
        "no script fallbacks left"
    );

    Err("No text provider available for script generation".into())
}
