use reqwest::Client;
use serde::{Deserialize, Serialize};

#[derive(Debug, Clone)]
pub struct VoyageEmbeddings {
    client: Client,
    api_key: String,
    base_url: String,
    model: String,
}

/// Input type for Voyage AI embeddings
/// See: https://docs.voyageai.com/docs/embeddings
#[derive(Debug, Clone, Copy)]
pub enum InputType {
    /// Use for documents being stored in vector database
    /// Prepends: "Represent the document for retrieval: "
    Document,
    /// Use for search queries
    /// Prepends: "Represent the query for retrieving supporting documents: "
    Query,
}

impl InputType {
    fn as_str(&self) -> &str {
        match self {
            InputType::Document => "document",
            InputType::Query => "query",
        }
    }
}

#[derive(Debug, Serialize)]
struct EmbeddingRequest {
    input: Vec<String>,
    model: String,
    #[serde(skip_serializing_if = "Option::is_none")]
    input_type: Option<String>,
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
            // Using voyage-3.5 for balanced performance and cost
            // See: https://docs.voyageai.com/docs/embeddings
            model: "voyage-3.5".to_string(),
        }
    }

    /// Generate embeddings with optional input type specification
    ///
    /// # Arguments
    /// * `texts` - Text content to embed
    /// * `input_type` - Optional: Specify "document" for stored content or "query" for searches
    ///                  This significantly improves retrieval quality
    pub async fn generate_embeddings_with_type(
        &self,
        texts: Vec<String>,
        input_type: Option<InputType>
    ) -> Result<Vec<Vec<f32>>, String> {
        let request = EmbeddingRequest {
            input: texts,
            model: self.model.clone(),
            input_type: input_type.map(|t| t.as_str().to_string()),
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

    /// Generate embeddings (backward compatible - defaults to document type)
    pub async fn generate_embeddings(&self, texts: Vec<String>) -> Result<Vec<Vec<f32>>, String> {
        self.generate_embeddings_with_type(texts, Some(InputType::Document)).await
    }

    /// Generate single embedding (backward compatible - defaults to document type)
    pub async fn generate_single_embedding(&self, text: String) -> Result<Vec<f32>, String> {
        let embeddings = self.generate_embeddings(vec![text]).await?;
        embeddings.into_iter().next()
            .ok_or_else(|| "No embedding returned".to_string())
    }

    /// Generate single embedding for a document (explicit type)
    pub async fn embed_document(&self, text: String) -> Result<Vec<f32>, String> {
        let embeddings = self.generate_embeddings_with_type(vec![text], Some(InputType::Document)).await?;
        embeddings.into_iter().next()
            .ok_or_else(|| "No embedding returned".to_string())
    }

    /// Generate single embedding for a query (explicit type)
    pub async fn embed_query(&self, text: String) -> Result<Vec<f32>, String> {
        let embeddings = self.generate_embeddings_with_type(vec![text], Some(InputType::Query)).await?;
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
