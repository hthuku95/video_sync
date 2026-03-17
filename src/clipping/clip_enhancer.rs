// src/clipping/clip_enhancer.rs
//
// Phase C+ — Gemini-driven per-clip enhancement in the clipping pipeline.
//
// Runs between clip extraction (Phase C) and thumbnail generation.
// For each extracted clip:
//   1. Extract 3 JPEG frames at 15 / 50 / 85 % of clip duration.
//   2. Send frames + ffprobe metadata to Gemini (multi-image request — no video upload).
//   3. Gemini returns a JSON enhancement plan listing specific tools to apply.
//   4. Apply only those tools in sequence using temp files (never modifies on failure).
//   5. Overwrite the clip with the enhanced version.
//
// All errors are best-effort — if enhancement fails, the original clip is kept unchanged.

use crate::clipping::ai_clipper::ExtractedClipData;
use crate::utils::ffmpeg_utils::{cleanup_temp_files, create_temp_file, extract_frame_at_timestamp};
use crate::AppState;
use base64::prelude::BASE64_STANDARD;
use base64::Engine;
use serde::{Deserialize, Serialize};
use std::sync::Arc;

pub struct ClipEnhancer {
    pub app_state: Arc<AppState>,
}

/// Lightweight quality metadata derived from ffprobe.
struct ClipQualityMetadata {
    duration_secs: f64,
    file_size_mb: f64,
    fps: f64,
    has_audio: bool,
}

/// The enhancement plan returned by Gemini.
#[derive(Debug, Deserialize, Serialize)]
struct ClipEnhancementPlan {
    needs_enhancement: bool,
    tools: Vec<String>,
    #[serde(default)]
    reasoning: String,
}

impl ClipEnhancer {
    pub fn new(app_state: Arc<AppState>) -> Self {
        Self { app_state }
    }

    // ── Public entry points ───────────────────────────────────────────────────

    /// Enhance all clips sequentially (best-effort — never returns Err).
    pub async fn enhance_clips_with_ai(
        &self,
        clips: &mut Vec<ExtractedClipData>,
        content_type: &str,
    ) {
        for clip in clips.iter_mut() {
            self.enhance_clip(clip, content_type).await;
        }
    }

    /// Enhance a single clip (best-effort — never returns Err, logs warnings on failure).
    pub async fn enhance_clip(&self, clip: &mut ExtractedClipData, content_type: &str) {
        tracing::info!(
            "🔍 Phase C+ inspecting clip {} ({}) for AI enhancements",
            clip.clip_number,
            clip.ai_title
        );

        match self.run_enhancement(clip, content_type).await {
            Ok((0, _, _)) => {
                tracing::info!("✅ Clip {}: no enhancements needed", clip.clip_number);
            }
            Ok((applied, tools, reasoning)) => {
                tracing::info!(
                    "✨ Clip {}: {} enhancement(s) applied",
                    clip.clip_number,
                    applied
                );
                clip.ai_tags.push(format!("ai_enhanced_{}tools", applied));
                clip.enhancement_applied = true;
                clip.enhancement_tools = tools;
                clip.enhancement_reasoning = Some(reasoning);
            }
            Err(e) => {
                tracing::warn!(
                    "⚠️  Clip {}: enhancement skipped ({}), keeping original",
                    clip.clip_number,
                    e
                );
            }
        }
    }

    // ── Internal pipeline ─────────────────────────────────────────────────────

    async fn run_enhancement(
        &self,
        clip: &ExtractedClipData,
        content_type: &str,
    ) -> Result<(usize, Vec<String>, String), String> {
        // Need Gemini client — skip if not configured
        let gemini_client = self
            .app_state
            .gemini_client
            .as_ref()
            .ok_or("Gemini client not available — skipping enhancement")?;

        // Step 1: assess clip quality via ffprobe
        let metadata = assess_clip_quality(&clip.local_clip_path)?;

        // Step 2: extract 3 inspection frames
        let frame_paths = extract_inspection_frames(&clip.local_clip_path, clip.clip_number)?;

        // Step 3: ask Gemini for an enhancement plan
        let plan = ask_gemini_for_plan(
            gemini_client,
            &clip.ai_title,
            content_type,
            &metadata,
            &frame_paths,
        )
        .await;

        // Always clean up frame files regardless of success/failure
        cleanup_temp_files(&frame_paths);

        let plan = plan?;

        if !plan.needs_enhancement || plan.tools.is_empty() {
            return Ok((0, Vec::new(), String::new()));
        }

        tracing::info!(
            "🛠  Clip {}: Gemini selected {} tool(s): {:?} — \"{}\"",
            clip.clip_number,
            plan.tools.len(),
            plan.tools,
            plan.reasoning
        );

        // Step 4: apply enhancement tools (sync FFmpeg chain via spawn_blocking)
        let tool_count = plan.tools.len();
        let applied_tools = plan.tools.clone();
        let reasoning = plan.reasoning.clone();
        let clip_path = clip.local_clip_path.clone();
        tokio::task::spawn_blocking(move || apply_enhancement_plan(&clip_path, &plan))
            .await
            .map_err(|e| format!("Enhancement task panicked: {}", e))??;

        Ok((tool_count, applied_tools, reasoning))
    }
}

// ── Free functions ────────────────────────────────────────────────────────────

/// Assess clip quality using ffprobe.
fn assess_clip_quality(clip_path: &str) -> Result<ClipQualityMetadata, String> {
    let meta = crate::core::analyze_video(clip_path)?;
    Ok(ClipQualityMetadata {
        duration_secs: meta.duration_seconds,
        file_size_mb: meta.file_size_mb,
        fps: meta.fps,
        has_audio: meta.has_audio,
    })
}

/// Extract 3 inspection frames at 15%, 50%, 85% of clip duration.
fn extract_inspection_frames(
    clip_path: &str,
    clip_number: i32,
) -> Result<Vec<String>, String> {
    let duration = crate::core::get_video_duration(clip_path)?;

    let timestamps = [duration * 0.15, duration * 0.50, duration * 0.85];

    let mut frame_paths = Vec::new();
    for (i, &ts) in timestamps.iter().enumerate() {
        let frame_path = create_temp_file(
            &format!("clip{}_inspect_frame{}", clip_number, i),
            "jpg",
        );
        match extract_frame_at_timestamp(clip_path, ts, &frame_path) {
            Ok(path) => frame_paths.push(path),
            Err(e) => {
                cleanup_temp_files(&frame_paths);
                return Err(format!(
                    "Frame {}/{} extraction failed: {}",
                    i + 1,
                    timestamps.len(),
                    e
                ));
            }
        }
    }

    Ok(frame_paths)
}

/// Ask Gemini to analyze the clip frames and return an enhancement plan.
async fn ask_gemini_for_plan(
    gemini_client: &crate::gemini_client::GeminiClient,
    title: &str,
    content_type: &str,
    metadata: &ClipQualityMetadata,
    frame_paths: &[String],
) -> Result<ClipEnhancementPlan, String> {
    // Read all frames into memory
    let mut frame_bytes_vec: Vec<Vec<u8>> = Vec::new();
    for path in frame_paths {
        let bytes = tokio::fs::read(path)
            .await
            .map_err(|e| format!("Failed to read frame {}: {}", path, e))?;
        frame_bytes_vec.push(bytes);
    }

    let prompt = format!(
        r#"You are a professional video editor analyzing a YouTube Shorts clip for quality improvements.

Clip title: {title}
Content type: {content_type}
Duration: {dur:.1}s | File size: {size:.1}MB | FPS: {fps:.0} | Has audio: {audio}

The {n} frames below are sampled at 15%, 50%, and 85% of the clip duration.

Choose ONLY enhancements that will clearly improve this specific clip. Be conservative — if quality is already good, return needs_enhancement=false with an empty tools array.

Available tools:
- extra_stabilize: Additional stabilization pass (use ONLY if shaky/jittery motion is clearly visible)
- vibrance_boost: Boost color saturation/vibrancy (use if colors look flat or washed out)
- color_temperature: Adjust white balance (use if scene looks too warm, cool, or green-tinted)
- exposure_fix: Adjust brightness (use if clip is noticeably too dark or overexposed)
- sharpen: Extra sharpening pass (use if video looks soft or blurry)
- deflicker: Remove flickering (use if there is visible frame-to-frame brightness variation)
- audio_denoise: Remove background noise (use if content type suggests noisy environment)
- audio_boost: Increase loudness (use if content is quiet narration or soft speech)

Respond with valid JSON only — no markdown, no code fences:
{{"needs_enhancement": true, "tools": ["tool1"], "reasoning": "brief reason"}}"#,
        title = title,
        content_type = content_type,
        dur = metadata.duration_secs,
        size = metadata.file_size_mb,
        fps = metadata.fps,
        audio = metadata.has_audio,
        n = frame_paths.len(),
    );

    // Build multi-image request: text prompt + frame images as InlineData parts
    let mut content_parts = vec![crate::gemini_client::Part::Text { text: prompt }];
    for frame_bytes in frame_bytes_vec {
        content_parts.push(crate::gemini_client::Part::InlineData {
            inline_data: crate::gemini_client::InlineData {
                mime_type: "image/jpeg".to_string(),
                data: BASE64_STANDARD.encode(&frame_bytes),
            },
        });
    }

    let request = crate::gemini_client::GenerateContentRequest {
        contents: vec![crate::gemini_client::Content {
            role: Some("user".to_string()),
            parts: content_parts,
        }],
        generation_config: None,
        tools: None,
        tool_config: None,
        system_instruction: None,
    };

    let response = gemini_client
        .generate_content(request)
        .await
        .map_err(|e| format!("Gemini enhancement analysis failed: {}", e))?;

    let response_text = response
        .candidates
        .first()
        .and_then(|c| c.content.as_ref())
        .and_then(|content| content.parts.first())
        .and_then(|part| {
            if let crate::gemini_client::Part::Text { text } = part {
                Some(text.clone())
            } else {
                None
            }
        })
        .ok_or("Gemini returned empty response for enhancement plan")?;

    let json_str = strip_json_fences(&response_text);
    serde_json::from_str::<ClipEnhancementPlan>(json_str).map_err(|e| {
        format!(
            "Failed to parse Gemini enhancement plan: {} — raw: {}",
            e, json_str
        )
    })
}

/// Strip ```json ... ``` or ``` ... ``` wrappers from a Gemini response.
fn strip_json_fences(s: &str) -> &str {
    let s = s.trim();
    let s = s.strip_prefix("```json").unwrap_or(s);
    let s = s.strip_prefix("```").unwrap_or(s);
    let s = s.strip_suffix("```").unwrap_or(s);
    s.trim()
}

/// Apply the enhancement plan to the clip in-place using a temp-file chain.
/// On any failure, the original clip is preserved unchanged.
fn apply_enhancement_plan(clip_path: &str, plan: &ClipEnhancementPlan) -> Result<(), String> {
    if !plan.needs_enhancement || plan.tools.is_empty() {
        return Ok(());
    }

    let mut temp_files: Vec<String> = Vec::new();
    let mut current_input = clip_path.to_string();

    for tool in &plan.tools {
        let temp_out = create_temp_file(&format!("enhance_{}", tool), "mp4");

        let result = match tool.as_str() {
            "extra_stabilize" => {
                let r = crate::visual::stabilize_video_2pass(
                    &current_input,
                    &temp_out,
                    7,
                    15,
                    12,
                    0.0,
                );
                // Clean up the .trf sidecar created by vidstabdetect
                std::fs::remove_file(format!("{}.trf", current_input)).ok();
                r
            }
            "vibrance_boost" => {
                crate::visual::adjust_vibrance(&current_input, &temp_out, 0.3, 0.0, 0.0, 0.0)
            }
            "color_temperature" => {
                crate::visual::adjust_color_temperature(&current_input, &temp_out, 0.1, 1.0)
            }
            "exposure_fix" => {
                crate::visual::adjust_exposure(&current_input, &temp_out, 0.5, 0.05)
            }
            "sharpen" => crate::visual::apply_cas(&current_input, &temp_out, 0.5, 7),
            "deflicker" => crate::visual::remove_flicker(&current_input, &temp_out, 2, "median"),
            "audio_denoise" => {
                crate::audio::denoise_audio_fft(&current_input, &temp_out, -25.0, 0.3, false)
            }
            "audio_boost" => {
                crate::audio::normalize_loudness(&current_input, &temp_out, -12.0, 11.0, -1.0)
            }
            unknown => {
                tracing::warn!("Unknown enhancement tool '{}' — skipping", unknown);
                continue;
            }
        };

        match result {
            Ok(_) => {
                // Clean up previous temp file (never remove the original clip)
                if current_input != clip_path {
                    std::fs::remove_file(&current_input).ok();
                    temp_files.retain(|f| f != &current_input);
                }
                temp_files.push(temp_out.clone());
                current_input = temp_out;
            }
            Err(e) => {
                // Tool failed — clean up and preserve the original unchanged
                cleanup_temp_files(&temp_files);
                std::fs::remove_file(&temp_out).ok();
                return Err(format!("Enhancement tool '{}' failed: {}", tool, e));
            }
        }
    }

    // Overwrite original clip with the final enhanced version
    if current_input != clip_path {
        std::fs::copy(&current_input, clip_path).map_err(|e| {
            cleanup_temp_files(&[current_input.clone()]);
            format!("Failed to save enhanced clip: {}", e)
        })?;
        std::fs::remove_file(&current_input).ok();
    }

    Ok(())
}
