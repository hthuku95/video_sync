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

use crate::clipping::gemini_video_analyzer::ViralMoment;
use crate::clipping::models::ClippingConfig;
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
    ) -> Result<Vec<ExtractedClipData>, String> {
        tracing::info!(
            "🎬 Extracting {} clips from job {} in parallel",
            moments.len(),
            job_id
        );

        // Launch all clip extractions in parallel
        let mut handles = Vec::new();

        for (index, moment) in moments.iter().enumerate() {
            let clip_path = format!("outputs/clip_{}_{}.mp4", job_id, index + 1);
            let thumbnail_path = format!("outputs/thumb_{}_{}.jpg", job_id, index + 1);

            let video_path = video_path.to_string();
            let moment = moment.clone();
            let clip_number = (index + 1) as i32;

            let handle = tokio::task::spawn_blocking(move || {
                extract_single_clip(
                    clip_number,
                    &video_path,
                    &clip_path,
                    &thumbnail_path,
                    &moment,
                )
            });

            handles.push(handle);
        }

        // Collect results
        let mut extracted_clips = Vec::new();
        for handle in handles {
            match handle.await {
                Ok(Ok(clip)) => extracted_clips.push(clip),
                Ok(Err(e)) => tracing::warn!("Clip extraction failed (skipping): {}", e),
                Err(e) => tracing::warn!("Clip extraction task panicked (skipping): {}", e),
            }
        }

        if extracted_clips.is_empty() {
            return Err("All clip extractions failed".to_string());
        }

        tracing::info!(
            "✅ Extracted {}/{} clips successfully for job {}",
            extracted_clips.len(),
            moments.len(),
            job_id
        );

        // Phase C+: AI thumbnail selection — replace ffmpeg frame with Gemini-selected best frame.
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
) -> Result<ExtractedClipData, String> {
    tracing::info!(
        "✂️  Clip {}: trimming {:.1}s–{:.1}s ({:.1}s) → {}",
        clip_number,
        moment.start_sec,
        moment.end_sec,
        moment.duration(),
        clip_path
    );

    // Trim the clip
    crate::core::trim_video(
        video_path,
        clip_path,
        moment.start_sec,
        moment.end_sec,
    ).map_err(|e| format!("Clip {} trim failed: {}", clip_number, e))?;

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
}
