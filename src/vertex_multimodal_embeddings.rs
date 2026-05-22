use crate::gemini_client::{
    EmbedContent, EmbedContentRequest, InlineData, Part,
};
use base64::Engine;
use reqwest::Client;
use serde::Deserialize;
use std::path::Path;

#[derive(Clone)]
pub struct VertexMultimodalEmbeddingsClient {
    client: Client,
    project_id: String,
    location: String,
    model: String,
    output_dimensionality: Option<u32>,
}

#[derive(Debug, Deserialize)]
struct VertexEmbedContentResponse {
    embedding: crate::gemini_client::Embedding,
}

#[derive(Debug, Deserialize)]
struct MetadataTokenResponse {
    access_token: String,
}

impl VertexMultimodalEmbeddingsClient {
    pub fn from_env() -> Option<Self> {
        let project_id = std::env::var("VERTEX_AI_PROJECT_ID")
            .or_else(|_| std::env::var("GCP_PROJECT_ID"))
            .ok()?;

        let location = std::env::var("VERTEX_AI_LOCATION")
            .or_else(|_| std::env::var("GOOGLE_CLOUD_LOCATION"))
            .unwrap_or_else(|_| "us".to_string());

        let model = std::env::var("VERTEX_MULTIMODAL_EMBEDDING_MODEL")
            .unwrap_or_else(|_| "gemini-embedding-2".to_string());

        let output_dimensionality = std::env::var("GEMINI_EMBEDDING2_DIMENSIONS")
            .ok()
            .and_then(|value| value.parse::<u32>().ok());

        Some(Self {
            client: Client::new(),
            project_id,
            location,
            model,
            output_dimensionality,
        })
    }

    pub async fn embed_review_artifact(
        &self,
        review_text: &str,
        title: Option<&str>,
        local_file_path: Option<&str>,
    ) -> Result<Vec<f32>, Box<dyn std::error::Error + Send + Sync>> {
        let document_text = format!(
            "title: {} | text: {}",
            title.unwrap_or("none"),
            review_text
        );

        let mut parts = vec![Part::Text {
            text: document_text,
        }];

        if let Some(path) = local_file_path {
            if let Some(part) = self.part_from_local_file(path).await? {
                parts.push(part);
            }
        }

        self.embed_parts(parts).await
    }

    async fn embed_parts(
        &self,
        parts: Vec<Part>,
    ) -> Result<Vec<f32>, Box<dyn std::error::Error + Send + Sync>> {
        let token = self.access_token().await?;
        let url = format!(
            "https://aiplatform.{}.rep.googleapis.com/v1/projects/{}/locations/{}/publishers/google/models/{}:embedContent",
            self.location, self.project_id, self.location, self.model
        );

        let request = EmbedContentRequest {
            model: self.model.clone(),
            content: EmbedContent { parts },
            output_dimensionality: self.output_dimensionality,
        };

        let response = self
            .client
            .post(&url)
            .header("Authorization", format!("Bearer {}", token))
            .header("Content-Type", "application/json")
            .json(&request)
            .send()
            .await?;

        if response.status().is_success() {
            let parsed: VertexEmbedContentResponse = response.json().await?;
            Ok(parsed.embedding.values)
        } else {
            let status = response.status();
            let body = response.text().await.unwrap_or_default();
            Err(format!("Vertex multimodal embedding API error ({}): {}", status, body).into())
        }
    }

    async fn access_token(&self) -> Result<String, Box<dyn std::error::Error + Send + Sync>> {
        if let Ok(token) = std::env::var("VERTEX_AI_ACCESS_TOKEN") {
            if !token.trim().is_empty() {
                return Ok(token);
            }
        }

        let response = self
            .client
            .get("http://metadata.google.internal/computeMetadata/v1/instance/service-accounts/default/token")
            .header("Metadata-Flavor", "Google")
            .send()
            .await?;

        if !response.status().is_success() {
            let status = response.status();
            let body = response.text().await.unwrap_or_default();
            return Err(format!(
                "Failed to fetch Vertex access token from metadata server ({}): {}",
                status, body
            )
            .into());
        }

        let token: MetadataTokenResponse = response.json().await?;
        Ok(token.access_token)
    }

    async fn part_from_local_file(
        &self,
        file_path: &str,
    ) -> Result<Option<Part>, Box<dyn std::error::Error + Send + Sync>> {
        let path = Path::new(file_path);
        if !path.exists() {
            return Ok(None);
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
            _ => return Ok(None),
        };

        let max_inline_bytes = std::env::var("VERTEX_MULTIMODAL_MAX_INLINE_BYTES")
            .ok()
            .and_then(|value| value.parse::<u64>().ok())
            .unwrap_or(8 * 1024 * 1024);

        let metadata = std::fs::metadata(path)?;
        if metadata.len() > max_inline_bytes {
            tracing::info!(
                file_path = file_path,
                size_bytes = metadata.len(),
                max_inline_bytes = max_inline_bytes,
                "Skipping inline multimodal embedding payload because file exceeds inline size cap"
            );
            return Ok(None);
        }

        let bytes = std::fs::read(path)?;
        let encoded = base64::prelude::BASE64_STANDARD.encode(bytes);
        Ok(Some(Part::InlineData {
            inline_data: InlineData {
                mime_type: mime_type.to_string(),
                data: encoded,
            },
        }))
    }
}
