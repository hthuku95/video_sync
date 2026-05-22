#![allow(dead_code, unused_imports)]
// src/services/video_vectorization.rs
use crate::gemini_client::GeminiClient;
use crate::AppState;
use base64::prelude::*;
use serde::{Deserialize, Serialize};
use serde_json::json;
use std::process::Command;
use std::sync::Arc;
use tokio::fs;
use tracing::{info, warn};

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct VideoFrameMetadata {
    pub frame_number: u32,
    pub timestamp_seconds: f64,
    pub frame_path: String,
    pub description: String,
    pub visual_features: Vec<String>,
}

#[derive(Debug, Serialize, Deserialize)]
pub struct VideoVectorData {
    pub file_id: String,
    pub session_id: String,
    pub user_id: Option<i32>,
    pub frame_metadata: Vec<VideoFrameMetadata>,
    pub video_summary: String,
    pub total_frames: u32,
    pub duration_seconds: f64,
}

pub struct VideoVectorizationService;

impl VideoVectorizationService {
    /// Update job heartbeat to prevent stuck detection during long processing
    async fn update_heartbeat(job_id: Option<i32>, db_pool: &sqlx::PgPool, message: &str) {
        if let Some(id) = job_id {
            match sqlx::query("UPDATE clipping_jobs SET updated_at = NOW() WHERE id = $1")
                .bind(id)
                .execute(db_pool)
                .await
            {
                Ok(_) => {
                    tracing::debug!("💓 Heartbeat: Job {} - {}", id, message);
                }
                Err(e) => {
                    tracing::warn!("Failed to update heartbeat for job {}: {}", id, e);
                }
            }
        }
    }

    /// Extract keyframes from video and generate embeddings for storage in Qdrant
    pub async fn process_video_for_vectorization(
        video_file_path: &str,
        file_id: &str,
        session_id: &str,
        user_id: Option<i32>,
        state: &Arc<AppState>,
        job_id: Option<i32>, // For heartbeat updates to prevent stuck detection
    ) -> Result<(), Box<dyn std::error::Error + Send + Sync>> {
        info!(
            "Starting video vectorization for file: {} ({})",
            video_file_path, file_id
        );

        // Step 1: Extract keyframes from video
        let frames_dir = format!("temp_frames/{}", file_id);
        fs::create_dir_all(&frames_dir).await?;

        let mut keyframes = Self::extract_keyframes(video_file_path, &frames_dir).await?;
        info!("Extracted {} keyframes from video", keyframes.len());

        // Heartbeat: Keyframe extraction complete
        Self::update_heartbeat(job_id, &state.db_pool, "Keyframes extracted").await;

        // Step 1.5: Apply frame limit to prevent excessive processing time
        let max_frames = std::env::var("MAX_FRAMES_PER_VIDEO")
            .ok()
            .and_then(|v| v.parse::<usize>().ok())
            .unwrap_or(100); // Default: 100 frames max

        if keyframes.len() > max_frames {
            info!(
                "⚠️ Video has {} frames, sampling down to {} frames for performance",
                keyframes.len(),
                max_frames
            );

            // Sample frames evenly across the video to maintain coverage
            let step = keyframes.len() / max_frames;
            keyframes = keyframes
                .iter()
                .step_by(step)
                .take(max_frames)
                .cloned()
                .collect();

            info!("✅ Sampled to {} frames (step: {})", keyframes.len(), step);
        }

        // Step 2: Analyze each frame using multimodal vision (Claude OR Gemini)
        // NOTE: Frame ANALYSIS can use Claude vision (preferred for Claude agents) OR Gemini multimodal
        // Frame EMBEDDINGS (Step 4) can separately use Voyage AI or Gemini

        // Prefer Claude if available, fallback to Gemini
        let use_claude_vision = state.claude_client.is_some();

        if !use_claude_vision && state.gemini_client.is_none() {
            return Err("No vision-capable AI model available (need Claude or Gemini)".into());
        }

        // PERFORMANCE OPTIMIZATION: Parallelize frame analysis
        // Process multiple frames concurrently to reduce total processing time by 3-5x
        let concurrency_limit = std::env::var("FRAME_ANALYSIS_CONCURRENCY")
            .ok()
            .and_then(|v| v.parse::<usize>().ok())
            .unwrap_or(3); // Default: 3 concurrent frame analyses

        let per_frame_timeout_secs = std::env::var("FRAME_ANALYSIS_TIMEOUT_SECONDS")
            .ok()
            .and_then(|v| v.parse::<u64>().ok())
            .unwrap_or(30); // Default: 30 second timeout per frame

        info!(
            "Analyzing {} frames with concurrency={}, timeout={}s per frame",
            keyframes.len(),
            concurrency_limit,
            per_frame_timeout_secs
        );

        use futures::stream::{self, StreamExt};
        use tokio::time::{timeout, Duration};

        let frame_futures = keyframes
            .iter()
            .enumerate()
            .map(|(frame_number, frame_path)| {
                let frame_path = frame_path.clone();
                let state = state.clone();
                let frame_timeout = Duration::from_secs(per_frame_timeout_secs);

                async move {
                    // Wrap frame analysis with timeout
                    let analysis_future = async {
                        if use_claude_vision {
                            Self::analyze_frame_with_claude(
                                &frame_path,
                                frame_number as u32,
                                state.claude_client.as_ref().unwrap(),
                            )
                            .await
                        } else {
                            Self::analyze_frame_with_gemini(
                                &frame_path,
                                frame_number as u32,
                                state.gemini_client.as_ref().unwrap(),
                            )
                            .await
                        }
                    };

                    match timeout(frame_timeout, analysis_future).await {
                        Ok(Ok(metadata)) => Some(metadata),
                        Ok(Err(e)) => {
                            warn!("Failed to analyze frame {}: {}", frame_number, e);
                            None
                        }
                        Err(_) => {
                            warn!(
                                "Frame {} analysis timed out after {}s",
                                frame_number, per_frame_timeout_secs
                            );
                            None
                        }
                    }
                }
            });

        // Process frames in parallel with concurrency limit
        let frame_metadata: Vec<VideoFrameMetadata> = stream::iter(frame_futures)
            .buffer_unordered(concurrency_limit)
            .filter_map(|result| async move { result })
            .collect()
            .await;

        info!(
            "Successfully analyzed {}/{} frames",
            frame_metadata.len(),
            keyframes.len()
        );

        // Heartbeat: Frame analysis complete
        Self::update_heartbeat(job_id, &state.db_pool, "Frame analysis complete").await;

        // Step 3: Generate overall video summary using frame analysis
        let video_summary = if use_claude_vision {
            Self::generate_video_summary_with_claude(
                &frame_metadata,
                state.claude_client.as_ref().unwrap(),
            )
            .await?
        } else {
            Self::generate_video_summary_with_gemini(
                &frame_metadata,
                state.gemini_client.as_ref().unwrap(),
            )
            .await?
        };

        // Step 4: Create embeddings and store in Qdrant
        let vector_data = VideoVectorData {
            file_id: file_id.to_string(),
            session_id: session_id.to_string(),
            user_id,
            frame_metadata: frame_metadata.clone(),
            video_summary: video_summary.clone(),
            total_frames: frame_metadata.len() as u32,
            duration_seconds: Self::get_video_duration(video_file_path).await?,
        };

        Self::store_video_embeddings(&vector_data, state).await?;

        // Heartbeat: Embeddings stored
        Self::update_heartbeat(job_id, &state.db_pool, "Embeddings stored in Qdrant").await;

        // Step 5: Clean up temporary frames
        let _ = fs::remove_dir_all(&frames_dir).await;

        info!(
            "Successfully vectorized video: {} with {} frame embeddings",
            file_id,
            frame_metadata.len()
        );
        Ok(())
    }

    /// Extract keyframes from video using FFmpeg
    async fn extract_keyframes(
        video_path: &str,
        output_dir: &str,
    ) -> Result<Vec<String>, Box<dyn std::error::Error + Send + Sync>> {
        info!("Extracting keyframes from video: {}", video_path);

        // Use FFmpeg to extract keyframes at 1-second intervals
        let output_pattern = format!("{}/frame_%04d.jpg", output_dir);

        let output = Command::new("ffmpeg")
            .arg("-i")
            .arg(video_path)
            .arg("-vf")
            .arg("select='eq(pict_type,I)',scale=640:360") // Extract I-frames and scale down
            .arg("-vsync")
            .arg("vfr")
            .arg("-q:v")
            .arg("2") // High quality JPEG
            .arg(&output_pattern)
            .arg("-y") // Overwrite existing files
            .output()?;

        if !output.status.success() {
            return Err(
                format!("FFmpeg failed: {}", String::from_utf8_lossy(&output.stderr)).into(),
            );
        }

        // Get list of extracted frames
        let mut frames = Vec::new();
        let mut frame_num = 1;
        loop {
            let frame_path = format!("{}/frame_{:04}.jpg", output_dir, frame_num);
            if tokio::fs::metadata(&frame_path).await.is_ok() {
                frames.push(frame_path);
                frame_num += 1;
            } else {
                break;
            }
        }

        Ok(frames)
    }

    /// Analyze individual frame using Gemini 2.5 Flash multimodal model
    async fn analyze_frame_with_gemini(
        frame_path: &str,
        frame_number: u32,
        gemini_client: &GeminiClient,
    ) -> Result<VideoFrameMetadata, Box<dyn std::error::Error + Send + Sync>> {
        // Read frame data as base64
        let frame_data = fs::read(frame_path).await?;
        let _frame_base64 = base64::prelude::BASE64_STANDARD.encode(&frame_data);

        let analysis_prompt = format!(
            "Analyze this video frame (frame #{}) and provide a detailed description. 
            Focus on:
            1. Main subjects and objects in the scene
            2. Actions or activities taking place
            3. Visual style, colors, and composition
            4. Text or graphics visible in the frame
            5. Overall mood and context
            
            Respond in JSON format with 'description' and 'visual_features' (array of key features).",
            frame_number
        );

        let analysis_result = gemini_client
            .analyze_video_content(frame_path, Some(analysis_prompt))
            .await?;

        // Parse the AI response to extract structured data
        let (description, visual_features) = Self::parse_frame_analysis(&analysis_result);

        // Calculate timestamp based on frame number (assuming 1 frame per second for keyframes)
        let timestamp_seconds = frame_number as f64;

        Ok(VideoFrameMetadata {
            frame_number,
            timestamp_seconds,
            frame_path: frame_path.to_string(),
            description,
            visual_features,
        })
    }

    /// Analyze individual frame using Claude vision API
    async fn analyze_frame_with_claude(
        frame_path: &str,
        frame_number: u32,
        claude_client: &crate::claude_client::ClaudeClient,
    ) -> Result<VideoFrameMetadata, Box<dyn std::error::Error + Send + Sync>> {
        // Read frame data as base64
        let frame_data = fs::read(frame_path).await?;
        let frame_base64 = BASE64_STANDARD.encode(&frame_data);

        let analysis_prompt = format!(
            "Analyze this video frame (frame #{}) and provide a detailed description.
            Focus on:
            1. Main subjects and objects in the scene
            2. Actions or activities taking place
            3. Visual style, colors, and composition
            4. Text or graphics visible in the frame
            5. Overall mood and context

            Respond in JSON format with 'description' and 'visual_features' (array of key features).",
            frame_number
        );

        // Build Claude request with image
        use crate::claude_client::{ClaudeContent, ClaudeMessage, ContentBlock, ImageSource};

        let messages = vec![ClaudeMessage {
            role: "user".to_string(),
            content: ClaudeContent::Blocks(vec![
                ContentBlock::Image {
                    source: ImageSource {
                        source_type: "base64".to_string(),
                        media_type: "image/jpeg".to_string(),
                        data: frame_base64,
                    },
                },
                ContentBlock::Text {
                    text: analysis_prompt,
                },
            ]),
        }];

        let response = claude_client.generate_content(messages, None, None).await?;

        // Extract text from response
        let analysis_result = match &response.content.first() {
            Some(crate::claude_client::ResponseContent::Text { text }) => text.clone(),
            _ => return Err("No text response from Claude".into()),
        };

        // Parse the AI response to extract structured data
        let (description, visual_features) = Self::parse_frame_analysis(&analysis_result);

        // Calculate timestamp based on frame number
        let timestamp_seconds = frame_number as f64;

        Ok(VideoFrameMetadata {
            frame_number,
            timestamp_seconds,
            frame_path: frame_path.to_string(),
            description,
            visual_features,
        })
    }

    /// Parse AI analysis response to extract description and features
    fn parse_frame_analysis(analysis_result: &str) -> (String, Vec<String>) {
        // Try to parse as JSON first
        if let Ok(parsed) = serde_json::from_str::<serde_json::Value>(analysis_result) {
            let description = parsed
                .get("description")
                .and_then(|v| v.as_str())
                .unwrap_or(analysis_result)
                .to_string();

            let features = parsed
                .get("visual_features")
                .and_then(|v| v.as_array())
                .map(|arr| {
                    arr.iter()
                        .filter_map(|v| v.as_str().map(String::from))
                        .collect()
                })
                .unwrap_or_else(|| vec!["unstructured_analysis".to_string()]);

            (description, features)
        } else {
            // Fallback to using the raw text as description
            (
                analysis_result.to_string(),
                vec!["text_analysis".to_string()],
            )
        }
    }

    /// Generate overall video summary from frame analyses using Gemini
    async fn generate_video_summary_with_gemini(
        frame_metadata: &[VideoFrameMetadata],
        gemini_client: &GeminiClient,
    ) -> Result<String, Box<dyn std::error::Error + Send + Sync>> {
        if frame_metadata.is_empty() {
            return Ok("No frames analyzed".to_string());
        }

        let frame_descriptions: Vec<String> = frame_metadata
            .iter()
            .map(|f| format!("Frame {}: {}", f.frame_number, f.description))
            .collect();

        let summary_prompt = format!(
            "Based on these video frame analyses, create a comprehensive summary of the video content:
            
            {}
            
            Provide a 2-3 sentence summary that captures:
            1. The main theme/content of the video
            2. Key visual elements and style
            3. Overall narrative or message",
            frame_descriptions.join("\n")
        );

        // Create a proper GenerateContentRequest
        use crate::gemini_client::{Content, GenerateContentRequest, Part};

        let request = GenerateContentRequest {
            contents: vec![Content {
                parts: vec![Part::Text {
                    text: summary_prompt,
                }],
                role: Some("user".to_string()),
            }],
            tools: None,
            generation_config: None,
            tool_config: None,
            system_instruction: None,
        };

        let response = gemini_client.generate_content(request).await?;

        // Extract text from response
        let summary = response
            .candidates
            .first()
            .and_then(|c| c.content.as_ref())
            .and_then(|content| content.parts.first())
            .and_then(|part| match part {
                Part::Text { text } => Some(text.clone()),
                _ => None,
            })
            .unwrap_or_else(|| "Failed to generate video summary".to_string());

        Ok(summary)
    }

    /// Generate overall video summary from frame analyses using Claude
    async fn generate_video_summary_with_claude(
        frame_metadata: &[VideoFrameMetadata],
        claude_client: &crate::claude_client::ClaudeClient,
    ) -> Result<String, Box<dyn std::error::Error + Send + Sync>> {
        if frame_metadata.is_empty() {
            return Ok("No frames analyzed".to_string());
        }

        let frame_descriptions: Vec<String> = frame_metadata
            .iter()
            .map(|f| format!("Frame {}: {}", f.frame_number, f.description))
            .collect();

        let summary_prompt = format!(
            "Based on these video frame analyses, create a comprehensive summary of the video content:

            {}

            Provide a 2-3 sentence summary that captures:
            1. The main theme/content of the video
            2. Key visual elements and style
            3. Overall narrative or message",
            frame_descriptions.join("\n")
        );

        // Create Claude request
        use crate::claude_client::{ClaudeContent, ClaudeMessage};

        let messages = vec![ClaudeMessage {
            role: "user".to_string(),
            content: ClaudeContent::Text(summary_prompt),
        }];

        let response = claude_client.generate_content(messages, None, None).await?;

        // Extract text from response
        let summary = match &response.content.first() {
            Some(crate::claude_client::ResponseContent::Text { text }) => text.clone(),
            _ => "Failed to generate video summary".to_string(),
        };

        Ok(summary)
    }

    /// Store video embeddings in Qdrant vector database
    async fn store_video_embeddings(
        vector_data: &VideoVectorData,
        state: &Arc<AppState>,
    ) -> Result<(), Box<dyn std::error::Error + Send + Sync>> {
        let qdrant_client = match &state.qdrant_client {
            Some(client) => client,
            None => return Err("Qdrant client not available".into()),
        };

        // CRITICAL: Support BOTH Voyage AI (for Claude) and Gemini embeddings
        // Prefer Voyage AI if available (better Claude compatibility), fallback to Gemini

        // 1. Store video-level embedding with provider tracking
        let (video_embedding, video_provider) =
            if let Some(ref voyage_embeddings) = state.voyage_embeddings {
                // Use Voyage AI for Claude-compatible embeddings
                info!("Using Voyage AI embeddings for video vectorization");
                match voyage_embeddings
                    .generate_single_embedding(vector_data.video_summary.clone())
                    .await
                {
                    Ok(emb) => (emb, crate::qdrant_client::EmbeddingProvider::Voyage),
                    Err(e) => {
                        warn!("Voyage AI embedding failed, falling back to Gemini: {}", e);
                        // Fallback to Gemini
                        if let Some(ref gemini_client) = state.gemini_client {
                            (
                                Self::generate_text_embedding_gemini(
                                    &vector_data.video_summary,
                                    gemini_client,
                                )
                                .await?,
                                crate::qdrant_client::EmbeddingProvider::Gemini,
                            )
                        } else {
                            return Err(
                                "No embedding provider available (need Voyage AI or Gemini)".into(),
                            );
                        }
                    }
                }
            } else if let Some(ref gemini_client) = state.gemini_client {
                // Use Gemini embeddings
                info!("Using Gemini embeddings for video vectorization");
                (
                    Self::generate_text_embedding_gemini(&vector_data.video_summary, gemini_client)
                        .await?,
                    crate::qdrant_client::EmbeddingProvider::Gemini,
                )
            } else {
                return Err("No embedding provider available (need Voyage AI or Gemini)".into());
            };

        let video_point_id = format!("video_{}", vector_data.file_id);
        let video_payload = json!({
            "content_type": "video_summary",
            "file_id": vector_data.file_id,
            "session_id": vector_data.session_id,
            "user_id": vector_data.user_id,
            "content": vector_data.video_summary,
            "total_frames": vector_data.total_frames,
            "duration_seconds": vector_data.duration_seconds,
            "timestamp": chrono::Utc::now().to_rfc3339(),
            "embedding_provider": video_provider.vector_name()
        });

        qdrant_client
            .upsert_point(
                &video_point_id,
                &video_embedding,
                &video_payload,
                video_provider,
            )
            .await?;
        info!(
            "Stored video-level embedding for file: {} using {:?}",
            vector_data.file_id, video_provider
        );

        // 2. Store frame-level embeddings (use same embedding provider as video-level)
        for frame in &vector_data.frame_metadata {
            let (frame_embedding, frame_provider) = if let Some(ref voyage_embeddings) =
                state.voyage_embeddings
            {
                // Use Voyage AI
                match voyage_embeddings
                    .generate_single_embedding(frame.description.clone())
                    .await
                {
                    Ok(emb) => (emb, crate::qdrant_client::EmbeddingProvider::Voyage),
                    Err(e) => {
                        warn!(
                            "Voyage AI embedding failed for frame {}, falling back to Gemini: {}",
                            frame.frame_number, e
                        );
                        if let Some(ref gemini_client) = state.gemini_client {
                            (
                                Self::generate_text_embedding_gemini(
                                    &frame.description,
                                    gemini_client,
                                )
                                .await?,
                                crate::qdrant_client::EmbeddingProvider::Gemini,
                            )
                        } else {
                            return Err("No embedding provider available".into());
                        }
                    }
                }
            } else if let Some(ref gemini_client) = state.gemini_client {
                // Use Gemini
                (
                    Self::generate_text_embedding_gemini(&frame.description, gemini_client).await?,
                    crate::qdrant_client::EmbeddingProvider::Gemini,
                )
            } else {
                return Err("No embedding provider available".into());
            };

            let frame_point_id = format!("frame_{}_f{}", vector_data.file_id, frame.frame_number);
            let frame_payload = json!({
                "content_type": "video_frame",
                "file_id": vector_data.file_id,
                "session_id": vector_data.session_id,
                "user_id": vector_data.user_id,
                "content": frame.description,
                "frame_number": frame.frame_number,
                "timestamp_seconds": frame.timestamp_seconds,
                "visual_features": frame.visual_features,
                "timestamp": chrono::Utc::now().to_rfc3339(),
                "embedding_provider": frame_provider.vector_name()
            });

            qdrant_client
                .upsert_point(
                    &frame_point_id,
                    &frame_embedding,
                    &frame_payload,
                    frame_provider,
                )
                .await?;
        }

        info!(
            "Stored {} frame embeddings for file: {}",
            vector_data.frame_metadata.len(),
            vector_data.file_id
        );
        Ok(())
    }

    /// Generate text embedding using Gemini embedding model
    async fn generate_text_embedding_gemini(
        text: &str,
        gemini_client: &GeminiClient,
    ) -> Result<Vec<f32>, Box<dyn std::error::Error + Send + Sync>> {
        // Use the existing embed_content method
        let embedding = gemini_client.embed_content(text).await?;
        Ok(embedding)
    }

    /// Get video duration using FFprobe
    async fn get_video_duration(
        video_path: &str,
    ) -> Result<f64, Box<dyn std::error::Error + Send + Sync>> {
        let output = Command::new("ffprobe")
            .arg("-v")
            .arg("quiet")
            .arg("-show_entries")
            .arg("format=duration")
            .arg("-of")
            .arg("csv=p=0")
            .arg(video_path)
            .output()?;

        if !output.status.success() {
            return Err("Failed to get video duration".into());
        }

        let duration_str = String::from_utf8(output.stdout)?;
        let duration: f64 = duration_str.trim().parse()?;

        Ok(duration)
    }

    /// Search for similar video content using vector similarity
    pub async fn search_similar_video_content(
        query: &str,
        session_id: &str,
        limit: usize,
        state: &Arc<AppState>,
    ) -> Result<Vec<serde_json::Value>, Box<dyn std::error::Error + Send + Sync>> {
        // Generate embedding for the search query (support both Voyage AI and Gemini)
        let (query_embedding, provider) =
            if let Some(ref voyage_embeddings) = state.voyage_embeddings {
                (
                    voyage_embeddings
                        .generate_single_embedding(query.to_string())
                        .await?,
                    crate::qdrant_client::EmbeddingProvider::Voyage,
                )
            } else if let Some(ref gemini_client) = state.gemini_client {
                (
                    Self::generate_text_embedding_gemini(query, gemini_client).await?,
                    crate::qdrant_client::EmbeddingProvider::Gemini,
                )
            } else {
                return Err("No embedding provider available".into());
            };

        // Search in Qdrant with session filter
        let filter = json!({
            "must": [
                {
                    "key": "session_id",
                    "match": {
                        "value": session_id
                    }
                }
            ]
        });

        let qdrant_client = match &state.qdrant_client {
            Some(client) => client,
            None => return Err("Qdrant client not available".into()),
        };

        let search_results = qdrant_client
            .search_points(&query_embedding, limit, Some(&filter), provider)
            .await?;

        Ok(search_results)
    }

    /// Store video analysis from Gemini into the `video_content` Qdrant collection.
    ///
    /// This replaces the 100+ frame embedding approach:
    /// - One embedding from the video summary (Voyage AI preferred, Gemini fallback)
    /// - Stored in `video_content` collection with full viral moments payload
    /// - Deterministic point ID from UUID v5 of video_id (idempotent upsert)
    pub async fn store_video_analysis_from_gemini(
        video_id: &str,
        youtube_url: &str,
        user_id: Option<i32>,
        channel_id: Option<&str>,
        analysis: &crate::clipping::gemini_video_analyzer::VideoAnalysis,
        state: &Arc<AppState>,
    ) -> Result<(), Box<dyn std::error::Error + Send + Sync>> {
        let qdrant_client = match &state.qdrant_client {
            Some(c) => c,
            None => {
                warn!("Qdrant client not available — skipping video content storage");
                return Ok(());
            }
        };

        // Generate ONE embedding from the video summary
        let (embedding, provider) = if let Some(ref voyage) = state.voyage_embeddings {
            match voyage
                .generate_single_embedding(analysis.video_summary.clone())
                .await
            {
                Ok(emb) => (emb, crate::qdrant_client::EmbeddingProvider::Voyage),
                Err(e) => {
                    warn!("Voyage embedding failed: {} — trying Gemini", e);
                    if let Some(ref gemini) = state.gemini_client {
                        let emb = gemini.embed_content(&analysis.video_summary).await?;
                        (emb, crate::qdrant_client::EmbeddingProvider::Gemini)
                    } else {
                        return Err("No embedding provider available".into());
                    }
                }
            }
        } else if let Some(ref gemini) = state.gemini_client {
            let emb = gemini.embed_content(&analysis.video_summary).await?;
            (emb, crate::qdrant_client::EmbeddingProvider::Gemini)
        } else {
            return Err("No embedding provider available".into());
        };

        // Ensure the collection exists
        let _ = qdrant_client.ensure_video_content_collection().await;

        // Serialize viral moments
        let viral_moments_json =
            serde_json::to_string(&analysis.viral_moments).unwrap_or_else(|_| "[]".to_string());

        // Build payload
        let payload = serde_json::json!({
            "video_id": video_id,
            "source_type": "youtube",
            "youtube_url": youtube_url,
            "user_id": user_id.map(|id| id.to_string()),
            "channel_id": channel_id,
            "summary": analysis.video_summary,
            "content_category": analysis.content_type,
            "overall_quality": analysis.overall_quality,
            "viral_moments_json": viral_moments_json,
            "viral_moments_count": analysis.viral_moments.len(),
            "analyzed_at": chrono::Utc::now().to_rfc3339(),
            "embedding_provider": provider.vector_name(),
        });

        qdrant_client
            .store_video_content(video_id, payload, embedding, provider)
            .await?;

        info!(
            "✅ Stored video analysis in video_content: video_id={}, {} viral moments, quality={:.2}",
            video_id, analysis.viral_moments.len(), analysis.overall_quality
        );
        Ok(())
    }

    /// Retrieve video analysis from Qdrant by file path
    /// This allows LLMs to "view" a video by reading its vectorized content
    pub async fn retrieve_video_analysis(
        video_file_path: &str,
        state: &Arc<AppState>,
    ) -> Result<serde_json::Value, Box<dyn std::error::Error + Send + Sync>> {
        // Derive file_id from the path in the same way it was set during vectorization.
        //
        // Vectorization in clipping_job.rs calls:
        //   process_video_for_vectorization(&video_path, &job.source_video_id, ...)
        // so main video file_id = source_video_id = the YouTube video ID.
        //
        // For paths like "downloads/clipping_355_jNQXAC9IVRw.mp4":
        //   stem = "clipping_355_jNQXAC9IVRw" → strip "clipping_{job_id}_" → "jNQXAC9IVRw"
        //
        // For clip paths like "outputs/clip_355_1.mp4":
        //   stem = "clip_355_1" → used as-is (matches format!("clip_{}_{}", job_id, index+1))
        //
        // Fallback: hash of full path (old behavior for unknown patterns).

        let path_stem = std::path::Path::new(video_file_path)
            .file_stem()
            .and_then(|s| s.to_str())
            .unwrap_or("");

        let file_id = if path_stem.starts_with("clipping_") {
            // "clipping_{job_id}_{video_id}" → extract video_id (last segment after '_')
            path_stem
                .rsplit('_')
                .next()
                .unwrap_or(path_stem)
                .to_string()
        } else if path_stem.starts_with("clip_") {
            // "clip_{job_id}_{index}" → use full stem
            path_stem.to_string()
        } else {
            // Unknown format: fall back to hash of path
            use std::collections::hash_map::DefaultHasher;
            use std::hash::{Hash, Hasher};
            let mut hasher = DefaultHasher::new();
            video_file_path.hash(&mut hasher);
            format!("{:x}", hasher.finish())
        };

        let qdrant_client = match &state.qdrant_client {
            Some(client) => client,
            None => return Err("Qdrant client not available".into()),
        };

        // Retrieve video-level summary
        let _video_point_id = format!("video_{}", file_id);

        // Search for video summary point
        let filter = json!({
            "must": [
                {
                    "key": "file_id",
                    "match": {
                        "value": file_id
                    }
                },
                {
                    "key": "content_type",
                    "match": {
                        "value": "video_summary"
                    }
                }
            ]
        });

        // Use search with zero vector (we just want to filter and retrieve)
        // Determine provider based on available clients (prefer Voyage, fallback to Gemini)
        let provider = if state.voyage_embeddings.is_some() {
            crate::qdrant_client::EmbeddingProvider::Voyage
        } else {
            crate::qdrant_client::EmbeddingProvider::Gemini
        };
        let zero_vector = provider.zero_vector();
        let results = qdrant_client
            .search_points(&zero_vector, 1, Some(&filter), provider)
            .await?;

        if results.is_empty() {
            return Err(format!("No vectorized data found for video: {}", video_file_path).into());
        }

        let video_payload = results[0].clone();

        // Retrieve frame-level data
        let frame_filter = json!({
            "must": [
                {
                    "key": "file_id",
                    "match": {
                        "value": file_id
                    }
                },
                {
                    "key": "content_type",
                    "match": {
                        "value": "video_frame"
                    }
                }
            ]
        });

        let frame_results = qdrant_client
            .search_points(&zero_vector, 50, Some(&frame_filter), provider)
            .await?;

        // Compile comprehensive analysis
        let analysis = json!({
            "file_path": video_file_path,
            "file_id": file_id,
            "video_summary": video_payload.get("content").and_then(|v| v.as_str()).unwrap_or("No summary"),
            "duration_seconds": video_payload.get("duration_seconds").and_then(|v| v.as_f64()).unwrap_or(0.0),
            "total_frames": video_payload.get("total_frames").and_then(|v| v.as_u64()).unwrap_or(0),
            "session_id": video_payload.get("session_id").and_then(|v| v.as_str()).unwrap_or("unknown"),
            "frame_count": frame_results.len(),
            "frames": frame_results.iter().map(|f| {
                json!({
                    "frame_number": f.get("frame_number").and_then(|v| v.as_u64()).unwrap_or(0),
                    "timestamp": f.get("timestamp_seconds").and_then(|v| v.as_f64()).unwrap_or(0.0),
                    "description": f.get("content").and_then(|v| v.as_str()).unwrap_or(""),
                    "visual_features": f.get("visual_features").and_then(|v| v.as_array()).unwrap_or(&vec![]).clone(),
                })
            }).collect::<Vec<_>>()
        });

        Ok(analysis)
    }
}
