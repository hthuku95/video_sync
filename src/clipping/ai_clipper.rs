// AI-powered viral clip identification and extraction

use crate::clipping::models::{ClipCandidate, ClippingConfig, ReviewResult};
use crate::clipping::performance_tracker::PerformanceTracker;
use crate::clipping::thumbnail_generator::ThumbnailGenerator;
use crate::services::VideoVectorizationService;
use crate::AppState;
use std::sync::Arc;
use tokio::time::Duration;

pub struct AiClipper {
    pub app_state: Arc<AppState>,
}

impl AiClipper {
    pub fn new(app_state: Arc<AppState>) -> Self {
        Self { app_state }
    }

    /// Extract viral clips from a video using AI analysis
    pub async fn extract_viral_clips(
        &self,
        job_id: i32,
        video_path: &str,
        config: &ClippingConfig,
    ) -> Result<Vec<ExtractedClipData>, String> {
        tracing::info!("🎬 Starting AI clip extraction for job {}", job_id);

        // Step 1: Retrieve vectorized video analysis
        let video_analysis = self.get_video_analysis(video_path).await?;

        // Step 2: Use AI to identify viral moments
        let clip_candidates = self
            .identify_viral_moments(&video_analysis, config)
            .await?;

        if clip_candidates.is_empty() {
            return Err("AI could not identify any viral moments in the video".to_string());
        }

        tracing::info!("Found {} clip candidates", clip_candidates.len());

        // Step 3: Extract each clip using trim_video
        let mut extracted_clips = Vec::new();
        for (index, candidate) in clip_candidates.iter().enumerate() {
            let clip_path = format!("outputs/clip_{}_{}.mp4", job_id, index + 1);

            tracing::info!(
                "Extracting clip {} ({:.1}s - {:.1}s)",
                index + 1,
                candidate.start_time,
                candidate.end_time
            );

            // Use existing trim_video tool
            match crate::core::trim_video(
                video_path,
                &clip_path,
                candidate.start_time,
                candidate.end_time,
            ) {
                Ok(_) => {
                    // Vectorize the extracted clip for review
                    if let Err(e) = VideoVectorizationService::process_video_for_vectorization(
                        &clip_path,
                        &format!("clip_{}_{}", job_id, index + 1),
                        &format!("clipping_job_{}", job_id),
                        None,
                        &self.app_state,
                        None, // No job_id context for clip vectorization
                    )
                    .await
                    {
                        tracing::warn!("Failed to vectorize clip: {}", e);
                    }

                    // ENHANCEMENT: Generate AI thumbnail for the clip
                    let thumbnail_path = self.generate_clip_thumbnail(
                        &clip_path,
                        &candidate.title,
                        &candidate.viral_factors,
                    ).await;

                    if let Err(ref e) = thumbnail_path {
                        tracing::warn!("Failed to generate thumbnail for clip {}: {}", index + 1, e);
                    }

                    // Review the clip
                    let review_result = self.review_clip(&clip_path, &candidate.criteria).await?;

                    if !review_result.passed {
                        tracing::warn!("Clip {} failed review: {}", index + 1, review_result.feedback);
                        continue; // Skip failed clips
                    }

                    extracted_clips.push(ExtractedClipData {
                        clip_number: (index + 1) as i32,
                        local_clip_path: clip_path,
                        start_time_seconds: candidate.start_time,
                        end_time_seconds: candidate.end_time,
                        duration_seconds: candidate.end_time - candidate.start_time,
                        ai_title: candidate.title.clone(),
                        ai_description: candidate.description.clone(),
                        ai_tags: candidate.tags.clone(),
                        ai_confidence_score: candidate.confidence,
                        viral_factors: candidate.viral_factors.clone(),
                        custom_thumbnail_path: thumbnail_path.ok(),  // Store thumbnail path if generation succeeded
                    });
                }
                Err(e) => {
                    tracing::error!("Failed to extract clip {}: {}", index + 1, e);
                    continue;
                }
            }
        }

        if extracted_clips.is_empty() {
            return Err("All clip extractions failed or were rejected".to_string());
        }

        tracing::info!("✅ Successfully extracted {} clips", extracted_clips.len());
        Ok(extracted_clips)
    }

    /// Get video analysis from Qdrant vectorization
    async fn get_video_analysis(&self, video_path: &str) -> Result<String, String> {
        tracing::info!("Retrieving video analysis from vector database");

        // Use the existing retrieve_video_analysis function
        match VideoVectorizationService::retrieve_video_analysis(
            video_path,
            &self.app_state,
        )
        .await
        {
            Ok(analysis_json) => {
                // Convert JSON to formatted string for AI prompt
                Ok(serde_json::to_string_pretty(&analysis_json)
                    .unwrap_or_else(|_| "Failed to format analysis".to_string()))
            }
            Err(e) => Err(format!("Failed to retrieve video analysis: {}", e)),
        }
    }

    /// Use AI to identify viral moments in the video (ENHANCED: uses learned optimal viral factors)
    async fn identify_viral_moments(
        &self,
        video_analysis: &str,
        config: &ClippingConfig,
    ) -> Result<Vec<ClipCandidate>, String> {
        // ENHANCEMENT: Get learned optimal viral factors from performance data
        let performance_tracker = PerformanceTracker::new(self.app_state.db_pool.clone());
        let optimal_factors = performance_tracker
            .get_optimized_viral_factors()
            .await
            .unwrap_or_else(|_| vec![
                "dramatic_hook".to_string(),
                "surprising_moment".to_string(),
                "emotional_peak".to_string(),
            ]);

        // ENHANCEMENT: Get optimal duration range from learned data
        let (optimal_min, optimal_max) = performance_tracker
            .get_optimal_duration_range()
            .await
            .unwrap_or((config.min_clip_duration_seconds, config.max_clip_duration_seconds));

        let factors_hint = if !optimal_factors.is_empty() {
            format!(
                "PRIORITIZE THESE HIGH-PERFORMING VIRAL FACTORS (learned from past performance):\n{}\n\n",
                optimal_factors.iter().map(|f| format!("  - {}", f)).collect::<Vec<_>>().join("\n")
            )
        } else {
            String::new()
        };

        let prompt = format!(
            r#"Analyze this video and identify exactly {} viral clip opportunities for YouTube Shorts.

VIDEO ANALYSIS:
{}

{}REQUIREMENTS:
- Each clip must be between {} and {} seconds
- OPTIMAL DURATION (based on learning): {}-{} seconds
- Focus on: dramatic hooks, surprising moments, emotional peaks, action sequences, plot twists
- Clips should work as standalone content
- IMPORTANT: Prioritize the high-performing viral factors listed above if present in the video

For EACH clip, provide in this exact JSON format:
[
  {{
    "start_time": <seconds as float>,
    "end_time": <seconds as float>,
    "title": "<engaging YouTube Short title, max 60 chars>",
    "description": "<compelling description for YouTube>",
    "tags": ["tag1", "tag2", "tag3"],
    "confidence": <0.0 to 1.0>,
    "viral_factors": ["hook detected", "dramatic reveal", etc.],
    "criteria": "<why this moment is viral>"
  }}
]

Provide ONLY the JSON array, no other text."#,
            config.clips_per_video,
            video_analysis,
            factors_hint,
            config.min_clip_duration_seconds,
            config.max_clip_duration_seconds,
            optimal_min,
            optimal_max
        );

        // Call AI agent (Claude or Gemini based on config)
        let ai_response = self.call_ai_agent(&prompt).await?;

        // Parse JSON response
        self.parse_clip_candidates(&ai_response)
    }

    /// Call AI agent with prompt
    async fn call_ai_agent(&self, prompt: &str) -> Result<String, String> {
        // Use Claude client if available
        if let Some(ref claude_client) = self.app_state.claude_client {
            match claude_client.generate_text(prompt).await {
                Ok(response) => Ok(response),
                Err(e) => Err(format!("Claude AI error: {}", e)),
            }
        }
        // Fallback to Gemini
        else if let Some(ref gemini_client) = self.app_state.gemini_client {
            // Build GenerateContentRequest
            let request = crate::gemini_client::GenerateContentRequest {
                contents: vec![crate::gemini_client::Content {
                    role: Some("user".to_string()),
                    parts: vec![crate::gemini_client::Part::Text {
                        text: prompt.to_string(),
                    }],
                }],
                generation_config: None,
                tools: None,
                tool_config: None,
            };

            match gemini_client.generate_content(request).await {
                Ok(response) => {
                    // Extract text from response
                    let text = response
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
                        .unwrap_or_else(|| "No text response".to_string());

                    Ok(text)
                }
                Err(e) => Err(format!("Gemini AI error: {}", e)),
            }
        } else {
            Err("No AI client available".to_string())
        }
    }

    /// Parse AI response into clip candidates
    fn parse_clip_candidates(&self, ai_response: &str) -> Result<Vec<ClipCandidate>, String> {
        // Extract JSON from response (handle markdown code blocks)
        let json_str = if ai_response.contains("```") {
            // Extract content between ```json and ```
            let start = ai_response.find("[").unwrap_or(0);
            let end = ai_response.rfind("]").unwrap_or(ai_response.len());
            &ai_response[start..=end]
        } else {
            ai_response.trim()
        };

        // Parse JSON
        let parsed: Result<Vec<serde_json::Value>, _> = serde_json::from_str(json_str);

        match parsed {
            Ok(clips_json) => {
                let mut candidates = Vec::new();
                for clip in clips_json {
                    let candidate = ClipCandidate {
                        start_time: clip["start_time"].as_f64().unwrap_or(0.0),
                        end_time: clip["end_time"].as_f64().unwrap_or(0.0),
                        title: clip["title"].as_str().unwrap_or("Viral Clip").to_string(),
                        description: clip["description"].as_str().unwrap_or("").to_string(),
                        tags: clip["tags"]
                            .as_array()
                            .map(|arr| {
                                arr.iter()
                                    .filter_map(|v| v.as_str().map(|s| s.to_string()))
                                    .collect()
                            })
                            .unwrap_or_default(),
                        confidence: clip["confidence"].as_f64().unwrap_or(0.5),
                        viral_factors: clip["viral_factors"]
                            .as_array()
                            .map(|arr| {
                                arr.iter()
                                    .filter_map(|v| v.as_str().map(|s| s.to_string()))
                                    .collect()
                            })
                            .unwrap_or_default(),
                        criteria: clip["criteria"].as_str().unwrap_or("").to_string(),
                    };
                    candidates.push(candidate);
                }
                Ok(candidates)
            }
            Err(e) => Err(format!("Failed to parse AI response as JSON: {}", e)),
        }
    }

    /// Wait for clip vectorization to complete (FIXED: intelligent polling instead of fixed delay)
    async fn wait_for_clip_vectorization(&self, clip_path: &str) -> Result<(), String> {
        let max_attempts = 30; // 30 seconds max
        for attempt in 0..max_attempts {
            // Check if embeddings exist in Qdrant
            match VideoVectorizationService::retrieve_video_analysis(clip_path, &self.app_state).await {
                Ok(_) => {
                    tracing::info!("✅ Clip vectorization completed after {} seconds", attempt + 1);
                    return Ok(());
                }
                Err(_) if attempt < max_attempts - 1 => {
                    // Not ready yet, wait 1 second and try again
                    tokio::time::sleep(Duration::from_secs(1)).await;
                }
                Err(e) => {
                    return Err(format!("Clip vectorization timed out after 30 seconds: {}", e));
                }
            }
        }
        Err("Clip vectorization timed out".to_string())
    }

    /// Review extracted clip for quality using vectorized analysis
    async fn review_clip(
        &self,
        clip_path: &str,
        original_criteria: &str,
    ) -> Result<ReviewResult, String> {
        // FIXED: Intelligent polling instead of fixed 3-second delay
        self.wait_for_clip_vectorization(clip_path).await?;

        // Retrieve the vectorized clip analysis to actually VIEW the clip
        let clip_analysis = match VideoVectorizationService::retrieve_video_analysis(
            clip_path,
            &self.app_state,
        )
        .await
        {
            Ok(analysis) => {
                // Extract summary for review
                analysis.get("video_summary")
                    .and_then(|v| v.as_str())
                    .unwrap_or("Analysis unavailable")
                    .to_string()
            }
            Err(e) => {
                tracing::warn!("Could not retrieve vectorized clip analysis (proceeding without): {}", e);
                "Vectorization pending - reviewing based on metadata only".to_string()
            }
        };

        let review_prompt = format!(
            r#"Review this video clip for YouTube Shorts suitability.

ORIGINAL SELECTION CRITERIA:
{}

ACTUAL CLIP CONTENT (from AI vision analysis):
{}

VERIFY:
1. Duration is appropriate (60-120 seconds)
2. Contains the intended viral moment described in criteria
3. Video quality is good (no artifacts, clear)
4. Suitable for YouTube Shorts format
5. Content matches the selection criteria

Respond with:
- PASS if the clip meets all criteria
- FAIL if the clip has issues or doesn't match criteria

Then provide brief feedback explaining your decision."#,
            original_criteria,
            clip_analysis
        );

        let review_response = self.call_ai_agent(&review_prompt).await?;

        let passed = review_response.to_uppercase().contains("PASS");

        if !passed {
            tracing::info!("Clip review feedback: {}", review_response);
        }

        Ok(ReviewResult {
            passed,
            feedback: review_response,
        })
    }

    /// Generate AI-powered thumbnail for a clip (HYBRID: best frame + text overlay)
    async fn generate_clip_thumbnail(
        &self,
        clip_path: &str,
        title: &str,
        viral_factors: &[String],
    ) -> Result<String, String> {
        tracing::info!("🎨 Generating AI thumbnail for clip");

        let thumbnail_generator = ThumbnailGenerator::new(self.app_state.clone());

        thumbnail_generator
            .generate_thumbnail(clip_path, title, viral_factors)
            .await
    }
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
    pub custom_thumbnail_path: Option<String>,  // AI-generated thumbnail
}
