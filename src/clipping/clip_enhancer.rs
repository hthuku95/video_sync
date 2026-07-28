// src/clipping/clip_enhancer.rs
//
// Phase C+ — multimodal (Ollama → Gemini) per-clip enhancement in the clipping pipeline.
//
// Runs between clip extraction (Phase C) and thumbnail generation.
// For each extracted clip:
//   1. Extract 3 JPEG frames at 15 / 50 / 85 % of clip duration.
//   2. Send frames + ffprobe metadata to Ollama (gemma4:12b multimodal via generate_text_with_images)
//      or Gemini (fallback) (multi-image request — no video upload).
//   3. LLM returns a JSON enhancement plan listing specific tools to apply.
//   4. Apply only those tools in sequence using temp files (never modifies on failure).
//   5. Overwrite the clip with the enhanced version.
//
// All errors are best-effort — if enhancement fails, the original clip is kept unchanged.

use crate::clipping::ai_clipper::ExtractedClipData;
use crate::utils::ffmpeg_utils::{
    cleanup_temp_files, create_temp_file, extract_frame_at_timestamp,
};
use crate::AppState;
use base64::prelude::BASE64_STANDARD;
use base64::Engine;
use serde::{Deserialize, Serialize};
use std::sync::Arc;
use std::time::Duration;

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
        // Step 1: assess clip quality via ffprobe
        let metadata = assess_clip_quality(&clip.local_clip_path)?;

        // Step 2: extract 3 inspection frames
        let frame_paths = extract_inspection_frames(&clip.local_clip_path, clip.clip_number)?;

        // Step 3: ask multimodal for an enhancement plan (Ollama → Gemini fallback)
        let plan_future = ask_multimodal_for_plan(
            &self.app_state,
            &clip.ai_title,
            content_type,
            &metadata,
            &frame_paths,
        );
        let plan_result =
            tokio::time::timeout(Duration::from_secs(90), plan_future).await;

        // Always clean up frame files regardless of success/failure/timeout
        cleanup_temp_files(&frame_paths);

        let plan = match plan_result {
            Err(_elapsed) => {
                tracing::warn!(
                    "Clip {}: enhancement timed out after 90s, keeping original",
                    clip.clip_number
                );
                return Ok((0, Vec::new(), String::new()));
            }
            Ok(result) => result?,
        };

        if !plan.needs_enhancement || plan.tools.is_empty() {
            return Ok((0, Vec::new(), String::new()));
        }

        tracing::info!(
            "🛠  Clip {}: selected {} tool(s): {:?} — \"{}\"",
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
fn extract_inspection_frames(clip_path: &str, clip_number: i32) -> Result<Vec<String>, String> {
    let duration = crate::core::get_video_duration(clip_path)?;

    let timestamps = [duration * 0.15, duration * 0.50, duration * 0.85];

    let mut frame_paths = Vec::new();
    for (i, &ts) in timestamps.iter().enumerate() {
        let frame_path =
            create_temp_file(&format!("clip{}_inspect_frame{}", clip_number, i), "jpg");
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

/// Ask Ollama (first) or Gemini (fallback) to analyze the clip frames and return an enhancement plan.
async fn ask_multimodal_for_plan(
    app_state: &AppState,
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

    let prompt = build_enhancement_prompt(title, content_type, metadata, frame_paths.len());

    // Try Ollama first (gemma4:12b multimodal via generate_text_with_images)
    if let Some(ollama_client) = &app_state.ollama_client {
        let frame_pairs: Vec<(String, Vec<u8>)> = frame_bytes_vec
            .iter()
            .map(|bytes| (String::new(), bytes.clone()))
            .collect();

        match tokio::time::timeout(Duration::from_secs(45), async {
            ollama_client
                .generate_text_with_images(&prompt, frame_pairs)
                .await
        })
        .await
        {
            Ok(Ok(response)) => {
                let json_str = strip_json_fences(&response);
                match serde_json::from_str::<ClipEnhancementPlan>(json_str) {
                    Ok(plan) => {
                        tracing::info!(
                            "Ollama plan for '{}': {} tools selected",
                            title,
                            plan.tools.len()
                        );
                        return Ok(plan);
                    }
                    Err(e) => {
                        tracing::warn!(
                            "Ollama plan parse error ({}), falling back to Gemini. Raw: {}",
                            e,
                            json_str
                        );
                    }
                }
            }
            Ok(Err(e)) => {
                tracing::warn!("Ollama enhancement failed: {}, falling back to Gemini", e);
            }
            Err(_) => {
                tracing::warn!("Ollama enhancement timed out, falling back to Gemini");
            }
        }
    }

    // Fallback: Gemini (if available)
    let gemini_client = match &app_state.gemini_client {
        Some(c) => c,
        None => {
            tracing::warn!("No Gemini client available either — skipping enhancement");
            return Ok(ClipEnhancementPlan {
                needs_enhancement: false,
                tools: vec![],
                reasoning: "no_provider".to_string(),
            });
        }
    };

    // Build multi-image request: text prompt + frame images as InlineData parts
    let mut content_parts = vec![crate::gemini_client::Part::Text { text: prompt }];
    for frame_bytes in &frame_bytes_vec {
        content_parts.push(crate::gemini_client::Part::InlineData {
            inline_data: crate::gemini_client::InlineData {
                mime_type: "image/jpeg".to_string(),
                data: BASE64_STANDARD.encode(frame_bytes),
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

    match gemini_client.generate_content(request).await {
        Ok(response) => {
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
                .unwrap_or_default();

            let json_str = strip_json_fences(&response_text);
            match serde_json::from_str::<ClipEnhancementPlan>(json_str) {
                Ok(plan) => Ok(plan),
                Err(e) => {
                    tracing::warn!(
                        "Gemini plan parse error ({}), skipping enhancement. Raw: {}",
                        e,
                        json_str
                    );
                    Ok(ClipEnhancementPlan {
                        needs_enhancement: false,
                        tools: vec![],
                        reasoning: "parse_error".to_string(),
                    })
                }
            }
        }
        Err(e) => {
            tracing::warn!(
                "Gemini enhancement failed ({}), keeping original",
                e
            );
            Ok(ClipEnhancementPlan {
                needs_enhancement: false,
                tools: vec![],
                reasoning: "gemini_failed".to_string(),
            })
        }
    }
}

fn build_enhancement_prompt(
    title: &str,
    content_type: &str,
    metadata: &ClipQualityMetadata,
    frame_count: usize,
) -> String {
    format!(
        r#"You are a professional video editor analyzing a YouTube Shorts clip for quality improvements.

Clip title: {title}
Content type: {content_type}
Duration: {dur:.1}s | File size: {size:.1}MB | FPS: {fps:.0} | Has audio: {audio}

The {n} frames below are sampled at 15%, 50%, and 85% of the clip duration.

Choose ONLY enhancements that will clearly improve this specific clip. Be conservative — if quality is already good, return needs_enhancement=false with an empty tools array. Pick at most 3 tools.

Available visual tools:
- extra_stabilize: Additional stabilization pass (ONLY if shaky/jittery motion is clearly visible)
- vibrance_boost: Boost color saturation/vibrancy (if colors look flat or washed out)
- color_temperature: Adjust white balance (if scene looks too warm, cool, or green-tinted)
- exposure_fix: Adjust brightness/contrast (if clip is noticeably too dark or overexposed)
- sharpen: Extra sharpening pass (if video looks soft or blurry)
- deflicker: Remove flickering (if visible frame-to-frame brightness variation)
- color_balance: Fine-tune shadow/midtone/highlight color balance (if color grading is off)
- normalize_video: Normalize histogram for more even exposure across frames
- denoise_video: Spatial+temporal denoising (if video looks grainy or noisy)
- add_vignette: Subtle vignette to focus attention (for cinematic or talking-head content)
- split_tone: Apply cool shadows + warm highlights color grade (cinematic look)
- hue_adjust: Tweak hue/saturation of specific color range

Available audio tools:
- audio_denoise: Remove background hiss/noise (use if noisy environment evident)
- audio_boost: Increase loudness of quiet narration or soft speech
- audio_compress: Dynamic range compression (if audio is too dynamic — loud parts much louder than quiet)
- audio_gate: Noise gate to silence background noise between speech segments
- audio_highpass: High-pass filter to remove low-frequency rumble (indoor/outdoor recording)
- audio_bass_boost: Boost bass frequencies (music, podcast, entertainment content)
- audio_treble_boost: Boost high frequencies for clearer speech clarity
- audio_limiter: Final loudness limiter to prevent clipping and ensure broadcast level

Respond with valid JSON only — no markdown, no code fences:
{{"needs_enhancement": true, "tools": ["tool1", "tool2"], "reasoning": "brief reason"}}"#,
        title = title,
        content_type = content_type,
        dur = metadata.duration_secs,
        size = metadata.file_size_mb,
        fps = metadata.fps,
        audio = metadata.has_audio,
        n = frame_count,
    )
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
                let r =
                    crate::visual::stabilize_video_2pass(&current_input, &temp_out, 7, 15, 12, 0.0);
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
            "exposure_fix" => crate::visual::adjust_exposure(&current_input, &temp_out, 0.5, 0.05),
            "sharpen" => crate::visual::apply_cas(&current_input, &temp_out, 0.5, 7),
            "deflicker" => crate::visual::remove_flicker(&current_input, &temp_out, 2, "median"),
            "audio_denoise" => {
                crate::audio::denoise_audio_fft(&current_input, &temp_out, -25.0, 0.3, false)
            }
            "audio_boost" => {
                crate::audio::normalize_loudness(&current_input, &temp_out, -12.0, 11.0, -1.0)
            }
            // ── Extended visual tools ─────────────────────────────────────────
            "color_balance" => {
                // Slight warm highlights, neutral shadows/midtones
                crate::visual::color_balance(
                    &current_input,
                    &temp_out,
                    (0.0, 0.0, 0.0),     // shadows: neutral
                    (0.0, 0.0, 0.0),     // midtones: neutral
                    (0.05, 0.02, -0.03), // highlights: warm
                )
            }
            "normalize_video" => crate::visual::normalize_video(&current_input, &temp_out, 10),
            "denoise_video" => {
                // Moderate spatial+temporal denoise (luma_spatial, luma_temporal, chroma_spatial, chroma_temporal)
                crate::visual::denoise_video(&current_input, &temp_out, 4.0, 3.0, 3.0, 2.0)
            }
            "add_vignette" => {
                crate::visual::add_vignette(
                    &current_input,
                    &temp_out,
                    std::f64::consts::PI / 5.0, // ~36° angle
                    "forward",
                )
            }
            "split_tone" => {
                // Cool shadows (240° blue-ish), warm highlights (40° orange-ish)
                crate::visual::split_tone(
                    &current_input,
                    &temp_out,
                    240.0,
                    0.08, // shadow hue + saturation
                    40.0,
                    0.08, // highlight hue + saturation
                    0.5,  // balance
                )
            }
            "hue_adjust" => {
                // Gentle saturation boost via hue filter (no hue shift)
                crate::visual::adjust_hue(&current_input, &temp_out, 0.0, 1.15)
            }
            // ── Extended audio tools ──────────────────────────────────────────
            "audio_compress" => {
                // Moderate compression: -18dB threshold, 4:1 ratio, 5ms attack, 100ms release, 2dB makeup
                crate::audio::compress_audio(&current_input, &temp_out, -18.0, 4.0, 5.0, 100.0, 2.0)
            }
            "audio_gate" => {
                // Noise gate: -35dB threshold, 2:1 ratio, 5ms attack, 150ms release
                crate::audio::gate_audio(&current_input, &temp_out, -35.0, 2.0, 5.0, 150.0)
            }
            "audio_highpass" => {
                // Remove low-frequency rumble below 80 Hz
                crate::audio::filter_highpass(&current_input, &temp_out, 80.0, 2, 0.7)
            }
            "audio_bass_boost" => {
                // +3dB around 100 Hz
                crate::audio::adjust_bass(&current_input, &temp_out, 3.0, 100.0, 200.0)
            }
            "audio_treble_boost" => {
                // +2dB around 8 kHz for clearer speech/presence
                crate::audio::adjust_treble(&current_input, &temp_out, 2.0, 8000.0, 200.0)
            }
            "audio_limiter" => {
                // Broadcast limiter: -1dB ceiling, 5ms attack, 50ms release
                crate::audio::audio_limiter(&current_input, &temp_out, -1.0, 5.0, 50.0, true)
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
