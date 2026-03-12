// AI-Powered Thumbnail Generator for YouTube Shorts
// Implements Hybrid Approach: Best frame selection + AI text overlay

use crate::gemini_client::GeminiClient;
use crate::AppState;
use base64::Engine;  // For base64 encoding/decoding
use std::sync::Arc;
use std::path::Path;

pub struct ThumbnailGenerator {
    app_state: Arc<AppState>,
}

impl ThumbnailGenerator {
    pub fn new(app_state: Arc<AppState>) -> Self {
        Self { app_state }
    }

    /// Generate optimal thumbnail for a clip.
    /// Steps:
    /// 1. Extract multiple frames from the clip at strategic timestamps
    /// 2. Use Gemini vision to select the best frame
    /// 3. Save the selected frame as the final thumbnail
    ///
    /// Note: text overlay via image generation is not implemented yet —
    /// the standard Gemini text API does not return image data.
    pub async fn generate_thumbnail(
        &self,
        clip_path: &str,
        _clip_title: &str,
        viral_factors: &[String],
    ) -> Result<String, String> {
        tracing::info!("🎨 Generating AI thumbnail for clip: {}", clip_path);

        // Step 1: Extract candidate frames at strategic timestamps
        let candidate_frames = self.extract_candidate_frames(clip_path).await?;

        if candidate_frames.is_empty() {
            return Err("Failed to extract any frames from clip".to_string());
        }

        // Step 2: Select best frame using Gemini vision
        let best_frame_path = self.select_best_frame(&candidate_frames, viral_factors).await?;

        tracing::info!("✅ AI selected best frame: {}", best_frame_path);

        // Step 3: Copy selected frame to outputs/ alongside the clip file
        let thumbnail_path = format!(
            "outputs/ai_thumb_{}.jpg",
            Path::new(clip_path)
                .file_stem()
                .and_then(|s| s.to_str())
                .unwrap_or("clip")
        );
        tokio::fs::copy(&best_frame_path, &thumbnail_path)
            .await
            .map_err(|e| format!("Failed to save AI thumbnail: {}", e))?;

        // Cleanup all temporary candidate frames
        for frame_path in &candidate_frames {
            let _ = tokio::fs::remove_file(frame_path).await;
        }

        tracing::info!("✅ AI thumbnail saved: {}", thumbnail_path);
        Ok(thumbnail_path)
    }

    /// Extract multiple frames from clip at strategic timestamps
    /// For Shorts (30-90s), we extract at: 10%, 40%, 70% (avoiding very start/end)
    async fn extract_candidate_frames(&self, clip_path: &str) -> Result<Vec<String>, String> {
        tracing::info!("Extracting candidate frames from clip");

        // Get clip duration
        let clip_metadata = crate::core::analyze_video(clip_path)
            .map_err(|e| format!("Failed to analyze clip: {}", e))?;

        let duration = clip_metadata.duration_seconds;

        // Extract frames at 10%, 40%, 70% of duration
        let timestamps = vec![
            duration * 0.10,  // 10% - after any intro
            duration * 0.40,  // 40% - mid-section (often peak moment)
            duration * 0.70,  // 70% - near climax
        ];

        let mut frame_paths = Vec::new();

        for (idx, timestamp) in timestamps.iter().enumerate() {
            let frame_path = format!(
                "thumbnails/temp_frame_{}_{}.jpg",
                Path::new(clip_path)
                    .file_stem()
                    .and_then(|s| s.to_str())
                    .unwrap_or("clip"),
                idx
            );

            // Ensure thumbnails directory exists
            if let Some(parent) = Path::new(&frame_path).parent() {
                tokio::fs::create_dir_all(parent)
                    .await
                    .map_err(|e| format!("Failed to create thumbnails directory: {}", e))?;
            }

            // Extract frame using existing thumbnail creation function
            // YouTube recommends 1280x720 minimum for thumbnails
            match crate::transform::create_thumbnail_scaled(
                clip_path,
                &frame_path,
                *timestamp,
                1280,
                720,
            ) {
                Ok(_) => {
                    tracing::info!("Extracted frame at {:.1}s: {}", timestamp, frame_path);
                    frame_paths.push(frame_path);
                }
                Err(e) => {
                    tracing::warn!("Failed to extract frame at {:.1}s: {}", timestamp, e);
                }
            }
        }

        if frame_paths.is_empty() {
            return Err("Failed to extract any frames".to_string());
        }

        Ok(frame_paths)
    }

    /// Use AI to select the best frame for thumbnail
    /// Analyzes visual appeal, clarity, energy, and relevance to viral_factors
    async fn select_best_frame(
        &self,
        candidate_frames: &[String],
        viral_factors: &[String],
    ) -> Result<String, String> {
        tracing::info!("🤖 Using AI to select best thumbnail frame");

        if candidate_frames.len() == 1 {
            return Ok(candidate_frames[0].clone());
        }

        // Use Gemini vision to analyze frames
        let gemini_client = self
            .app_state
            .gemini_client
            .as_ref()
            .ok_or("Gemini client not available")?;

        // Read all frames into memory for analysis
        let mut frame_data_vec = Vec::new();
        for frame_path in candidate_frames {
            let frame_bytes = tokio::fs::read(frame_path)
                .await
                .map_err(|e| format!("Failed to read frame: {}", e))?;
            frame_data_vec.push(frame_bytes);
        }

        let viral_factors_text = if !viral_factors.is_empty() {
            format!("Viral factors: {}", viral_factors.join(", "))
        } else {
            "".to_string()
        };

        let prompt = format!(
            r#"You are analyzing {} candidate frames for a YouTube Shorts thumbnail. {}

ANALYZE each frame and select the BEST one based on:
1. Visual Impact - Bright, clear, eye-catching
2. Energy Level - Dynamic, engaging, not boring
3. Facial Expressions (if present) - Emotional, expressive
4. Composition - Well-framed, not cluttered
5. Relevance to Content - Represents the viral moment

Respond with ONLY the number of the best frame (0, 1, or 2).
No explanation, just the number."#,
            candidate_frames.len(),
            viral_factors_text
        );

        // Build request with all frames
        let mut content_parts = vec![crate::gemini_client::Part::Text {
            text: prompt.clone(),
        }];

        for frame_bytes in frame_data_vec {
            content_parts.push(crate::gemini_client::Part::InlineData {
                inline_data: crate::gemini_client::InlineData {
                    mime_type: "image/jpeg".to_string(),
                    data: base64::prelude::BASE64_STANDARD.encode(&frame_bytes),
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
            .map_err(|e| format!("AI analysis failed: {}", e))?;

        // Parse response to get selected frame index
        let response_text = response
            .candidates
            .first()
            .and_then(|c| c.content.as_ref())
            .and_then(|content| content.parts.first())
            .and_then(|part| {
                if let crate::gemini_client::Part::Text { text } = part {
                    Some(text.as_str())
                } else {
                    None
                }
            })
            .unwrap_or("0");

        let selected_index: usize = response_text
            .trim()
            .parse()
            .unwrap_or(0)
            .min(candidate_frames.len() - 1);

        tracing::info!("AI selected frame index: {}", selected_index);

        Ok(candidate_frames[selected_index].clone())
    }

    /// Generate final thumbnail with text overlay using Gemini image generation
    /// This creates a polished, professional thumbnail with the clip title overlaid
    async fn generate_thumbnail_with_overlay(
        &self,
        base_frame_path: &str,
        clip_title: &str,
    ) -> Result<String, String> {
        tracing::info!("🎨 Generating thumbnail with text overlay");

        let gemini_client = self
            .app_state
            .gemini_client
            .as_ref()
            .ok_or("Gemini client not available")?;

        // Read the selected base frame
        let base_frame_bytes = tokio::fs::read(base_frame_path)
            .await
            .map_err(|e| format!("Failed to read base frame: {}", e))?;

        let base_frame_base64 = base64::prelude::BASE64_STANDARD.encode(&base_frame_bytes);

        // Create prompt for thumbnail generation with overlay
        let prompt = format!(
            r#"Create a YouTube Shorts thumbnail based on this video frame.

REQUIREMENTS:
1. Keep the base image composition exactly as shown
2. Add bold, eye-catching text overlay with the title: "{}"
3. Use bright colors for text (white with black outline, or yellow/orange)
4. Position text in upper or lower third for maximum visibility
5. Add subtle darkening/brightening behind text for readability
6. Make it look professional and click-worthy
7. Ensure text is large and easily readable on mobile devices

OUTPUT: A polished YouTube thumbnail (16:9 aspect ratio, 1280x720px minimum)"#,
            clip_title
        );

        // Build request with base frame as context
        let request = crate::gemini_client::GenerateContentRequest {
            contents: vec![crate::gemini_client::Content {
                role: Some("user".to_string()),
                parts: vec![
                    crate::gemini_client::Part::Text {
                        text: prompt,
                    },
                    crate::gemini_client::Part::InlineData {
                        inline_data: crate::gemini_client::InlineData {
                            mime_type: "image/jpeg".to_string(),
                            data: base_frame_base64,
                        },
                    },
                ],
            }],
            generation_config: Some(crate::gemini_client::GenerationConfig {
                temperature: 0.4,  // Lower temperature for consistency
                top_p: 0.8,
                top_k: 40,
                max_output_tokens: 8192,
            }),
            tools: None,
            tool_config: None,
            system_instruction: None,
        };

        // Generate thumbnail with overlay
        let response = gemini_client
            .generate_content(request)
            .await
            .map_err(|e| format!("Thumbnail generation failed: {}", e))?;

        // Extract generated image data
        let image_data = response
            .candidates
            .first()
            .and_then(|c| c.content.as_ref())
            .and_then(|content| content.parts.first())
            .and_then(|part| {
                if let crate::gemini_client::Part::InlineData { inline_data } = part {
                    Some(inline_data.data.as_str())
                } else {
                    None
                }
            })
            .ok_or("No image data in response")?;

        // Decode base64 image data
        let thumbnail_bytes = base64::prelude::BASE64_STANDARD
            .decode(image_data)
            .map_err(|e| format!("Failed to decode thumbnail: {}", e))?;

        // Save final thumbnail
        let thumbnail_path = format!(
            "thumbnails/thumbnail_{}.jpg",
            Path::new(base_frame_path)
                .file_stem()
                .and_then(|s| s.to_str())
                .unwrap_or("clip")
        );

        tokio::fs::write(&thumbnail_path, thumbnail_bytes)
            .await
            .map_err(|e| format!("Failed to save thumbnail: {}", e))?;

        tracing::info!("✅ Thumbnail with overlay saved: {}", thumbnail_path);

        Ok(thumbnail_path)
    }

    /// Cleanup temporary candidate frames (keep only the final thumbnail)
    async fn cleanup_candidate_frames(&self, candidate_frames: &[String], best_frame: &str) {
        for frame_path in candidate_frames {
            // Don't delete the best frame if it's in the list
            if frame_path != best_frame {
                if let Err(e) = tokio::fs::remove_file(frame_path).await {
                    tracing::warn!("Failed to cleanup frame {}: {}", frame_path, e);
                }
            }
        }
    }

    /// Get learned optimal thumbnail strategy from performance data
    /// Returns: (generation_method, text_overlay_style, frame_selection_strategy)
    pub async fn get_optimal_thumbnail_strategy(
        &self,
    ) -> Result<(String, String, String), String> {
        let query = "
            SELECT generation_method, text_overlay_style, frame_selection_strategy
            FROM thumbnail_performance_analysis
            WHERE total_clips >= 5
            ORDER BY performance_score DESC
            LIMIT 1
        ";

        let result = sqlx::query_as::<_, (String, String, String)>(query)
            .fetch_optional(&self.app_state.db_pool)
            .await
            .map_err(|e| format!("Failed to fetch optimal strategy: {}", e))?;

        Ok(result.unwrap_or((
            "hybrid".to_string(),
            "bold_text_with_outline".to_string(),
            "ai_vision_analysis".to_string(),
        )))
    }
}

/// Thumbnail generation result
#[derive(Debug, Clone)]
pub struct ThumbnailResult {
    pub thumbnail_path: String,
    pub generation_method: String,
    pub selected_frame_timestamp: f64,
}
