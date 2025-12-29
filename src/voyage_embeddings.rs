use reqwest::Client;
use serde::{Deserialize, Serialize};

#[derive(Debug, Clone)]
pub struct VoyageEmbeddings {
    client: Client,
    api_key: String,
    base_url: String,
    model: String,
}

#[derive(Debug, Serialize)]
struct EmbeddingRequest {
    input: Vec<String>,
    model: String,
}

#[derive(Debug, Deserialize)]
struct EmbeddingResponse {
    data: Vec<EmbeddingData>,
}

#[derive(Debug, Deserialize)]
struct EmbeddingData {
    embedding: Vec<f32>,
}

impl VoyageEmbeddings {
    pub fn new(api_key: String) -> Self {
        Self {
            client: Client::new(),
            api_key,
            base_url: "https://api.voyageai.com/v1".to_string(),
            model: "voyage-3".to_string(),
        }
    }

    pub async fn generate_embeddings(&self, texts: Vec<String>) -> Result<Vec<Vec<f32>>, String> {
        let request = EmbeddingRequest {
            input: texts,
            model: self.model.clone(),
        };

        let response = self
            .client
            .post(format!("{}/embeddings", self.base_url))
            .header("Authorization", format!("Bearer {}", self.api_key))
            .header("Content-Type", "application/json")
            .json(&request)
            .send()
            .await
            .map_err(|e| format!("Voyage AI API request failed: {}", e))?;

        let status = response.status();
        if !status.is_success() {
            let error_text = response.text().await.unwrap_or_default();
            return Err(format!("Voyage AI API error ({}): {}", status, error_text));
        }

        let embedding_response: EmbeddingResponse = response
            .json()
            .await
            .map_err(|e| format!("Failed to parse Voyage AI response: {}", e))?;

        Ok(embedding_response.data.into_iter().map(|d| d.embedding).collect())
    }

    pub async fn generate_single_embedding(&self, text: String) -> Result<Vec<f32>, String> {
        let embeddings = self.generate_embeddings(vec![text]).await?;
        embeddings.into_iter().next()
            .ok_or_else(|| "No embedding returned".to_string())
    }
}

// Fallback: Simple text-based embeddings (for development without Voyage AI key)
pub fn simple_text_embedding(text: &str, dimensions: usize) -> Vec<f32> {
    use std::collections::hash_map::DefaultHasher;
    use std::hash::{Hash, Hasher};

    let mut hasher = DefaultHasher::new();
    text.hash(&mut hasher);
    let hash = hasher.finish();

    let mut embedding = vec![0.0; dimensions];
    for (i, val) in embedding.iter_mut().enumerate() {
        let seed = hash.wrapping_add(i as u64);
        *val = ((seed % 1000) as f32 - 500.0) / 500.0; // Range: -1.0 to 1.0
    }

    // Normalize
    let magnitude: f32 = embedding.iter().map(|x| x * x).sum::<f32>().sqrt();
    if magnitude > 0.0 {
        for val in embedding.iter_mut() {
            *val /= magnitude;
        }
    }

    embedding
}
