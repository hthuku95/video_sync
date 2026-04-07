// AI-powered viral clip identification and extraction — NEW ARCHITECTURE
//
// Single Gemini video analysis call via YouTube URL, replacing the old
// 100+ sequential frame-by-frame approach.
//
// Pipeline:
//   Phase A: analyze_video_from_url (1 Gemini call, 30-90 seconds)
//   Phase B: download video (only if Phase A found quality clips)
//   Phase C: parallel FFmpeg trim for all clips
//   Phase D: thumbnail extraction at Gemini-specified timestamp
//   (Vectorization is handled separately in video_vectorization.rs)

use crate::clipping::clip_enhancer::ClipEnhancer;
use crate::clipping::gemini_video_analyzer::ViralMoment;
use crate::clipping::thumbnail_generator::ThumbnailGenerator;
use crate::AppState;
use std::sync::Arc;

pub struct AiClipper {
    pub app_state: Arc<AppState>,
}

impl AiClipper {
    pub fn new(app_state: Arc<AppState>) -> Self {
        Self { app_state }
    }

    /// Extract viral clips from a downloaded video file using Gemini analysis results.
    ///
    /// This is Phase C of the new pipeline — the actual FFmpeg trimming.
    /// Phase A (Gemini analysis) and Phase B (download) happen in clipping_job.rs.
    ///
    /// All clips are extracted in parallel using tokio spawn_blocking.
    pub async fn extract_clips_from_moments(
        &self,
        job_id: i32,
        video_path: &str,
        moments: &[ViralMoment],
        content_type: &str,
    ) -> Result<Vec<ExtractedClipData>, String> {
        tracing::info!(
            "🎬 Extracting {} clips from job {} in parallel (content_type={})",
            moments.len(),
            job_id,
            content_type,
        );

        // Launch all clip extractions in parallel
        let mut handles = Vec::new();

        for (index, moment) in moments.iter().enumerate() {
            let clip_path = format!("outputs/clip_{}_{}.mp4", job_id, index + 1);
            let thumbnail_path = format!("outputs/thumb_{}_{}.jpg", job_id, index + 1);

            let video_path = video_path.to_string();
            let moment = moment.clone();
            let clip_number = (index + 1) as i32;
            let content_type = content_type.to_string();

            let handle = tokio::task::spawn_blocking(move || {
                extract_single_clip(
                    clip_number,
                    &video_path,
                    &clip_path,
                    &thumbnail_path,
                    &moment,
                    &content_type,
                )
            });

            handles.push(handle);
        }

        // Collect results
        let mut extracted_clips = Vec::new();
        let mut first_error: Option<String> = None;
        for handle in handles {
            match handle.await {
                Ok(Ok(clip)) => extracted_clips.push(clip),
                Ok(Err(e)) => {
                    tracing::warn!("Clip extraction failed (skipping): {}", e);
                    if first_error.is_none() {
                        first_error = Some(e);
                    }
                }
                Err(e) => {
                    tracing::warn!("Clip extraction task panicked (skipping): {}", e);
                    if first_error.is_none() {
                        first_error = Some(format!("task panicked: {}", e));
                    }
                }
            }
        }

        if extracted_clips.is_empty() {
            let reason = first_error.unwrap_or_else(|| "unknown — no clips attempted".to_string());
            return Err(format!("All clip extractions failed. First error: {}", reason));
        }

        tracing::info!(
            "✅ Extracted {}/{} clips successfully for job {}",
            extracted_clips.len(),
            moments.len(),
            job_id
        );

        // Phase C+: Gemini-driven per-clip enhancement.
        // Gemini inspects 3 frames per clip and selects specific FFmpeg tools to apply
        // (stabilize, vibrance, sharpen, denoise, etc.) based on actual clip quality.
        // Best-effort — if enhancement fails the original clip is preserved unchanged.
        if self.app_state.gemini_client.is_some() {
            tracing::info!("🎬 Phase C+: running AI clip enhancement for job {}", job_id);
            let enhancer = ClipEnhancer::new(self.app_state.clone());
            enhancer
                .enhance_clips_with_ai(&mut extracted_clips, content_type)
                .await;
        }

        // Phase C++: AI thumbnail selection — replace ffmpeg frame with Gemini-selected best frame.
        // Falls back to ffmpeg thumbnail if Gemini is unavailable or rate-limited.
        if self.app_state.gemini_client.is_some() {
            let thumbnail_gen = ThumbnailGenerator::new(self.app_state.clone());
            for clip in &mut extracted_clips {
                match thumbnail_gen
                    .generate_thumbnail(&clip.local_clip_path, &clip.ai_title, &clip.viral_factors)
                    .await
                {
                    Ok(ai_thumb) => {
                        tracing::info!(
                            "🎨 AI thumbnail for clip {}: {}",
                            clip.clip_number,
                            ai_thumb
                        );
                        clip.custom_thumbnail_path = Some(ai_thumb);
                        clip.thumbnail_generation_method = Some("ai_gemini".to_string());
                    }
                    Err(e) => {
                        tracing::warn!(
                            "AI thumbnail for clip {} failed (keeping ffmpeg fallback): {}",
                            clip.clip_number,
                            e
                        );
                    }
                }
            }
        }

        Ok(extracted_clips)
    }
}

/// Extract a single clip synchronously (runs in spawn_blocking).
fn extract_single_clip(
    clip_number: i32,
    video_path: &str,
    clip_path: &str,
    thumbnail_path: &str,
    moment: &ViralMoment,
    content_type: &str,
) -> Result<ExtractedClipData, String> {
    tracing::info!(
        "✂️  Clip {}: trimming {:.1}s–{:.1}s ({:.1}s) → {}",
        clip_number,
        moment.start_sec,
        moment.end_sec,
        moment.duration(),
        clip_path
    );

    // Two-step extraction to handle large source files efficiently:
    //   Step 1: extract the raw segment with -c copy (fast, minimal I/O, no filter overhead)
    //   Step 2: apply the full filter chain to the small segment file
    //
    // This avoids the "scan entire 700 MB file to find moov atom" problem that
    // causes 10-minute FFmpeg timeouts on Render's slow ephemeral disk.
    let segment_path = format!("{}.segment.mp4", clip_path);
    let segment_duration = moment.end_sec - moment.start_sec + 2.0; // +2s buffer for keyframe alignment

    // Step 1: copy-extract segment (10–60 seconds, no re-encode).
    // -loglevel error suppresses frame-by-frame progress to avoid large stderr buffers.
    let seg_status = std::process::Command::new("ffmpeg")
        .args([
            "-y",
            "-loglevel", "error",
            "-ss", &moment.start_sec.to_string(),
            "-i", video_path,
            "-t", &segment_duration.to_string(),
            "-c", "copy",
            "-avoid_negative_ts", "make_zero",
            &segment_path,
        ])
        .stdout(std::process::Stdio::null())
        .stderr(std::process::Stdio::piped())
        .output()
        .map_err(|e| format!("Clip {} segment extraction spawn failed: {}", clip_number, e))?;

    if !seg_status.status.success() {
        let stderr = String::from_utf8_lossy(&seg_status.stderr);
        return Err(format!("Clip {} segment extraction failed: {}", clip_number, &stderr[..stderr.len().min(400)]));
    }
    tracing::info!("✅ Clip {} segment extracted: {}", clip_number, segment_path);

    // Step 2: apply the full filter chain to the small segment (starts at t=0)
    let result = crate::core::trim_and_convert_to_shorts(
        &segment_path,
        clip_path,
        0.0,
        moment.end_sec - moment.start_sec,
        &moment.title,
        content_type,
    ).map_err(|e| format!("Clip {} shorts-enhance failed: {}", clip_number, e));

    // Always clean up the temp segment, even on failure
    let _ = std::fs::remove_file(&segment_path);
    result?;

    // Extract thumbnail at the Gemini-specified timestamp.
    // Retry up to 3 times with a brief sleep — the trimmed file may not be
    // fully flushed on the first attempt when clips are extracted in parallel.
    let relative_ts = (moment.thumbnail_sec - moment.start_sec).max(0.0);
    let custom_thumbnail = {
        let mut result = None;
        for attempt in 1u32..=3 {
            match crate::utils::ffmpeg_utils::extract_frame_at_timestamp(
                clip_path,
                relative_ts,
                thumbnail_path,
            ) {
                Ok(path) => {
                    result = Some(path);
                    break;
                }
                Err(e) => {
                    if attempt < 3 {
                        tracing::warn!(
                            "Clip {} thumbnail attempt {}/3 failed: {}. Retrying in {}ms...",
                            clip_number, attempt, e, 200 * attempt
                        );
                        std::thread::sleep(std::time::Duration::from_millis(200 * attempt as u64));
                    } else {
                        tracing::warn!(
                            "Clip {} thumbnail failed after 3 attempts: {}",
                            clip_number, e
                        );
                    }
                }
            }
        }
        result
    };

    let has_thumb = custom_thumbnail.is_some();
    Ok(ExtractedClipData {
        clip_number,
        local_clip_path: clip_path.to_string(),
        start_time_seconds: moment.start_sec,
        end_time_seconds: moment.end_sec,
        duration_seconds: moment.duration(),
        ai_title: moment.title.clone(),
        ai_description: moment.hook.clone(),
        ai_tags: moment.viral_factors.clone(),
        ai_confidence_score: moment.quality_score,
        viral_factors: moment.viral_factors.clone(),
        custom_thumbnail_path: custom_thumbnail,
        thumbnail_generation_method: if has_thumb { Some("ffmpeg_timestamp".to_string()) } else { None },
        enhancement_applied: false,
        enhancement_tools: Vec::new(),
        enhancement_reasoning: None,
        r2_clip_key: None,
        r2_thumb_key: None,
        r2_clip_url: None,
    })
}

/// Extracted clip data (before database insertion)
#[derive(Debug, Clone)]
pub struct ExtractedClipData {
    pub clip_number: i32,
    pub local_clip_path: String,
    pub start_time_seconds: f64,
    pub end_time_seconds: f64,
    pub duration_seconds: f64,
    pub ai_title: String,
    pub ai_description: String,
    pub ai_tags: Vec<String>,
    pub ai_confidence_score: f64,
    pub viral_factors: Vec<String>,
    pub custom_thumbnail_path: Option<String>,
    /// How the thumbnail was generated: "ai_gemini", "ffmpeg_timestamp", or None
    pub thumbnail_generation_method: Option<String>,
    /// Set to true after Phase C+ applies at least one FFmpeg enhancement tool
    pub enhancement_applied: bool,
    /// Which FFmpeg tools were applied during Phase C+
    pub enhancement_tools: Vec<String>,
    /// Gemini's reasoning for the chosen tools
    pub enhancement_reasoning: Option<String>,
    // R2 storage fields — set after upload in Phase C
    pub r2_clip_key: Option<String>,
    pub r2_thumb_key: Option<String>,
    pub r2_clip_url: Option<String>,
}
