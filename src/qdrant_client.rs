use qdrant_client::qdrant::{
    CreateCollectionBuilder, CreateFieldIndexCollectionBuilder, Distance, PointStruct, 
    SearchPointsBuilder, UpsertPointsBuilder, VectorParamsBuilder, FieldType
};
use qdrant_client::{Qdrant, Payload};
use serde::{Deserialize, Serialize};
use serde_json::json;
use std::collections::HashMap;
use uuid::Uuid;

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
        api_key: Option<String>
    ) -> Result<Self, Box<dyn std::error::Error + Send + Sync>> {
        let mut client_builder = Qdrant::from_url(&url);
        
        if let Some(key) = api_key {
            client_builder = client_builder.api_key(key);
        }
        
        let client = client_builder.build()?;
        
        Ok(Self {
            client,
            collection_name: "chat_memory".to_string(),
        })
    }

    pub async fn create_collection(&self) -> Result<(), Box<dyn std::error::Error + Send + Sync>> {
        tracing::info!("Creating Qdrant collection: {}", self.collection_name);

        // Create collection with 1024 dimensions for Voyage AI embeddings (primary)
        // Note: Gemini embeddings are 768, but Voyage is 1024 and is our primary provider
        let result = self.client
            .create_collection(
                CreateCollectionBuilder::new(&self.collection_name)
                    .vectors_config(VectorParamsBuilder::new(1024, Distance::Cosine))
            )
            .await;

        match result {
            Ok(_) => {
                tracing::info!("Successfully created Qdrant collection: {}", self.collection_name);
                
                // Create payload field indexes for efficient filtering
                self.create_payload_indexes().await?;
                
                Ok(())
            }
            Err(e) => {
                let error_msg = e.to_string();
                if error_msg.contains("already exists") {
                    tracing::debug!("Qdrant collection '{}' already exists, ensuring indexes exist", self.collection_name);
                    
                    // Still try to create indexes in case they're missing
                    self.create_payload_indexes().await?;
                } else {
                    tracing::warn!("Failed to create Qdrant collection '{}': {}", self.collection_name, e);
                }
                Ok(()) // Collection might already exist, which is fine
            }
        }
    }

    async fn create_payload_indexes(&self) -> Result<(), Box<dyn std::error::Error + Send + Sync>> {
        tracing::info!("Creating payload field indexes for efficient filtering...");
        
        // Create index for session_id field (for session-based filtering)
        let session_id_index = self.client
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
                if e.to_string().contains("already exists") || e.to_string().contains("Index already exists") {
                    tracing::debug!("session_id index already exists, skipping");
                } else {
                    tracing::warn!("Failed to create session_id index: {}", e);
                }
            }
        }
        
        // Create index for user_id field (for user-based filtering) 
        let user_id_index = self.client
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
                if e.to_string().contains("already exists") || e.to_string().contains("Index already exists") {
                    tracing::debug!("user_id index already exists, skipping");
                } else {
                    tracing::warn!("Failed to create user_id index: {}", e);
                }
            }
        }
        
        // Create index for timestamp field (for time-based filtering)
        let timestamp_index = self.client
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
                if e.to_string().contains("already exists") || e.to_string().contains("Index already exists") {
                    tracing::debug!("timestamp index already exists, skipping");
                } else {
                    tracing::warn!("Failed to create timestamp index: {}", e);
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
    ) -> Result<String, Box<dyn std::error::Error + Send + Sync>> {
        // Generate embedding using Voyage AI
        let embedding = voyage_client.generate_single_embedding(user_message.to_string()).await?;

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

        // Create payload from document
        let payload: Payload = json!({
            "session_id": document.session_id,
            "user_id": document.user_id,
            "timestamp": document.timestamp.to_rfc3339(),
            "user_message": document.user_message,
            "agent_response": document.agent_response,
            "context": document.context,
            "files_referenced": document.files_referenced
        }).try_into().unwrap();

        // Create point
        let point = PointStruct::new(
            document.id.clone(),
            embedding,
            payload
        );

        // Upsert point to collection
        self.client
            .upsert_points(
                UpsertPointsBuilder::new(&self.collection_name, vec![point])
                    .wait(true),
            )
            .await?;

        tracing::debug!("Stored chat memory with Voyage embeddings, ID: {}", document.id);
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

        // Create payload from document
        let payload: Payload = json!({
            "session_id": document.session_id,
            "user_id": document.user_id,
            "timestamp": document.timestamp.to_rfc3339(),
            "user_message": document.user_message,
            "agent_response": document.agent_response,
            "context": document.context,
            "files_referenced": document.files_referenced
        }).try_into().unwrap();

        // Create point
        let point = PointStruct::new(
            document.id.clone(),
            embedding,
            payload
        );

        // Upsert point to collection
        self.client
            .upsert_points(
                UpsertPointsBuilder::new(&self.collection_name, vec![point])
                    .wait(true),
            )
            .await?;

        tracing::debug!("Stored chat memory with ID: {}", document.id);
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
        let query_embedding = voyage_client.generate_single_embedding(query.to_string()).await?;

        // Search for similar vectors
        let search_result = self.client
            .search_points(
                SearchPointsBuilder::new(&self.collection_name, query_embedding, limit as u64)
                    .filter(qdrant_client::qdrant::Filter {
                        must: vec![qdrant_client::qdrant::Condition {
                            condition_one_of: Some(
                                qdrant_client::qdrant::condition::ConditionOneOf::Field(
                                    qdrant_client::qdrant::FieldCondition {
                                        key: "session_id".to_string(),
                                        r#match: Some(qdrant_client::qdrant::Match {
                                            match_value: Some(
                                                qdrant_client::qdrant::r#match::MatchValue::Keyword(
                                                    session_id.to_string()
                                                )
                                            ),
                                        }),
                                        ..Default::default()
                                    }
                                ),
                            ),
                        }],
                        ..Default::default()
                    })
                    .with_payload(true)
            )
            .await?;

        // Convert search results to documents
        let mut documents = Vec::new();
        for scored_point in search_result.result {
            let payload = scored_point.payload;
            let point_id = match scored_point.id {
                Some(id) => match id.point_id_options {
                    Some(qdrant_client::qdrant::point_id::PointIdOptions::Uuid(uuid)) => uuid,
                    Some(qdrant_client::qdrant::point_id::PointIdOptions::Num(num)) => num.to_string(),
                    None => continue,
                },
                None => continue,
            };

            let doc = ChatMemoryDocument {
                id: point_id,
                session_id: payload.get("session_id")
                    .and_then(|v| v.as_str())
                    .map(|s| s.to_string())
                    .unwrap_or_default(),
                user_id: payload.get("user_id")
                    .and_then(|v| v.as_str())
                    .map(|s| s.to_string()),
                timestamp: payload.get("timestamp")
                    .and_then(|v| v.as_str())
                    .and_then(|s| chrono::DateTime::parse_from_rfc3339(s).ok())
                    .map(|dt| dt.with_timezone(&chrono::Utc))
                    .unwrap_or_else(chrono::Utc::now),
                user_message: payload.get("user_message")
                    .and_then(|v| v.as_str())
                    .map(|s| s.to_string())
                    .unwrap_or_default(),
                agent_response: payload.get("agent_response")
                    .and_then(|v| v.as_str())
                    .map(|s| s.to_string())
                    .unwrap_or_default(),
                context: HashMap::new(),
                files_referenced: payload.get("files_referenced")
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

        // Search for similar vectors
        let search_result = self.client
            .search_points(
                SearchPointsBuilder::new(&self.collection_name, query_embedding, limit as u64)
                    .filter(qdrant_client::qdrant::Filter {
                        must: vec![qdrant_client::qdrant::Condition {
                            condition_one_of: Some(
                                qdrant_client::qdrant::condition::ConditionOneOf::Field(
                                    qdrant_client::qdrant::FieldCondition {
                                        key: "session_id".to_string(),
                                        r#match: Some(qdrant_client::qdrant::Match {
                                            match_value: Some(
                                                qdrant_client::qdrant::r#match::MatchValue::Keyword(
                                                    session_id.to_string()
                                                )
                                            ),
                                        }),
                                        ..Default::default()
                                    }
                                ),
                            ),
                        }],
                        ..Default::default()
                    })
                    .with_payload(true)
            )
            .await?;

        let mut documents = Vec::new();
        for scored_point in search_result.result {
            let payload = scored_point.payload;
            let point_id = match scored_point.id {
                Some(id) => match id.point_id_options {
                    Some(qdrant_client::qdrant::point_id::PointIdOptions::Uuid(uuid)) => uuid,
                    Some(qdrant_client::qdrant::point_id::PointIdOptions::Num(num)) => num.to_string(),
                    None => continue,
                },
                None => continue,
            };
                
            let doc = ChatMemoryDocument {
                id: point_id,
                session_id: payload.get("session_id")
                    .and_then(|v| v.as_str())
                    .map(|s| s.to_string())
                    .unwrap_or_default(),
                user_id: payload.get("user_id")
                    .and_then(|v| v.as_str())
                    .map(|s| s.to_string()),
                timestamp: payload.get("timestamp")
                    .and_then(|v| v.as_str())
                    .and_then(|s| chrono::DateTime::parse_from_rfc3339(s).ok())
                    .map(|dt| dt.with_timezone(&chrono::Utc))
                    .unwrap_or_else(chrono::Utc::now),
                user_message: payload.get("user_message")
                    .and_then(|v| v.as_str())
                    .map(|s| s.to_string())
                    .unwrap_or_default(),
                agent_response: payload.get("agent_response")
                    .and_then(|v| v.as_str())
                    .map(|s| s.to_string())
                    .unwrap_or_default(),
                context: payload.get("context")
                    .and_then(|v| {
                        let json_val: serde_json::Value = serde_json::to_value(v).ok()?;
                        serde_json::from_value(json_val).ok()
                    })
                    .unwrap_or_default(),
                files_referenced: payload.get("files_referenced")
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
        // For getting recent history, we can search with a zero vector since we only care about the filter
        let zero_vector = vec![0.0f32; 768];
        
        let search_result = self.client
            .search_points(
                SearchPointsBuilder::new(&self.collection_name, zero_vector, limit as u64)
                    .filter(qdrant_client::qdrant::Filter {
                        must: vec![qdrant_client::qdrant::Condition {
                            condition_one_of: Some(
                                qdrant_client::qdrant::condition::ConditionOneOf::Field(
                                    qdrant_client::qdrant::FieldCondition {
                                        key: "session_id".to_string(),
                                        r#match: Some(qdrant_client::qdrant::Match {
                                            match_value: Some(
                                                qdrant_client::qdrant::r#match::MatchValue::Keyword(
                                                    session_id.to_string()
                                                )
                                            ),
                                        }),
                                        ..Default::default()
                                    }
                                ),
                            ),
                        }],
                        ..Default::default()
                    })
                    .with_payload(true)
            )
            .await?;

        let mut documents = Vec::new();
        for scored_point in search_result.result {
            let payload = scored_point.payload;
            let point_id = match scored_point.id {
                Some(id) => match id.point_id_options {
                    Some(qdrant_client::qdrant::point_id::PointIdOptions::Uuid(uuid)) => uuid,
                    Some(qdrant_client::qdrant::point_id::PointIdOptions::Num(num)) => num.to_string(),
                    None => continue,
                },
                None => continue,
            };
                
            let doc = ChatMemoryDocument {
                id: point_id,
                session_id: payload.get("session_id")
                    .and_then(|v| v.as_str())
                    .map(|s| s.to_string())
                    .unwrap_or_default(),
                user_id: payload.get("user_id")
                    .and_then(|v| v.as_str())
                    .map(|s| s.to_string()),
                timestamp: payload.get("timestamp")
                    .and_then(|v| v.as_str())
                    .and_then(|s| chrono::DateTime::parse_from_rfc3339(s).ok())
                    .map(|dt| dt.with_timezone(&chrono::Utc))
                    .unwrap_or_else(chrono::Utc::now),
                user_message: payload.get("user_message")
                    .and_then(|v| v.as_str())
                    .map(|s| s.to_string())
                    .unwrap_or_default(),
                agent_response: payload.get("agent_response")
                    .and_then(|v| v.as_str())
                    .map(|s| s.to_string())
                    .unwrap_or_default(),
                context: payload.get("context")
                    .and_then(|v| {
                        let json_val: serde_json::Value = serde_json::to_value(v).ok()?;
                        serde_json::from_value(json_val).ok()
                    })
                    .unwrap_or_default(),
                files_referenced: payload.get("files_referenced")
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
        let similar_conversations = self.search_similar_conversations_with_voyage(query, session_id, 3, voyage_client).await?;

        let mut context = String::new();

        if !recent_history.is_empty() {
            context.push_str("Recent conversation history:\n");
            for memory in recent_history.iter().rev() { // Reverse to show chronologically
                context.push_str(&format!(
                    "User: {}\nAssistant: {}\n\n",
                    memory.user_message,
                    memory.agent_response
                ));
            }
        }

        if !similar_conversations.is_empty() {
            context.push_str("Similar past conversations:\n");
            for memory in &similar_conversations {
                context.push_str(&format!(
                    "User: {}\nAssistant: {}\n\n",
                    memory.user_message,
                    memory.agent_response
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
        let similar_conversations = self.search_similar_conversations_with_gemini(query, session_id, 3, gemini_client).await?;

        let mut context = String::new();

        if !recent_history.is_empty() {
            context.push_str("Recent conversation history:\n");
            for memory in recent_history.iter().rev() { // Reverse to show chronologically
                context.push_str(&format!(
                    "User: {}\nAssistant: {}\n\n",
                    memory.user_message,
                    memory.agent_response
                ));
            }
        }

        if !similar_conversations.is_empty() {
            context.push_str("Similar past conversations:\n");
            for memory in &similar_conversations {
                context.push_str(&format!(
                    "User: {}\nAssistant: {}\n\n",
                    memory.user_message,
                    memory.agent_response
                ));
            }
        }

        Ok(context)
    }

    /// Upsert a single point into the collection
    pub async fn upsert_point(
        &self,
        point_id: &str,
        vector: &[f32],
        payload: &serde_json::Value,
    ) -> Result<(), Box<dyn std::error::Error + Send + Sync>> {
        use qdrant_client::qdrant::{PointStruct, UpsertPointsBuilder};
        
        // Convert JSON payload to Qdrant payload format
        let mut qdrant_payload = std::collections::HashMap::new();
        if let Some(obj) = payload.as_object() {
            for (key, value) in obj {
                qdrant_payload.insert(key.clone(), value.clone().into());
            }
        }
        
        let point = PointStruct::new(
            point_id.to_string(),
            vector.to_vec(),
            qdrant_payload,
        );

        let upsert_request = UpsertPointsBuilder::new(&self.collection_name, vec![point])
            .wait(true);

        self.client.upsert_points(upsert_request).await?;
        Ok(())
    }

    /// Search for similar points in the collection
    pub async fn search_points(
        &self,
        query_vector: &[f32],
        limit: usize,
        filter: Option<&serde_json::Value>,
    ) -> Result<Vec<serde_json::Value>, Box<dyn std::error::Error + Send + Sync>> {
        use qdrant_client::qdrant::{SearchPointsBuilder, Filter, Condition};
        
        let mut search_builder = SearchPointsBuilder::new(&self.collection_name, query_vector.to_vec(), limit as u64)
            .with_payload(true);

        // Apply filter if provided
        if let Some(filter_json) = filter {
            if let Some(must_conditions) = filter_json.get("must") {
                if let Some(conditions) = must_conditions.as_array() {
                    let mut filter_conditions = Vec::new();
                    
                    for condition in conditions {
                        if let (Some(key), Some(match_obj)) = (condition.get("key"), condition.get("match")) {
                            if let (Some(key_str), Some(value)) = (key.as_str(), match_obj.get("value")) {
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
            result_obj.insert("score".to_string(), serde_json::Value::Number(serde_json::Number::from_f64(hit.score as f64).unwrap()));
            
            for (key, value) in hit.payload {
                result_obj.insert(key, serde_json::to_value(value)?);
            }
            
            results.push(serde_json::Value::Object(result_obj));
        }
        
        Ok(results)
    }
}