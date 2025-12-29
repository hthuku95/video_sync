use reqwest::Client;
use serde::{Deserialize, Serialize};
use std::collections::HashMap;
use uuid::Uuid;

#[derive(Debug, Clone)]
pub struct AstraDBClient {
    client: Client,
    api_endpoint: String,
    application_token: String,
    keyspace: String,
    collection: String,
}

#[derive(Debug, Serialize, Deserialize)]
pub struct ChatMemoryDocument {
    #[serde(rename = "_id")]
    pub id: String,
    pub session_id: String,
    pub user_id: Option<String>,
    pub timestamp: chrono::DateTime<chrono::Utc>,
    pub user_message: String,
    pub agent_response: String,
    pub context: HashMap<String, serde_json::Value>,
    pub files_referenced: Vec<String>,
    #[serde(rename = "$vector")]
    pub vector: Vec<f32>,
}

#[derive(Debug, Serialize, Deserialize)]
pub struct VectorSearchQuery {
    #[serde(rename = "$vector")]
    pub vector: Vec<f32>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub limit: Option<u32>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub filter: Option<HashMap<String, serde_json::Value>>,
}

#[derive(Debug, Serialize, Deserialize)]
pub struct AstraDBResponse<T> {
    pub data: Vec<T>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub errors: Option<Vec<AstraDBError>>,
}

#[derive(Debug, Serialize, Deserialize)]
pub struct AstraDBError {
    pub message: String,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub error_code: Option<String>,
}

impl AstraDBClient {
    pub fn new(api_endpoint: String, application_token: String, keyspace: String) -> Self {
        Self {
            client: Client::new(),
            api_endpoint,
            application_token,
            keyspace,
            collection: "chat_memory".to_string(),
        }
    }

    pub async fn store_chat_memory(
        &self,
        session_id: &str,
        user_id: Option<&str>,
        user_message: &str,
        agent_response: &str,
        files_referenced: Vec<String>,
        context: HashMap<String, serde_json::Value>,
    ) -> Result<String, Box<dyn std::error::Error + Send + Sync>> {
        let vector = self.generate_embedding(user_message).await?;
        
        let document = ChatMemoryDocument {
            id: Uuid::new_v4().to_string(),
            session_id: session_id.to_string(),
            user_id: user_id.map(|s| s.to_string()),
            timestamp: chrono::Utc::now(),
            user_message: user_message.to_string(),
            agent_response: agent_response.to_string(),
            context,
            files_referenced,
            vector,
        };

        let url = format!(
            "{}/v1/{}",
            self.api_endpoint,
            self.collection
        );

        let response = self
            .client
            .post(&url)
            .header("X-Cassandra-Token", &self.application_token)
            .header("Content-Type", "application/json")
            .json(&document)
            .send()
            .await?;

        if response.status().is_success() {
            Ok(document.id)
        } else {
            let error_text = response.text().await?;
            Err(format!("Failed to store chat memory: {}", error_text).into())
        }
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
        let vector = self.generate_embedding_with_client(user_message, gemini_client).await?;
        
        let document = ChatMemoryDocument {
            id: Uuid::new_v4().to_string(),
            session_id: session_id.to_string(),
            user_id: user_id.map(|s| s.to_string()),
            timestamp: chrono::Utc::now(),
            user_message: user_message.to_string(),
            agent_response: agent_response.to_string(),
            context,
            files_referenced,
            vector,
        };

        let url = format!(
            "{}/v1/{}",
            self.api_endpoint,
            self.collection
        );

        let response = self
            .client
            .post(&url)
            .header("X-Cassandra-Token", &self.application_token)
            .header("Content-Type", "application/json")
            .json(&document)
            .send()
            .await?;

        if response.status().is_success() {
            Ok(document.id)
        } else {
            let error_text = response.text().await?;
            Err(format!("Failed to store chat memory: {}", error_text).into())
        }
    }

    pub async fn search_similar_conversations(
        &self,
        query: &str,
        session_id: &str,
        limit: u32,
    ) -> Result<Vec<ChatMemoryDocument>, Box<dyn std::error::Error + Send + Sync>> {
        let query_vector = self.generate_embedding(query).await?;
        
        let mut filter = HashMap::new();
        filter.insert("session_id".to_string(), serde_json::Value::String(session_id.to_string()));

        let search_query = VectorSearchQuery {
            vector: query_vector,
            limit: Some(limit),
            filter: Some(filter),
        };

        let url = format!(
            "{}/v1/{}/vector-search",
            self.api_endpoint,
            self.collection
        );

        let response = self
            .client
            .post(&url)
            .header("X-Cassandra-Token", &self.application_token)
            .header("Content-Type", "application/json")
            .json(&search_query)
            .send()
            .await?;

        if response.status().is_success() {
            let astra_response: AstraDBResponse<ChatMemoryDocument> = response.json().await?;
            Ok(astra_response.data)
        } else {
            let error_text = response.text().await?;
            Err(format!("Failed to search chat memory: {}", error_text).into())
        }
    }

    pub async fn search_similar_conversations_with_gemini(
        &self,
        query: &str,
        session_id: &str,
        limit: u32,
        gemini_client: &crate::gemini_client::GeminiClient,
    ) -> Result<Vec<ChatMemoryDocument>, Box<dyn std::error::Error + Send + Sync>> {
        let query_vector = self.generate_embedding_with_client(query, gemini_client).await?;
        
        let mut filter = HashMap::new();
        filter.insert("session_id".to_string(), serde_json::Value::String(session_id.to_string()));

        let search_query = VectorSearchQuery {
            vector: query_vector,
            limit: Some(limit),
            filter: Some(filter),
        };

        let url = format!(
            "{}/v1/{}/vector-search",
            self.api_endpoint,
            self.collection
        );

        let response = self
            .client
            .post(&url)
            .header("X-Cassandra-Token", &self.application_token)
            .header("Content-Type", "application/json")
            .json(&search_query)
            .send()
            .await?;

        if response.status().is_success() {
            let astra_response: AstraDBResponse<ChatMemoryDocument> = response.json().await?;
            Ok(astra_response.data)
        } else {
            let error_text = response.text().await?;
            Err(format!("Failed to search chat memory: {}", error_text).into())
        }
    }

    pub async fn get_session_history(
        &self,
        session_id: &str,
        limit: u32,
    ) -> Result<Vec<ChatMemoryDocument>, Box<dyn std::error::Error + Send + Sync>> {
        let mut filter = HashMap::new();
        filter.insert("session_id".to_string(), serde_json::Value::String(session_id.to_string()));

        let url = format!(
            "{}/v1/{}",
            self.api_endpoint,
            self.collection
        );

        let query = serde_json::json!({
            "filter": filter,
            "sort": { "timestamp": -1 },
            "limit": limit
        });

        let response = self
            .client
            .post(&format!("{}/find", url))
            .header("X-Cassandra-Token", &self.application_token)
            .header("Content-Type", "application/json")
            .json(&query)
            .send()
            .await?;

        if response.status().is_success() {
            let astra_response: AstraDBResponse<ChatMemoryDocument> = response.json().await?;
            Ok(astra_response.data)
        } else {
            let error_text = response.text().await?;
            Err(format!("Failed to get session history: {}", error_text).into())
        }
    }

    pub async fn create_collection(&self) -> Result<(), Box<dyn std::error::Error + Send + Sync>> {
        let url = format!("{}/v1/{}", self.api_endpoint, self.collection);
        
        let collection_config = serde_json::json!({
            "name": self.collection,
            "options": {
                "vector": {
                    "dimension": 768,
                    "metric": "cosine",
                }
            }
        });

        let response = self
            .client
            .put(&url)
            .header("X-Cassandra-Token", &self.application_token)
            .header("Content-Type", "application/json")
            .json(&collection_config)
            .send()
            .await?;

        if response.status().is_success() {
            tracing::info!("Successfully created Astra DB collection: {}", self.collection);
            Ok(())
        } else {
            let error_text = response.text().await?;
            tracing::warn!("Failed to create collection (may already exist): {}", error_text);
            Ok(()) // Collection might already exist, which is fine
        }
    }

    // Generate real embeddings using external Gemini client
    async fn generate_embedding_with_client(
        &self, 
        text: &str, 
        gemini_client: &crate::gemini_client::GeminiClient
    ) -> Result<Vec<f32>, Box<dyn std::error::Error + Send + Sync>> {
        gemini_client.embed_content(text).await
    }

    // Fallback method using text hashing for when Gemini client is not available
    async fn generate_embedding(&self, text: &str) -> Result<Vec<f32>, Box<dyn std::error::Error + Send + Sync>> {
        use sha2::{Digest, Sha256};
        
        // Create a simple deterministic embedding from text hash
        let mut hasher = Sha256::new();
        hasher.update(text.as_bytes());
        let hash = hasher.finalize();
        
        // Convert hash to 768-dimensional vector (to match Gemini embedding dimensions)
        let mut vector = Vec::with_capacity(768);
        for i in 0..768 {
            let byte_index = i % hash.len();
            let value = (hash[byte_index] as f32 - 128.0) / 128.0; // Normalize to [-1, 1]
            vector.push(value);
        }
        
        Ok(vector)
    }

    pub async fn build_context_for_query(
        &self,
        query: &str,
        session_id: &str,
    ) -> Result<String, Box<dyn std::error::Error + Send + Sync>> {
        // Get recent conversation history
        let recent_history = self.get_session_history(session_id, 5).await?;
        
        // Get similar past conversations
        let similar_conversations = self.search_similar_conversations(query, session_id, 3).await?;
        
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
}