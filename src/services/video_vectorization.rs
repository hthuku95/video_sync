// src/services/video_vectorization.rs
use crate::gemini_client::GeminiClient;
use crate::AppState;
use serde::{Deserialize, Serialize};
use serde_json::json;
use std::process::Command;
use std::sync::Arc;
use tokio::fs;
use tracing::{info, warn};
use base64::prelude::*;

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
    /// Extract keyframes from video and generate embeddings for storage in Qdrant
    pub async fn process_video_for_vectorization(
        video_file_path: &str,
        file_id: &str,
        session_id: &str,
        user_id: Option<i32>,
        state: &Arc<AppState>,
    ) -> Result<(), Box<dyn std::error::Error + Send + Sync>> {
        info!("Starting video vectorization for file: {} ({})", video_file_path, file_id);

        // Step 1: Extract keyframes from video
        let frames_dir = format!("temp_frames/{}", file_id);
        fs::create_dir_all(&frames_dir).await?;
        
        let keyframes = Self::extract_keyframes(video_file_path, &frames_dir).await?;
        info!("Extracted {} keyframes from video", keyframes.len());

        // Step 2: Analyze each frame using Gemini multimodal model
        let mut frame_metadata = Vec::new();
        let gemini_client = match &state.gemini_client {
            Some(client) => client,
            None => return Err("Gemini client not available".into()),
        };
        
        for (frame_number, frame_path) in keyframes.iter().enumerate() {
            match Self::analyze_frame_with_gemini(frame_path, frame_number as u32, gemini_client).await {
                Ok(metadata) => {
                    frame_metadata.push(metadata);
                },
                Err(e) => {
                    warn!("Failed to analyze frame {}: {}", frame_number, e);
                }
            }
        }

        // Step 3: Generate overall video summary using frame analysis
        let video_summary = Self::generate_video_summary(&frame_metadata, gemini_client).await?;

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

        // Step 5: Clean up temporary frames
        let _ = fs::remove_dir_all(&frames_dir).await;

        info!("Successfully vectorized video: {} with {} frame embeddings", file_id, frame_metadata.len());
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
            return Err(format!("FFmpeg failed: {}", String::from_utf8_lossy(&output.stderr)).into());
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
        let frame_base64 = base64::prelude::BASE64_STANDARD.encode(&frame_data);
        
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

        let analysis_result = gemini_client.analyze_video_content(frame_path, Some(analysis_prompt)).await?;
        
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

    /// Parse AI analysis response to extract description and features
    fn parse_frame_analysis(analysis_result: &str) -> (String, Vec<String>) {
        // Try to parse as JSON first
        if let Ok(parsed) = serde_json::from_str::<serde_json::Value>(analysis_result) {
            let description = parsed.get("description")
                .and_then(|v| v.as_str())
                .unwrap_or(analysis_result)
                .to_string();
            
            let features = parsed.get("visual_features")
                .and_then(|v| v.as_array())
                .map(|arr| arr.iter()
                    .filter_map(|v| v.as_str().map(String::from))
                    .collect())
                .unwrap_or_else(|| vec!["unstructured_analysis".to_string()]);
                
            (description, features)
        } else {
            // Fallback to using the raw text as description
            (analysis_result.to_string(), vec!["text_analysis".to_string()])
        }
    }

    /// Generate overall video summary from frame analyses
    async fn generate_video_summary(
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
        use crate::gemini_client::{GenerateContentRequest, Content, Part};
        
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
        };
        
        let response = gemini_client.generate_content(request).await?;
        
        // Extract text from response
        let summary = response.candidates
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

    /// Store video embeddings in Qdrant vector database
    async fn store_video_embeddings(
        vector_data: &VideoVectorData,
        state: &Arc<AppState>,
    ) -> Result<(), Box<dyn std::error::Error + Send + Sync>> {
        let qdrant_client = match &state.qdrant_client {
            Some(client) => client,
            None => return Err("Qdrant client not available".into()),
        };

        // Generate embeddings for video summary and each frame description
        let gemini_client = match &state.gemini_client {
            Some(client) => client,
            None => return Err("Gemini client not available".into()),
        };

        // 1. Store video-level embedding
        let video_embedding = Self::generate_text_embedding(&vector_data.video_summary, gemini_client).await?;
        
        let video_point_id = format!("video_{}", vector_data.file_id);
        let video_payload = json!({
            "content_type": "video_summary",
            "file_id": vector_data.file_id,
            "session_id": vector_data.session_id,
            "user_id": vector_data.user_id,
            "content": vector_data.video_summary,
            "total_frames": vector_data.total_frames,
            "duration_seconds": vector_data.duration_seconds,
            "timestamp": chrono::Utc::now().to_rfc3339()
        });

        qdrant_client.upsert_point(&video_point_id, &video_embedding, &video_payload).await?;
        info!("Stored video-level embedding for file: {}", vector_data.file_id);

        // 2. Store frame-level embeddings
        for frame in &vector_data.frame_metadata {
            let frame_embedding = Self::generate_text_embedding(&frame.description, gemini_client).await?;
            
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
                "timestamp": chrono::Utc::now().to_rfc3339()
            });

            qdrant_client.upsert_point(&frame_point_id, &frame_embedding, &frame_payload).await?;
        }

        info!("Stored {} frame embeddings for file: {}", vector_data.frame_metadata.len(), vector_data.file_id);
        Ok(())
    }

    /// Generate text embedding using Gemini embedding model
    async fn generate_text_embedding(
        text: &str,
        gemini_client: &GeminiClient,
    ) -> Result<Vec<f32>, Box<dyn std::error::Error + Send + Sync>> {
        // Use the existing embed_content method
        let embedding = gemini_client.embed_content(text).await?;
        Ok(embedding)
    }

    /// Get video duration using FFprobe
    async fn get_video_duration(video_path: &str) -> Result<f64, Box<dyn std::error::Error + Send + Sync>> {
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
        let gemini_client = match &state.gemini_client {
            Some(client) => client,
            None => return Err("Gemini client not available".into()),
        };
        
        // Generate embedding for the search query
        let query_embedding = Self::generate_text_embedding(query, gemini_client).await?;

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
            .search_points(&query_embedding, limit, Some(&filter))
            .await?;

        Ok(search_results)
    }

    /// Retrieve video analysis from Qdrant by file path
    /// This allows LLMs to "view" a video by reading its vectorized content
    pub async fn retrieve_video_analysis(
        video_file_path: &str,
        state: &Arc<AppState>,
    ) -> Result<serde_json::Value, Box<dyn std::error::Error + Send + Sync>> {
        // Generate file_id from path (same as used during vectorization)
        use std::collections::hash_map::DefaultHasher;
        use std::hash::{Hash, Hasher};

        let mut hasher = DefaultHasher::new();
        video_file_path.hash(&mut hasher);
        let file_id = format!("{:x}", hasher.finish());

        let qdrant_client = match &state.qdrant_client {
            Some(client) => client,
            None => return Err("Qdrant client not available".into()),
        };

        // Retrieve video-level summary
        let video_point_id = format!("video_{}", file_id);

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
        let zero_vector = vec![0.0; 768];
        let results = qdrant_client.search_points(&zero_vector, 1, Some(&filter)).await?;

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

        let frame_results = qdrant_client.search_points(&zero_vector, 50, Some(&frame_filter)).await?;

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