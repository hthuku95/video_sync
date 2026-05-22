use crate::{
    gemini_client::{InlineData, Part},
    qdrant_client::EmbeddingProvider,
    AppState,
};
use base64::Engine;
use serde::{Deserialize, Serialize};
use serde_json::json;
use std::process::Command;
use std::sync::Arc;

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct MediaReviewArtifact {
    pub review_id: String,
    pub asset_kind: String,
    pub source_type: String,
    pub service_slug: Option<String>,
    pub owner_user_id: Option<i32>,
    pub output_url: Option<String>,
    pub source_url: Option<String>,
    pub prompt: Option<String>,
    pub title: Option<String>,
    pub company: Option<String>,
    pub review_status: String,
    pub qa_score: Option<i32>,
    pub qa_feedback: Option<String>,
    pub narration_text: Option<String>,
    pub visual_direction: Option<String>,
    pub transcript_excerpt: Option<String>,
    pub tags: Vec<String>,
}

pub struct MediaReviewService;

impl MediaReviewService {
    fn multimodal_review_model() -> String {
        std::env::var("GEMINI_MULTIMODAL_REVIEW_EMBEDDING_MODEL")
            .unwrap_or_else(|_| "models/gemini-embedding-2".to_string())
    }

    fn review_model() -> String {
        std::env::var("GEMINI_REVIEW_EMBEDDING_MODEL")
            .unwrap_or_else(|_| "models/gemini-embedding-001".to_string())
    }

    fn review_dimensions() -> u32 {
        std::env::var("GEMINI_EMBEDDING2_DIMENSIONS")
            .ok()
            .and_then(|value| value.parse::<u32>().ok())
            .unwrap_or(1536)
    }

    fn upload_video_segment_seconds() -> f64 {
        std::env::var("GEMINI_MULTIMODAL_VIDEO_SEGMENT_SECONDS")
            .ok()
            .and_then(|value| value.parse::<f64>().ok())
            .unwrap_or(110.0)
    }

    fn upload_video_overlap_seconds() -> f64 {
        std::env::var("GEMINI_MULTIMODAL_VIDEO_SEGMENT_OVERLAP_SECONDS")
            .ok()
            .and_then(|value| value.parse::<f64>().ok())
            .unwrap_or(10.0)
    }

    fn build_review_text(artifact: &MediaReviewArtifact) -> String {
        let mut lines = vec![
            format!("Asset kind: {}", artifact.asset_kind),
            format!("Source type: {}", artifact.source_type),
            format!("Review status: {}", artifact.review_status),
        ];

        if let Some(service_slug) = artifact.service_slug.as_deref() {
            lines.push(format!("Service slug: {service_slug}"));
        }
        if let Some(title) = artifact.title.as_deref() {
            lines.push(format!("Title: {title}"));
        }
        if let Some(company) = artifact.company.as_deref() {
            lines.push(format!("Company: {company}"));
        }
        if let Some(url) = artifact.source_url.as_deref() {
            lines.push(format!("Source URL: {url}"));
        }
        if let Some(url) = artifact.output_url.as_deref() {
            lines.push(format!("Output URL: {url}"));
        }
        if let Some(score) = artifact.qa_score {
            lines.push(format!("QA score: {score}/10"));
        }
        if let Some(feedback) = artifact.qa_feedback.as_deref() {
            lines.push(format!("QA feedback: {feedback}"));
        }
        if let Some(direction) = artifact.visual_direction.as_deref() {
            lines.push(format!("Visual direction: {direction}"));
        }
        if let Some(narration) = artifact.narration_text.as_deref() {
            lines.push(format!("Narration: {narration}"));
        }
        if let Some(transcript) = artifact.transcript_excerpt.as_deref() {
            lines.push(format!("Transcript excerpt: {transcript}"));
        }
        if let Some(prompt) = artifact.prompt.as_deref() {
            lines.push(format!("Prompt: {prompt}"));
        }
        if !artifact.tags.is_empty() {
            lines.push(format!("Tags: {}", artifact.tags.join(", ")));
        }

        lines.join("\n")
    }

    fn local_media_path(artifact: &MediaReviewArtifact) -> Option<String> {
        artifact
            .output_url
            .as_deref()
            .filter(|value| std::path::Path::new(value).exists())
            .map(|value| value.to_string())
            .or_else(|| {
                artifact
                    .source_url
                    .as_deref()
                    .filter(|value| std::path::Path::new(value).exists())
                    .map(|value| value.to_string())
            })
    }

    fn review_document_text(artifact: &MediaReviewArtifact, review_text: &str) -> String {
        format!(
            "title: {} | text: {}",
            artifact.title.as_deref().unwrap_or("none"),
            review_text
        )
    }

    fn multimodal_part_from_local_file(file_path: &str) -> Option<Part> {
        let path = std::path::Path::new(file_path);
        if !path.exists() {
            return None;
        }

        let ext = path
            .extension()
            .and_then(|ext| ext.to_str())
            .map(|ext| ext.to_ascii_lowercase())
            .unwrap_or_default();

        let mime_type = match ext.as_str() {
            "png" => "image/png",
            "jpg" | "jpeg" => "image/jpeg",
            "webp" => "image/webp",
            "bmp" => "image/bmp",
            "heic" => "image/heic",
            "heif" => "image/heif",
            "avif" => "image/avif",
            "mp3" => "audio/mpeg",
            "wav" => "audio/wav",
            "mp4" => "video/mp4",
            "mov" => "video/quicktime",
            _ => return None,
        };

        let max_inline_bytes = std::env::var("GEMINI_MULTIMODAL_REVIEW_MAX_INLINE_BYTES")
            .ok()
            .and_then(|value| value.parse::<u64>().ok())
            .unwrap_or(8 * 1024 * 1024);

        let metadata = std::fs::metadata(path).ok()?;
        if metadata.len() > max_inline_bytes {
            tracing::info!(
                file_path = file_path,
                size_bytes = metadata.len(),
                max_inline_bytes = max_inline_bytes,
                "Skipping Gemini multimodal inline embedding because artifact exceeds inline size cap"
            );
            return None;
        }

        let bytes = std::fs::read(path).ok()?;
        Some(Part::InlineData {
            inline_data: InlineData {
                mime_type: mime_type.to_string(),
                data: base64::prelude::BASE64_STANDARD.encode(bytes),
            },
        })
    }

    fn uploadable_mime_type(file_path: &str) -> Option<&'static str> {
        let path = std::path::Path::new(file_path);
        let ext = path
            .extension()
            .and_then(|ext| ext.to_str())
            .map(|ext| ext.to_ascii_lowercase())
            .unwrap_or_default();

        match ext.as_str() {
            "png" => Some("image/png"),
            "jpg" | "jpeg" => Some("image/jpeg"),
            "webp" => Some("image/webp"),
            "bmp" => Some("image/bmp"),
            "heic" => Some("image/heic"),
            "heif" => Some("image/heif"),
            "avif" => Some("image/avif"),
            "mp3" => Some("audio/mpeg"),
            "wav" => Some("audio/wav"),
            "m4a" => Some("audio/mp4"),
            "aac" => Some("audio/aac"),
            "ogg" => Some("audio/ogg"),
            "flac" => Some("audio/flac"),
            "mp4" => Some("video/mp4"),
            "mov" => Some("video/quicktime"),
            "webm" => Some("video/webm"),
            "pdf" => Some("application/pdf"),
            _ => None,
        }
    }

    fn is_video_path(file_path: &str) -> bool {
        matches!(
            std::path::Path::new(file_path)
                .extension()
                .and_then(|ext| ext.to_str())
                .map(|ext| ext.to_ascii_lowercase())
                .as_deref(),
            Some("mp4" | "mov" | "webm")
        )
    }

    fn video_duration_seconds(file_path: &str) -> Option<f64> {
        crate::utils::ffmpeg_utils::get_media_info(file_path, "duration")
            .ok()
            .and_then(|value| value.trim().parse::<f64>().ok())
    }

    fn build_video_segment_plan(duration_secs: f64) -> Vec<(f64, f64)> {
        let segment_len = Self::upload_video_segment_seconds().clamp(1.0, 120.0);
        let overlap = Self::upload_video_overlap_seconds().clamp(0.0, segment_len - 1.0);

        if duration_secs <= 120.0 {
            return vec![(0.0, duration_secs.max(0.1))];
        }

        let mut plan = Vec::new();
        let mut start = 0.0f64;
        while start < duration_secs {
            let remaining = duration_secs - start;
            let current_len = remaining.min(segment_len);
            plan.push((start, current_len));
            if start + current_len >= duration_secs {
                break;
            }
            start += segment_len - overlap;
        }

        plan
    }

    fn extract_video_segment(
        source_path: &str,
        start_secs: f64,
        duration_secs: f64,
        segment_index: usize,
    ) -> Result<String, String> {
        let output_path = crate::utils::ffmpeg_utils::create_temp_file(
            &format!("gemini_mm_segment_{}", segment_index),
            "mp4",
        );
        let mut command = Command::new("ffmpeg");
        command
            .arg("-ss")
            .arg(format!("{:.3}", start_secs))
            .arg("-t")
            .arg(format!("{:.3}", duration_secs))
            .arg("-i")
            .arg(source_path)
            .arg("-c")
            .arg("copy")
            .arg("-avoid_negative_ts")
            .arg("make_zero")
            .arg("-y")
            .arg(&output_path);

        crate::utils::ffmpeg_utils::execute_ffmpeg_command(command)?;
        Ok(output_path)
    }

    fn average_embeddings(
        embeddings: &[Vec<f32>],
    ) -> Result<Vec<f32>, Box<dyn std::error::Error + Send + Sync>> {
        let first = embeddings
            .first()
            .ok_or("No embeddings available to average for Gemini multimodal review")?;
        let mut average = vec![0.0f32; first.len()];
        for embedding in embeddings {
            if embedding.len() != average.len() {
                return Err("Embedding dimension mismatch while averaging video chunks".into());
            }
            for (index, value) in embedding.iter().enumerate() {
                average[index] += *value;
            }
        }
        let count = embeddings.len() as f32;
        for value in &mut average {
            *value /= count;
        }
        Ok(average)
    }

    async fn embed_uploaded_media(
        state: &Arc<AppState>,
        artifact: &MediaReviewArtifact,
        review_document: &str,
        local_media_path: &str,
    ) -> Result<Vec<f32>, Box<dyn std::error::Error + Send + Sync>> {
        let gemini = state
            .gemini_client
            .as_ref()
            .ok_or("Gemini client unavailable for uploaded media embedding")?;
        let mime_type = Self::uploadable_mime_type(local_media_path)
            .ok_or("Unsupported media type for Gemini uploaded embedding")?;

        if Self::is_video_path(local_media_path) {
            let duration_secs = Self::video_duration_seconds(local_media_path).unwrap_or(0.0);
            let segment_plan = Self::build_video_segment_plan(duration_secs);

            if segment_plan.len() == 1 && duration_secs > 0.0 && duration_secs <= 120.0 {
                return gemini
                    .embed_uploaded_file_with_model(
                        local_media_path,
                        mime_type,
                        &Self::multimodal_review_model(),
                        Some(Self::review_dimensions()),
                        Some(review_document),
                    )
                    .await;
            }

            tracing::info!(
                review_id = %artifact.review_id,
                asset_kind = %artifact.asset_kind,
                duration_secs = duration_secs,
                chunk_count = segment_plan.len(),
                "Embedding long video in Gemini multimodal chunked segments"
            );

            let mut segment_paths = Vec::new();
            let mut embeddings = Vec::new();
            for (index, (start_secs, len_secs)) in segment_plan.iter().copied().enumerate() {
                let segment_path =
                    Self::extract_video_segment(local_media_path, start_secs, len_secs, index)?;
                let segment_prompt = format!(
                    "{}\nVideo segment {}/{} from {:.1}s to {:.1}s of the source video.",
                    review_document,
                    index + 1,
                    segment_plan.len(),
                    start_secs,
                    start_secs + len_secs
                );
                let embedding = gemini
                    .embed_uploaded_file_with_model(
                        &segment_path,
                        mime_type,
                        &Self::multimodal_review_model(),
                        Some(Self::review_dimensions()),
                        Some(&segment_prompt),
                    )
                    .await?;
                embeddings.push(embedding);
                segment_paths.push(segment_path);
            }
            crate::utils::ffmpeg_utils::cleanup_temp_files(&segment_paths);
            return Self::average_embeddings(&embeddings);
        }

        gemini
            .embed_uploaded_file_with_model(
                local_media_path,
                mime_type,
                &Self::multimodal_review_model(),
                Some(Self::review_dimensions()),
                Some(review_document),
            )
            .await
    }

    pub async fn store_artifact(
        state: &Arc<AppState>,
        artifact: MediaReviewArtifact,
    ) -> Result<(), Box<dyn std::error::Error + Send + Sync>> {
        let qdrant = match &state.qdrant_client {
            Some(client) => client,
            None => return Ok(()),
        };
        let gemini = match &state.gemini_client {
            Some(client) => client,
            None => return Ok(()),
        };

        qdrant.ensure_media_review_collection().await?;

        let review_text = Self::build_review_text(&artifact);
        let review_document = Self::review_document_text(&artifact, &review_text);
        let local_media_path = Self::local_media_path(&artifact);

        let embedding = if let Some(part) = local_media_path
            .as_deref()
            .and_then(Self::multimodal_part_from_local_file)
        {
            match gemini
                .embed_parts_with_model(
                    vec![
                        Part::Text {
                            text: review_document.clone(),
                        },
                        part,
                    ],
                    &Self::multimodal_review_model(),
                    Some(Self::review_dimensions()),
                )
                .await
                {
                Ok(values) => values,
                Err(error) => {
                    tracing::warn!(
                        review_id = %artifact.review_id,
                        asset_kind = %artifact.asset_kind,
                        error = %error,
                        "Gemini multimodal embedding failed; falling back to secondary embedding paths"
                    );
                    Self::fallback_embedding(
                        state,
                        &artifact,
                        &review_text,
                        &review_document,
                        local_media_path.as_deref(),
                    )
                    .await?
                }
            }
        } else {
            Self::fallback_embedding(
                state,
                &artifact,
                &review_text,
                &review_document,
                local_media_path.as_deref(),
            )
            .await?
        };

        let payload = json!({
            "review_id": artifact.review_id,
            "asset_kind": artifact.asset_kind,
            "source_type": artifact.source_type,
            "service_slug": artifact.service_slug,
            "owner_user_id": artifact.owner_user_id,
            "output_url": artifact.output_url,
            "source_url": artifact.source_url,
            "prompt": artifact.prompt,
            "title": artifact.title,
            "company": artifact.company,
            "review_status": artifact.review_status,
            "qa_score": artifact.qa_score,
            "qa_feedback": artifact.qa_feedback,
            "narration_text": artifact.narration_text,
            "visual_direction": artifact.visual_direction,
            "transcript_excerpt": artifact.transcript_excerpt,
            "tags": artifact.tags,
            "review_text": review_text,
            "embedding_provider": EmbeddingProvider::GeminiEmbedding2.vector_name(),
            "created_at": chrono::Utc::now().to_rfc3339(),
        });

        qdrant
            .store_media_review(&artifact.review_id, payload, embedding)
            .await?;

        Ok(())
    }

    async fn fallback_embedding(
        state: &Arc<AppState>,
        artifact: &MediaReviewArtifact,
        review_text: &str,
        review_document: &str,
        local_media_path: Option<&str>,
    ) -> Result<Vec<f32>, Box<dyn std::error::Error + Send + Sync>> {
        let gemini = match &state.gemini_client {
            Some(client) => client,
            None => return Err("Gemini client unavailable for fallback embedding".into()),
        };

        if let Some(local_media_path) = local_media_path {
            match Self::embed_uploaded_media(state, artifact, review_document, local_media_path).await
            {
                Ok(values) => return Ok(values),
                Err(error) => {
                    tracing::warn!(
                        review_id = %artifact.review_id,
                        asset_kind = %artifact.asset_kind,
                        error = %error,
                        "Gemini uploaded-media embedding failed; falling back to secondary embedding paths"
                    );
                }
            }
        }

        if let Some(vertex_client) = state.vertex_multimodal_embeddings.as_ref() {
            match vertex_client
                .embed_review_artifact(
                    review_text,
                    artifact.title.as_deref(),
                    local_media_path,
                )
                .await
            {
                Ok(values) => Ok(values),
                Err(error) => {
                    tracing::warn!(
                        review_id = %artifact.review_id,
                        asset_kind = %artifact.asset_kind,
                        error = %error,
                        "Vertex multimodal embedding failed; falling back to legacy Gemini review embedding"
                    );
                    gemini
                        .embed_content_with_model(
                            review_text,
                            &Self::review_model(),
                            Some(Self::review_dimensions()),
                        )
                        .await
                }
            }
        } else {
            gemini
                .embed_content_with_model(
                    review_text,
                    &Self::review_model(),
                    Some(Self::review_dimensions()),
                )
                .await
        }
    }
}

#[cfg(test)]
mod tests {
    use super::MediaReviewService;

    #[test]
    fn long_videos_are_split_into_overlapping_chunks() {
        let plan = MediaReviewService::build_video_segment_plan(305.0);
        assert!(plan.len() >= 3);
        assert_eq!(plan[0].0, 0.0);
        for window in plan.windows(2) {
            assert!(window[1].0 < window[0].0 + window[0].1);
        }
        let last = plan.last().unwrap();
        assert!(last.0 + last.1 >= 305.0);
    }

    #[test]
    fn short_videos_stay_single_chunk() {
        let plan = MediaReviewService::build_video_segment_plan(90.0);
        assert_eq!(plan, vec![(0.0, 90.0)]);
    }

    #[test]
    fn averaged_embeddings_preserve_dimension_and_mean() {
        let averaged = MediaReviewService::average_embeddings(&[
            vec![1.0, 3.0, 5.0],
            vec![3.0, 5.0, 7.0],
        ])
        .unwrap();
        assert_eq!(averaged, vec![2.0, 4.0, 6.0]);
    }
}
