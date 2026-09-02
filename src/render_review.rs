//! LLM-powered QA review for ALL rendered outputs — Blender jobs,
//! FFmpeg video edits, clip jobs, auto-generated videos, anything that
//! produces a file we hand back to a user or paying client.
//!
//! Runs after render, before return. Flow:
//!   render_complete(url, prompt, tool) → fallback chain:
//!     1. NVIDIA NIM (images/audio only — skip video, no native support)
//!     2. AWS Bedrock (Llama 4 Maverick — images/audio, text)
//!     3. Ollama/Gemma 4 12B (full video native)
//!     4. Gemini 2.5 Flash (full video inlineData)
//!   returns `{pass: bool, score, feedback, retry_hint}` → if pass, caller
//!   hands the URL to the user; if fail + first attempt, caller retries with
//!   the hint appended to the prompt; if fail after retry, caller still
//!   returns the URL but with a warning flag the UI surfaces.
//!
//! Also writes one row per review to `blender_render_reviews` so the
//! admin dashboard can see pass/fail rate per tool over time.

use crate::gemini_client::GeminiClient;
use crate::nvidia_nim_client::NvidiaNimClient;
use crate::ollama_client::OllamaClient;
use crate::AppState;
use base64::engine::general_purpose::STANDARD as BASE64_ENGINE;
use base64::Engine;
use serde::{Deserialize, Serialize};
use std::path::{Path, PathBuf};
use std::sync::atomic::{AtomicU32, Ordering};
use std::sync::Arc;
use std::time::Duration;

/// Global counter for Gemini API calls this server session.
/// Resets on server restart. Free tier: 20 requests/day for gemini-2.5-flash.
/// We stop at 18 to leave buffer.
static GEMINI_VIDEO_CALLS: AtomicU32 = AtomicU32::new(0);
const GEMINI_DAILY_LIMIT: u32 = 18;

/// Minimum acceptable score to pass the render. Chosen at 6 because our
/// review prompt scores 1-10; 6+ means "buyer-quality", 5- means "redo".
const PASS_THRESHOLD: i32 = 6;

/// Deterministic artifact floor (Sep 2 2026, §CRIT): a video artifact smaller
/// than this is corrupt/empty — no reviewer needed to know it's garbage.
/// Catches the 0-byte publish incident (empty MP4s uploaded and "review
/// passed at score 0" because every reviewer failure path was fail-open).
const ARTIFACT_MIN_BYTES: u64 = 50_000;
/// Local video artifacts shorter than this are truncated/corrupt.
const ARTIFACT_MIN_DURATION_SECS: f64 = 1.0;

/// Deterministic pre-review validation for VIDEO artifacts. Returns
/// Some(reason) when the artifact is provably invalid — no LLM involved.
/// Images/thumbnails are skipped (no duration, often legitimately small).
/// Network HEAD failures return None (don't block on infra noise).
async fn validate_video_artifact(output_url: &str) -> Option<String> {
    let ext = output_url
        .rsplit('.')
        .next()
        .unwrap_or("")
        .split('?')
        .next()
        .unwrap_or("")
        .to_lowercase();
    if !matches!(ext.as_str(), "mp4" | "mov" | "mkv" | "webm" | "avi") {
        return None;
    }

    if output_url.starts_with("http://") || output_url.starts_with("https://") {
        // Remote: ranged GET (HEAD would break GET-presigned SigV4 URLs).
        // Parse total size from Content-Range: "bytes 0-0/12345678".
        let client = reqwest::Client::builder()
            .timeout(Duration::from_secs(20))
            .build()
            .ok()?;
        let resp = client
            .get(output_url)
            .header("Range", "bytes=0-0")
            .send()
            .await
            .ok()?;
        let total = resp
            .headers()
            .get("content-range")
            .and_then(|v| v.to_str().ok())
            .and_then(|v| v.rsplit('/').next().and_then(|t| t.parse::<u64>().ok()));
        match total {
            Some(size) if size < ARTIFACT_MIN_BYTES => {
                return Some(format!(
                    "ARTIFACT_INVALID: remote video is only {} bytes (minimum {})",
                    size, ARTIFACT_MIN_BYTES
                ));
            }
            Some(_) => return None,
            // No Content-Range (server ignored Range): check content-length.
            None => {
                if let Some(len) = resp.content_length() {
                    if len > 1 && len < ARTIFACT_MIN_BYTES {
                        return Some(format!(
                            "ARTIFACT_INVALID: remote video is only {} bytes (minimum {})",
                            len, ARTIFACT_MIN_BYTES
                        ));
                    }
                }
                return None;
            }
        }
    }

    // Local path: size + ffprobe duration.
    let meta = match tokio::fs::metadata(output_url).await {
        Ok(m) if m.is_file() => m,
        _ => {
            return Some(format!(
                "ARTIFACT_INVALID: video artifact does not exist at '{}'",
                output_url
            ));
        }
    };
    if meta.len() < ARTIFACT_MIN_BYTES {
        return Some(format!(
            "ARTIFACT_INVALID: video artifact is only {} bytes (minimum {})",
            meta.len(),
            ARTIFACT_MIN_BYTES
        ));
    }

    // Best-effort duration check via ffprobe; skip silently if ffprobe is
    // unavailable (exit code NotFound) so review still runs.
    let probe = tokio::process::Command::new("ffprobe")
        .args([
            "-v", "error",
            "-show_entries", "format=duration",
            "-of", "default=noprint_wrappers=1:nokey=1",
            output_url,
        ])
        .output()
        .await;
    if let Ok(out) = probe {
        if out.status.success() {
            let dur: f64 = String::from_utf8_lossy(&out.stdout)
                .trim()
                .parse()
                .unwrap_or(0.0);
            if dur < ARTIFACT_MIN_DURATION_SECS {
                return Some(format!(
                    "ARTIFACT_INVALID: video duration is {:.2}s (minimum {}s) — likely truncated",
                    dur, ARTIFACT_MIN_DURATION_SECS
                ));
            }
        }
    }
    None
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct ReviewResult {
    pub pass: bool,
    pub score: i32,
    pub feedback: String,
    pub retry_hint: Option<String>,
}

/// Review a rendered output using the four-provider fallback chain:
///   1. NVIDIA NIM (images/audio only, skip video)
///   2. AWS Bedrock (images/audio/text)
///   3. Ollama/Gemma 4 12B (full video native, images, audio)
///   4. Gemini 2.5 Flash (full video inlineData, images, audio)
pub async fn review_render(
    state: &Arc<AppState>,
    output_url: &str,
    original_prompt: &str,
    tool_name: &str,
    delivery_id: Option<uuid::Uuid>,
) -> ReviewResult {
    let prompt = review_prompt(original_prompt, tool_name, output_url);

    // ── DETERMINISTIC GATE (§CRIT, Sep 2 2026) ──
    // Before spending any reviewer calls: reject provably-invalid video
    // artifacts (0-byte / truncated). This is the boundary that failed during
    // the empty-MP4 incident — every reviewer failure path used to fail OPEN
    // (pass:true, score:0), so garbage got published silently.
    if let Some(reason) = validate_video_artifact(output_url).await {
        tracing::warn!("render_review artifact gate rejected '{}': {}", output_url, reason);
        persist_review(
            state,
            tool_name,
            &ReviewResult {
                pass: false,
                score: 0,
                feedback: reason.clone(),
                retry_hint: Some(
                    "Re-render the output; the previous artifact was empty or corrupt."
                        .to_string(),
                ),
            },
            delivery_id,
            output_url,
        )
        .await;
        return ReviewResult {
            pass: false,
            score: 0,
            feedback: reason,
            retry_hint: Some(
                "Re-render the output; the previous artifact was empty or corrupt."
                    .to_string(),
            ),
        };
    }

    // 1. Try NVIDIA NIM vision model (images/audio only, skip video)
    let ext = output_url.rsplit('.').next().unwrap_or("").to_lowercase();
    let is_video = matches!(ext.as_str(), "mp4" | "mov" | "mkv" | "webm" | "avi");
    if !is_video {
        if let Some(nim) = state.nvidia_nim_vision_client.as_ref() {
            let review = run_review_via_nim(nim, output_url, &prompt).await;
            if review.score > 0 {
                persist_review(state, tool_name, &review, delivery_id, output_url).await;
                return review;
            }
        }
    }

    // 2. Try Bedrock (images/audio/text)
    if let Some(bedrock) = state.bedrock_client.as_ref() {
        let review = run_review_via_bedrock(bedrock, output_url, &prompt, is_video).await;
        if review.score > 0 {
            persist_review(state, tool_name, &review, delivery_id, output_url).await;
            return review;
        }
    }

    // 3. Try Ollama/Gemma 4 12B (full video native, images, audio)
    if let Some(ollama) = state.ollama_client.as_ref() {
        let review = run_review_via_ollama(ollama, output_url, &prompt).await;
        if review.score > 0 {
            persist_review(state, tool_name, &review, delivery_id, output_url).await;
            return review;
        }
    }

    // 4. Fall back to Gemini (full video inlineData, images, audio)
    let gemini = match state
        .video_gemini_client
        .as_ref()
        .or(state.gemini_client.as_ref())
    {
        Some(g) => g,
        None => {
            return ReviewResult {
                pass: false,
                score: 0,
                feedback: "No reviewer (NIM, Bedrock, Ollama, or Gemini) configured — QA review skipped".to_string(),
                retry_hint: None,
            };
        }
    };

    let review = run_review_via_gemini(gemini, output_url, &prompt).await;
    persist_review(state, tool_name, &review, delivery_id, output_url).await;
    review
}

/// Run review via NVIDIA NIM vision model (images/audio only).
async fn run_review_via_nim(
    nim: &NvidiaNimClient,
    output_url: &str,
    prompt: &str,
) -> ReviewResult {
    let response = match multimodal_review_via_nim(nim, output_url, prompt).await {
        Ok(r) => r,
        Err(e) => {
            return ReviewResult {
                pass: false,
                score: 0,
                feedback: format!("NIM review call failed: {}", e),
                retry_hint: None,
            };
        }
    };
    parse_review_response(&response)
}

/// Run review via Bedrock (images/audio/text).
async fn run_review_via_bedrock(
    bedrock: &Arc<crate::bedrock_client::BedrockClient>,
    output_url: &str,
    prompt: &str,
    is_video: bool,
) -> ReviewResult {
    if is_video {
        return ReviewResult {
            pass: false,
            score: 0,
            feedback: "Bedrock (Llama 4) does not natively support video input through Converse API — skipped".to_string(),
            retry_hint: None,
        };
    }
    let response = match multimodal_review_via_bedrock(bedrock, output_url, prompt).await {
        Ok(r) => r,
        Err(e) => {
            return ReviewResult {
                pass: false,
                score: 0,
                feedback: format!("Bedrock review call failed: {}", e),
                retry_hint: None,
            };
        }
    };
    parse_review_response(&response)
}

/// Run review via Ollama/Gemma 4 12B (full video native, images, audio).
async fn run_review_via_ollama(
    ollama: &OllamaClient,
    output_url: &str,
    prompt: &str,
) -> ReviewResult {
    let response = match multimodal_review_via_ollama(ollama, output_url, prompt).await {
        Ok(r) => r,
        Err(e) => {
            return ReviewResult {
                pass: false,
                score: 0,
                feedback: format!("Ollama review call failed: {}", e),
                retry_hint: None,
            };
        }
    };
    parse_review_response(&response)
}

/// Run review via Gemini (fallback path, full video native).
async fn run_review_via_gemini(
    gemini: &GeminiClient,
    output_url: &str,
    prompt: &str,
) -> ReviewResult {
    let calls = GEMINI_VIDEO_CALLS.fetch_add(1, Ordering::Relaxed) + 1;
    if calls > GEMINI_DAILY_LIMIT {
        return ReviewResult {
            pass: false,
            score: 0,
            feedback: format!(
                "Gemini daily quota reached ({}/20 calls this session) — QA skipped. \
                 The render was accepted without Gemini review.",
                GEMINI_DAILY_LIMIT
            ),
            retry_hint: None,
        };
    }

    let response = match multimodal_review_via_gemini(gemini, output_url, prompt).await {
        Ok(r) => r,
        Err(e) => {
            let err_text = e.to_string();
            let is_429 = err_text.contains("429") || err_text.contains("RESOURCE_EXHAUSTED") || err_text.contains("quota");

            if is_429 {
                if let Some(delay_secs) = parse_gemini_retry_delay(&err_text) {
                    tracing::warn!(
                        "Gemini 429 (call #{}/20). Retrying after {}s delay...",
                        calls, delay_secs
                    );
                    tokio::time::sleep(Duration::from_secs(delay_secs)).await;

                    match multimodal_review_via_gemini(gemini, output_url, prompt).await {
                        Ok(r) => return parse_review_response(&r),
                        Err(retry_err) => {
                            return ReviewResult {
                                pass: false,
                                score: 0,
                                feedback: format!(
                                    "Gemini review failed on retry (call #{}/20): {}",
                                    calls, retry_err
                                ),
                                retry_hint: None,
                            };
                        }
                    }
                }
            }

            return ReviewResult {
                pass: false,
                score: 0,
                feedback: format!("Gemini review call failed (call #{}/20): {}", calls, err_text),
                retry_hint: None,
            };
        }
    };
    parse_review_response(&response)
}

/// Parse retryDelay seconds from a Gemini 429 error response JSON.
/// The API returns: "retryDelay": "54.852817327s"
fn parse_gemini_retry_delay(err_text: &str) -> Option<u64> {
    // Look for pattern: "retryDelay": "XYZs"
    let start = err_text.find(r#""retryDelay":"#)?;
    let after = &err_text[start + 14..];
    let quote_start = after.find('"')?;
    let after_quote = &after[quote_start + 1..];
    let quote_end = after_quote.find('"')?;
    let delay_str = &after_quote[..quote_end];
    let delay_str = delay_str.trim_end_matches('s');
    let secs: f64 = delay_str.parse().ok()?;
    // Clamp to reasonable range
    Some((secs.ceil() as u64).clamp(1, 120))
}

fn parse_review_response(response: &str) -> ReviewResult {
    let cleaned = response
        .trim()
        .trim_start_matches("```json")
        .trim_start_matches("```")
        .trim_end_matches("```")
        .trim();

    let parsed: serde_json::Value = match serde_json::from_str(cleaned) {
        Ok(v) => v,
        Err(_) => {
            return ReviewResult {
                pass: false,
                score: 0,
                feedback: format!(
                    "Unparseable review response: {}",
                    cleaned.chars().take(120).collect::<String>()
                ),
                retry_hint: None,
            };
        }
    };

    let score = parsed.get("score").and_then(|s| s.as_i64()).unwrap_or(0) as i32;
    let feedback = parsed
        .get("feedback")
        .and_then(|f| f.as_str())
        .unwrap_or("")
        .to_string();
    let retry_hint = parsed
        .get("retry_hint")
        .and_then(|h| h.as_str())
        .filter(|s| !s.is_empty())
        .map(|s| s.to_string());

    ReviewResult {
        pass: score >= PASS_THRESHOLD,
        score,
        feedback,
        retry_hint,
    }
}

fn review_prompt(original_prompt: &str, tool_name: &str, output_url: &str) -> String {
    format!(
        r#"You are a strict QA reviewer for an AI video production studio. A client has paid (or is about to pay) for this output — your job is to verify it matches the original brief and is delivery-quality.

Original brief / request:
"""
{original_prompt}
"""

Tool used to render: {tool_name}
Output artifact path or URL: {output_url}

Inspect the attached media itself when available, not just the filename/path.

Rate the output 1-10 against these criteria (mentally — only return the final score):
- Does it match what the brief asked for?
- Is the visual/render/audio quality good (no artifacts, clean composition, no obvious glitches)?
- Would you be comfortable sending this to a paying client as-is?
- For video: correct duration? smooth playback? audio if expected?
- For image: correct aspect ratio? readable text if any? no distortion?
- For audio: clean narration/music/sound design? understandable? no clipping or obvious defects?

Score ≥6 = ready to ship. Score ≤5 = needs re-render or redo.

If score ≤5, give a CONCRETE retry hint — one sentence the caller can prepend to the prompt to fix the specific issue.

Return ONLY JSON, no markdown:
{{"score": 8, "feedback": "Clean composition, readable title, matches the brief.", "retry_hint": null}}
or
{{"score": 4, "feedback": "Text too small to read on mobile.", "retry_hint": "Use a larger centered title and increase contrast."}}"#,
        original_prompt = original_prompt,
        tool_name = tool_name,
        output_url = output_url,
    )
}

/// Download a cloud URL to a temp file and return its path.
/// Returns `None` if the URL doesn't look like a remote URL.
async fn download_to_temp(output_url: &str) -> Result<Option<(PathBuf, Vec<u8>)>, Box<dyn std::error::Error + Send + Sync>> {
    if !output_url.starts_with("http://") && !output_url.starts_with("https://") {
        return Ok(None);
    }
    let response = reqwest::get(output_url).await?;
    let bytes = response.bytes().await.unwrap_or_default().to_vec();
    if bytes.is_empty() {
        return Ok(None);
    }
    let ext = output_url
        .rsplit('.')
        .next()
        .and_then(|s| if s.len() <= 5 { Some(s) } else { None })
        .unwrap_or("mp4");
    let temp_path = std::env::temp_dir().join(format!("review_{}.{}", uuid::Uuid::new_v4(), ext));
    std::fs::write(&temp_path, &bytes)?;
    tracing::debug!(url = %output_url, path = %temp_path.display(), "Downloaded cloud URL to temp file for QA review");
    Ok(Some((temp_path, bytes)))
}

/// Analyze media via NVIDIA NIM vision model (images and audio only — no native video).
async fn multimodal_review_via_nim(
    nim: &NvidiaNimClient,
    output_url: &str,
    prompt: &str,
) -> Result<String, Box<dyn std::error::Error + Send + Sync>> {
    let _downloaded = download_to_temp(output_url).await?;
    let (local_path_buf, local_url_owned) = match _downloaded.as_ref() {
        Some((path, _)) => (Some(path.clone()), Some(path.to_string_lossy().to_string())),
        None => (None, None),
    };
    let local_path: &Path = local_path_buf.as_deref().unwrap_or_else(|| Path::new(output_url));
    let local_url: &str = local_url_owned.as_deref().unwrap_or(output_url);

    if !local_path.exists() {
        return nim.generate_text(prompt).await.map_err(Into::into);
    }

    let ext = local_path
        .extension()
        .and_then(|ext| ext.to_str())
        .map(|ext| ext.to_ascii_lowercase())
        .unwrap_or_default();

    let result = if is_image_extension(&ext) {
        let bytes = std::fs::read(local_path)?;
        nim.analyze_image_bytes(&bytes, prompt).await.map_err(Into::into)
    } else if is_video_extension(&ext) {
        Err("NVIDIA NIM does not support native video analysis — falling back to native video model".into())
    } else if let Some(mime_type) = audio_mime_type(&ext) {
        let bytes = std::fs::read(local_path)?;
        nim.analyze_audio_bytes(&bytes, mime_type, prompt).await.map_err(Into::into)
    } else {
        nim.generate_text(prompt).await.map_err(Into::into)
    };

    if let Some((temp_path, _)) = _downloaded {
        let _ = std::fs::remove_file(&temp_path);
    }

    result
}

/// Analyze media via Ollama/Gemma 4 12B (full video native, images, audio).
async fn multimodal_review_via_ollama(
    ollama: &OllamaClient,
    output_url: &str,
    prompt: &str,
) -> Result<String, Box<dyn std::error::Error + Send + Sync>> {
    let _downloaded = download_to_temp(output_url).await?;
    let (local_path_buf, local_url_owned) = match _downloaded.as_ref() {
        Some((path, _)) => (Some(path.clone()), Some(path.to_string_lossy().to_string())),
        None => (None, None),
    };
    let local_path: &Path = local_path_buf.as_deref().unwrap_or_else(|| Path::new(output_url));
    let local_url: &str = local_url_owned.as_deref().unwrap_or(output_url);

    if !local_path.exists() {
        return ollama.generate_text(prompt).await.map_err(Into::into);
    }

    let ext = local_path
        .extension()
        .and_then(|ext| ext.to_str())
        .map(|ext| ext.to_ascii_lowercase())
        .unwrap_or_default();

    let result = if is_video_extension(&ext) {
        // Read full video and send to Ollama native API (Gemma 4 natively understands video)
        let video_bytes = tokio::fs::read(local_path).await?;
        let b64 = BASE64_ENGINE.encode(&video_bytes);
        let body = serde_json::json!({
            "model": ollama.model_id(),
            "messages": [{"role": "user", "content": prompt, "images": [b64]}],
            "stream": false,
            "options": {"num_predict": 2048, "temperature": 0.3}
        });
        let resp = ollama.chat_native(body).await?;
        Ok(resp)
    } else if is_image_extension(&ext) {
        let bytes = std::fs::read(local_path)?;
        ollama.generate_text_with_images(prompt, vec![("".to_string(), bytes)]).await.map_err(Into::into)
    } else if let Some(_mime_type) = audio_mime_type(&ext) {
        let bytes = std::fs::read(local_path)?;
        let b64 = BASE64_ENGINE.encode(&bytes);
        let body = serde_json::json!({
            "model": ollama.model_id(),
            "messages": [{"role": "user", "content": prompt, "images": [b64]}],
            "stream": false,
            "options": {"num_predict": 2048, "temperature": 0.3}
        });
        let resp = ollama.chat_native(body).await?;
        Ok(resp)
    } else {
        ollama.generate_text(prompt).await.map_err(Into::into)
    };

    if let Some((temp_path, _)) = _downloaded {
        let _ = std::fs::remove_file(&temp_path);
    }

    result
}

/// Analyze media via Bedrock (text only — Llama 4 Maverick Converse API supports
/// image/video content blocks, but the SDK image/video support needs builder
/// imports not yet exposed from bedrock_client.rs. For images/video, NIM/Ollama
/// handle those before Bedrock in the chain.
async fn multimodal_review_via_bedrock(
    bedrock: &Arc<crate::bedrock_client::BedrockClient>,
    _output_url: &str,
    prompt: &str,
) -> Result<String, Box<dyn std::error::Error + Send + Sync>> {
    use crate::bedrock_client::BedrockResponse;
    use aws_sdk_bedrockruntime::types::{ContentBlock, ConversationRole, Message};

    let msg = Message::builder()
        .role(ConversationRole::User)
        .content(ContentBlock::Text(prompt.to_string()))
        .build()
        .map_err(|e| format!("Failed to build Bedrock message: {}", e))?;

    let response = bedrock
        .generate_single("You are a strict QA reviewer.", &[msg], &[])
        .await
        .map_err(|e| format!("Bedrock QA review failed: {}", e))?;

    match response {
        BedrockResponse::Text(t) => Ok(t),
        _ => Err("Bedrock returned tool calls instead of text".into()),
    }
}

/// Analyze media via Gemini (full video native, images, audio).
async fn multimodal_review_via_gemini(
    gemini: &GeminiClient,
    output_url: &str,
    prompt: &str,
) -> Result<String, Box<dyn std::error::Error + Send + Sync>> {
    let _downloaded = download_to_temp(output_url).await?;
    let (local_path_buf, local_url_owned) = match _downloaded.as_ref() {
        Some((path, _)) => (Some(path.clone()), Some(path.to_string_lossy().to_string())),
        None => (None, None),
    };
    let local_path: &Path = local_path_buf.as_deref().unwrap_or_else(|| Path::new(output_url));
    let local_url: &str = local_url_owned.as_deref().unwrap_or(output_url);

    if !local_path.exists() {
        return gemini.generate_text(prompt).await;
    }

    let ext = local_path
        .extension()
        .and_then(|ext| ext.to_str())
        .map(|ext| ext.to_ascii_lowercase())
        .unwrap_or_default();

    let result = if is_image_extension(&ext) {
        let bytes = std::fs::read(local_path)?;
        gemini.analyze_image_bytes(&bytes, prompt).await
    } else if is_video_extension(&ext) {
        gemini
            .analyze_video_content(local_url, Some(prompt.to_string()))
            .await
    } else if let Some(mime_type) = audio_mime_type(&ext) {
        let bytes = std::fs::read(local_path)?;
        gemini.analyze_audio_bytes(&bytes, mime_type, prompt).await
    } else {
        gemini.generate_text(prompt).await
    };

    if let Some((temp_path, _)) = _downloaded {
        let _ = std::fs::remove_file(&temp_path);
    }

    result
}

fn is_image_extension(ext: &str) -> bool {
    matches!(ext, "png" | "jpg" | "jpeg" | "webp" | "gif")
}

fn is_video_extension(ext: &str) -> bool {
    matches!(ext, "mp4" | "mov" | "mkv" | "webm" | "avi")
}

fn audio_mime_type(ext: &str) -> Option<&'static str> {
    match ext {
        "mp3" => Some("audio/mpeg"),
        "wav" => Some("audio/wav"),
        "aac" => Some("audio/aac"),
        "flac" => Some("audio/flac"),
        "ogg" => Some("audio/ogg"),
        "m4a" => Some("audio/mp4"),
        _ => None,
    }
}

/// Append the review to the audit table. Non-fatal — logging only.
async fn persist_review(
    state: &Arc<AppState>,
    tool_name: &str,
    review: &ReviewResult,
    delivery_id: Option<uuid::Uuid>,
    output_url: &str,
) {
    let _ = sqlx::query(
        "INSERT INTO blender_render_reviews
           (tool_name, delivery_id, output_url, pass, score, feedback, retry_hint)
         VALUES ($1, $2, $3, $4, $5, $6, $7)",
    )
    .bind(tool_name)
    .bind(delivery_id)
    .bind(output_url)
    .bind(review.pass)
    .bind(review.score)
    .bind(&review.feedback)
    .bind(review.retry_hint.as_deref())
    .execute(&state.db_pool)
    .await;
}
