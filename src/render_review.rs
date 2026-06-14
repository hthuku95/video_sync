//! LLM-powered QA review for ALL rendered outputs — Blender jobs,
//! FFmpeg video edits, clip jobs, auto-generated videos, anything that
//! produces a file we hand back to a user or paying client.
//!
//! Runs after render, before return. Flow:
//!   render_complete(url, prompt, tool) → NVIDIA NIM vision model scores 1-10,
//!   returns `{pass: bool, score, feedback, retry_hint}` → if pass, caller
//!   hands the URL to the user; if fail + first attempt, caller retries with
//!   the hint appended to the prompt; if fail after retry, caller still
//!   returns the URL but with a warning flag the UI surfaces.
//!
//! Uses NVIDIA NIM vision model (nemotron-3-nano-omni) as the primary reviewer
//! to avoid Gemini free-tier rate limits. Falls back to Gemini if NIM is
//! unavailable.
//!
//! Also writes one row per review to `blender_render_reviews` so the
//! admin dashboard can see pass/fail rate per tool over time.

use crate::gemini_client::GeminiClient;
use crate::nvidia_nim_client::NvidiaNimClient;
use crate::AppState;
use serde::{Deserialize, Serialize};
use std::path::{Path, PathBuf};
use std::sync::Arc;

/// Minimum acceptable score to pass the render. Chosen at 6 because our
/// review prompt scores 1-10; 6+ means "buyer-quality", 5- means "redo".
const PASS_THRESHOLD: i32 = 6;

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct ReviewResult {
    pub pass: bool,
    pub score: i32,
    pub feedback: String,
    pub retry_hint: Option<String>,
}

/// Review a rendered output. Best-effort — never blocks the pipeline;
/// returns a "pass with score 0" result if no reviewer is available.
///
/// Primary reviewer: NVIDIA NIM vision model (nemotron-3-nano-omni) to
/// avoid Gemini free-tier rate limits. Falls back to Gemini if NIM is
/// unavailable.
pub async fn review_render(
    state: &Arc<AppState>,
    output_url: &str,
    original_prompt: &str,
    tool_name: &str,
    delivery_id: Option<uuid::Uuid>,
) -> ReviewResult {
    let prompt = review_prompt(original_prompt, tool_name, output_url);

    // 1. Try NVIDIA NIM vision model first
    if let Some(nim) = state.nvidia_nim_vision_client.as_ref() {
        let review = run_review_via_nim(nim, output_url, &prompt).await;
        if review.score > 0 {
            persist_review(state, tool_name, &review, delivery_id, output_url).await;
            return review;
        }
    }

    // 2. Fall back to Gemini
    let gemini = match state
        .video_gemini_client
        .as_ref()
        .or(state.gemini_client.as_ref())
    {
        Some(g) => g,
        None => {
            return ReviewResult {
                pass: true,
                score: 0,
                feedback: "No reviewer (NIM or Gemini) configured — QA review skipped".to_string(),
                retry_hint: None,
            };
        }
    };

    let review = run_review_via_gemini(gemini, output_url, &prompt).await;
    persist_review(state, tool_name, &review, delivery_id, output_url).await;
    review
}

/// Run review via NVIDIA NIM vision model (primary path).
async fn run_review_via_nim(
    nim: &NvidiaNimClient,
    output_url: &str,
    prompt: &str,
) -> ReviewResult {
    let response = match multimodal_review_via_nim(nim, output_url, prompt).await {
        Ok(r) => r,
        Err(e) => {
            return ReviewResult {
                pass: true,
                score: 0,
                feedback: format!("NIM review call failed: {}", e),
                retry_hint: None,
            };
        }
    };

    parse_review_response(&response)
}

/// Run review via Gemini (fallback path).
async fn run_review_via_gemini(
    gemini: &GeminiClient,
    output_url: &str,
    prompt: &str,
) -> ReviewResult {
    let response = match multimodal_review_via_gemini(gemini, output_url, prompt).await {
        Ok(r) => r,
        Err(e) => {
            return ReviewResult {
                pass: true,
                score: 0,
                feedback: format!("Gemini review call failed: {}", e),
                retry_hint: None,
            };
        }
    };

    parse_review_response(&response)
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
                pass: true,
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

/// Analyze media via NVIDIA NIM vision model (images, audio, or video frames).
async fn multimodal_review_via_nim(
    nim: &NvidiaNimClient,
    output_url: &str,
    prompt: &str,
) -> Result<String, Box<dyn std::error::Error + Send + Sync>> {
    // If URL is a cloud URL (not a local path), download to temp for analysis
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
        let frame_path = format!("{}.review_frame.png", local_url);
        let status = std::process::Command::new("ffmpeg")
            .args([
                "-y", "-i", local_url,
                "-vframes", "1",
                "-q:v", "2",
                &frame_path,
            ])
            .status()
            .map_err(|e| format!("ffmpeg not found: {}", e))?;
        if status.success() {
            if let Ok(bytes) = std::fs::read(&frame_path) {
                let r = nim.analyze_image_bytes(&bytes, prompt).await.map_err(Into::into);
                let _ = std::fs::remove_file(&frame_path);
                r
            } else {
                Err("Failed to read video frame".into())
            }
        } else {
            Err("Failed to extract video frame for NIM review".into())
        }
    } else if let Some(mime_type) = audio_mime_type(&ext) {
        let bytes = std::fs::read(local_path)?;
        nim.analyze_audio_bytes(&bytes, mime_type, prompt).await.map_err(Into::into)
    } else {
        nim.generate_text(prompt).await.map_err(Into::into)
    };

    // Clean up temp file if we downloaded one
    if let Some((temp_path, _)) = _downloaded {
        let _ = std::fs::remove_file(&temp_path);
    }

    result
}

/// Analyze media via Gemini (fallback path).
async fn multimodal_review_via_gemini(
    gemini: &GeminiClient,
    output_url: &str,
    prompt: &str,
) -> Result<String, Box<dyn std::error::Error + Send + Sync>> {
    // If URL is a cloud URL (not a local path), download to temp for analysis
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

    // Clean up temp file if we downloaded one
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
