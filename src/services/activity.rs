//! Customer-safe production activity.
//!
//! The internal workflow event log is intentionally rich: it carries raw agent
//! progress, tool names, review notes and diagnostics. Customers must never see
//! prompts, tool arguments, URLs, API errors or model output. This module turns
//! those internal records into a small, stable vocabulary of stage labels that
//! the campaign UIs can render.
//!
//! Rules:
//! - Never return raw event `details`.
//! - Never return raw error strings.
//! - Only return labels from this file's allow-list.
//! - Unknown events degrade to a generic "Working on your video" label.

use serde_json::Value;

/// Turn one raw agent-progress line into a customer-safe label.
pub fn public_progress_label(raw: &str) -> String {
    let lower = raw.to_lowercase();

    // Tool calls are the most useful signal. Extract only the tool name and map
    // it through a fixed allow-list; never echo arguments or file paths.
    if let Some(idx) = raw.find("Detected tool call:") {
        let tool = raw[idx + "Detected tool call:".len()..]
            .trim()
            .split_whitespace()
            .next()
            .unwrap_or("")
            .trim_matches(|c: char| !c.is_ascii_alphanumeric() && c != '_');
        return tool_label(tool).to_string();
    }
    if let Some(idx) = raw.find("calling:") {
        let tool = raw[idx + "calling:".len()..]
            .trim()
            .split_whitespace()
            .next()
            .unwrap_or("")
            .trim_matches(|c: char| !c.is_ascii_alphanumeric() && c != '_');
        return tool_label(tool).to_string();
    }

    if raw.contains("💬") {
        return "Producer note received".to_string();
    }
    if lower.contains("cancel") {
        return "Cancelled".to_string();
    }
    if lower.contains("review") {
        return "Reviewing the cut".to_string();
    }
    if lower.contains("voiceover") || lower.contains("voice over") || lower.contains("text_to_speech") {
        return "Preparing narration".to_string();
    }
    if lower.contains("subtitle") || lower.contains("caption") || lower.contains("transcrib") {
        return "Preparing captions".to_string();
    }
    if lower.contains("merge") || lower.contains("assemble") || lower.contains("final cut") {
        return "Assembling the final cut".to_string();
    }
    if lower.contains("render") {
        return "Rendering visuals".to_string();
    }
    if lower.contains("crawl") || lower.contains("website") || lower.contains("search") {
        return "Researching the source".to_string();
    }
    if lower.contains("processing") || lower.contains("analyz") {
        return "Analyzing the brief".to_string();
    }
    if lower.contains("completed") || lower.contains("done") {
        return "Step completed".to_string();
    }

    "Working on your video".to_string()
}

fn tool_label(tool: &str) -> &'static str {
    match tool {
        "generate_video_script" => "Writing the script",
        "hyperframes_render" => "Rendering motion graphics",
        "manim_execute_script" => "Rendering diagrams",
        "browserbase_crawl_website" => "Reading the website",
        "vectorize_crawled_content" | "search_crawled_content" => "Researching the brand",
        "generate_clip_compilation" => "Finding the best moments",
        "add_voiceover_to_video" | "generate_text_to_speech" => "Recording narration",
        "transcribe_audio_url" | "add_subtitles" => "Preparing captions",
        "merge_videos" | "apply_xfade_transition" | "add_transition" => "Assembling the final cut",
        "review_video" | "view_image" => "Reviewing the cut",
        "submit_final_answer" => "Finalizing the video",
        "download_asset" => "Gathering brand assets",
        "create_thumbnail_hd" | "generate_image" => "Designing visuals",
        "add_text_overlay" | "add_overlay" => "Polishing the graphics",
        "adjust_color" | "adjust_curves" | "adjust_levels" | "apply_lut" => "Color grading",
        "generate_music" | "generate_sound_effect" | "add_audio" => "Mixing audio",
        _ => "Running a production step",
    }
}

/// Human label for an internal `current_step` value.
pub fn public_step_label(step: &str) -> String {
    let s = step.trim();
    if s.is_empty() {
        return "Preparing your video".to_string();
    }
    match s {
        "queued" => "Waiting for a render slot",
        "planning" | "execution_plan" => "Planning the production",
        "agentic_attempt" => "Producing the video",
        "completed" => "Ready",
        "cancelled" | "cancelled_by_user" => "Cancelled",
        "agentic_review_passed" => "Quality review passed",
        "agentic_review_unavailable" => "Quality check completed",
        "agentic_pipeline_failed" => "Production needs attention",
        _ if s.contains("review") => "Reviewing the cut",
        _ if s.contains("attempt") => "Producing the video",
        _ if s.contains("queued") => "Waiting for a render slot",
        _ => "Working on your video",
    }
    .to_string()
}

/// Customer-safe label for one persisted workflow event. Returns None for
/// internal-only events that should not surface in the customer activity feed.
pub fn public_event_label(event_type: &str, message: &str, details: &Value) -> Option<String> {
    match event_type {
        "agent_progress" => Some(public_progress_label(message)),
        "user_feedback" => Some("Your note was sent to the producer".to_string()),
        "execution_plan" => Some("Production plan ready".to_string()),
        "agentic_attempt" => {
            let label = message.trim();
            if label.is_empty() {
                Some("Producer started".to_string())
            } else {
                // Message is generated by the pipeline ("Attempt N of M").
                Some(format!("Producer started — {label}"))
            }
        }
        "agentic_review_passed" => Some("Quality review passed".to_string()),
        "agentic_review_unavailable" => Some("Quality check completed".to_string()),
        "agentic_pipeline_failed" | "failed" => Some("Production needs attention".to_string()),
        "completed" => Some("Video ready".to_string()),
        "cancelled" => Some("Cancelled".to_string()),
        "progress" => Some(public_step_label(message)),
        "artifact" => Some("New render ready".to_string()),
        // Internal diagnostics, tool traces and usage records stay internal.
        "tool_trace" | "llm_call" | "usage" | "debug" => None,
        _ => None,
    }
}

/// Coarse production stage used by the UI progress indicator.
/// 1 = queued, 2 = planning, 3 = producing, 4 = review, 5 = ready.
pub fn stage_for(status: &str, current_step: &str, latest_event: Option<&str>) -> (i32, &'static str) {
    let step = current_step.to_ascii_lowercase();
    let event = latest_event.unwrap_or("").to_ascii_lowercase();

    if status == "completed" || step == "completed" {
        return (5, "Ready");
    }
    if status == "cancelled" || step.contains("cancelled") {
        return (5, "Cancelled");
    }
    if status == "failed" || event.contains("failed") || step.contains("failed") {
        return (5, "Needs attention");
    }
    if step.contains("review") || event.contains("review") {
        return (4, "Quality review");
    }
    if step == "agentic_attempt"
        || step == "running"
        || event == "agent_progress"
        || event == "artifact"
        || event == "agentic_attempt"
    {
        return (3, "Producing");
    }
    if step == "planning" || step == "execution_plan" || event == "execution_plan" {
        return (2, "Planning");
    }
    if step == "queued" || status == "queued" {
        return (1, "Queued");
    }
    (2, "Preparing")
}

/// Short, bounded copy of a user's own note for display back to that user.
pub fn visible_user_note(details: &Value) -> Option<String> {
    let text = details
        .get("message")
        .and_then(|v| v.as_str())
        .or_else(|| details.get("feedback").and_then(|v| v.as_str()))?;
    let trimmed = text.trim();
    if trimmed.is_empty() {
        return None;
    }
    let mut out: String = trimmed.chars().take(240).collect();
    if trimmed.chars().count() > 240 {
        out.push('…');
    }
    Some(out)
}
