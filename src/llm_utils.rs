/// LLM utility functions — multi-provider text generation with automatic fallback.
///
/// Priority order for text-only tasks:
///   1. NVIDIA NIM (Gemma 4 31B, 40 RPM, own quota pool)
///   2. Gemma 4 via Gemini API (own quota pool, separate from Gemini Flash)
///   3. Primary Gemini client (last resort)
///   4. DeepSeek V4 (OpenAI-compatible, 1M context, cheap fallback)
use crate::deepseek_client::DeepSeekClient;
use crate::gemini_client::GeminiClient;
use crate::nvidia_nim_client::NvidiaNimClient;

/// Generate text using the best available LLM, with automatic fallback.
///
/// Use this for all text-only tasks (DM scripts, prospect scoring, outreach messages,
/// code generation) to avoid hitting Gemini Flash quota limits.
/// Do NOT use this for video analysis — call GeminiClient::analyze_video_from_url directly.
pub async fn generate_text_best_effort(
    nvidia: Option<&NvidiaNimClient>,
    gemma: Option<&GeminiClient>,
    gemini: Option<&GeminiClient>,
    deepseek: Option<&DeepSeekClient>,
    prompt: &str,
) -> Result<String, Box<dyn std::error::Error + Send + Sync>> {
    // Tier 1: NVIDIA NIM — 40 RPM, dedicated quota
    if let Some(client) = nvidia {
        match client.generate_text(prompt).await {
            Ok(result) => {
                tracing::debug!("✅ Text generated via NVIDIA NIM (Gemma 4)");
                return Ok(result);
            }
            Err(e) => {
                tracing::warn!("⚠️ NVIDIA NIM failed, trying Gemma fallback: {}", e);
            }
        }
    }

    // Tier 2: Gemma 4 via Gemini API — own quota, separate from Gemini Flash
    if let Some(client) = gemma {
        match client.generate_text(prompt).await {
            Ok(result) => {
                tracing::debug!("✅ Text generated via Gemma 4 (Gemini API)");
                return Ok(result);
            }
            Err(e) => {
                tracing::warn!(
                    "⚠️ Gemma client failed, trying Gemini Flash fallback: {}",
                    e
                );
            }
        }
    }

    // Tier 3: Primary Gemini Flash
    if let Some(client) = gemini {
        match client.generate_text(prompt).await {
            Ok(result) => {
                tracing::debug!("✅ Text generated via Gemini Flash");
                return Ok(result);
            }
            Err(e) => {
                tracing::warn!("⚠️ Gemini failed, trying DeepSeek fallback: {}", e);
            }
        }
    }

    // Tier 4: DeepSeek V4 — OpenAI-compatible, always-on fallback
    if let Some(client) = deepseek {
        match client.generate_text(prompt).await {
            Ok(result) => {
                tracing::debug!("✅ Text generated via DeepSeek V4");
                return Ok(result);
            }
            Err(e) => {
                tracing::warn!("⚠️ DeepSeek also failed: {}", e);
            }
        }
    }

    Err("No LLM client configured for text generation".into())
}
