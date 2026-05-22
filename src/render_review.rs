//! LLM-powered QA review for ALL rendered outputs — Blender jobs,
//! FFmpeg video edits, clip jobs, auto-generated videos, anything that
//! produces a file we hand back to a user or paying client.
//!
//! Runs after render, before return. Flow:
//!   render_complete(url, prompt, tool) → Gemini scores 1-10, returns
//!   `{pass: bool, score, feedback, retry_hint}` → if pass, caller hands
//!   the URL to the user; if fail + first attempt, caller retries with
//!   the hint appended to the prompt; if fail after retry, caller still
//!   returns the URL but with a warning flag the UI surfaces.
//!
//! Also writes one row per review to `blender_render_reviews` so the
//! admin dashboard can see pass/fail rate per tool over time.

use crate::gemini_client::GeminiClient;
use crate::AppState;
use serde::{Deserialize, Serialize};
use std::path::Path;
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
/// returns a "pass with score 0" result if Gemini is unavailable.
pub async fn review_render(
    state: &Arc<AppState>,
    output_url: &str,
    original_prompt: &str,
    tool_name: &str,
    delivery_id: Option<uuid::Uuid>,
) -> ReviewResult {
    // Prefer the dedicated video_gemini client (separate quota) so we
    // don't eat into the main agent's rate limit.
    let gemini = match state
        .video_gemini_client
        .as_ref()
        .or(state.gemini_client.as_ref())
    {
        Some(g) => g,
        None => {
            return ReviewResult {
                pass: true, // fail-open — no reviewer ≠ blocked render
                score: 0,
                feedback: "Gemini not configured — QA review skipped".to_string(),
                retry_hint: None,
            };
        }
    };

    let review = run_review(gemini, output_url, original_prompt, tool_name).await;
    persist_review(state, tool_name, &review, delivery_id, output_url).await;
    review
}

/// The actual Gemini call. Returns a fresh ReviewResult.
async fn run_review(
    gemini: &GeminiClient,
    output_url: &str,
    original_prompt: &str,
    tool_name: &str,
) -> ReviewResult {
    let prompt = review_prompt(original_prompt, tool_name, output_url);
    let response = match multimodal_review_response(gemini, output_url, &prompt).await {
        Ok(r) => r,
        Err(e) => {
            return ReviewResult {
                pass: true, // fail-open if Gemini errors
                score: 0,
                feedback: format!("Review call failed: {}", e),
                retry_hint: None,
            };
        }
    };

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

async fn multimodal_review_response(
    gemini: &GeminiClient,
    output_url: &str,
    prompt: &str,
) -> Result<String, Box<dyn std::error::Error + Send + Sync>> {
    let path = Path::new(output_url);
    if path.exists() {
        let ext = path
            .extension()
            .and_then(|ext| ext.to_str())
            .map(|ext| ext.to_ascii_lowercase())
            .unwrap_or_default();

        if is_image_extension(&ext) {
            let bytes = std::fs::read(path)?;
            return gemini.analyze_image_bytes(&bytes, prompt).await;
        }

        if is_video_extension(&ext) {
            return gemini
                .analyze_video_content(path.to_string_lossy().as_ref(), Some(prompt.to_string()))
                .await;
        }

        if let Some(mime_type) = audio_mime_type(&ext) {
            let bytes = std::fs::read(path)?;
            return gemini.analyze_audio_bytes(&bytes, mime_type, prompt).await;
        }
    }

    gemini.generate_text(prompt).await
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
