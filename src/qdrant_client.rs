use qdrant_client::qdrant::{
    Condition, CreateCollectionBuilder, CreateFieldIndexCollectionBuilder, Distance, FieldType,
    Filter, PointStruct, ScrollPointsBuilder, SearchPointsBuilder, UpsertPointsBuilder,
    VectorParamsBuilder, VectorParamsMap, Vectors, VectorsConfig,
};
use qdrant_client::Qdrant;
use serde::{Deserialize, Serialize};
use serde_json::json;
use std::collections::HashMap;
use uuid::Uuid;

/// Embedding provider enum for multi-vector support
/// Qdrant Named Vectors allow different embedding dimensions in one collection
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum EmbeddingProvider {
    /// Voyage AI embeddings (1024 dimensions) - primary for Claude compatibility
    Voyage,
    /// Gemini embeddings (768 dimensions) - fallback provider
    Gemini,
    /// Gemini Embeddings 2 review vectors.
    GeminiEmbedding2,
    /// Qwen3-VL multimodal embeddings (1536 dimensions) - fallback multimodal provider
    QwenVL,
}

impl EmbeddingProvider {
    /// Get the vector name used in Qdrant named vectors
    pub fn vector_name(&self) -> &str {
        match self {
            Self::Voyage => "voyage",
            Self::Gemini => "gemini",
            Self::GeminiEmbedding2 => "gemini_mm",
            Self::QwenVL => "qwen_vl",
        }
    }

    /// Get zero vector for this provider (used in filter-only searches)
    pub fn zero_vector(&self) -> Vec<f32> {
        match self {
            Self::Voyage => vec![0.0; 1024],
            Self::Gemini => vec![0.0; 768],
            Self::GeminiEmbedding2 => vec![0.0; gemini_embedding2_dimensions()],
            Self::QwenVL => vec![0.0; qwen_vl_dimensions()],
        }
    }

    /// Get embedding dimensions for this provider
    pub fn dimensions(&self) -> usize {
        match self {
            Self::Voyage => 1024,
            Self::Gemini => 768,
            Self::GeminiEmbedding2 => gemini_embedding2_dimensions(),
            Self::QwenVL => qwen_vl_dimensions(),
        }
    }

    /// Infer provider from vector dimensions
    pub fn from_dimensions(dims: usize) -> Result<Self, String> {
        match dims {
            1024 => Ok(Self::Voyage),
            768 => Ok(Self::Gemini),
            d if d == gemini_embedding2_dimensions() => Ok(Self::GeminiEmbedding2),
            d if d == qwen_vl_dimensions() => Ok(Self::QwenVL),
            _ => Err(format!(
                "Unknown embedding dimension: {}. Expected 1024 (Voyage), 768 (Gemini), {} (Gemini Embeddings 2), or {} (Qwen-VL)",
                dims,
                gemini_embedding2_dimensions(),
                qwen_vl_dimensions()
            )),
        }
    }
}

fn gemini_embedding2_dimensions() -> usize {
    std::env::var("GEMINI_EMBEDDING2_DIMENSIONS")
        .ok()
        .and_then(|value| value.parse::<usize>().ok())
        .unwrap_or(1536)
}

/// Qwen3-VL embeddings are configured to 1536 dims so they share the same
/// Qdrant named-vector space as Gemini Embeddings 2 (no collection migration).
fn qwen_vl_dimensions() -> usize {
    std::env::var("QWEN_VL_DIMENSIONS")
        .ok()
        .and_then(|value| value.parse::<usize>().ok())
        .unwrap_or(1536)
}

#[derive(Clone)]
pub struct QdrantClient {
    client: Qdrant,
    collection_name: String,
}

#[derive(Debug, Serialize, Deserialize, Clone)]
pub struct ChatMemoryDocument {
    pub id: String,
    pub session_id: String,
    pub user_id: Option<String>,
    pub timestamp: chrono::DateTime<chrono::Utc>,
    pub user_message: String,
    pub agent_response: String,
    pub context: HashMap<String, serde_json::Value>,
    pub files_referenced: Vec<String>,
}

impl QdrantClient {
    pub async fn new(
        url: String,
        api_key: Option<String>,
    ) -> Result<Self, Box<dyn std::error::Error + Send + Sync>> {
        let mut client_builder = Qdrant::from_url(&url);

        if let Some(key) = api_key {
            client_builder = client_builder.api_key(key);
        }

        let client = client_builder.build()?;

        Ok(Self {
            client,
            collection_name: "agent_memory".to_string(),
        })
    }

    pub async fn create_collection(&self) -> Result<(), Box<dyn std::error::Error + Send + Sync>> {
        tracing::info!(
            "Creating Qdrant collection with named vectors: {}",
            self.collection_name
        );

        // Create collection with NAMED VECTORS to support multiple embedding providers
        // This allows both Voyage (1024 dims) and Gemini (768 dims) in the same collection
        let mut named_vectors = HashMap::new();

        // Voyage AI vector (1024 dimensions) - primary for Claude compatibility
        named_vectors.insert(
            "voyage".to_string(),
            VectorParamsBuilder::new(1024, Distance::Cosine).build(),
        );

        // Gemini vector (768 dimensions) - fallback provider
        named_vectors.insert(
            "gemini".to_string(),
            VectorParamsBuilder::new(768, Distance::Cosine).build(),
        );
        named_vectors.insert(
            "gemini_mm".to_string(),
            VectorParamsBuilder::new(gemini_embedding2_dimensions() as u64, Distance::Cosine)
                .build(),
        );
        named_vectors.insert(
            "qwen_vl".to_string(),
            VectorParamsBuilder::new(qwen_vl_dimensions() as u64, Distance::Cosine).build(),
        );

        let result = self
            .client
            .create_collection(
                CreateCollectionBuilder::new(&self.collection_name).vectors_config(VectorsConfig {
                    config: Some(qdrant_client::qdrant::vectors_config::Config::ParamsMap(
                        VectorParamsMap { map: named_vectors },
                    )),
                }),
            )
            .await;

        match result {
            Ok(_) => {
                tracing::info!(
                    "Successfully created Qdrant collection: {}",
                    self.collection_name
                );

                // Create payload field indexes for efficient filtering
                self.create_payload_indexes().await?;

                Ok(())
            }
            Err(e) => {
                let error_msg = e.to_string();
                if error_msg.contains("already exists") {
                    tracing::debug!(
                        "Qdrant collection '{}' already exists, ensuring indexes exist",
                        self.collection_name
                    );

                    // Still try to create indexes in case they're missing
                    self.create_payload_indexes().await?;
                } else {
                    tracing::warn!(
                        "Failed to create Qdrant collection '{}': {}",
                        self.collection_name,
                        e
                    );
                }
                Ok(()) // Collection might already exist, which is fine
            }
        }
    }

    async fn create_payload_indexes(&self) -> Result<(), Box<dyn std::error::Error + Send + Sync>> {
        tracing::info!("Creating payload field indexes for efficient filtering...");

        // Create index for session_id field (for session-based filtering)
        let session_id_index = self
            .client
            .create_field_index(
                CreateFieldIndexCollectionBuilder::new(
                    &self.collection_name,
                    "session_id",
                    FieldType::Keyword,
                )
                .wait(true),
            )
            .await;

        match session_id_index {
            Ok(_) => tracing::info!("✅ Created session_id index successfully"),
            Err(e) => {
                if e.to_string().contains("already exists")
                    || e.to_string().contains("Index already exists")
                {
                    tracing::debug!("session_id index already exists, skipping");
                } else {
                    tracing::warn!("Failed to create session_id index: {}", e);
                }
            }
        }

        // Create index for user_id field (for user-based filtering)
        let user_id_index = self
            .client
            .create_field_index(
                CreateFieldIndexCollectionBuilder::new(
                    &self.collection_name,
                    "user_id",
                    FieldType::Keyword,
                )
                .wait(true),
            )
            .await;

        match user_id_index {
            Ok(_) => tracing::info!("✅ Created user_id index successfully"),
            Err(e) => {
                if e.to_string().contains("already exists")
                    || e.to_string().contains("Index already exists")
                {
                    tracing::debug!("user_id index already exists, skipping");
                } else {
                    tracing::warn!("Failed to create user_id index: {}", e);
                }
            }
        }

        // Create index for timestamp field (for time-based filtering)
        let timestamp_index = self
            .client
            .create_field_index(
                CreateFieldIndexCollectionBuilder::new(
                    &self.collection_name,
                    "timestamp",
                    FieldType::Keyword, // Using Keyword instead of Datetime for compatibility
                )
                .wait(true),
            )
            .await;

        match timestamp_index {
            Ok(_) => tracing::info!("✅ Created timestamp index successfully"),
            Err(e) => {
                if e.to_string().contains("already exists")
                    || e.to_string().contains("Index already exists")
                {
                    tracing::debug!("timestamp index already exists, skipping");
                } else {
                    tracing::warn!("Failed to create timestamp index: {}", e);
                }
            }
        }

        // Create index for file_id field (for video vectorization retrieval)
        let file_id_index = self
            .client
            .create_field_index(
                CreateFieldIndexCollectionBuilder::new(
                    &self.collection_name,
                    "file_id",
                    FieldType::Keyword,
                )
                .wait(true),
            )
            .await;

        match file_id_index {
            Ok(_) => tracing::info!("✅ Created file_id index successfully"),
            Err(e) => {
                if e.to_string().contains("already exists")
                    || e.to_string().contains("Index already exists")
                {
                    tracing::debug!("file_id index already exists, skipping");
                } else {
                    tracing::warn!("Failed to create file_id index: {}", e);
                }
            }
        }

        // Create index for content_type field (for filtering video frames vs summaries)
        let content_type_index = self
            .client
            .create_field_index(
                CreateFieldIndexCollectionBuilder::new(
                    &self.collection_name,
                    "content_type",
                    FieldType::Keyword,
                )
                .wait(true),
            )
            .await;

        match content_type_index {
            Ok(_) => tracing::info!("✅ Created content_type index successfully"),
            Err(e) => {
                if e.to_string().contains("already exists")
                    || e.to_string().contains("Index already exists")
                {
                    tracing::debug!("content_type index already exists, skipping");
                } else {
                    tracing::warn!("Failed to create content_type index: {}", e);
                }
            }
        }

        tracing::info!("🎯 Qdrant payload indexing setup complete - enhanced performance for chat history retrieval");
        Ok(())
    }

    pub async fn store_chat_memory_with_voyage(
        &self,
        session_id: &str,
        user_id: Option<&str>,
        user_message: &str,
        agent_response: &str,
        files_referenced: Vec<String>,
        context: HashMap<String, serde_json::Value>,
        voyage_client: &crate::voyage_embeddings::VoyageEmbeddings,
        feature: Option<&str>,
    ) -> Result<String, Box<dyn std::error::Error + Send + Sync>> {
        // Generate embedding using Voyage AI
        let embedding = voyage_client
            .generate_single_embedding(user_message.to_string())
            .await?;

        let document = ChatMemoryDocument {
            id: Uuid::new_v4().to_string(),
            session_id: session_id.to_string(),
            user_id: user_id.map(|s| s.to_string()),
            timestamp: chrono::Utc::now(),
            user_message: user_message.to_string(),
            agent_response: agent_response.to_string(),
            context,
            files_referenced,
        };

        // Create named vectors for Voyage provider
        let mut named_vectors = HashMap::new();
        named_vectors.insert(
            EmbeddingProvider::Voyage.vector_name().to_string(),
            embedding,
        );

        // Add provider metadata to payload
        let payload_value: serde_json::Value = json!({
            "session_id": document.session_id,
            "user_id": document.user_id,
            "feature": feature.unwrap_or("general"),
            "timestamp": document.timestamp.to_rfc3339(),
            "user_message": document.user_message,
            "agent_response": document.agent_response,
            "context": document.context,
            "files_referenced": document.files_referenced,
            "embedding_provider": "voyage"
        });

        let mut qdrant_payload: std::collections::HashMap<String, qdrant_client::qdrant::Value> =
            std::collections::HashMap::new();
        if let Some(obj) = payload_value.as_object() {
            for (key, value) in obj {
                qdrant_payload.insert(key.clone(), value.clone().into());
            }
        }

        // Create point with named vector
        let point = PointStruct::new(
            document.id.clone(),
            Vectors::from(named_vectors),
            qdrant_payload,
        );

        // Upsert point to collection
        self.client
            .upsert_points(UpsertPointsBuilder::new(&self.collection_name, vec![point]).wait(true))
            .await?;

        tracing::debug!(
            "Stored chat memory with Voyage embeddings, ID: {}",
            document.id
        );
        Ok(document.id)
    }

    pub async fn store_chat_memory_with_gemini(
        &self,
        session_id: &str,
        user_id: Option<&str>,
        user_message: &str,
        agent_response: &str,
        files_referenced: Vec<String>,
        context: HashMap<String, serde_json::Value>,
        gemini_client: &crate::gemini_client::GeminiClient,
        feature: Option<&str>,
    ) -> Result<String, Box<dyn std::error::Error + Send + Sync>> {
        // Generate embedding using Gemini
        let embedding = gemini_client.embed_content(user_message).await?;

        let document = ChatMemoryDocument {
            id: Uuid::new_v4().to_string(),
            session_id: session_id.to_string(),
            user_id: user_id.map(|s| s.to_string()),
            timestamp: chrono::Utc::now(),
            user_message: user_message.to_string(),
            agent_response: agent_response.to_string(),
            context,
            files_referenced,
        };

        // Create named vectors for Gemini provider
        let mut named_vectors = HashMap::new();
        named_vectors.insert(
            EmbeddingProvider::Gemini.vector_name().to_string(),
            embedding,
        );

        // Add provider metadata to payload
        let payload_value: serde_json::Value = json!({
            "session_id": document.session_id,
            "user_id": document.user_id,
            "feature": feature.unwrap_or("general"),
            "timestamp": document.timestamp.to_rfc3339(),
            "user_message": document.user_message,
            "agent_response": document.agent_response,
            "context": document.context,
            "files_referenced": document.files_referenced,
            "embedding_provider": "gemini"
        });

        let mut qdrant_payload: std::collections::HashMap<String, qdrant_client::qdrant::Value> =
            std::collections::HashMap::new();
        if let Some(obj) = payload_value.as_object() {
            for (key, value) in obj {
                qdrant_payload.insert(key.clone(), value.clone().into());
            }
        }

        // Create point with named vector
        let point = PointStruct::new(
            document.id.clone(),
            Vectors::from(named_vectors),
            qdrant_payload,
        );

        // Upsert point to collection
        self.client
            .upsert_points(UpsertPointsBuilder::new(&self.collection_name, vec![point]).wait(true))
            .await?;

        tracing::debug!(
            "Stored chat memory with Gemini embeddings, ID: {}",
            document.id
        );
        Ok(document.id)
    }

    /// Store chat memory using Gemini Embedding 2 (1536d multimodal).
    pub async fn store_chat_memory_with_gemini2(
        &self,
        session_id: &str,
        user_id: Option<&str>,
        user_message: &str,
        agent_response: &str,
        files_referenced: Vec<String>,
        context: HashMap<String, serde_json::Value>,
        gemini_client: &crate::gemini_client::GeminiClient,
        feature: Option<&str>,
    ) -> Result<String, Box<dyn std::error::Error + Send + Sync>> {
        let embedding = gemini_client
            .embed_content_with_model(user_message, "models/gemini-embedding-2", Some(1536))
            .await?;

        let document = ChatMemoryDocument {
            id: Uuid::new_v4().to_string(),
            session_id: session_id.to_string(),
            user_id: user_id.map(|s| s.to_string()),
            timestamp: chrono::Utc::now(),
            user_message: user_message.to_string(),
            agent_response: agent_response.to_string(),
            context,
            files_referenced,
        };

        let mut named_vectors = HashMap::new();
        named_vectors.insert(
            EmbeddingProvider::GeminiEmbedding2.vector_name().to_string(),
            embedding,
        );

        let payload_value: serde_json::Value = json!({
            "session_id": document.session_id,
            "user_id": document.user_id,
            "feature": feature.unwrap_or("general"),
            "timestamp": document.timestamp.to_rfc3339(),
            "user_message": document.user_message,
            "agent_response": document.agent_response,
            "context": document.context,
            "files_referenced": document.files_referenced,
            "embedding_provider": "gemini_embedding2"
        });

        let mut qdrant_payload: std::collections::HashMap<String, qdrant_client::qdrant::Value> =
            std::collections::HashMap::new();
        if let Some(obj) = payload_value.as_object() {
            for (key, value) in obj {
                qdrant_payload.insert(key.clone(), value.clone().into());
            }
        }

        let point = PointStruct::new(
            document.id.clone(),
            Vectors::from(named_vectors),
            qdrant_payload,
        );

        self.client
            .upsert_points(UpsertPointsBuilder::new(&self.collection_name, vec![point]).wait(true))
            .await?;

        tracing::debug!(
            "Stored chat memory with Gemini Embedding 2, ID: {}",
            document.id
        );
        Ok(document.id)
    }

    /// Store chat memory using Qwen3-VL multimodal embeddings (DashScope REST).
    pub async fn store_chat_memory_with_qwen(
        &self,
        session_id: &str,
        user_id: Option<&str>,
        user_message: &str,
        agent_response: &str,
        files_referenced: Vec<String>,
        context: HashMap<String, serde_json::Value>,
        qwen_client: &crate::qwen_client::QwenClient,
        feature: Option<&str>,
    ) -> Result<String, Box<dyn std::error::Error + Send + Sync>> {
        let embedding = qwen_client.embed_text(user_message, Some(1536)).await?;

        let document = ChatMemoryDocument {
            id: Uuid::new_v4().to_string(),
            session_id: session_id.to_string(),
            user_id: user_id.map(|s| s.to_string()),
            timestamp: chrono::Utc::now(),
            user_message: user_message.to_string(),
            agent_response: agent_response.to_string(),
            context,
            files_referenced,
        };

        let mut named_vectors = HashMap::new();
        named_vectors.insert(
            EmbeddingProvider::QwenVL.vector_name().to_string(),
            embedding,
        );

        let payload_value: serde_json::Value = json!({
            "session_id": document.session_id,
            "user_id": document.user_id,
            "feature": feature.unwrap_or("general"),
            "timestamp": document.timestamp.to_rfc3339(),
            "user_message": document.user_message,
            "agent_response": document.agent_response,
            "context": document.context,
            "files_referenced": document.files_referenced,
            "embedding_provider": "qwen_vl"
        });

        let mut qdrant_payload: std::collections::HashMap<String, qdrant_client::qdrant::Value> =
            std::collections::HashMap::new();
        if let Some(obj) = payload_value.as_object() {
            for (key, value) in obj {
                qdrant_payload.insert(key.clone(), value.clone().into());
            }
        }

        let point = PointStruct::new(
            document.id.clone(),
            Vectors::from(named_vectors),
            qdrant_payload,
        );

        self.client
            .upsert_points(UpsertPointsBuilder::new(&self.collection_name, vec![point]).wait(true))
            .await?;

        tracing::debug!(
            "Stored chat memory with Qwen-VL, ID: {}",
            document.id
        );
        Ok(document.id)
    }

    pub async fn search_similar_conversations_with_voyage(
        &self,
        query: &str,
        session_id: &str,
        limit: u32,
        voyage_client: &crate::voyage_embeddings::VoyageEmbeddings,
    ) -> Result<Vec<ChatMemoryDocument>, Box<dyn std::error::Error + Send + Sync>> {
        // Generate query embedding using Voyage AI
        let query_embedding = voyage_client
            .generate_single_embedding(query.to_string())
            .await?;

        // Search for similar vectors using Voyage named vector
        let search_result = self
            .client
            .search_points(
                SearchPointsBuilder::new(&self.collection_name, query_embedding, limit as u64)
                    .vector_name(EmbeddingProvider::Voyage.vector_name())
                    .filter(qdrant_client::qdrant::Filter {
                        must: vec![qdrant_client::qdrant::Condition {
                            condition_one_of: Some(
                                qdrant_client::qdrant::condition::ConditionOneOf::Field(
                                    qdrant_client::qdrant::FieldCondition {
                                        key: "session_id".to_string(),
                                        r#match: Some(qdrant_client::qdrant::Match {
                                            match_value: Some(
                                                qdrant_client::qdrant::r#match::MatchValue::Keyword(
                                                    session_id.to_string(),
                                                ),
                                            ),
                                        }),
                                        ..Default::default()
                                    },
                                ),
                            ),
                        }],
                        ..Default::default()
                    })
                    .with_payload(true),
            )
            .await?;

        // Convert search results to documents
        let mut documents = Vec::new();
        for scored_point in search_result.result {
            let payload = scored_point.payload;
            let point_id = match scored_point.id {
                Some(id) => match id.point_id_options {
                    Some(qdrant_client::qdrant::point_id::PointIdOptions::Uuid(uuid)) => uuid,
                    Some(qdrant_client::qdrant::point_id::PointIdOptions::Num(num)) => {
                        num.to_string()
                    }
                    None => continue,
                },
                None => continue,
            };

            let doc = ChatMemoryDocument {
                id: point_id,
                session_id: payload
                    .get("session_id")
                    .and_then(|v| v.as_str())
                    .map(|s| s.to_string())
                    .unwrap_or_default(),
                user_id: payload
                    .get("user_id")
                    .and_then(|v| v.as_str())
                    .map(|s| s.to_string()),
                timestamp: payload
                    .get("timestamp")
                    .and_then(|v| v.as_str())
                    .and_then(|s| chrono::DateTime::parse_from_rfc3339(s).ok())
                    .map(|dt| dt.with_timezone(&chrono::Utc))
                    .unwrap_or_else(chrono::Utc::now),
                user_message: payload
                    .get("user_message")
                    .and_then(|v| v.as_str())
                    .map(|s| s.to_string())
                    .unwrap_or_default(),
                agent_response: payload
                    .get("agent_response")
                    .and_then(|v| v.as_str())
                    .map(|s| s.to_string())
                    .unwrap_or_default(),
                context: HashMap::new(),
                files_referenced: payload
                    .get("files_referenced")
                    .and_then(|v| {
                        let json_val: serde_json::Value = serde_json::to_value(v).ok()?;
                        serde_json::from_value(json_val).ok()
                    })
                    .unwrap_or_default(),
            };
            documents.push(doc);
        }

        Ok(documents)
    }

    pub async fn search_similar_conversations_with_gemini(
        &self,
        query: &str,
        session_id: &str,
        limit: u32,
        gemini_client: &crate::gemini_client::GeminiClient,
    ) -> Result<Vec<ChatMemoryDocument>, Box<dyn std::error::Error + Send + Sync>> {
        // Generate query embedding
        let query_embedding = gemini_client.embed_content(query).await?;

        // Search for similar vectors using Gemini named vector
        let search_result = self
            .client
            .search_points(
                SearchPointsBuilder::new(&self.collection_name, query_embedding, limit as u64)
                    .vector_name(EmbeddingProvider::Gemini.vector_name())
                    .filter(qdrant_client::qdrant::Filter {
                        must: vec![qdrant_client::qdrant::Condition {
                            condition_one_of: Some(
                                qdrant_client::qdrant::condition::ConditionOneOf::Field(
                                    qdrant_client::qdrant::FieldCondition {
                                        key: "session_id".to_string(),
                                        r#match: Some(qdrant_client::qdrant::Match {
                                            match_value: Some(
                                                qdrant_client::qdrant::r#match::MatchValue::Keyword(
                                                    session_id.to_string(),
                                                ),
                                            ),
                                        }),
                                        ..Default::default()
                                    },
                                ),
                            ),
                        }],
                        ..Default::default()
                    })
                    .with_payload(true),
            )
            .await?;

        let mut documents = Vec::new();
        for scored_point in search_result.result {
            let payload = scored_point.payload;
            let point_id = match scored_point.id {
                Some(id) => match id.point_id_options {
                    Some(qdrant_client::qdrant::point_id::PointIdOptions::Uuid(uuid)) => uuid,
                    Some(qdrant_client::qdrant::point_id::PointIdOptions::Num(num)) => {
                        num.to_string()
                    }
                    None => continue,
                },
                None => continue,
            };

            let doc = ChatMemoryDocument {
                id: point_id,
                session_id: payload
                    .get("session_id")
                    .and_then(|v| v.as_str())
                    .map(|s| s.to_string())
                    .unwrap_or_default(),
                user_id: payload
                    .get("user_id")
                    .and_then(|v| v.as_str())
                    .map(|s| s.to_string()),
                timestamp: payload
                    .get("timestamp")
                    .and_then(|v| v.as_str())
                    .and_then(|s| chrono::DateTime::parse_from_rfc3339(s).ok())
                    .map(|dt| dt.with_timezone(&chrono::Utc))
                    .unwrap_or_else(chrono::Utc::now),
                user_message: payload
                    .get("user_message")
                    .and_then(|v| v.as_str())
                    .map(|s| s.to_string())
                    .unwrap_or_default(),
                agent_response: payload
                    .get("agent_response")
                    .and_then(|v| v.as_str())
                    .map(|s| s.to_string())
                    .unwrap_or_default(),
                context: payload
                    .get("context")
                    .and_then(|v| {
                        let json_val: serde_json::Value = serde_json::to_value(v).ok()?;
                        serde_json::from_value(json_val).ok()
                    })
                    .unwrap_or_default(),
                files_referenced: payload
                    .get("files_referenced")
                    .and_then(|v| {
                        let json_val: serde_json::Value = serde_json::to_value(v).ok()?;
                        serde_json::from_value(json_val).ok()
                    })
                    .unwrap_or_default(),
            };
            documents.push(doc);
        }

        Ok(documents)
    }

    /// Search similar conversations using Gemini Embedding 2 (1536d multimodal).
    pub async fn search_similar_conversations_with_gemini2(
        &self,
        query: &str,
        session_id: &str,
        limit: u32,
        gemini_client: &crate::gemini_client::GeminiClient,
    ) -> Result<Vec<ChatMemoryDocument>, Box<dyn std::error::Error + Send + Sync>> {
        let query_embedding = gemini_client
            .embed_content_with_model(query, "models/gemini-embedding-2", Some(1536))
            .await?;

        let search_result = self
            .client
            .search_points(
                SearchPointsBuilder::new(&self.collection_name, query_embedding, limit as u64)
                    .vector_name(EmbeddingProvider::GeminiEmbedding2.vector_name())
                    .filter(qdrant_client::qdrant::Filter {
                        must: vec![qdrant_client::qdrant::Condition {
                            condition_one_of: Some(
                                qdrant_client::qdrant::condition::ConditionOneOf::Field(
                                    qdrant_client::qdrant::FieldCondition {
                                        key: "session_id".to_string(),
                                        r#match: Some(qdrant_client::qdrant::Match {
                                            match_value: Some(
                                                qdrant_client::qdrant::r#match::MatchValue::Keyword(
                                                    session_id.to_string(),
                                                ),
                                            ),
                                        }),
                                        ..Default::default()
                                    },
                                ),
                            ),
                        }],
                        ..Default::default()
                    })
                    .with_payload(true),
            )
            .await?;

        let mut documents = Vec::new();
        for scored_point in search_result.result {
            let payload = scored_point.payload;
            let point_id = match scored_point.id {
                Some(id) => match id.point_id_options {
                    Some(qdrant_client::qdrant::point_id::PointIdOptions::Uuid(uuid)) => uuid,
                    Some(qdrant_client::qdrant::point_id::PointIdOptions::Num(num)) => {
                        num.to_string()
                    }
                    None => continue,
                },
                None => continue,
            };

            let doc = ChatMemoryDocument {
                id: point_id,
                session_id: payload
                    .get("session_id")
                    .and_then(|v| v.as_str())
                    .map(|s| s.to_string())
                    .unwrap_or_default(),
                user_id: payload
                    .get("user_id")
                    .and_then(|v| v.as_str())
                    .map(|s| s.to_string()),
                timestamp: payload
                    .get("timestamp")
                    .and_then(|v| v.as_str())
                    .and_then(|s| chrono::DateTime::parse_from_rfc3339(s).ok())
                    .map(|dt| dt.with_timezone(&chrono::Utc))
                    .unwrap_or_else(chrono::Utc::now),
                user_message: payload
                    .get("user_message")
                    .and_then(|v| v.as_str())
                    .map(|s| s.to_string())
                    .unwrap_or_default(),
                agent_response: payload
                    .get("agent_response")
                    .and_then(|v| v.as_str())
                    .map(|s| s.to_string())
                    .unwrap_or_default(),
                context: payload
                    .get("context")
                    .and_then(|v| {
                        let json_val: serde_json::Value = serde_json::to_value(v).ok()?;
                        serde_json::from_value(json_val).ok()
                    })
                    .unwrap_or_default(),
                files_referenced: payload
                    .get("files_referenced")
                    .and_then(|v| {
                        let json_val: serde_json::Value = serde_json::to_value(v).ok()?;
                        serde_json::from_value(json_val).ok()
                    })
                    .unwrap_or_default(),
            };
            documents.push(doc);
        }

        Ok(documents)
    }

    /// Search similar conversations using Qwen3-VL multimodal embeddings.
    pub async fn search_similar_conversations_with_qwen(
        &self,
        query: &str,
        session_id: &str,
        limit: u32,
        qwen_client: &crate::qwen_client::QwenClient,
    ) -> Result<Vec<ChatMemoryDocument>, Box<dyn std::error::Error + Send + Sync>> {
        let query_embedding = qwen_client.embed_text(query, Some(1536)).await?;

        let search_result = self
            .client
            .search_points(
                SearchPointsBuilder::new(&self.collection_name, query_embedding, limit as u64)
                    .vector_name(EmbeddingProvider::QwenVL.vector_name())
                    .filter(qdrant_client::qdrant::Filter {
                        must: vec![qdrant_client::qdrant::Condition {
                            condition_one_of: Some(
                                qdrant_client::qdrant::condition::ConditionOneOf::Field(
                                    qdrant_client::qdrant::FieldCondition {
                                        key: "session_id".to_string(),
                                        r#match: Some(qdrant_client::qdrant::Match {
                                            match_value: Some(
                                                qdrant_client::qdrant::r#match::MatchValue::Keyword(
                                                    session_id.to_string(),
                                                ),
                                            ),
                                        }),
                                        ..Default::default()
                                    },
                                ),
                            ),
                        }],
                        ..Default::default()
                    })
                    .with_payload(true),
            )
            .await?;

        let mut documents = Vec::new();
        for scored_point in search_result.result {
            let payload = scored_point.payload;
            let point_id = match scored_point.id {
                Some(id) => match id.point_id_options {
                    Some(qdrant_client::qdrant::point_id::PointIdOptions::Uuid(uuid)) => uuid,
                    Some(qdrant_client::qdrant::point_id::PointIdOptions::Num(num)) => {
                        num.to_string()
                    }
                    None => continue,
                },
                None => continue,
            };

            let doc = ChatMemoryDocument {
                id: point_id,
                session_id: payload
                    .get("session_id")
                    .and_then(|v| v.as_str())
                    .map(|s| s.to_string())
                    .unwrap_or_default(),
                user_id: payload
                    .get("user_id")
                    .and_then(|v| v.as_str())
                    .map(|s| s.to_string()),
                timestamp: payload
                    .get("timestamp")
                    .and_then(|v| v.as_str())
                    .and_then(|s| chrono::DateTime::parse_from_rfc3339(s).ok())
                    .map(|dt| dt.with_timezone(&chrono::Utc))
                    .unwrap_or_else(chrono::Utc::now),
                user_message: payload
                    .get("user_message")
                    .and_then(|v| v.as_str())
                    .map(|s| s.to_string())
                    .unwrap_or_default(),
                agent_response: payload
                    .get("agent_response")
                    .and_then(|v| v.as_str())
                    .map(|s| s.to_string())
                    .unwrap_or_default(),
                context: payload
                    .get("context")
                    .and_then(|v| {
                        let json_val: serde_json::Value = serde_json::to_value(v).ok()?;
                        serde_json::from_value(json_val).ok()
                    })
                    .unwrap_or_default(),
                files_referenced: payload
                    .get("files_referenced")
                    .and_then(|v| {
                        let json_val: serde_json::Value = serde_json::to_value(v).ok()?;
                        serde_json::from_value(json_val).ok()
                    })
                    .unwrap_or_default(),
            };
            documents.push(doc);
        }

        Ok(documents)
    }

    pub async fn get_session_history(
        &self,
        session_id: &str,
        limit: u32,
    ) -> Result<Vec<ChatMemoryDocument>, Box<dyn std::error::Error + Send + Sync>> {
        // Use Qdrant scroll API instead of zero-vector search.
        // Scroll is designed for filter-only retrieval — O(log n) via payload index,
        // not an O(n) scan like searching with a zero vector.
        let scroll_result = self
            .client
            .scroll(
                ScrollPointsBuilder::new(&self.collection_name)
                    .filter(Filter::must([Condition::matches(
                        "session_id",
                        session_id.to_string(),
                    )]))
                    .limit(limit)
                    .with_payload(true),
            )
            .await;

        let scroll_result = match scroll_result {
            Ok(r) => r,
            Err(e) => {
                tracing::warn!("Qdrant scroll failed for session history: {}", e);
                return Ok(Vec::new());
            }
        };

        let mut documents = Vec::new();
        for point in scroll_result.result {
            let payload = point.payload;
            let point_id = match point.id {
                Some(id) => match id.point_id_options {
                    Some(qdrant_client::qdrant::point_id::PointIdOptions::Uuid(uuid)) => uuid,
                    Some(qdrant_client::qdrant::point_id::PointIdOptions::Num(num)) => {
                        num.to_string()
                    }
                    None => continue,
                },
                None => continue,
            };

            let doc = ChatMemoryDocument {
                id: point_id,
                session_id: payload
                    .get("session_id")
                    .and_then(|v| v.as_str())
                    .map(|s| s.to_string())
                    .unwrap_or_default(),
                user_id: payload
                    .get("user_id")
                    .and_then(|v| v.as_str())
                    .map(|s| s.to_string()),
                timestamp: payload
                    .get("timestamp")
                    .and_then(|v| v.as_str())
                    .and_then(|s| chrono::DateTime::parse_from_rfc3339(s).ok())
                    .map(|dt| dt.with_timezone(&chrono::Utc))
                    .unwrap_or_else(chrono::Utc::now),
                user_message: payload
                    .get("user_message")
                    .and_then(|v| v.as_str())
                    .map(|s| s.to_string())
                    .unwrap_or_default(),
                agent_response: payload
                    .get("agent_response")
                    .and_then(|v| v.as_str())
                    .map(|s| s.to_string())
                    .unwrap_or_default(),
                context: payload
                    .get("context")
                    .and_then(|v| {
                        let json_val: serde_json::Value = serde_json::to_value(v).ok()?;
                        serde_json::from_value(json_val).ok()
                    })
                    .unwrap_or_default(),
                files_referenced: payload
                    .get("files_referenced")
                    .and_then(|v| {
                        let json_val: serde_json::Value = serde_json::to_value(v).ok()?;
                        serde_json::from_value(json_val).ok()
                    })
                    .unwrap_or_default(),
            };
            documents.push(doc);
        }

        // Sort by timestamp (newest first)
        documents.sort_by(|a, b| b.timestamp.cmp(&a.timestamp));

        Ok(documents)
    }

    pub async fn build_context_for_query_with_voyage(
        &self,
        query: &str,
        session_id: &str,
        voyage_client: &crate::voyage_embeddings::VoyageEmbeddings,
    ) -> Result<String, Box<dyn std::error::Error + Send + Sync>> {
        // Get recent conversation history
        let recent_history = self.get_session_history(session_id, 5).await?;

        // Get similar past conversations using Voyage embeddings
        let similar_conversations = self
            .search_similar_conversations_with_voyage(query, session_id, 3, voyage_client)
            .await?;

        let mut context = String::new();

        if !recent_history.is_empty() {
            context.push_str("Recent conversation history:\n");
            for memory in recent_history.iter().rev() {
                // Reverse to show chronologically
                context.push_str(&format!(
                    "User: {}\nAssistant: {}\n\n",
                    memory.user_message, memory.agent_response
                ));
            }
        }

        if !similar_conversations.is_empty() {
            context.push_str("Similar past conversations:\n");
            for memory in &similar_conversations {
                context.push_str(&format!(
                    "User: {}\nAssistant: {}\n\n",
                    memory.user_message, memory.agent_response
                ));
            }
        }

        Ok(context)
    }

    pub async fn build_context_for_query_with_gemini(
        &self,
        query: &str,
        session_id: &str,
        gemini_client: &crate::gemini_client::GeminiClient,
    ) -> Result<String, Box<dyn std::error::Error + Send + Sync>> {
        // Get recent conversation history
        let recent_history = self.get_session_history(session_id, 5).await?;

        // Get similar past conversations using Gemini embeddings
        let similar_conversations = self
            .search_similar_conversations_with_gemini(query, session_id, 3, gemini_client)
            .await?;

        let mut context = String::new();

        if !recent_history.is_empty() {
            context.push_str("Recent conversation history:\n");
            for memory in recent_history.iter().rev() {
                // Reverse to show chronologically
                context.push_str(&format!(
                    "User: {}\nAssistant: {}\n\n",
                    memory.user_message, memory.agent_response
                ));
            }
        }

        if !similar_conversations.is_empty() {
            context.push_str("Similar past conversations:\n");
            for memory in &similar_conversations {
                context.push_str(&format!(
                    "User: {}\nAssistant: {}\n\n",
                    memory.user_message, memory.agent_response
                ));
            }
        }

        Ok(context)
    }

    /// Unified RAG context retrieval: Gemini Embedding 2 → Voyage → Gemini text-embedding-004.
    pub async fn build_context_for_query(
        &self,
        query: &str,
        session_id: &str,
        voyage: Option<&crate::voyage_embeddings::VoyageEmbeddings>,
        gemini: Option<&crate::gemini_client::GeminiClient>,
        qwen: Option<&crate::qwen_client::QwenClient>,
    ) -> Result<Option<String>, String> {
        // Tier 1: Gemini Embedding 2 (1536d, multimodal)
        if let Some(g) = gemini {
            match self.build_context_for_query_with_gemini2(query, session_id, g).await {
                Ok(ctx) if !ctx.is_empty() => return Ok(Some(ctx)),
                Ok(_) => {} // empty context, try next tier
                Err(e) => tracing::warn!("Gemini Embedding 2 RAG failed: {}", e),
            }
        }

        // Tier 1b: Qwen3-VL (1536d, multimodal, DashScope)
        if let Some(q) = qwen {
            match self.build_context_for_query_with_qwen(query, session_id, q).await {
                Ok(ctx) if !ctx.is_empty() => return Ok(Some(ctx)),
                Ok(_) => {}
                Err(e) => tracing::warn!("Qwen-VL RAG failed: {}", e),
            }
        }

        // Tier 2: Voyage AI (1024d, text)
        if let Some(v) = voyage {
            match self.build_context_for_query_with_voyage(query, session_id, v).await {
                Ok(ctx) if !ctx.is_empty() => return Ok(Some(ctx)),
                Ok(_) => {}
                Err(e) => tracing::warn!("Voyage RAG failed: {}", e),
            }
        }

        // Tier 3: Gemini text-embedding-004 (768d, text)
        if let Some(g) = gemini {
            match self.build_context_for_query_with_gemini(query, session_id, g).await {
                Ok(ctx) if !ctx.is_empty() => return Ok(Some(ctx)),
                Ok(_) => {}
                Err(e) => tracing::warn!("Gemini text-embedding-004 RAG failed: {}", e),
            }
        }

        Ok(None)
    }

    /// Unified chat memory storage: Gemini Embedding 2 → Qwen-VL → Voyage → Gemini text-embedding-004.
    /// Returns `Ok(())` if any provider succeeded, or `Err` with the last failure.
    pub async fn store_chat_memory(
        &self,
        session_id: &str,
        user_id: Option<&str>,
        user_message: &str,
        agent_response: &str,
        files_referenced: Vec<String>,
        context: HashMap<String, serde_json::Value>,
        voyage: Option<&crate::voyage_embeddings::VoyageEmbeddings>,
        gemini: Option<&crate::gemini_client::GeminiClient>,
        qwen: Option<&crate::qwen_client::QwenClient>,
        feature: Option<&str>,
    ) -> Result<(), String> {
        let mut last_err = String::new();

        // Tier 1: Gemini Embedding 2
        if let Some(g) = gemini {
            match self
                .store_chat_memory_with_gemini2(
                    session_id, user_id, user_message, agent_response,
                    files_referenced.clone(), context.clone(), g, feature,
                )
                .await
            {
                Ok(_) => return Ok(()),
                Err(e) => {
                    last_err = format!("Gemini Embedding 2: {}", e);
                    tracing::warn!("{}", last_err);
                }
            }
        }

        // Tier 1b: Qwen3-VL (DashScope multimodal)
        if let Some(q) = qwen {
            match self
                .store_chat_memory_with_qwen(
                    session_id, user_id, user_message, agent_response,
                    files_referenced.clone(), context.clone(), q, feature,
                )
                .await
            {
                Ok(_) => return Ok(()),
                Err(e) => {
                    last_err = format!("Qwen-VL: {}", e);
                    tracing::warn!("{}", last_err);
                }
            }
        }

        // Tier 2: Voyage AI
        if let Some(v) = voyage {
            match self
                .store_chat_memory_with_voyage(
                    session_id, user_id, user_message, agent_response,
                    files_referenced.clone(), context.clone(), v, feature,
                )
                .await
            {
                Ok(_) => return Ok(()),
                Err(e) => {
                    last_err = format!("Voyage: {}", e);
                    tracing::warn!("{}", last_err);
                }
            }
        }

        // Tier 3: Gemini text-embedding-004
        if let Some(g) = gemini {
            if let Err(e) = self
                .store_chat_memory_with_gemini(
                    session_id, user_id, user_message, agent_response,
                    files_referenced, context, g, feature,
                )
                .await
            {
                last_err = format!("Gemini text-embedding-004: {}", e);
                tracing::warn!("{}", last_err);
            } else {
                return Ok(());
            }
        }

        if last_err.is_empty() {
            Err("No embedding provider available".to_string())
        } else {
            Err(last_err)
        }
    }

    /// Build RAG context using Gemini Embedding 2 (1536d multimodal).
    pub async fn build_context_for_query_with_gemini2(
        &self,
        query: &str,
        session_id: &str,
        gemini_client: &crate::gemini_client::GeminiClient,
    ) -> Result<String, Box<dyn std::error::Error + Send + Sync>> {
        let recent_history = self.get_session_history(session_id, 5).await?;

        let similar_conversations = self
            .search_similar_conversations_with_gemini2(query, session_id, 3, gemini_client)
            .await?;

        let mut context = String::new();

        if !recent_history.is_empty() {
            context.push_str("Recent conversation history:\n");
            for memory in recent_history.iter().rev() {
                context.push_str(&format!(
                    "User: {}\nAssistant: {}\n\n",
                    memory.user_message, memory.agent_response
                ));
            }
        }

        if !similar_conversations.is_empty() {
            context.push_str("Similar past conversations:\n");
            for memory in &similar_conversations {
                context.push_str(&format!(
                    "User: {}\nAssistant: {}\n\n",
                    memory.user_message, memory.agent_response
                ));
            }
        }

        Ok(context)
    }

    /// Build RAG context using Qwen3-VL multimodal embeddings.
    pub async fn build_context_for_query_with_qwen(
        &self,
        query: &str,
        session_id: &str,
        qwen_client: &crate::qwen_client::QwenClient,
    ) -> Result<String, Box<dyn std::error::Error + Send + Sync>> {
        let recent_history = self.get_session_history(session_id, 5).await?;

        let similar_conversations = self
            .search_similar_conversations_with_qwen(query, session_id, 3, qwen_client)
            .await?;

        let mut context = String::new();

        if !recent_history.is_empty() {
            context.push_str("Recent conversation history:\n");
            for memory in recent_history.iter().rev() {
                context.push_str(&format!(
                    "User: {}\nAssistant: {}\n\n",
                    memory.user_message, memory.agent_response
                ));
            }
        }

        if !similar_conversations.is_empty() {
            context.push_str("Similar past conversations:\n");
            for memory in &similar_conversations {
                context.push_str(&format!(
                    "User: {}\nAssistant: {}\n\n",
                    memory.user_message, memory.agent_response
                ));
            }
        }

        Ok(context)
    }

    /// Upsert a single point into the collection with named vector support
    ///
    /// This method now requires specifying the embedding provider to use the correct
    /// named vector in Qdrant (voyage or gemini)
    /// Convert a string video_id to a deterministic numeric point ID using UUID v5.
    /// UUID v5 is stable across runs (unlike DefaultHasher) and has negligible collision probability.
    fn video_id_to_point_id(video_id: &str) -> u64 {
        // Use UUID v5 with a fixed namespace then fold to u64
        let ns = uuid::Uuid::NAMESPACE_URL;
        let v5 = Uuid::new_v5(&ns, video_id.as_bytes());
        let bytes = v5.as_bytes();
        u64::from_be_bytes([
            bytes[0], bytes[1], bytes[2], bytes[3], bytes[4], bytes[5], bytes[6], bytes[7],
        ])
    }

    pub async fn upsert_point(
        &self,
        point_id: &str,
        vector: &[f32],
        payload: &serde_json::Value,
        provider: EmbeddingProvider,
    ) -> Result<(), Box<dyn std::error::Error + Send + Sync>> {
        use qdrant_client::qdrant::{PointStruct, UpsertPointsBuilder};

        // Validate vector dimensions match provider
        if vector.len() != provider.dimensions() {
            return Err(format!(
                "Vector dimension mismatch: got {} but expected {} for provider {:?}",
                vector.len(),
                provider.dimensions(),
                provider
            )
            .into());
        }

        // Convert JSON payload to Qdrant payload format
        let mut qdrant_payload: std::collections::HashMap<String, qdrant_client::qdrant::Value> =
            std::collections::HashMap::new();
        if let Some(obj) = payload.as_object() {
            for (key, value) in obj {
                qdrant_payload.insert(key.clone(), value.clone().into());
            }
        }

        // Add provider metadata to payload for tracking
        qdrant_payload.insert(
            "embedding_provider".to_string(),
            serde_json::Value::String(provider.vector_name().to_string()).into(),
        );

        // Store original string ID in payload for reference
        qdrant_payload.insert(
            "original_point_id".to_string(),
            serde_json::Value::String(point_id.to_string()).into(),
        );

        // Create named vector map
        let mut named_vectors = HashMap::new();
        named_vectors.insert(provider.vector_name().to_string(), vector.to_vec());

        // Use deterministic numeric point ID (UUID v5 folded to u64) to avoid
        // "Unable to parse UUID" errors for non-UUID strings like "video_5Io13pZlK0M"
        let numeric_point_id = Self::video_id_to_point_id(point_id);

        let point = PointStruct::new(
            numeric_point_id,
            Vectors::from(named_vectors),
            qdrant_payload,
        );

        let upsert_request =
            UpsertPointsBuilder::new(&self.collection_name, vec![point]).wait(true);

        self.client.upsert_points(upsert_request).await?;
        tracing::debug!(
            "Upserted point: string_id='{}', numeric_id={}",
            point_id,
            numeric_point_id
        );
        Ok(())
    }

    /// Search for similar points in the collection with named vector support
    ///
    /// This method now requires specifying the embedding provider to search
    /// the correct named vector in Qdrant
    pub async fn search_points(
        &self,
        query_vector: &[f32],
        limit: usize,
        filter: Option<&serde_json::Value>,
        provider: EmbeddingProvider,
    ) -> Result<Vec<serde_json::Value>, Box<dyn std::error::Error + Send + Sync>> {
        use qdrant_client::qdrant::{Condition, Filter, SearchPointsBuilder};

        // Validate vector dimensions match provider
        if query_vector.len() != provider.dimensions() {
            return Err(format!(
                "Query vector dimension mismatch: got {} but expected {} for provider {:?}",
                query_vector.len(),
                provider.dimensions(),
                provider
            )
            .into());
        }

        let mut search_builder =
            SearchPointsBuilder::new(&self.collection_name, query_vector.to_vec(), limit as u64)
                .with_payload(true)
                .vector_name(provider.vector_name());

        // Apply filter if provided
        if let Some(filter_json) = filter {
            if let Some(must_conditions) = filter_json.get("must") {
                if let Some(conditions) = must_conditions.as_array() {
                    let mut filter_conditions = Vec::new();

                    for condition in conditions {
                        if let (Some(key), Some(match_obj)) =
                            (condition.get("key"), condition.get("match"))
                        {
                            if let (Some(key_str), Some(value)) =
                                (key.as_str(), match_obj.get("value"))
                            {
                                let condition = if let Some(str_value) = value.as_str() {
                                    Condition::matches(key_str, str_value.to_string())
                                } else if let Some(int_value) = value.as_i64() {
                                    Condition::matches(key_str, int_value)
                                } else if let Some(bool_value) = value.as_bool() {
                                    Condition::matches(key_str, bool_value)
                                } else {
                                    continue;
                                };
                                filter_conditions.push(condition);
                            }
                        }
                    }

                    if !filter_conditions.is_empty() {
                        let filter = Filter::must(filter_conditions);
                        search_builder = search_builder.filter(filter);
                    }
                }
            }
        }

        let search_result = self.client.search_points(search_builder).await?;

        // Convert results to JSON format
        let mut results = Vec::new();
        for hit in search_result.result {
            let mut result_obj = serde_json::Map::new();
            let point_id = match hit.id.unwrap().point_id_options.unwrap() {
                qdrant_client::qdrant::point_id::PointIdOptions::Uuid(uuid) => uuid,
                qdrant_client::qdrant::point_id::PointIdOptions::Num(num) => num.to_string(),
            };
            result_obj.insert("id".to_string(), serde_json::Value::String(point_id));
            result_obj.insert(
                "score".to_string(),
                serde_json::Value::Number(serde_json::Number::from_f64(hit.score as f64).unwrap()),
            );

            for (key, value) in hit.payload {
                result_obj.insert(key, serde_json::to_value(value)?);
            }

            results.push(serde_json::Value::Object(result_obj));
        }

        Ok(results)
    }

    // =========================================================================
    // New collection methods: video_content + extracted_clips
    // =========================================================================

    /// Create (or verify) the `video_content` collection.
    /// One point per analyzed video — deduplication, content discovery, editing context.
    pub async fn ensure_video_content_collection(
        &self,
    ) -> Result<(), Box<dyn std::error::Error + Send + Sync>> {
        let collection_name = "video_content";
        let mut named_vectors = HashMap::new();
        named_vectors.insert(
            "voyage".to_string(),
            VectorParamsBuilder::new(1024, Distance::Cosine).build(),
        );
        named_vectors.insert(
            "gemini".to_string(),
            VectorParamsBuilder::new(768, Distance::Cosine).build(),
        );
        named_vectors.insert(
            "gemini_mm".to_string(),
            VectorParamsBuilder::new(gemini_embedding2_dimensions() as u64, Distance::Cosine)
                .build(),
        );

        let result = self
            .client
            .create_collection(
                CreateCollectionBuilder::new(collection_name).vectors_config(VectorsConfig {
                    config: Some(qdrant_client::qdrant::vectors_config::Config::ParamsMap(
                        VectorParamsMap { map: named_vectors },
                    )),
                }),
            )
            .await;

        match result {
            Ok(_) => tracing::info!("✅ Created video_content collection"),
            Err(e) if e.to_string().contains("already exists") => {
                tracing::debug!("video_content collection already exists")
            }
            Err(e) => tracing::warn!("Failed to create video_content collection: {}", e),
        }

        // Payload indexes
        for (field, field_type) in &[
            ("video_id", FieldType::Keyword),
            ("source_type", FieldType::Keyword),
            ("user_id", FieldType::Keyword),
            ("channel_id", FieldType::Keyword),
            ("content_category", FieldType::Keyword),
        ] {
            let _ = self
                .client
                .create_field_index(
                    CreateFieldIndexCollectionBuilder::new(collection_name, *field, *field_type)
                        .wait(true),
                )
                .await;
        }

        Ok(())
    }

    /// Create (or verify) the `extracted_clips` collection.
    /// One point per generated YouTube Short — clip search, quality analytics, upload tracking.
    pub async fn ensure_extracted_clips_collection(
        &self,
    ) -> Result<(), Box<dyn std::error::Error + Send + Sync>> {
        let collection_name = "extracted_clips";
        let mut named_vectors = HashMap::new();
        named_vectors.insert(
            "voyage".to_string(),
            VectorParamsBuilder::new(1024, Distance::Cosine).build(),
        );

        let result = self
            .client
            .create_collection(
                CreateCollectionBuilder::new(collection_name).vectors_config(VectorsConfig {
                    config: Some(qdrant_client::qdrant::vectors_config::Config::ParamsMap(
                        VectorParamsMap { map: named_vectors },
                    )),
                }),
            )
            .await;

        match result {
            Ok(_) => tracing::info!("✅ Created extracted_clips collection"),
            Err(e) if e.to_string().contains("already exists") => {
                tracing::debug!("extracted_clips collection already exists")
            }
            Err(e) => tracing::warn!("Failed to create extracted_clips collection: {}", e),
        }

        // Payload indexes
        for (field, field_type) in &[
            ("clipping_job_id", FieldType::Integer),
            ("source_video_id", FieldType::Keyword),
            ("destination_channel_id", FieldType::Keyword),
            ("upload_status", FieldType::Keyword),
        ] {
            let _ = self
                .client
                .create_field_index(
                    CreateFieldIndexCollectionBuilder::new(collection_name, *field, *field_type)
                        .wait(true),
                )
                .await;
        }

        Ok(())
    }

    /// Create (or verify) the `agent_memory` collection (replaces `chat_memory`).
    /// All AI conversation context — video editing, generation, clipping, general chat.
    pub async fn ensure_agent_memory_collection(
        &self,
    ) -> Result<(), Box<dyn std::error::Error + Send + Sync>> {
        let collection_name = "agent_memory";
        let mut named_vectors = HashMap::new();
        named_vectors.insert(
            "voyage".to_string(),
            VectorParamsBuilder::new(1024, Distance::Cosine).build(),
        );
        named_vectors.insert(
            "gemini".to_string(),
            VectorParamsBuilder::new(768, Distance::Cosine).build(),
        );
        named_vectors.insert(
            "gemini_mm".to_string(),
            VectorParamsBuilder::new(gemini_embedding2_dimensions() as u64, Distance::Cosine)
                .build(),
        );

        let result = self
            .client
            .create_collection(
                CreateCollectionBuilder::new(collection_name).vectors_config(VectorsConfig {
                    config: Some(qdrant_client::qdrant::vectors_config::Config::ParamsMap(
                        VectorParamsMap { map: named_vectors },
                    )),
                }),
            )
            .await;

        match result {
            Ok(_) => tracing::info!("✅ Created agent_memory collection"),
            Err(e) if e.to_string().contains("already exists") => {
                tracing::debug!("agent_memory collection already exists")
            }
            Err(e) => tracing::warn!("Failed to create agent_memory collection: {}", e),
        }

        // Payload indexes
        for (field, field_type) in &[
            ("session_id", FieldType::Keyword),
            ("user_id", FieldType::Keyword),
            ("feature", FieldType::Keyword),
            ("timestamp", FieldType::Keyword),
        ] {
            let _ = self
                .client
                .create_field_index(
                    CreateFieldIndexCollectionBuilder::new(collection_name, *field, *field_type)
                        .wait(true),
                )
                .await;
        }

        Ok(())
    }

    /// Create or verify the media review collection for multimodal QA artifacts.
    pub async fn ensure_media_review_collection(
        &self,
    ) -> Result<(), Box<dyn std::error::Error + Send + Sync>> {
        let collection_name = "media_review";
        let mut named_vectors = HashMap::new();
        named_vectors.insert(
            "gemini_mm".to_string(),
            VectorParamsBuilder::new(gemini_embedding2_dimensions() as u64, Distance::Cosine)
                .build(),
        );

        let result = self
            .client
            .create_collection(
                CreateCollectionBuilder::new(collection_name).vectors_config(VectorsConfig {
                    config: Some(qdrant_client::qdrant::vectors_config::Config::ParamsMap(
                        VectorParamsMap { map: named_vectors },
                    )),
                }),
            )
            .await;

        match result {
            Ok(_) => tracing::info!("✅ Created media_review collection"),
            Err(e) if e.to_string().contains("already exists") => {
                tracing::debug!("media_review collection already exists")
            }
            Err(e) => tracing::warn!("Failed to create media_review collection: {}", e),
        }

        for (field, field_type) in &[
            ("asset_kind", FieldType::Keyword),
            ("service_slug", FieldType::Keyword),
            ("source_type", FieldType::Keyword),
            ("owner_user_id", FieldType::Integer),
            ("review_status", FieldType::Keyword),
        ] {
            let _ = self
                .client
                .create_field_index(
                    CreateFieldIndexCollectionBuilder::new(collection_name, *field, *field_type)
                        .wait(true),
                )
                .await;
        }

        Ok(())
    }

    /// Check if a video has already been analyzed (deduplication check).
    /// Returns the stored VideoContentEntry payload if found, None otherwise.
    pub async fn video_already_analyzed(
        &self,
        video_id: &str,
    ) -> Result<Option<serde_json::Value>, Box<dyn std::error::Error + Send + Sync>> {
        let scroll_result = self
            .client
            .scroll(
                ScrollPointsBuilder::new("video_content")
                    .filter(Filter::must([Condition::matches(
                        "video_id",
                        video_id.to_string(),
                    )]))
                    .limit(1)
                    .with_payload(true),
            )
            .await?;

        if let Some(point) = scroll_result.result.into_iter().next() {
            let payload_json: serde_json::Value =
                serde_json::to_value(&point.payload).unwrap_or(serde_json::Value::Null);
            Ok(Some(payload_json))
        } else {
            Ok(None)
        }
    }

    /// Store a video content entry in the `video_content` collection.
    /// One point per video. Uses UUID v5 of video_id for deterministic deduplication.
    pub async fn store_video_content(
        &self,
        video_id: &str,
        payload: serde_json::Value,
        embedding: Vec<f32>,
        provider: EmbeddingProvider,
    ) -> Result<(), Box<dyn std::error::Error + Send + Sync>> {
        let mut qdrant_payload: std::collections::HashMap<String, qdrant_client::qdrant::Value> =
            std::collections::HashMap::new();
        if let Some(obj) = payload.as_object() {
            for (key, value) in obj {
                qdrant_payload.insert(key.clone(), value.clone().into());
            }
        }

        let mut named_vectors = HashMap::new();
        named_vectors.insert(provider.vector_name().to_string(), embedding);

        let point_id = Self::video_id_to_point_id(video_id);
        let point = PointStruct::new(point_id, Vectors::from(named_vectors), qdrant_payload);

        self.client
            .upsert_points(UpsertPointsBuilder::new("video_content", vec![point]).wait(true))
            .await?;

        tracing::info!("✅ Stored video content in Qdrant: video_id={}", video_id);
        Ok(())
    }

    /// Store an extracted clip entry in the `extracted_clips` collection.
    /// Point ID = the PostgreSQL `extracted_clips.id` (integer, no hashing needed).
    pub async fn store_extracted_clip(
        &self,
        clip_db_id: i32,
        payload: serde_json::Value,
        embedding: Vec<f32>,
    ) -> Result<(), Box<dyn std::error::Error + Send + Sync>> {
        let mut qdrant_payload: std::collections::HashMap<String, qdrant_client::qdrant::Value> =
            std::collections::HashMap::new();
        if let Some(obj) = payload.as_object() {
            for (key, value) in obj {
                qdrant_payload.insert(key.clone(), value.clone().into());
            }
        }

        let mut named_vectors = HashMap::new();
        named_vectors.insert("voyage".to_string(), embedding);

        let point = PointStruct::new(
            clip_db_id as u64,
            Vectors::from(named_vectors),
            qdrant_payload,
        );

        self.client
            .upsert_points(UpsertPointsBuilder::new("extracted_clips", vec![point]).wait(true))
            .await?;

        tracing::debug!("Stored extracted clip in Qdrant: clip_id={}", clip_db_id);
        Ok(())
    }

    pub async fn store_media_review(
        &self,
        review_id: &str,
        payload: serde_json::Value,
        embedding: Vec<f32>,
    ) -> Result<(), Box<dyn std::error::Error + Send + Sync>> {
        let mut qdrant_payload: std::collections::HashMap<String, qdrant_client::qdrant::Value> =
            std::collections::HashMap::new();
        if let Some(obj) = payload.as_object() {
            for (key, value) in obj {
                qdrant_payload.insert(key.clone(), value.clone().into());
            }
        }

        let mut named_vectors = HashMap::new();
        named_vectors.insert(
            EmbeddingProvider::GeminiEmbedding2.vector_name().to_string(),
            embedding,
        );

        let point_id = Self::video_id_to_point_id(review_id);
        let point = PointStruct::new(point_id, Vectors::from(named_vectors), qdrant_payload);

        self.client
            .upsert_points(UpsertPointsBuilder::new("media_review", vec![point]).wait(true))
            .await?;

        tracing::info!("✅ Stored media review in Qdrant: review_id={}", review_id);
        Ok(())
    }

    /// Get all clips for a clipping job from the `extracted_clips` collection.
    pub async fn get_clips_for_job(
        &self,
        job_id: i32,
    ) -> Result<Vec<serde_json::Value>, Box<dyn std::error::Error + Send + Sync>> {
        let scroll_result = self
            .client
            .scroll(
                ScrollPointsBuilder::new("extracted_clips")
                    .filter(Filter::must([Condition::matches(
                        "clipping_job_id",
                        job_id as i64,
                    )]))
                    .limit(50)
                    .with_payload(true),
            )
            .await?;

        let results = scroll_result
            .result
            .into_iter()
            .map(|point| serde_json::to_value(&point.payload).unwrap_or(serde_json::Value::Null))
            .collect();

        Ok(results)
    }
}
