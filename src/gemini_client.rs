use base64::prelude::*;
use rand::Rng;
use reqwest::Client;
use serde::{Deserialize, Serialize};
use serde_json::Value;
use std::collections::HashMap;
use std::sync::Arc;

/// Maximum number of concurrent Gemini API calls across the entire process.
/// At 600 jobs/day target (25/hr), 5 permits allows Phase A to process ~5 jobs simultaneously
/// without exceeding paid-tier RPM limits. Raise to 8 if on a higher quota tier.
const GEMINI_MAX_CONCURRENT: usize = 5;

#[derive(Debug, Clone)]
pub struct GeminiClient {
    client: Client,
    api_key: String,
    base_url: String,
    /// Model used for generate_text() and generate_content() calls.
    /// Default: "gemini-2.5-flash". Override with new_with_model() for Gemma 4.
    pub text_model: String,
    /// Semaphore that limits concurrent in-flight Gemini API calls.
    semaphore: Arc<tokio::sync::Semaphore>,
}

/// Parse the retry delay from a Gemini 429 error body.
///
/// The API embeds the hint in `error.details[].retryDelay` (e.g. "52s")
/// per the `google.rpc.RetryInfo` proto. Falls back to scanning the
/// human-readable message for "Please retry in Xs", then to `default_secs`.
fn parse_gemini_retry_delay(error_body: &str, default_secs: f64) -> f64 {
    if let Ok(v) = serde_json::from_str::<serde_json::Value>(error_body) {
        // Primary: error.details[].retryDelay (e.g. "52s" or "52.5s")
        if let Some(details) = v["error"]["details"].as_array() {
            for detail in details {
                if let Some(rd) = detail["retryDelay"].as_str() {
                    let secs_str = rd.trim_end_matches('s');
                    if let Ok(secs) = secs_str.parse::<f64>() {
                        return secs;
                    }
                }
            }
        }
        // Fallback: "Please retry in 25.755477156s." in error.message
        if let Some(msg) = v["error"]["message"].as_str() {
            let marker = "Please retry in ";
            if let Some(start) = msg.find(marker) {
                let rest = &msg[start + marker.len()..];
                if let Some(end) = rest.find('s') {
                    if let Ok(secs) = rest[..end].parse::<f64>() {
                        return secs;
                    }
                }
            }
        }
    }
    default_secs
}

#[derive(Debug, Serialize, Deserialize)]
pub struct GenerateContentRequest {
    pub contents: Vec<Content>,
    pub tools: Option<Vec<Tool>>,
    #[serde(rename = "generationConfig")]
    pub generation_config: Option<GenerationConfig>,
    #[serde(rename = "toolConfig", skip_serializing_if = "Option::is_none")]
    pub tool_config: Option<ToolConfig>,
    #[serde(rename = "systemInstruction", skip_serializing_if = "Option::is_none")]
    pub system_instruction: Option<Content>,
}

#[derive(Debug, Serialize, Deserialize, Clone)]
pub struct Content {
    #[serde(default)]
    pub parts: Vec<Part>,
    pub role: Option<String>,
}

#[derive(Debug, Serialize, Deserialize, Clone)]
#[serde(untagged)]
pub enum Part {
    Text {
        text: String,
    },
    FunctionCall {
        #[serde(rename = "functionCall")]
        function_call: FunctionCall,
    },
    FunctionResponse {
        #[serde(rename = "functionResponse")]
        function_response: FunctionResponse,
    },
    InlineData {
        #[serde(rename = "inlineData")]
        inline_data: InlineData,
    },
    FileData {
        #[serde(rename = "fileData")]
        file_data: FileData,
    },
}

#[derive(Debug, Serialize, Deserialize, Clone)]
pub struct InlineData {
    #[serde(rename = "mimeType")]
    pub mime_type: String,
    pub data: String, // base64 encoded data
}

#[derive(Debug, Serialize, Deserialize, Clone)]
pub struct FileData {
    #[serde(rename = "mimeType")]
    pub mime_type: String,
    #[serde(rename = "fileUri")]
    pub file_uri: String,
}

#[derive(Debug, Serialize, Deserialize, Clone)]
pub struct FunctionCall {
    pub name: String,
    pub args: HashMap<String, Value>,
    #[serde(
        rename = "thoughtSignature",
        skip_serializing_if = "Option::is_none",
        default
    )]
    pub thought_signature: Option<String>,
}

#[derive(Debug, Serialize, Deserialize, Clone)]
pub struct FunctionResponse {
    pub name: String,
    pub response: HashMap<String, Value>,
    #[serde(
        rename = "thoughtSignature",
        skip_serializing_if = "Option::is_none",
        default
    )]
    pub thought_signature: Option<String>,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct Tool {
    #[serde(rename = "functionDeclarations")]
    pub function_declarations: Vec<FunctionDeclaration>,
}

#[derive(Debug, Serialize, Deserialize, Clone)]
pub struct FunctionDeclaration {
    pub name: String,
    pub description: String,
    pub parameters: Parameters,
}

#[derive(Debug, Serialize, Deserialize, Clone)]
pub struct Parameters {
    #[serde(rename = "type")]
    pub param_type: String,
    pub properties: HashMap<String, PropertyDefinition>,
    pub required: Vec<String>,
}

#[derive(Debug, Serialize, Deserialize, Clone)]
pub struct PropertyDefinition {
    #[serde(rename = "type")]
    pub prop_type: String,
    pub description: String,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub items: Option<Box<PropertyDefinition>>,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct GenerationConfig {
    pub temperature: f32,
    #[serde(rename = "topK")]
    pub top_k: u32,
    #[serde(rename = "topP")]
    pub top_p: f32,
    #[serde(rename = "maxOutputTokens")]
    pub max_output_tokens: u32,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct ToolConfig {
    #[serde(rename = "functionCallingConfig")]
    pub function_calling_config: FunctionCallingConfig,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct FunctionCallingConfig {
    pub mode: FunctionCallingMode,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub enum FunctionCallingMode {
    #[serde(rename = "AUTO")]
    Auto,
    #[serde(rename = "ANY")]
    Any,
    #[serde(rename = "NONE")]
    None,
}

#[derive(Debug, Serialize, Deserialize)]
pub struct GenerateContentResponse {
    pub candidates: Vec<Candidate>,
    #[serde(rename = "usageMetadata")]
    pub usage_metadata: Option<UsageMetadata>,
    #[serde(rename = "promptFeedback")]
    pub prompt_feedback: Option<PromptFeedback>,
}

#[derive(Debug, Serialize, Deserialize)]
pub struct Candidate {
    pub content: Option<Content>,
    #[serde(rename = "finishReason")]
    pub finish_reason: Option<String>,
    pub index: Option<u32>,
    #[serde(rename = "safetyRatings")]
    pub safety_ratings: Option<Vec<SafetyRating>>,
}

#[derive(Debug, Serialize, Deserialize)]
pub struct PromptFeedback {
    #[serde(rename = "blockReason")]
    pub block_reason: Option<String>,
    #[serde(rename = "safetyRatings")]
    pub safety_ratings: Option<Vec<SafetyRating>>,
}

#[derive(Debug, Serialize, Deserialize)]
pub struct SafetyRating {
    pub category: String,
    pub probability: String,
}

#[derive(Debug, Serialize, Deserialize)]
pub struct UsageMetadata {
    #[serde(rename = "promptTokenCount")]
    pub prompt_token_count: u32,
    #[serde(rename = "candidatesTokenCount")]
    pub candidates_token_count: u32,
    #[serde(rename = "totalTokenCount")]
    pub total_token_count: u32,
}

#[derive(Debug, Serialize, Deserialize)]
pub struct EmbedContentRequest {
    pub model: String,
    pub content: EmbedContent,
    #[serde(rename = "outputDimensionality")]
    pub output_dimensionality: Option<u32>,
}

#[derive(Debug, Serialize, Deserialize)]
pub struct EmbedContent {
    pub parts: Vec<Part>,
}

#[derive(Debug, Serialize, Deserialize)]
pub struct EmbedContentResponse {
    pub embedding: Embedding,
}

#[derive(Debug, Serialize, Deserialize)]
pub struct Embedding {
    pub values: Vec<f32>,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct UploadedFile {
    pub name: String,
    pub uri: String,
    #[serde(rename = "mimeType")]
    pub mime_type: String,
    #[serde(default)]
    pub state: Option<String>,
}

#[derive(Debug, Serialize, Deserialize)]
#[allow(dead_code)]
pub struct ImageGenerationRequest {
    pub prompt: String,
    pub model: String,
    pub output_mime_type: String,
}

#[derive(Debug, Serialize, Deserialize)]
#[allow(dead_code)]
pub struct ImageGenerationResponse {
    pub candidates: Vec<ImageCandidate>,
}

#[derive(Debug, Serialize, Deserialize)]
#[allow(dead_code)]
pub struct ImageCandidate {
    pub output: String, // Base64 encoded image
    #[serde(rename = "mimeType")]
    pub mime_type: String,
}

impl GeminiClient {
    pub fn new(api_key: String) -> Self {
        let text_model = std::env::var("GEMINI_TEXT_MODEL")
            .unwrap_or_else(|_| "gemini-2.5-flash".to_string());
        Self {
            client: Client::new(),
            api_key,
            base_url: "https://generativelanguage.googleapis.com/v1beta".to_string(),
            text_model,
            semaphore: Arc::new(tokio::sync::Semaphore::new(GEMINI_MAX_CONCURRENT)),
        }
    }

    /// Create a client that uses a specific model for generate_text() and generate_content().
    /// Use "gemma-4-27b-it" for Gemma 4 via Google AI Studio (free, own quota).
    pub fn new_with_model(api_key: String, model: String) -> Self {
        Self {
            client: Client::new(),
            api_key,
            base_url: "https://generativelanguage.googleapis.com/v1beta".to_string(),
            text_model: model,
            semaphore: Arc::new(tokio::sync::Semaphore::new(GEMINI_MAX_CONCURRENT)),
        }
    }

    pub async fn generate_content(
        &self,
        request: GenerateContentRequest,
    ) -> Result<GenerateContentResponse, Box<dyn std::error::Error + Send + Sync>> {
        // Acquire concurrency permit — released automatically when _permit is dropped.
        let _permit = self
            .semaphore
            .acquire()
            .await
            .map_err(|e| format!("Gemini semaphore error: {}", e))?;

        let url = format!(
            "{}/models/{}:generateContent?key={}",
            self.base_url, self.text_model, self.api_key
        );

        // Debug: Log the request to see if thought signatures are present
        if let Ok(_request_json) = serde_json::to_string_pretty(&request) {
            tracing::debug!(
                "Gemini API Request contents count: {}",
                request.contents.len()
            );
            for (i, content) in request.contents.iter().enumerate() {
                tracing::debug!(
                    "Content[{}]: role={:?}, parts_count={}",
                    i,
                    content.role,
                    content.parts.len()
                );
                for (j, part) in content.parts.iter().enumerate() {
                    match part {
                        Part::FunctionCall { function_call } => {
                            tracing::warn!(
                                "Content[{}].Part[{}]: FunctionCall name={}, has_signature={}",
                                i,
                                j,
                                function_call.name,
                                function_call.thought_signature.is_some()
                            );
                        }
                        Part::FunctionResponse { function_response } => {
                            tracing::debug!(
                                "Content[{}].Part[{}]: FunctionResponse name={}, has_signature={}",
                                i,
                                j,
                                function_response.name,
                                function_response.thought_signature.is_some()
                            );
                        }
                        _ => {}
                    }
                }
            }
        }

        // Retry loop: up to 4 attempts, respecting Gemini rate-limit (429) responses.
        let max_attempts = 4u32;
        let mut last_error: Box<dyn std::error::Error + Send + Sync> = "No attempts made".into();

        for attempt in 0..max_attempts {
            let response = self
                .client
                .post(&url)
                .header("Content-Type", "application/json")
                .json(&request)
                .send()
                .await?;

            let status = response.status();

            if status.is_success() {
                let response_text = response.text().await?;
                tracing::debug!(
                    "Gemini API response (truncated): {}...",
                    &response_text[..response_text.len().min(500)]
                );

                // Log thought signature in raw response
                if response_text.contains("thoughtSignature") {
                    tracing::warn!("✅ Raw response CONTAINS thoughtSignature field");
                } else {
                    tracing::error!("❌ Raw response MISSING thoughtSignature field");
                }

                match serde_json::from_str::<GenerateContentResponse>(&response_text) {
                    Ok(result) => {
                        // Check if thought signature was deserialized
                        if let Some(candidate) = result.candidates.first() {
                            if let Some(ref content) = candidate.content {
                                for (i, part) in content.parts.iter().enumerate() {
                                    if let Part::FunctionCall { function_call } = part {
                                        tracing::warn!("🔍 Deserialized Part[{}]: FunctionCall '{}' has_signature={}",
                                            i, function_call.name, function_call.thought_signature.is_some());
                                    }
                                }
                            }
                        }
                        return Ok(result);
                    }
                    Err(parse_error) => {
                        tracing::error!("Failed to parse Gemini response: {}", parse_error);
                        tracing::error!("Response body: {}", response_text);
                        return Err(format!("error decoding response body: {}", parse_error).into());
                    }
                }
            }

            let error_text = response.text().await?;

            // On 429 (rate limit), honour the server-specified retry-after delay and try again.
            if status.as_u16() == 429 && attempt < max_attempts - 1 {
                let retry_secs = parse_gemini_retry_delay(&error_text, 30.0);
                let wait_secs = (retry_secs + 5.0) as u64;
                tracing::warn!(
                    "⏳ Gemini rate limited (429, attempt {}/{}). \
                     Waiting {}s before retry…",
                    attempt + 1,
                    max_attempts,
                    wait_secs
                );
                tokio::time::sleep(tokio::time::Duration::from_secs(wait_secs)).await;
                last_error = format!("Gemini API error (rate limited): {}", error_text).into();
                continue;
            }

            // On 503 (model overloaded), retry with exponential backoff.
            if status.as_u16() == 503 && attempt < max_attempts - 1 {
                let wait_secs = (10u64 * 2u64.pow(attempt)).min(60);
                tracing::warn!(
                    "⚠️ Gemini model overloaded (503, attempt {}/{}). \
                     Waiting {}s before retry…",
                    attempt + 1,
                    max_attempts,
                    wait_secs
                );
                tokio::time::sleep(tokio::time::Duration::from_secs(wait_secs)).await;
                last_error = format!("Gemini API error (model overloaded): {}", error_text).into();
                continue;
            }

            // Non-retryable error.
            return Err(format!("Gemini API error: {}", error_text).into());
        }

        Err(last_error)
    }

    pub async fn embed_content(
        &self,
        text: &str,
    ) -> Result<Vec<f32>, Box<dyn std::error::Error + Send + Sync>> {
        self.embed_content_with_model(text, "models/text-embedding-004", Some(768))
            .await
    }

    pub async fn embed_content_with_model(
        &self,
        text: &str,
        model: &str,
        output_dimensionality: Option<u32>,
    ) -> Result<Vec<f32>, Box<dyn std::error::Error + Send + Sync>> {
        self.embed_parts_with_model(
            vec![Part::Text {
                text: text.to_string(),
            }],
            model,
            output_dimensionality,
        )
        .await
    }

    pub async fn embed_parts_with_model(
        &self,
        parts: Vec<Part>,
        model: &str,
        output_dimensionality: Option<u32>,
    ) -> Result<Vec<f32>, Box<dyn std::error::Error + Send + Sync>> {
        let url = format!(
            "{}/{}:embedContent?key={}",
            self.base_url, model, self.api_key
        );

        let request = EmbedContentRequest {
            model: model.to_string(),
            content: EmbedContent { parts },
            output_dimensionality,
        };

        let response = self
            .client
            .post(&url)
            .header("Content-Type", "application/json")
            .json(&request)
            .send()
            .await?;

        if response.status().is_success() {
            let result: EmbedContentResponse = response.json().await?;
            Ok(result.embedding.values)
        } else {
            let error_text = response.text().await?;
            Err(format!("Gemini Embedding API error: {}", error_text).into())
        }
    }

    fn api_root(&self) -> &str {
        self.base_url.trim_end_matches("/v1beta")
    }

    pub async fn upload_file(
        &self,
        file_path: &str,
        mime_type: &str,
        display_name: Option<&str>,
    ) -> Result<UploadedFile, Box<dyn std::error::Error + Send + Sync>> {
        let path = std::path::Path::new(file_path);
        let file_name = display_name
            .map(|value| value.to_string())
            .or_else(|| {
                path.file_name()
                    .and_then(|value| value.to_str())
                    .map(|value| value.to_string())
            })
            .unwrap_or_else(|| "uploaded-media".to_string());

        let start_url = format!(
            "{}/upload/v1beta/files?key={}",
            self.api_root(),
            self.api_key
        );
        let file_bytes = tokio::fs::read(path).await?;
        let start_response = self
            .client
            .post(&start_url)
            .header("X-Goog-Upload-Protocol", "resumable")
            .header("X-Goog-Upload-Command", "start")
            .header("X-Goog-Upload-Header-Content-Length", file_bytes.len())
            .header("X-Goog-Upload-Header-Content-Type", mime_type)
            .header("Content-Type", "application/json")
            .json(&serde_json::json!({
                "file": {
                    "display_name": file_name,
                }
            }))
            .send()
            .await?;

        if !start_response.status().is_success() {
            let error_text = start_response.text().await?;
            return Err(format!("Gemini file upload start error: {}", error_text).into());
        }

        let upload_url = start_response
            .headers()
            .get("x-goog-upload-url")
            .and_then(|value| value.to_str().ok())
            .map(|value| value.to_string())
            .ok_or("Gemini file upload start response missing upload URL")?;

        let finalize_response = self
            .client
            .post(&upload_url)
            .header("X-Goog-Upload-Offset", "0")
            .header("X-Goog-Upload-Command", "upload, finalize")
            .body(file_bytes)
            .send()
            .await?;

        if !finalize_response.status().is_success() {
            let error_text = finalize_response.text().await?;
            return Err(format!("Gemini file upload finalize error: {}", error_text).into());
        }

        let response_json: serde_json::Value = finalize_response.json().await?;
        let file = response_json
            .get("file")
            .cloned()
            .ok_or("Gemini file upload finalize response missing file object")?;

        let uploaded: UploadedFile = serde_json::from_value(file)?;
        self.wait_for_file_active(&uploaded.name).await
    }

    async fn wait_for_file_active(
        &self,
        file_name: &str,
    ) -> Result<UploadedFile, Box<dyn std::error::Error + Send + Sync>> {
        let status_url = format!(
            "{}/v1beta/{}?key={}",
            self.api_root(),
            file_name,
            self.api_key
        );

        for attempt in 0..30u32 {
            let response = self.client.get(&status_url).send().await?;
            if !response.status().is_success() {
                let error_text = response.text().await?;
                return Err(format!("Gemini file status error: {}", error_text).into());
            }

            let file: UploadedFile = response.json().await?;
            match file.state.as_deref() {
                Some("ACTIVE") | None => return Ok(file),
                Some("FAILED") => {
                    return Err(format!("Gemini file processing failed for {}", file_name).into())
                }
                _ => {
                    if attempt < 29 {
                        tokio::time::sleep(tokio::time::Duration::from_secs(2)).await;
                    } else {
                        return Err(format!(
                            "Timed out waiting for Gemini file to become ACTIVE: {}",
                            file_name
                        )
                        .into());
                    }
                }
            }
        }

        Err(format!(
            "Timed out waiting for Gemini file to become ACTIVE: {}",
            file_name
        )
        .into())
    }

    pub async fn embed_uploaded_file_with_model(
        &self,
        file_path: &str,
        mime_type: &str,
        model: &str,
        output_dimensionality: Option<u32>,
        prompt_text: Option<&str>,
    ) -> Result<Vec<f32>, Box<dyn std::error::Error + Send + Sync>> {
        let uploaded = self.upload_file(file_path, mime_type, None).await?;
        let mut parts = Vec::new();
        if let Some(text) = prompt_text {
            if !text.trim().is_empty() {
                parts.push(Part::Text {
                    text: text.to_string(),
                });
            }
        }
        parts.push(Part::FileData {
            file_data: FileData {
                mime_type: uploaded.mime_type,
                file_uri: uploaded.uri,
            },
        });

        self.embed_parts_with_model(parts, model, output_dimensionality)
            .await
    }

    /// Resolve a user-supplied model alias to a concrete Gemini model ID.
    fn resolve_image_model(model: Option<&str>) -> &str {
        match model {
            Some("fast") | Some("nano") => "gemini-3.1-flash-image",
            Some("quality") | Some("pro") | None => "gemini-3-pro-image",
            Some(explicit) => explicit,
        }
    }

    /// Generate an image using Nano Banana Pro (Gemini 3 Pro Image Preview)
    /// Supports 10 aspect ratios: 1:1, 2:3, 3:2, 3:4, 4:3, 4:5, 5:4, 9:16, 16:9, 21:9
    /// Supports 3 resolutions: 1K (1024px), 2K (2048px), 4K (4096px)
    pub async fn generate_image(
        &self,
        prompt: &str,
        aspect_ratio: Option<&str>,
        image_size: Option<&str>,
        model: Option<&str>,
    ) -> Result<Vec<u8>, Box<dyn std::error::Error + Send + Sync>> {
        let _permit = self
            .semaphore
            .acquire()
            .await
            .map_err(|e| format!("Gemini semaphore error: {}", e))?;

        let model_id = Self::resolve_image_model(model);

        let mut config_map = serde_json::Map::new();

        let mut image_config = serde_json::Map::new();
        image_config.insert(
            "aspectRatio".to_string(),
            serde_json::Value::String(aspect_ratio.unwrap_or("1:1").to_string()),
        );
        image_config.insert(
            "imageSize".to_string(),
            serde_json::Value::String(image_size.unwrap_or("2K").to_string()),
        );
        config_map.insert(
            "imageConfig".to_string(),
            serde_json::Value::Object(image_config),
        );

        config_map.insert(
            "responseModalities".to_string(),
            serde_json::Value::Array(vec![serde_json::Value::String("IMAGE".to_string())]),
        );

        let request = serde_json::json!({
            "contents": [{
                "parts": [{ "text": prompt }],
                "role": "user"
            }],
            "generationConfig": config_map
        });

        let url = format!(
            "{}/models/{}:generateContent?key={}",
            self.base_url, model_id, self.api_key
        );

        tracing::debug!(
            "generate_image ({}) request: {}",
            model_id,
            serde_json::to_string_pretty(&request)?
        );

        let response = self
            .client
            .post(&url)
            .header("Content-Type", "application/json")
            .json(&request)
            .send()
            .await?;

        if response.status().is_success() {
            let response_text = response.text().await?;
            tracing::debug!("generate_image response: {}", response_text);

            let response_json: serde_json::Value = serde_json::from_str(&response_text)?;

            if let Some(candidates) = response_json["candidates"].as_array() {
                if let Some(candidate) = candidates.first() {
                    if let Some(content) = candidate.get("content") {
                        if let Some(parts) = content["parts"].as_array() {
                            for part in parts {
                                if let Some(inline_data) = part.get("inlineData") {
                                    if let Some(data) = inline_data["data"].as_str() {
                                        let image_bytes =
                                            BASE64_STANDARD.decode(data).map_err(|e| {
                                                format!("Failed to decode base64 image: {}", e)
                                            })?;
                                        return Ok(image_bytes);
                                    }
                                }
                            }
                        }
                    }
                }
            }

            Err("No image data found in response".into())
        } else if response.status().as_u16() == 429 {
            let error_json: serde_json::Value = serde_json::from_str(&response.text().await.unwrap_or_default()).unwrap_or_default();
            let retry_seconds = error_json["error"]["details"]
                .as_array()
                .and_then(|details| {
                    details.iter().find_map(|d| {
                        d["retryDelay"]
                            .as_str()
                            .and_then(|s| s.trim_end_matches('s').parse::<u64>().ok())
                    })
                })
                .unwrap_or(30);
            tracing::warn!("Gemini image generation hit 429 quota, retrying after {}s", retry_seconds);
            tokio::time::sleep(std::time::Duration::from_secs(retry_seconds)).await;
            let retry_response = self.client.post(&url)
                .header("Content-Type", "application/json")
                .json(&request)
                .send()
                .await?;
            let retry_status = retry_response.status();
            let retry_text = retry_response.text().await.unwrap_or_default();
            if retry_status.is_success() {
                let response_json: serde_json::Value = serde_json::from_str(&retry_text)?;
                if let Some(candidates) = response_json["candidates"].as_array() {
                    if let Some(candidate) = candidates.first() {
                        if let Some(content) = candidate.get("content") {
                            if let Some(parts) = content["parts"].as_array() {
                                for part in parts {
                                    if let Some(inline_data) = part.get("inlineData") {
                                        if let Some(data) = inline_data["data"].as_str() {
                                            let image_bytes = BASE64_STANDARD.decode(data).map_err(|e| format!("Failed to decode base64 image: {}", e))?;
                                            return Ok(image_bytes);
                                        }
                                    }
                                }
                            }
                        }
                    }
                }
            }
            Err(format!("Gemini image generation 429 even after retry ({}): {}", model_id, retry_text).into())
        } else {
            let error_text = response.text().await?;
            Err(format!(
                "Gemini image generation API error ({}): {}",
                model_id, error_text
            )
            .into())
        }
    }

    /// Edit an existing image using Gemini image generation (image-in → image-out).
    ///
    /// Passes `image_bytes` as an inline JPEG alongside a text `prompt` describing
    /// the desired edits. Returns the edited image as raw bytes.
    pub async fn edit_image(
        &self,
        prompt: &str,
        image_bytes: &[u8],
        aspect_ratio: Option<&str>,
        model: Option<&str>,
    ) -> Result<Vec<u8>, Box<dyn std::error::Error + Send + Sync>> {
        let _permit = self
            .semaphore
            .acquire()
            .await
            .map_err(|e| format!("Gemini semaphore error: {}", e))?;

        let model_id = Self::resolve_image_model(model);
        let image_b64 = BASE64_STANDARD.encode(image_bytes);

        let mut config_map = serde_json::Map::new();
        config_map.insert(
            "responseModalities".to_string(),
            serde_json::json!(["TEXT", "IMAGE"]),
        );
        let mut image_config = serde_json::Map::new();
        image_config.insert(
            "aspectRatio".to_string(),
            serde_json::Value::String(aspect_ratio.unwrap_or("16:9").to_string()),
        );
        config_map.insert(
            "imageConfig".to_string(),
            serde_json::Value::Object(image_config),
        );

        let request = serde_json::json!({
            "contents": [{
                "role": "user",
                "parts": [
                    { "text": prompt },
                    {
                        "inlineData": {
                            "mimeType": "image/jpeg",
                            "data": image_b64
                        }
                    }
                ]
            }],
            "generationConfig": config_map
        });

        let url = format!(
            "{}/models/{}:generateContent?key={}",
            self.base_url, model_id, self.api_key
        );

        tracing::debug!("edit_image ({}) prompt: {}", model_id, prompt);

        let response = self
            .client
            .post(&url)
            .header("Content-Type", "application/json")
            .json(&request)
            .send()
            .await?;

        if response.status().is_success() {
            let response_text = response.text().await?;
            let response_json: serde_json::Value = serde_json::from_str(&response_text)?;

            if let Some(candidates) = response_json["candidates"].as_array() {
                if let Some(candidate) = candidates.first() {
                    if let Some(parts) = candidate["content"]["parts"].as_array() {
                        for part in parts {
                            if let Some(inline_data) = part.get("inlineData") {
                                if let Some(data) = inline_data["data"].as_str() {
                                    let bytes = BASE64_STANDARD
                                        .decode(data)
                                        .map_err(|e| format!("Failed to decode image: {}", e))?;
                                    return Ok(bytes);
                                }
                            }
                        }
                    }
                }
            }

            Err(format!(
                "No image data in response: {}",
                &response_text[..response_text.len().min(300)]
            )
            .into())
        } else {
            let error_text = response.text().await?;
            Err(format!("Gemini image edit API error ({}): {}", model_id, error_text).into())
        }
    }

    /// Backward-compatible alias — used internally by the thumbnail pipeline.
    pub async fn generate_image_from_frame(
        &self,
        prompt: &str,
        image_bytes: &[u8],
        aspect_ratio: Option<&str>,
    ) -> Result<Vec<u8>, Box<dyn std::error::Error + Send + Sync>> {
        self.edit_image(prompt, image_bytes, aspect_ratio, None)
            .await
    }

    /// Consolidated apply_ffmpeg_filter tool — replaces 60+ individual one-to-one FFmpeg filter wrappers.
    /// The model passes any FFmpeg video filter name with parameters as a key-value object.
    pub fn apply_ffmpeg_filter_tool() -> FunctionDeclaration {
        FunctionDeclaration {
            name: "apply_ffmpeg_filter".to_string(),
            description: "A single parameterized tool for ALL video/audio filter operations. Pass the exact FFmpeg filter name (e.g. 'gblur', 'colorkey', 'fade', 'edgedetect', 'hflip', 'negate', 'eq', 'curves', 'lut', 'colorbalance', 'unsharp', 'nlmeans', 'hqdn3d') plus filter-specific parameters as a JSON object. Covers hundreds of FFmpeg filters: blur/sharpen, color grading, keying, transforms, denoise, deinterlace, scaling, cropping, rotation, audio filters, and more.".to_string(),
            parameters: Parameters {
                param_type: "object".to_string(),
                properties: {
                    let mut props = std::collections::HashMap::new();
                    props.insert("input_file".to_string(), PropertyDefinition {
                        prop_type: "string".to_string(),
                        description: "Path to the input video file".to_string(),
                        items: None,
                    });
                    props.insert("output_file".to_string(), PropertyDefinition {
                        prop_type: "string".to_string(),
                        description: "Path to save the filtered output video".to_string(),
                        items: None,
                    });
                    props.insert("filter_name".to_string(), PropertyDefinition {
                        prop_type: "string".to_string(),
                        description: "FFmpeg filter name (e.g., gblur, avgblur, smartblur, colorbalance, edgedetect, negate, fade, hflip, colorkey, etc.)".to_string(),
                        items: None,
                    });
                    props.insert("params".to_string(), PropertyDefinition {
                        prop_type: "object".to_string(),
                        description: "Filter-specific parameters as key-value pairs. Example: {\"sigma\": 3.0, \"steps\": 1}".to_string(),
                        items: None,
                    });
                    props
                },
                required: vec!["input_file".to_string(), "output_file".to_string(), "filter_name".to_string()],
            },
        }
    }

    /// Consolidated apply_audio_ffmpeg_filter tool — replaces 30+ individual one-to-one FFmpeg audio filter wrappers.
    pub fn apply_audio_ffmpeg_filter_tool() -> FunctionDeclaration {
        FunctionDeclaration {
            name: "apply_audio_ffmpeg_filter".to_string(),
            description: "A single parameterized tool for ALL audio filter and audio processing operations. Pass the exact FFmpeg audio filter name (e.g. 'volume', 'equalizer', 'bass', 'treble', 'aecho', 'chorus', 'dynaudnorm', 'loudnorm', 'afftdn', 'acompressor', 'silenceremove', 'atempo', 'asetrate') plus filter-specific parameters as a JSON object. Covers all FFmpeg audio filters: volume/level adjustment, EQ and tone shaping, dynamics processing (compressor/limiter/gate), noise reduction, time/pitch stretching, echo/reverb/modulation effects, audio normalization, silence detection/removal, channel mixing, and delay/padding/trimming.".to_string(),
            parameters: Parameters {
                param_type: "object".to_string(),
                properties: {
                    let mut props = std::collections::HashMap::new();
                    props.insert("input_file".to_string(), PropertyDefinition {
                        prop_type: "string".to_string(),
                        description: "Path to the input audio or video file".to_string(),
                        items: None,
                    });
                    props.insert("output_file".to_string(), PropertyDefinition {
                        prop_type: "string".to_string(),
                        description: "Path to save the filtered output".to_string(),
                        items: None,
                    });
                    props.insert("filter_name".to_string(), PropertyDefinition {
                        prop_type: "string".to_string(),
                        description: "FFmpeg audio filter name (e.g., volume, equalizer, bass, treble, aecho, chorus, dynaudnorm, afftdn, etc.)".to_string(),
                        items: None,
                    });
                    props.insert("params".to_string(), PropertyDefinition {
                        prop_type: "object".to_string(),
                        description: "Filter-specific parameters as key-value pairs. Example: {\"frequency\": 1000, \"width\": 200, \"gain\": 3}".to_string(),
                        items: None,
                    });
                    props
                },
                required: vec!["input_file".to_string(), "output_file".to_string(), "filter_name".to_string()],
            },
        }
    }

    /// Consolidated blender_generate_scene_type — replaces 28 individual blender_generate_* tools.
    pub fn blender_generate_scene_type_tool() -> FunctionDeclaration {
        FunctionDeclaration {
            name: "blender_generate_scene_type".to_string(),
            description: "Generate 3D/2D animated scenes of any length using natural language. Use for: product mockups and device UI mockups, title cards and intro/outro sequences, explainer scenes and motion graphics, lower-thirds and text overlays, abstract backgrounds and particle effects, thumbnail images, logos and reveals, cinematic B-roll. Describe exactly what you want in the prompt and pass optional style/duration/reference_image_url params.".to_string(),
            parameters: Parameters {
                param_type: "object".to_string(),
                properties: {
                    let mut props = std::collections::HashMap::new();
                    props.insert("prompt".to_string(), PropertyDefinition {
                        prop_type: "string".to_string(),
                        description: "Natural language description of the scene. Be specific about objects, materials, lighting, camera angles, animations, and mood.".to_string(),
                        items: None,
                    });
                    props.insert("params".to_string(), PropertyDefinition {
                        prop_type: "object".to_string(),
                        description: "JSON object with optional keys: style (cinematic|minimal|energetic|calm|neon|toon), reference_image_url (URL), duration (float, seconds)".to_string(),
                        items: None,
                    });
                    props.insert("output_type".to_string(), PropertyDefinition {
                        prop_type: "string".to_string(),
                        description: "\"video\" (MP4, default) | \"image\" (PNG thumbnail)".to_string(),
                        items: None,
                    });
                    props
                },
                required: vec!["prompt".to_string()],
            },
        }
    }

    /// Consolidated export_video — replaces 12 individual encode_* tools.
    pub fn export_video_tool() -> FunctionDeclaration {
        FunctionDeclaration {
            name: "export_video".to_string(),
            description: "Export/re-encode a video to the desired format, codec, resolution, and quality. Use this instead of individual encode tools. Supports H.264, H.265/HEVC, VP9, AV1, ProRes, DNxHD, GIF, WebM, and more. Configurable: codec, bitrate, resolution, framerate, pixel format, color range, HDR metadata, hardware acceleration (NVENC/VAAPI/QSV).".to_string(),
            parameters: Parameters {
                param_type: "object".to_string(),
                properties: {
                    let mut props = std::collections::HashMap::new();
                    props.insert("input_file".to_string(), PropertyDefinition {
                        prop_type: "string".to_string(),
                        description: "Path to the input video file".to_string(),
                        items: None,
                    });
                    props.insert("output_file".to_string(), PropertyDefinition {
                        prop_type: "string".to_string(),
                        description: "Path to save the encoded output".to_string(),
                        items: None,
                    });
                    props.insert("codec".to_string(), PropertyDefinition {
                        prop_type: "string".to_string(),
                        description: "FFmpeg encoder name: libx264, libx265, libvpx-vp9, libaom-av1, prores_ks, dnxhd, h264_nvenc, h264_vaapi, h264_qsv, gif".to_string(),
                        items: None,
                    });
                    props.insert("params".to_string(), PropertyDefinition {
                        prop_type: "object".to_string(),
                        description: "Encoder-specific parameters as key-value pairs. Example: {\"crf\": 23, \"preset\": \"medium\", \"pix_fmt\": \"yuv420p\"}".to_string(),
                        items: None,
                    });
                    props
                },
                required: vec!["input_file".to_string(), "output_file".to_string(), "codec".to_string()],
            },
        }
    }

    /// Consolidated manim_execute_script — replaces 14+ individual blender_generate_* Manim tools.
    pub fn manim_execute_script_tool() -> FunctionDeclaration {
        FunctionDeclaration {
            name: "manim_execute_script".to_string(),
            description: "Generate animated math/science explainer visuals of any length using natural language. Use for: mathematical equation animations and LaTeX formulas, data visualizations (bar, line, pie, scatter charts), code animations and algorithm visualizations, network graphs and flowcharts, geometric proofs and vector fields, timelines and process diagrams, animated text and titles, 3D math scene rotations, matrix transformations. Pass the description, duration, background style, and quality settings.".to_string(),
            parameters: Parameters {
                param_type: "object".to_string(),
                properties: {
                    let mut props = std::collections::HashMap::new();
                    props.insert("description".to_string(), PropertyDefinition {
                        prop_type: "string".to_string(),
                        description: "Natural language description of the desired animation. Be specific about scene content, objects, colors, transforms, and text.".to_string(),
                        items: None,
                    });
                    props.insert("duration".to_string(), PropertyDefinition {
                        prop_type: "number".to_string(),
                        description: "Target clip duration in seconds (default 10)".to_string(),
                        items: None,
                    });
                    props.insert("background".to_string(), PropertyDefinition {
                        prop_type: "string".to_string(),
                        description: "Background style: \"dark\" | \"light\" | \"transparent\" (default \"dark\")".to_string(),
                        items: None,
                    });
                    props.insert("transparent".to_string(), PropertyDefinition {
                        prop_type: "boolean".to_string(),
                        description: "If true, render with alpha channel (ProRes .mov)".to_string(),
                        items: None,
                    });
                    props.insert("quality".to_string(), PropertyDefinition {
                        prop_type: "string".to_string(),
                        description: "Render quality: \"l\" (480p) | \"m\" (720p) | \"h\" (1080p) (default \"m\")".to_string(),
                        items: None,
                    });
                    props.insert("include_narration".to_string(), PropertyDefinition {
                        prop_type: "boolean".to_string(),
                        description: "If true, generate and attach VibeVoice narration".to_string(),
                        items: None,
                    });
                    props.insert("narration_text".to_string(), PropertyDefinition {
                        prop_type: "string".to_string(),
                        description: "Custom narration text (auto-generated from prompt if empty)".to_string(),
                        items: None,
                    });
                    props.insert("narration_speaker".to_string(), PropertyDefinition {
                        prop_type: "string".to_string(),
                        description: "VibeVoice speaker name (default \"Emma\")".to_string(),
                        items: None,
                    });
                    props
                },
                required: vec!["description".to_string()],
            },
        }
    }

    pub fn create_video_editing_tools() -> Vec<FunctionDeclaration> {
        vec![
            FunctionDeclaration {
                name: "trim_video".to_string(),
                description: "Trims a video to specified start and end times".to_string(),
                parameters: Parameters {
                    param_type: "object".to_string(),
                    properties: {
                        let mut props = HashMap::new();
                        props.insert("input_file".to_string(), PropertyDefinition {
                            prop_type: "string".to_string(),
                            description: "Path to the input video file".to_string(),
                            items: None,
                        });
                        props.insert("output_file".to_string(), PropertyDefinition {
                            prop_type: "string".to_string(),
                            description: "Path to save the trimmed video".to_string(),
                            items: None,
                        });
                        props.insert("start_seconds".to_string(), PropertyDefinition {
                            prop_type: "number".to_string(),
                            description: "Start time in seconds".to_string(),
                            items: None,
                        });
                        props.insert("end_seconds".to_string(), PropertyDefinition {
                            prop_type: "number".to_string(),
                            description: "End time in seconds".to_string(),
                            items: None,
                        });
                        props
                    },
                    required: vec!["input_file".to_string(), "output_file".to_string(), "start_seconds".to_string(), "end_seconds".to_string()],
                },
            },
            FunctionDeclaration {
                name: "merge_videos".to_string(),
                description: "Merges multiple video files into a single video".to_string(),
                parameters: Parameters {
                    param_type: "object".to_string(),
                    properties: {
                        let mut props = HashMap::new();
                        props.insert("input_files".to_string(), PropertyDefinition {
                            prop_type: "array".to_string(),
                            description: "Array of input video file paths".to_string(),
                            items: Some(Box::new(PropertyDefinition {
                                prop_type: "string".to_string(),
                                description: "Video file path".to_string(),
                                items: None,
                            })),
                        });
                        props.insert("output_file".to_string(), PropertyDefinition {
                            prop_type: "string".to_string(),
                            description: "Path to save the merged video".to_string(),
                            items: None,
                        });
                        props
                    },
                    required: vec!["input_files".to_string(), "output_file".to_string()],
                },
            },
            FunctionDeclaration {
                name: "analyze_video".to_string(),
                description: "Analyzes a video file and returns metadata".to_string(),
                parameters: Parameters {
                    param_type: "object".to_string(),
                    properties: {
                        let mut props = HashMap::new();
                        props.insert("input_file".to_string(), PropertyDefinition {
                            prop_type: "string".to_string(),
                            description: "Path to the video file to analyze".to_string(),
                            items: None,
                        });
                        props
                    },
                    required: vec!["input_file".to_string()],
                },
            },
            FunctionDeclaration {
                name: "add_text_overlay".to_string(),
                description: "Adds text overlay to a video at specified position".to_string(),
                parameters: Parameters {
                    param_type: "object".to_string(),
                    properties: {
                        let mut props = HashMap::new();
                        props.insert("input_file".to_string(), PropertyDefinition {
                            prop_type: "string".to_string(),
                            description: "Path to the input video file".to_string(),
                            items: None,
                        });
                        props.insert("output_file".to_string(), PropertyDefinition {
                            prop_type: "string".to_string(),
                            description: "Path to save the video with text overlay".to_string(),
                            items: None,
                        });
                        props.insert("text".to_string(), PropertyDefinition {
                            prop_type: "string".to_string(),
                            description: "The text to overlay on the video".to_string(),
                            items: None,
                        });
                        props.insert("x".to_string(), PropertyDefinition {
                            prop_type: "number".to_string(),
                            description: "X position of the text".to_string(),
                            items: None,
                        });
                        props.insert("y".to_string(), PropertyDefinition {
                            prop_type: "number".to_string(),
                            description: "Y position of the text".to_string(),
                            items: None,
                        });
                        props.insert("font_size".to_string(), PropertyDefinition {
                            prop_type: "number".to_string(),
                            description: "Font size (default: 24)".to_string(),
                            items: None,
                        });
                        props.insert("color".to_string(), PropertyDefinition {
                            prop_type: "string".to_string(),
                            description: "Text color (default: white)".to_string(),
                            items: None,
                        });
                        props
                    },
                    required: vec!["input_file".to_string(), "output_file".to_string(), "text".to_string(), "x".to_string(), "y".to_string()],
                },
            },
            FunctionDeclaration {
                name: "resize_video".to_string(),
                description: "Resizes a video to specified dimensions".to_string(),
                parameters: Parameters {
                    param_type: "object".to_string(),
                    properties: {
                        let mut props = HashMap::new();
                        props.insert("input_file".to_string(), PropertyDefinition {
                            prop_type: "string".to_string(),
                            description: "Path to the input video file".to_string(),
                            items: None,
                        });
                        props.insert("output_file".to_string(), PropertyDefinition {
                            prop_type: "string".to_string(),
                            description: "Path to save the resized video".to_string(),
                            items: None,
                        });
                        props.insert("width".to_string(), PropertyDefinition {
                            prop_type: "number".to_string(),
                            description: "Target width in pixels".to_string(),
                            items: None,
                        });
                        props.insert("height".to_string(), PropertyDefinition {
                            prop_type: "number".to_string(),
                            description: "Target height in pixels".to_string(),
                            items: None,
                        });
                        props
                    },
                    required: vec!["input_file".to_string(), "output_file".to_string(), "width".to_string(), "height".to_string()],
                },
            },
            FunctionDeclaration {
                name: "convert_format".to_string(),
                description: "Converts a video from one format to another".to_string(),
                parameters: Parameters {
                    param_type: "object".to_string(),
                    properties: {
                        let mut props = HashMap::new();
                        props.insert("input_file".to_string(), PropertyDefinition {
                            prop_type: "string".to_string(),
                            description: "Path to the input video file".to_string(),
                            items: None,
                        });
                        props.insert("output_file".to_string(), PropertyDefinition {
                            prop_type: "string".to_string(),
                            description: "Path to save the converted video".to_string(),
                            items: None,
                        });
                        props.insert("format".to_string(), PropertyDefinition {
                            prop_type: "string".to_string(),
                            description: "Target format (e.g., mp4, avi, mov, webm)".to_string(),
                            items: None,
                        });
                        props
                    },
                    required: vec!["input_file".to_string(), "output_file".to_string(), "format".to_string()],
                },
            },
            FunctionDeclaration {
                name: "adjust_volume".to_string(),
                description: "Adjusts the audio volume of a video".to_string(),
                parameters: Parameters {
                    param_type: "object".to_string(),
                    properties: {
                        let mut props = HashMap::new();
                        props.insert("input_file".to_string(), PropertyDefinition {
                            prop_type: "string".to_string(),
                            description: "Path to the input video file".to_string(),
                            items: None,
                        });
                        props.insert("output_file".to_string(), PropertyDefinition {
                            prop_type: "string".to_string(),
                            description: "Path to save the video with adjusted volume".to_string(),
                            items: None,
                        });
                        props.insert("volume_factor".to_string(), PropertyDefinition {
                            prop_type: "number".to_string(),
                            description: "Volume multiplier (1.0 = original, 0.5 = half, 2.0 = double)".to_string(),
                            items: None,
                        });
                        props
                    },
                    required: vec!["input_file".to_string(), "output_file".to_string(), "volume_factor".to_string()],
                },
            },
            // CRITICAL FIX: Add missing apply_filter tool for black and white conversion
            FunctionDeclaration {
                name: "apply_filter".to_string(),
                description: "Applies visual filters to a video including grayscale (black and white), sepia, blur, sharpen, vintage, brightness, contrast, and saturation filters".to_string(),
                parameters: Parameters {
                    param_type: "object".to_string(),
                    properties: {
                        let mut props = HashMap::new();
                        props.insert("input_file".to_string(), PropertyDefinition {
                            prop_type: "string".to_string(),
                            description: "Path to the input video file".to_string(),
                            items: None,
                        });
                        props.insert("output_file".to_string(), PropertyDefinition {
                            prop_type: "string".to_string(),
                            description: "Path to save the filtered video".to_string(),
                            items: None,
                        });
                        props.insert("filter_type".to_string(), PropertyDefinition {
                            prop_type: "string".to_string(),
                            description: "Type of filter to apply: 'grayscale' (black and white), 'sepia', 'blur', 'sharpen', 'vintage', 'brightness', 'contrast', 'saturation'".to_string(),
                            items: None,
                        });
                        props.insert("intensity".to_string(), PropertyDefinition {
                            prop_type: "number".to_string(),
                            description: "Filter intensity from 0.0 to 1.0 (default: 1.0)".to_string(),
                            items: None,
                        });
                        props
                    },
                    required: vec!["input_file".to_string(), "output_file".to_string(), "filter_type".to_string()],
                },
            },
            // Add all remaining missing tools (25 more)
            FunctionDeclaration {
                name: "split_video".to_string(),
                description: "Splits a video into multiple segments of specified duration".to_string(),
                parameters: Parameters {
                    param_type: "object".to_string(),
                    properties: {
                        let mut props = HashMap::new();
                        props.insert("input_file".to_string(), PropertyDefinition {
                            prop_type: "string".to_string(),
                            description: "Path to the input video file".to_string(),
                            items: None,
                        });
                        props.insert("output_prefix".to_string(), PropertyDefinition {
                            prop_type: "string".to_string(),
                            description: "Prefix for output segment files".to_string(),
                            items: None,
                        });
                        props.insert("segment_duration".to_string(), PropertyDefinition {
                            prop_type: "number".to_string(),
                            description: "Duration of each segment in seconds".to_string(),
                            items: None,
                        });
                        props
                    },
                    required: vec!["input_file".to_string(), "output_prefix".to_string(), "segment_duration".to_string()],
                },
            },
            FunctionDeclaration {
                name: "crop_video".to_string(),
                description: "Crops a video to specified dimensions and position".to_string(),
                parameters: Parameters {
                    param_type: "object".to_string(),
                    properties: {
                        let mut props = HashMap::new();
                        props.insert("input_file".to_string(), PropertyDefinition {
                            prop_type: "string".to_string(),
                            description: "Path to the input video file".to_string(),
                            items: None,
                        });
                        props.insert("output_file".to_string(), PropertyDefinition {
                            prop_type: "string".to_string(),
                            description: "Path to save the cropped video".to_string(),
                            items: None,
                        });
                        props.insert("x".to_string(), PropertyDefinition {
                            prop_type: "number".to_string(),
                            description: "X coordinate of crop area".to_string(),
                            items: None,
                        });
                        props.insert("y".to_string(), PropertyDefinition {
                            prop_type: "number".to_string(),
                            description: "Y coordinate of crop area".to_string(),
                            items: None,
                        });
                        props.insert("width".to_string(), PropertyDefinition {
                            prop_type: "number".to_string(),
                            description: "Width of crop area".to_string(),
                            items: None,
                        });
                        props.insert("height".to_string(), PropertyDefinition {
                            prop_type: "number".to_string(),
                            description: "Height of crop area".to_string(),
                            items: None,
                        });
                        props
                    },
                    required: vec!["input_file".to_string(), "output_file".to_string(), "x".to_string(), "y".to_string(), "width".to_string(), "height".to_string()],
                },
            },
            FunctionDeclaration {
                name: "rotate_video".to_string(),
                description: "Rotates a video by specified degrees".to_string(),
                parameters: Parameters {
                    param_type: "object".to_string(),
                    properties: {
                        let mut props = HashMap::new();
                        props.insert("input_file".to_string(), PropertyDefinition {
                            prop_type: "string".to_string(),
                            description: "Path to the input video file".to_string(),
                            items: None,
                        });
                        props.insert("output_file".to_string(), PropertyDefinition {
                            prop_type: "string".to_string(),
                            description: "Path to save the rotated video".to_string(),
                            items: None,
                        });
                        props.insert("degrees".to_string(), PropertyDefinition {
                            prop_type: "number".to_string(),
                            description: "Rotation angle in degrees (90, 180, 270)".to_string(),
                            items: None,
                        });
                        props
                    },
                    required: vec!["input_file".to_string(), "output_file".to_string(), "degrees".to_string()],
                },
            },
            FunctionDeclaration {
                name: "adjust_speed".to_string(),
                description: "Adjusts the playback speed of a video".to_string(),
                parameters: Parameters {
                    param_type: "object".to_string(),
                    properties: {
                        let mut props = HashMap::new();
                        props.insert("input_file".to_string(), PropertyDefinition {
                            prop_type: "string".to_string(),
                            description: "Path to the input video file".to_string(),
                            items: None,
                        });
                        props.insert("output_file".to_string(), PropertyDefinition {
                            prop_type: "string".to_string(),
                            description: "Path to save the speed-adjusted video".to_string(),
                            items: None,
                        });
                        props.insert("speed_factor".to_string(), PropertyDefinition {
                            prop_type: "number".to_string(),
                            description: "Speed multiplier (0.5 = half speed, 2.0 = double speed)".to_string(),
                            items: None,
                        });
                        props
                    },
                    required: vec!["input_file".to_string(), "output_file".to_string(), "speed_factor".to_string()],
                },
            },
            FunctionDeclaration {
                name: "flip_video".to_string(),
                description: "Flips a video horizontally or vertically".to_string(),
                parameters: Parameters {
                    param_type: "object".to_string(),
                    properties: {
                        let mut props = HashMap::new();
                        props.insert("input_file".to_string(), PropertyDefinition {
                            prop_type: "string".to_string(),
                            description: "Path to the input video file".to_string(),
                            items: None,
                        });
                        props.insert("output_file".to_string(), PropertyDefinition {
                            prop_type: "string".to_string(),
                            description: "Path to save the flipped video".to_string(),
                            items: None,
                        });
                        props.insert("direction".to_string(), PropertyDefinition {
                            prop_type: "string".to_string(),
                            description: "Flip direction: 'horizontal' or 'vertical'".to_string(),
                            items: None,
                        });
                        props
                    },
                    required: vec!["input_file".to_string(), "output_file".to_string(), "direction".to_string()],
                },
            },
            FunctionDeclaration {
                name: "add_overlay".to_string(),
                description: "Adds an image or video overlay on top of the main video".to_string(),
                parameters: Parameters {
                    param_type: "object".to_string(),
                    properties: {
                        let mut props = HashMap::new();
                        props.insert("input_file".to_string(), PropertyDefinition {
                            prop_type: "string".to_string(),
                            description: "Path to the input video file".to_string(),
                            items: None,
                        });
                        props.insert("output_file".to_string(), PropertyDefinition {
                            prop_type: "string".to_string(),
                            description: "Path to save the video with overlay".to_string(),
                            items: None,
                        });
                        props.insert("overlay_file".to_string(), PropertyDefinition {
                            prop_type: "string".to_string(),
                            description: "Path to the overlay image or video file".to_string(),
                            items: None,
                        });
                        props.insert("x".to_string(), PropertyDefinition {
                            prop_type: "number".to_string(),
                            description: "X position of the overlay".to_string(),
                            items: None,
                        });
                        props.insert("y".to_string(), PropertyDefinition {
                            prop_type: "number".to_string(),
                            description: "Y position of the overlay".to_string(),
                            items: None,
                        });
                        props
                    },
                    required: vec!["input_file".to_string(), "output_file".to_string(), "overlay_file".to_string(), "x".to_string(), "y".to_string()],
                },
            },
            FunctionDeclaration {
                name: "add_audio".to_string(),
                description: "Adds an audio track to a video or replaces existing audio".to_string(),
                parameters: Parameters {
                    param_type: "object".to_string(),
                    properties: {
                        let mut props = HashMap::new();
                        props.insert("input_file".to_string(), PropertyDefinition {
                            prop_type: "string".to_string(),
                            description: "Path to the input video file".to_string(),
                            items: None,
                        });
                        props.insert("output_file".to_string(), PropertyDefinition {
                            prop_type: "string".to_string(),
                            description: "Path to save the video with new audio".to_string(),
                            items: None,
                        });
                        props.insert("audio_file".to_string(), PropertyDefinition {
                            prop_type: "string".to_string(),
                            description: "Path to the audio file to add".to_string(),
                            items: None,
                        });
                        props.insert("replace".to_string(), PropertyDefinition {
                            prop_type: "boolean".to_string(),
                            description: "Whether to replace existing audio (true) or mix (false)".to_string(),
                            items: None,
                        });
                        props
                    },
                    required: vec!["input_file".to_string(), "output_file".to_string(), "audio_file".to_string()],
                },
            },
            FunctionDeclaration {
                name: "fade_audio".to_string(),
                description: "Applies fade in/out effects to video audio".to_string(),
                parameters: Parameters {
                    param_type: "object".to_string(),
                    properties: {
                        let mut props = HashMap::new();
                        props.insert("input_file".to_string(), PropertyDefinition {
                            prop_type: "string".to_string(),
                            description: "Path to the input video file".to_string(),
                            items: None,
                        });
                        props.insert("output_file".to_string(), PropertyDefinition {
                            prop_type: "string".to_string(),
                            description: "Path to save the video with fade effect".to_string(),
                            items: None,
                        });
                        props.insert("fade_in_duration".to_string(), PropertyDefinition {
                            prop_type: "number".to_string(),
                            description: "Fade in duration in seconds (0 for no fade in)".to_string(),
                            items: None,
                        });
                        props.insert("fade_out_duration".to_string(), PropertyDefinition {
                            prop_type: "number".to_string(),
                            description: "Fade out duration in seconds (0 for no fade out)".to_string(),
                            items: None,
                        });
                        props
                    },
                    required: vec!["input_file".to_string(), "output_file".to_string(), "fade_in_duration".to_string(), "fade_out_duration".to_string()],
                },
            },
            FunctionDeclaration {
                name: "compress_video".to_string(),
                description: "Compresses a video to reduce file size while maintaining quality".to_string(),
                parameters: Parameters {
                    param_type: "object".to_string(),
                    properties: {
                        let mut props = HashMap::new();
                        props.insert("input_file".to_string(), PropertyDefinition {
                            prop_type: "string".to_string(),
                            description: "Path to the input video file".to_string(),
                            items: None,
                        });
                        props.insert("output_file".to_string(), PropertyDefinition {
                            prop_type: "string".to_string(),
                            description: "Path to save the compressed video".to_string(),
                            items: None,
                        });
                        props.insert("quality".to_string(), PropertyDefinition {
                            prop_type: "string".to_string(),
                            description: "Compression quality: 'high', 'medium', 'low'".to_string(),
                            items: None,
                        });
                        props
                    },
                    required: vec!["input_file".to_string(), "output_file".to_string(), "quality".to_string()],
                },
            },
            FunctionDeclaration {
                name: "export_for_platform".to_string(),
                description: "Exports video optimized for specific platforms (YouTube, Instagram, TikTok, etc.)".to_string(),
                parameters: Parameters {
                    param_type: "object".to_string(),
                    properties: {
                        let mut props = HashMap::new();
                        props.insert("input_file".to_string(), PropertyDefinition {
                            prop_type: "string".to_string(),
                            description: "Path to the input video file".to_string(),
                            items: None,
                        });
                        props.insert("output_file".to_string(), PropertyDefinition {
                            prop_type: "string".to_string(),
                            description: "Path to save the platform-optimized video".to_string(),
                            items: None,
                        });
                        props.insert("platform".to_string(), PropertyDefinition {
                            prop_type: "string".to_string(),
                            description: "Target platform: 'youtube', 'instagram', 'tiktok', 'twitter', 'facebook'".to_string(),
                            items: None,
                        });
                        props
                    },
                    required: vec!["input_file".to_string(), "output_file".to_string(), "platform".to_string()],
                },
            },
            FunctionDeclaration {
                name: "picture_in_picture".to_string(),
                description: "Creates a picture-in-picture effect with two video sources".to_string(),
                parameters: Parameters {
                    param_type: "object".to_string(),
                    properties: {
                        let mut props = HashMap::new();
                        props.insert("main_video".to_string(), PropertyDefinition {
                            prop_type: "string".to_string(),
                            description: "Path to the main background video".to_string(),
                            items: None,
                        });
                        props.insert("pip_video".to_string(), PropertyDefinition {
                            prop_type: "string".to_string(),
                            description: "Path to the picture-in-picture video".to_string(),
                            items: None,
                        });
                        props.insert("output_file".to_string(), PropertyDefinition {
                            prop_type: "string".to_string(),
                            description: "Path to save the PiP video".to_string(),
                            items: None,
                        });
                        props.insert("x".to_string(), PropertyDefinition {
                            prop_type: "number".to_string(),
                            description: "X position of the PiP window".to_string(),
                            items: None,
                        });
                        props.insert("y".to_string(), PropertyDefinition {
                            prop_type: "number".to_string(),
                            description: "Y position of the PiP window".to_string(),
                            items: None,
                        });
                        props.insert("scale".to_string(), PropertyDefinition {
                            prop_type: "number".to_string(),
                            description: "Scale factor for PiP window (0.1 to 1.0)".to_string(),
                            items: None,
                        });
                        props
                    },
                    required: vec!["main_video".to_string(), "pip_video".to_string(), "output_file".to_string(), "x".to_string(), "y".to_string(), "scale".to_string()],
                },
            },
            FunctionDeclaration {
                name: "chroma_key".to_string(),
                description: "Applies chroma key (green screen) effect to replace background".to_string(),
                parameters: Parameters {
                    param_type: "object".to_string(),
                    properties: {
                        let mut props = HashMap::new();
                        props.insert("input_file".to_string(), PropertyDefinition {
                            prop_type: "string".to_string(),
                            description: "Path to the input video with green screen".to_string(),
                            items: None,
                        });
                        props.insert("background_file".to_string(), PropertyDefinition {
                            prop_type: "string".to_string(),
                            description: "Path to the background video or image".to_string(),
                            items: None,
                        });
                        props.insert("output_file".to_string(), PropertyDefinition {
                            prop_type: "string".to_string(),
                            description: "Path to save the chroma key video".to_string(),
                            items: None,
                        });
                        props.insert("key_color".to_string(), PropertyDefinition {
                            prop_type: "string".to_string(),
                            description: "Color to key out (default: green)".to_string(),
                            items: None,
                        });
                        props.insert("similarity".to_string(), PropertyDefinition {
                            prop_type: "number".to_string(),
                            description: "Color similarity threshold (0.0 to 1.0)".to_string(),
                            items: None,
                        });
                        props
                    },
                    required: vec!["input_file".to_string(), "background_file".to_string(), "output_file".to_string()],
                },
            },
            FunctionDeclaration {
                name: "split_screen".to_string(),
                description: "Creates a split screen effect with multiple video sources".to_string(),
                parameters: Parameters {
                    param_type: "object".to_string(),
                    properties: {
                        let mut props = HashMap::new();
                        props.insert("video1".to_string(), PropertyDefinition {
                            prop_type: "string".to_string(),
                            description: "Path to the first video".to_string(),
                            items: None,
                        });
                        props.insert("video2".to_string(), PropertyDefinition {
                            prop_type: "string".to_string(),
                            description: "Path to the second video".to_string(),
                            items: None,
                        });
                        props.insert("output_file".to_string(), PropertyDefinition {
                            prop_type: "string".to_string(),
                            description: "Path to save the split screen video".to_string(),
                            items: None,
                        });
                        props.insert("orientation".to_string(), PropertyDefinition {
                            prop_type: "string".to_string(),
                            description: "Split orientation: 'horizontal' or 'vertical'".to_string(),
                            items: None,
                        });
                        props
                    },
                    required: vec!["video1".to_string(), "video2".to_string(), "output_file".to_string(), "orientation".to_string()],
                },
            },
            FunctionDeclaration {
                name: "scale_video".to_string(),
                description: "Scales a video by a specific factor while maintaining aspect ratio".to_string(),
                parameters: Parameters {
                    param_type: "object".to_string(),
                    properties: {
                        let mut props = HashMap::new();
                        props.insert("input_file".to_string(), PropertyDefinition {
                            prop_type: "string".to_string(),
                            description: "Path to the input video file".to_string(),
                            items: None,
                        });
                        props.insert("output_file".to_string(), PropertyDefinition {
                            prop_type: "string".to_string(),
                            description: "Path to save the scaled video".to_string(),
                            items: None,
                        });
                        props.insert("scale_factor".to_string(), PropertyDefinition {
                            prop_type: "number".to_string(),
                            description: "Scale factor (0.5 = half size, 2.0 = double size)".to_string(),
                            items: None,
                        });
                        props
                    },
                    required: vec!["input_file".to_string(), "output_file".to_string(), "scale_factor".to_string()],
                },
            },
            FunctionDeclaration {
                name: "stabilize_video".to_string(),
                description: "Applies video stabilization to reduce camera shake".to_string(),
                parameters: Parameters {
                    param_type: "object".to_string(),
                    properties: {
                        let mut props = HashMap::new();
                        props.insert("input_file".to_string(), PropertyDefinition {
                            prop_type: "string".to_string(),
                            description: "Path to the input video file".to_string(),
                            items: None,
                        });
                        props.insert("output_file".to_string(), PropertyDefinition {
                            prop_type: "string".to_string(),
                            description: "Path to save the stabilized video".to_string(),
                            items: None,
                        });
                        props.insert("strength".to_string(), PropertyDefinition {
                            prop_type: "number".to_string(),
                            description: "Stabilization strength (1-10, higher = more stabilization)".to_string(),
                            items: None,
                        });
                        props
                    },
                    required: vec!["input_file".to_string(), "output_file".to_string(), "strength".to_string()],
                },
            },
            FunctionDeclaration {
                name: "create_thumbnail".to_string(),
                description: "Creates a thumbnail image from a video at specified time".to_string(),
                parameters: Parameters {
                    param_type: "object".to_string(),
                    properties: {
                        let mut props = HashMap::new();
                        props.insert("input_file".to_string(), PropertyDefinition {
                            prop_type: "string".to_string(),
                            description: "Path to the input video file".to_string(),
                            items: None,
                        });
                        props.insert("output_file".to_string(), PropertyDefinition {
                            prop_type: "string".to_string(),
                            description: "Path to save the thumbnail image".to_string(),
                            items: None,
                        });
                        props.insert("timestamp".to_string(), PropertyDefinition {
                            prop_type: "number".to_string(),
                            description: "Time in seconds to capture thumbnail".to_string(),
                            items: None,
                        });
                        props.insert("width".to_string(), PropertyDefinition {
                            prop_type: "number".to_string(),
                            description: "Thumbnail width in pixels".to_string(),
                            items: None,
                        });
                        props.insert("height".to_string(), PropertyDefinition {
                            prop_type: "number".to_string(),
                            description: "Thumbnail height in pixels".to_string(),
                            items: None,
                        });
                        props
                    },
                    required: vec!["input_file".to_string(), "output_file".to_string(), "timestamp".to_string()],
                },
            },
            FunctionDeclaration {
                name: "adjust_color".to_string(),
                description: "Adjusts color properties like brightness, contrast, saturation, and hue".to_string(),
                parameters: Parameters {
                    param_type: "object".to_string(),
                    properties: {
                        let mut props = HashMap::new();
                        props.insert("input_file".to_string(), PropertyDefinition {
                            prop_type: "string".to_string(),
                            description: "Path to the input video file".to_string(),
                            items: None,
                        });
                        props.insert("output_file".to_string(), PropertyDefinition {
                            prop_type: "string".to_string(),
                            description: "Path to save the color-adjusted video".to_string(),
                            items: None,
                        });
                        props.insert("brightness".to_string(), PropertyDefinition {
                            prop_type: "number".to_string(),
                            description: "Brightness adjustment (-1.0 to 1.0, 0 = no change)".to_string(),
                            items: None,
                        });
                        props.insert("contrast".to_string(), PropertyDefinition {
                            prop_type: "number".to_string(),
                            description: "Contrast adjustment (-1.0 to 1.0, 0 = no change)".to_string(),
                            items: None,
                        });
                        props.insert("saturation".to_string(), PropertyDefinition {
                            prop_type: "number".to_string(),
                            description: "Saturation adjustment (-1.0 to 1.0, 0 = no change)".to_string(),
                            items: None,
                        });
                        props.insert("hue".to_string(), PropertyDefinition {
                            prop_type: "number".to_string(),
                            description: "Hue adjustment in degrees (-180 to 180, 0 = no change)".to_string(),
                            items: None,
                        });
                        props
                    },
                    required: vec!["input_file".to_string(), "output_file".to_string()],
                },
            },
            FunctionDeclaration {
                name: "add_subtitles".to_string(),
                description: "Adds subtitles to a video from a text file or inline text".to_string(),
                parameters: Parameters {
                    param_type: "object".to_string(),
                    properties: {
                        let mut props = HashMap::new();
                        props.insert("input_file".to_string(), PropertyDefinition {
                            prop_type: "string".to_string(),
                            description: "Path to the input video file".to_string(),
                            items: None,
                        });
                        props.insert("output_file".to_string(), PropertyDefinition {
                            prop_type: "string".to_string(),
                            description: "Path to save the video with subtitles".to_string(),
                            items: None,
                        });
                        props.insert("subtitle_text".to_string(), PropertyDefinition {
                            prop_type: "string".to_string(),
                            description: "Subtitle text or path to subtitle file (.srt, .vtt)".to_string(),
                            items: None,
                        });
                        props.insert("font_size".to_string(), PropertyDefinition {
                            prop_type: "number".to_string(),
                            description: "Font size for subtitles (default: 20)".to_string(),
                            items: None,
                        });
                        props.insert("color".to_string(), PropertyDefinition {
                            prop_type: "string".to_string(),
                            description: "Subtitle color (default: white)".to_string(),
                            items: None,
                        });
                        props
                    },
                    required: vec!["input_file".to_string(), "output_file".to_string(), "subtitle_text".to_string()],
                },
            },
            FunctionDeclaration {
                name: "pexels_search".to_string(),
                description: "Searches Pexels for stock videos and images based on query".to_string(),
                parameters: Parameters {
                    param_type: "object".to_string(),
                    properties: {
                        let mut props = HashMap::new();
                        props.insert("query".to_string(), PropertyDefinition {
                            prop_type: "string".to_string(),
                            description: "Search query for stock content".to_string(),
                            items: None,
                        });
                        props.insert("media_type".to_string(), PropertyDefinition {
                            prop_type: "string".to_string(),
                            description: "Media type to search: 'videos' or 'photos'".to_string(),
                            items: None,
                        });
                        props.insert("per_page".to_string(), PropertyDefinition {
                            prop_type: "number".to_string(),
                            description: "Number of results to return (1-80, default: 15)".to_string(),
                            items: None,
                        });
                        props
                    },
                    required: vec!["query".to_string(), "media_type".to_string()],
                },
            },
            FunctionDeclaration {
                name: "sketchfab_search".to_string(),
                description: "Searches Sketchfab for 3D models by keyword. Use to find glTF/GLB models for Blender scenes, backgrounds, and props. Returns model UID, name, author, likes, and viewer URL.".to_string(),
                parameters: Parameters {
                    param_type: "object".to_string(),
                    properties: {
                        let mut props = HashMap::new();
                        props.insert("query".to_string(), PropertyDefinition {
                            prop_type: "string".to_string(),
                            description: "Search keyword for 3D models (e.g. 'robot', 'ocean', 'medieval castle')".to_string(),
                            items: None,
                        });
                        props.insert("categories".to_string(), PropertyDefinition {
                            prop_type: "string".to_string(),
                            description: "Optional. Filter by category slug (comma-separated). E.g. 'animals-pets,architecture,characters,science-technology,transport'.".to_string(),
                            items: None,
                        });
                        props.insert("animated".to_string(), PropertyDefinition {
                            prop_type: "boolean".to_string(),
                            description: "Optional. Filter for animated models only (true/false)".to_string(),
                            items: None,
                        });
                        props.insert("sort_by".to_string(), PropertyDefinition {
                            prop_type: "string".to_string(),
                            description: "Optional. Sort order: '-likeCount' (most liked), '-viewCount' (most viewed), '-createdAt' (newest), '-publishedAt' (recently published)".to_string(),
                            items: None,
                        });
                        props
                    },
                    required: vec!["query".to_string()],
                },
            },
            FunctionDeclaration {
                name: "sketchfab_get_model".to_string(),
                description: "Gets detailed information about a specific Sketchfab 3D model by UID. Returns full metadata including vertex/face count, tags, license, and animation info.".to_string(),
                parameters: Parameters {
                    param_type: "object".to_string(),
                    properties: {
                        let mut props = HashMap::new();
                        props.insert("uid".to_string(), PropertyDefinition {
                            prop_type: "string".to_string(),
                            description: "The model UID from a previous sketchfab_search result".to_string(),
                            items: None,
                        });
                        props
                    },
                    required: vec!["uid".to_string()],
                },
            },
            FunctionDeclaration {
                name: "sketchfab_download".to_string(),
                description: "Downloads a 3D model from Sketchfab by UID and uploads it to R2 cloud storage. Returns a permanent cloud URL for use in Blender scenes.".to_string(),
                parameters: Parameters {
                    param_type: "object".to_string(),
                    properties: {
                        let mut props = HashMap::new();
                        props.insert("uid".to_string(), PropertyDefinition {
                            prop_type: "string".to_string(),
                            description: "The model UID from a previous sketchfab_search result".to_string(),
                            items: None,
                        });
                        props.insert("format".to_string(), PropertyDefinition {
                            prop_type: "string".to_string(),
                            description: "Optional. Download format: 'gltf' (ZIP with .gltf + textures, default), 'usdz', or 'source' (original format if available)".to_string(),
                            items: None,
                        });
                        props
                    },
                    required: vec!["uid".to_string()],
                },
            },
            FunctionDeclaration {
                name: "analyze_image".to_string(),
                description: "Analyzes an image and provides detailed description using AI".to_string(),
                parameters: Parameters {
                    param_type: "object".to_string(),
                    properties: {
                        let mut props = HashMap::new();
                        props.insert("image_path".to_string(), PropertyDefinition {
                            prop_type: "string".to_string(),
                            description: "Path to the image file to analyze".to_string(),
                            items: None,
                        });
                        props.insert("analysis_type".to_string(), PropertyDefinition {
                            prop_type: "string".to_string(),
                            description: "Type of analysis: 'general', 'detailed', 'objects', 'colors'".to_string(),
                            items: None,
                        });
                        props
                    },
                    required: vec!["image_path".to_string()],
                },
            },
            FunctionDeclaration {
                name: "generate_text_to_speech".to_string(),
                description: "Generates speech audio from text using Eleven Labs TTS (with Gemini fallback). Supports 17+ premium voices with ultra-low latency (75ms). Perfect for narration, voiceovers, and character voices.".to_string(),
                parameters: Parameters {
                    param_type: "object".to_string(),
                    properties: {
                        let mut props = HashMap::new();
                        props.insert("text".to_string(), PropertyDefinition {
                            prop_type: "string".to_string(),
                            description: "Text to convert to speech".to_string(),
                            items: None,
                        });
                        props.insert("output_file".to_string(), PropertyDefinition {
                            prop_type: "string".to_string(),
                            description: "Path to save the generated audio file (e.g., 'outputs/narration.mp3')".to_string(),
                            items: None,
                        });
                        props.insert("voice".to_string(), PropertyDefinition {
                            prop_type: "string".to_string(),
                            description: "Voice name: Rachel (default, young female), Drew (male, news), Clyde (male, veteran), Bella (female, soft), Emily (female, calm), Adam (male, deep), Paul (male, reporter), Domi (female, strong), Elli (female, emotional), Grace (female, young), Matilda (female, warm), Arnold (male, crisp), Callum (male, hoarse), Daniel (male, deep), Ethan (male, young), Liam (male, articulate), Thomas (male, calm)".to_string(),
                            items: None,
                        });
                        props.insert("model".to_string(), PropertyDefinition {
                            prop_type: "string".to_string(),
                            description: "Model: 'eleven_flash_v2_5' (75ms latency, default), 'eleven_multilingual_v2' (highest quality), 'eleven_turbo_v2_5' (fast)".to_string(),
                            items: None,
                        });
                        props
                    },
                    required: vec!["text".to_string(), "output_file".to_string()],
                },
            },
            FunctionDeclaration {
                name: "generate_sound_effect".to_string(),
                description: "Generates custom sound effects from text descriptions using Eleven Labs. Create cinematic sound design, Foley, ambient sounds, impacts, transitions, etc. Duration: 0.5-30 seconds.".to_string(),
                parameters: Parameters {
                    param_type: "object".to_string(),
                    properties: {
                        let mut props = HashMap::new();
                        props.insert("description".to_string(), PropertyDefinition {
                            prop_type: "string".to_string(),
                            description: "Detailed description of the sound effect (e.g., 'cinematic explosion with rumble', 'door creaking slowly')".to_string(),
                            items: None,
                        });
                        props.insert("output_file".to_string(), PropertyDefinition {
                            prop_type: "string".to_string(),
                            description: "Path to save the sound effect (e.g., 'outputs/explosion.mp3')".to_string(),
                            items: None,
                        });
                        props.insert("duration_seconds".to_string(), PropertyDefinition {
                            prop_type: "number".to_string(),
                            description: "Duration in seconds (0.5-30, default: 5)".to_string(),
                            items: None,
                        });
                        props.insert("prompt_influence".to_string(), PropertyDefinition {
                            prop_type: "number".to_string(),
                            description: "How closely to follow prompt (0-1, default: 0.5). Higher = more precise".to_string(),
                            items: None,
                        });
                        props
                    },
                    required: vec!["description".to_string(), "output_file".to_string()],
                },
            },
            FunctionDeclaration {
                name: "generate_music".to_string(),
                description: "Generates studio-grade background music from text prompts using Eleven Music. Create music in any genre, mood, style. Supports custom structure, lyrics, tempo. Commercial use cleared. Duration: 10-300 seconds.".to_string(),
                parameters: Parameters {
                    param_type: "object".to_string(),
                    properties: {
                        let mut props = HashMap::new();
                        props.insert("prompt".to_string(), PropertyDefinition {
                            prop_type: "string".to_string(),
                            description: "Music description (e.g., 'upbeat electronic dance music 120 BPM', 'peaceful piano meditation', 'epic cinematic orchestral with drums'). Can include genre, mood, instruments, tempo, structure, lyrics.".to_string(),
                            items: None,
                        });
                        props.insert("output_file".to_string(), PropertyDefinition {
                            prop_type: "string".to_string(),
                            description: "Path to save the music file (e.g., 'outputs/background_music.mp3')".to_string(),
                            items: None,
                        });
                        props.insert("duration_seconds".to_string(), PropertyDefinition {
                            prop_type: "number".to_string(),
                            description: "Music duration in seconds (10-300, default: 30)".to_string(),
                            items: None,
                        });
                        props
                    },
                    required: vec!["prompt".to_string(), "output_file".to_string()],
                },
            },
            FunctionDeclaration {
                name: "add_voiceover_to_video".to_string(),
                description: "Convenience tool that generates voiceover speech and adds it to a video in one step. Combines text-to-speech generation with audio mixing automatically.".to_string(),
                parameters: Parameters {
                    param_type: "object".to_string(),
                    properties: {
                        let mut props = HashMap::new();
                        props.insert("input_video".to_string(), PropertyDefinition {
                            prop_type: "string".to_string(),
                            description: "Path to the input video file".to_string(),
                            items: None,
                        });
                        props.insert("voiceover_text".to_string(), PropertyDefinition {
                            prop_type: "string".to_string(),
                            description: "Text for the voiceover narration".to_string(),
                            items: None,
                        });
                        props.insert("output_video".to_string(), PropertyDefinition {
                            prop_type: "string".to_string(),
                            description: "Path to save the video with voiceover (e.g., 'outputs/narrated_video.mp4')".to_string(),
                            items: None,
                        });
                        props.insert("voice".to_string(), PropertyDefinition {
                            prop_type: "string".to_string(),
                            description: "Voice name (same as generate_text_to_speech, default: Rachel)".to_string(),
                            items: None,
                        });
                        props
                    },
                    required: vec!["input_video".to_string(), "voiceover_text".to_string(), "output_video".to_string()],
                },
            },
            FunctionDeclaration {
                name: "transcribe_audio_url".to_string(),
                description: "Transcribes speech from a public audio URL using the shared VibeVoice transcription service. Useful for voice notes, podcast clips, narration drafts, interviews, and subtitle prep.".to_string(),
                parameters: Parameters {
                    param_type: "object".to_string(),
                    properties: {
                        let mut props = HashMap::new();
                        props.insert("audio_url".to_string(), PropertyDefinition {
                            prop_type: "string".to_string(),
                            description: "Publicly accessible audio URL to transcribe".to_string(),
                            items: None,
                        });
                        props.insert("language".to_string(), PropertyDefinition {
                            prop_type: "string".to_string(),
                            description: "Optional language hint such as 'en', 'sw', or 'fr'".to_string(),
                            items: None,
                        });
                        props.insert("hotwords".to_string(), PropertyDefinition {
                            prop_type: "array".to_string(),
                            description: "Optional list of terms, names, or product words to bias the transcription toward".to_string(),
                            items: Some(Box::new(PropertyDefinition {
                                prop_type: "string".to_string(),
                                description: "Hotword".to_string(),
                                items: None,
                            })),
                        });
                        props
                    },
                    required: vec!["audio_url".to_string()],
                },
            },
            FunctionDeclaration {
                name: "generate_video_script".to_string(),
                description: "Generates a video script based on topic and requirements using AI".to_string(),
                parameters: Parameters {
                    param_type: "object".to_string(),
                    properties: {
                        let mut props = HashMap::new();
                        props.insert("topic".to_string(), PropertyDefinition {
                            prop_type: "string".to_string(),
                            description: "Topic or theme for the video script".to_string(),
                            items: None,
                        });
                        props.insert("duration".to_string(), PropertyDefinition {
                            prop_type: "number".to_string(),
                            description: "Target video duration in seconds".to_string(),
                            items: None,
                        });
                        props.insert("style".to_string(), PropertyDefinition {
                            prop_type: "string".to_string(),
                            description: "Script style: 'educational', 'entertainment', 'commercial', 'documentary'".to_string(),
                            items: None,
                        });
                        props.insert("tone".to_string(), PropertyDefinition {
                            prop_type: "string".to_string(),
                            description: "Script tone: 'casual', 'professional', 'humorous', 'serious'".to_string(),
                            items: None,
                        });
                        props
                    },
                    required: vec!["topic".to_string(), "duration".to_string()],
                },
            },
            FunctionDeclaration {
                name: "create_blank_video".to_string(),
                description: "Creates a blank video with specified color, duration, and dimensions".to_string(),
                parameters: Parameters {
                    param_type: "object".to_string(),
                    properties: {
                        let mut props = HashMap::new();
                        props.insert("output_file".to_string(), PropertyDefinition {
                            prop_type: "string".to_string(),
                            description: "Path to save the blank video".to_string(),
                            items: None,
                        });
                        props.insert("duration".to_string(), PropertyDefinition {
                            prop_type: "number".to_string(),
                            description: "Duration of the blank video in seconds".to_string(),
                            items: None,
                        });
                        props.insert("width".to_string(), PropertyDefinition {
                            prop_type: "number".to_string(),
                            description: "Video width in pixels".to_string(),
                            items: None,
                        });
                        props.insert("height".to_string(), PropertyDefinition {
                            prop_type: "number".to_string(),
                            description: "Video height in pixels".to_string(),
                            items: None,
                        });
                        props.insert("color".to_string(), PropertyDefinition {
                            prop_type: "string".to_string(),
                            description: "Background color (hex code or color name, default: black)".to_string(),
                            items: None,
                        });
                        props
                    },
                    required: vec!["output_file".to_string(), "duration".to_string(), "width".to_string(), "height".to_string()],
                },
            },
            FunctionDeclaration {
                name: "pexels_download_video".to_string(),
                description: "Downloads a video from Pexels given the video file URL".to_string(),
                parameters: Parameters {
                    param_type: "object".to_string(),
                    properties: {
                        let mut props = HashMap::new();
                        props.insert("video_url".to_string(), PropertyDefinition {
                            prop_type: "string".to_string(),
                            description: "Pexels video file URL (from pexels_search results)".to_string(),
                            items: None,
                        });
                        props.insert("output_file".to_string(), PropertyDefinition {
                            prop_type: "string".to_string(),
                            description: "Local path to save the downloaded video".to_string(),
                            items: None,
                        });
                        props.insert("quality".to_string(), PropertyDefinition {
                            prop_type: "string".to_string(),
                            description: "Video quality: 'hd', 'sd', 'low' (optional)".to_string(),
                            items: None,
                        });
                        props
                    },
                    required: vec!["video_url".to_string(), "output_file".to_string()],
                },
            },
            FunctionDeclaration {
                name: "pexels_download_photo".to_string(),
                description: "Downloads a photo from Pexels given the photo URL".to_string(),
                parameters: Parameters {
                    param_type: "object".to_string(),
                    properties: {
                        let mut props = HashMap::new();
                        props.insert("photo_url".to_string(), PropertyDefinition {
                            prop_type: "string".to_string(),
                            description: "Pexels photo URL (from pexels_search results)".to_string(),
                            items: None,
                        });
                        props.insert("output_file".to_string(), PropertyDefinition {
                            prop_type: "string".to_string(),
                            description: "Local path to save the downloaded photo".to_string(),
                            items: None,
                        });
                        props.insert("size".to_string(), PropertyDefinition {
                            prop_type: "string".to_string(),
                            description: "Photo size: 'original', 'large', 'medium', 'small' (optional)".to_string(),
                            items: None,
                        });
                        props
                    },
                    required: vec!["photo_url".to_string(), "output_file".to_string()],
                },
            },
            FunctionDeclaration {
                name: "pexels_get_trending".to_string(),
                description: "Gets trending/popular videos from Pexels without needing a search query".to_string(),
                parameters: Parameters {
                    param_type: "object".to_string(),
                    properties: {
                        let mut props = HashMap::new();
                        props.insert("per_page".to_string(), PropertyDefinition {
                            prop_type: "number".to_string(),
                            description: "Number of results to return (1-80, default: 15)".to_string(),
                            items: None,
                        });
                        props
                    },
                    required: vec![],
                },
            },
            FunctionDeclaration {
                name: "pexels_get_curated".to_string(),
                description: "Gets curated/hand-picked photos from Pexels without needing a search query".to_string(),
                parameters: Parameters {
                    param_type: "object".to_string(),
                    properties: {
                        let mut props = HashMap::new();
                        props.insert("per_page".to_string(), PropertyDefinition {
                            prop_type: "number".to_string(),
                            description: "Number of results to return (1-80, default: 15)".to_string(),
                            items: None,
                        });
                        props
                    },
                    required: vec![],
                },
            },
            FunctionDeclaration {
                name: "view_video".to_string(),
                description: "Views/analyzes a video by retrieving its vectorized embeddings from the database. This allows you to 'see' what's in a video without re-processing it. Use this to understand video content, verify edits, or check what a previously generated video contains. Returns detailed frame-by-frame analysis and overall summary. Accepts either a local path or a cloud URL (prefixed with https://).".to_string(),
                parameters: Parameters {
                    param_type: "object".to_string(),
                    properties: {
                        let mut props = HashMap::new();
                        props.insert("video_path".to_string(), PropertyDefinition {
                            prop_type: "string".to_string(),
                            description: "Path or cloud URL to the video file to view/analyze (e.g., 'outputs/edited_video.mp4' or an https:// cloud URL)".to_string(),
                            items: None,
                        });
                        props
                    },
                    required: vec!["video_path".to_string()],
                },
            },
            FunctionDeclaration {
                name: "review_video".to_string(),
                description: "Reviews an output video to verify it meets the user's original requirements. Use this in the final stage of video editing/generation to confirm quality before presenting to the user. Compares the video's vectorized analysis against the user's request to check if edits were applied correctly.".to_string(),
                parameters: Parameters {
                    param_type: "object".to_string(),
                    properties: {
                        let mut props = HashMap::new();
                        props.insert("video_path".to_string(), PropertyDefinition {
                            prop_type: "string".to_string(),
                            description: "Path or cloud URL to the output video to review (e.g., local path or https:// cloud URL)".to_string(),
                            items: None,
                        });
                        props.insert("original_request".to_string(), PropertyDefinition {
                            prop_type: "string".to_string(),
                            description: "The original user request/requirements to verify against".to_string(),
                            items: None,
                        });
                        props.insert("expected_features".to_string(), PropertyDefinition {
                            prop_type: "array".to_string(),
                            description: "List of expected features that should be present (e.g., ['grayscale filter', 'text overlay', 'trimmed to 10s'])".to_string(),
                            items: Some(Box::new(PropertyDefinition {
                                prop_type: "string".to_string(),
                                description: "Expected feature".to_string(),
                                items: None,
                            })),
                        });
                        props
                    },
                    required: vec!["video_path".to_string(), "original_request".to_string()],
                },
            },
            FunctionDeclaration {
                name: "view_image".to_string(),
                description: "Views/analyzes an image file using AI vision. Use this to verify generated images, inspect stock photos from Pexels, or check overlay images before using them in videos. Returns detailed analysis of content, colors, composition, style, and suitability for video use. Accepts either a local path or a cloud URL (prefixed with https://).".to_string(),
                parameters: Parameters {
                    param_type: "object".to_string(),
                    properties: {
                        let mut props = HashMap::new();
                        props.insert("image_path".to_string(), PropertyDefinition {
                            prop_type: "string".to_string(),
                            description: "Path or cloud URL to the image file to view/analyze (e.g., 'outputs/generated_logo.png' or an https:// cloud URL)".to_string(),
                            items: None,
                        });
                        props
                    },
                    required: vec!["image_path".to_string()],
                },
            },
            FunctionDeclaration {
                name: "generate_image".to_string(),
                description: "Generates an image from scratch using Google's Gemini image model based on a text prompt. Use when you need to create a custom image that doesn't exist yet — e.g. branded backgrounds, custom overlay graphics, title cards, logos. For editing an existing image file, use edit_image instead.".to_string(),
                parameters: Parameters {
                    param_type: "object".to_string(),
                    properties: {
                        let mut props = HashMap::new();
                        props.insert("prompt".to_string(), PropertyDefinition {
                            prop_type: "string".to_string(),
                            description: "Detailed text description of the image to generate. Be specific about style, lighting, composition, and details.".to_string(),
                            items: None,
                        });
                        props.insert("output_file".to_string(), PropertyDefinition {
                            prop_type: "string".to_string(),
                            description: "Path where the generated image should be saved (e.g., 'outputs/generated_overlay.png')".to_string(),
                            items: None,
                        });
                        props.insert("aspect_ratio".to_string(), PropertyDefinition {
                            prop_type: "string".to_string(),
                            description: "Aspect ratio: '1:1', '16:9', '9:16', '4:3', '3:4' (default: '1:1')".to_string(),
                            items: None,
                        });
                        props.insert("image_size".to_string(), PropertyDefinition {
                            prop_type: "string".to_string(),
                            description: "Resolution: '1K' (1024px), '2K' (2048px), '4K' (4096px) (default: '2K')".to_string(),
                            items: None,
                        });
                        props.insert("model".to_string(), PropertyDefinition {
                            prop_type: "string".to_string(),
                            description: "Model to use: 'fast'/'nano' for quick generation, 'quality'/'pro' for best results (default: 'quality'). Or pass an explicit Gemini model ID.".to_string(),
                            items: None,
                        });
                        props
                    },
                    required: vec!["prompt".to_string(), "output_file".to_string()],
                },
            },
            FunctionDeclaration {
                name: "text_to_image".to_string(),
                description: "Generates an image from a text prompt. Alias for generate_image — accepts 'text' instead of 'prompt' and supports 'number_of_images' for multiples. Produces a single image by default.".to_string(),
                parameters: Parameters {
                    param_type: "object".to_string(),
                    properties: {
                        let mut props = HashMap::new();
                        props.insert("prompt".to_string(), PropertyDefinition {
                            prop_type: "string".to_string(),
                            description: "Text prompt describing the image to generate.".to_string(),
                            items: None,
                        });
                        props.insert("text".to_string(), PropertyDefinition {
                            prop_type: "string".to_string(),
                            description: "Alternative to 'prompt' — text describing the image to generate.".to_string(),
                            items: None,
                        });
                        props.insert("output_file".to_string(), PropertyDefinition {
                            prop_type: "string".to_string(),
                            description: "Path where the generated image should be saved (default: 'outputs/text_to_image.png')".to_string(),
                            items: None,
                        });
                        props.insert("number_of_images".to_string(), PropertyDefinition {
                            prop_type: "integer".to_string(),
                            description: "How many images to generate (default: 1). When >1, the prompt is repeated that many times.".to_string(),
                            items: None,
                        });
                        props.insert("aspect_ratio".to_string(), PropertyDefinition {
                            prop_type: "string".to_string(),
                            description: "Aspect ratio: '1:1', '16:9', '9:16', '4:3', '3:4' (default: '1:1')".to_string(),
                            items: None,
                        });
                        props.insert("image_size".to_string(), PropertyDefinition {
                            prop_type: "string".to_string(),
                            description: "Resolution: '1K' (1024px), '2K' (2048px), '4K' (4096px) (default: '2K')".to_string(),
                            items: None,
                        });
                        props.insert("model".to_string(), PropertyDefinition {
                            prop_type: "string".to_string(),
                            description: "Model: 'fast'/'nano' for speed, 'quality'/'pro' for best results (default: 'quality')".to_string(),
                            items: None,
                        });
                        props
                    },
                    required: vec![],
                },
            },
            FunctionDeclaration {
                name: "edit_image".to_string(),
                description: "Edit or transform an existing image using AI. Use when you need to: modify a downloaded Pexels photo, add text/graphics to a video frame, change the style of an image, remove or replace elements, or create a variant of an existing image. Requires a path to the source image on disk. Example: extract a frame with extract_frames, then call edit_image to add a title overlay.".to_string(),
                parameters: Parameters {
                    param_type: "object".to_string(),
                    properties: {
                        let mut props = HashMap::new();
                        props.insert("input_image".to_string(), PropertyDefinition {
                            prop_type: "string".to_string(),
                            description: "Path to the source image file to edit (e.g., 'outputs/frame.jpg' or 'outputs/pexels_photo.jpg')".to_string(),
                            items: None,
                        });
                        props.insert("prompt".to_string(), PropertyDefinition {
                            prop_type: "string".to_string(),
                            description: "Instructions describing what edits to make (e.g., 'add bold white title text at the top saying VIDEOSYNC', 'make it look cinematic with warm tones', 'remove the background')".to_string(),
                            items: None,
                        });
                        props.insert("output_file".to_string(), PropertyDefinition {
                            prop_type: "string".to_string(),
                            description: "Path where the edited image should be saved (e.g., 'outputs/edited_overlay.jpg')".to_string(),
                            items: None,
                        });
                        props.insert("aspect_ratio".to_string(), PropertyDefinition {
                            prop_type: "string".to_string(),
                            description: "Output aspect ratio: '1:1', '16:9', '9:16', '4:3', '3:4' (default: '16:9')".to_string(),
                            items: None,
                        });
                        props.insert("model".to_string(), PropertyDefinition {
                            prop_type: "string".to_string(),
                            description: "Model to use: 'fast'/'nano' for quick edits, 'quality'/'pro' for best results (default: 'quality')".to_string(),
                            items: None,
                        });
                        props
                    },
                    required: vec!["input_image".to_string(), "prompt".to_string(), "output_file".to_string()],
                },
            },
            // ── Option A: Agentic video generation pipeline tools ──────────────
            FunctionDeclaration {
                name: "generate_video_queries".to_string(),
                description: "Generate diverse Pexels search queries from a high-level video topic. Use as the FIRST step when building a video from scratch via the agentic pipeline: generate_video_queries → pexels_search → analyze_pexels_thumbnail → pexels_download_video → verify_clip_quality_tool → merge_videos → run_video_qa.".to_string(),
                parameters: Parameters {
                    param_type: "object".to_string(),
                    properties: {
                        let mut props = HashMap::new();
                        props.insert("topic".to_string(), PropertyDefinition {
                            prop_type: "string".to_string(),
                            description: "The video topic or concept (e.g. 'sunrise over mountain peaks')".to_string(),
                            items: None,
                        });
                        props.insert("num_queries".to_string(), PropertyDefinition {
                            prop_type: "number".to_string(),
                            description: "How many search queries to generate (default: 5)".to_string(),
                            items: None,
                        });
                        props
                    },
                    required: vec!["topic".to_string()],
                },
            },
            FunctionDeclaration {
                name: "analyze_pexels_thumbnail".to_string(),
                description: "Download a Pexels thumbnail URL and score its relevance to the video topic using Gemini vision (1-10). Use AFTER pexels_search, on each result's video_pictures[0].picture URL, BEFORE pexels_download_video. Score >= 5 → proceed; score < 5 → skip.".to_string(),
                parameters: Parameters {
                    param_type: "object".to_string(),
                    properties: {
                        let mut props = HashMap::new();
                        props.insert("thumbnail_url".to_string(), PropertyDefinition {
                            prop_type: "string".to_string(),
                            description: "The Pexels thumbnail image URL from video_pictures[0].picture".to_string(),
                            items: None,
                        });
                        props.insert("topic".to_string(), PropertyDefinition {
                            prop_type: "string".to_string(),
                            description: "Your video topic — used to judge relevance".to_string(),
                            items: None,
                        });
                        props
                    },
                    required: vec!["thumbnail_url".to_string(), "topic".to_string()],
                },
            },
            FunctionDeclaration {
                name: "verify_clip_quality_tool".to_string(),
                description: "Run FFmpeg quality checks on a downloaded video clip: duration > 1s, no frozen frames, no black frames. Use AFTER pexels_download_video before adding clip to merge list.".to_string(),
                parameters: Parameters {
                    param_type: "object".to_string(),
                    properties: {
                        let mut props = HashMap::new();
                        props.insert("file_path".to_string(), PropertyDefinition {
                            prop_type: "string".to_string(),
                            description: "Path to the downloaded clip to verify".to_string(),
                            items: None,
                        });
                        props
                    },
                    required: vec!["file_path".to_string()],
                },
            },
            FunctionDeclaration {
                name: "run_video_qa".to_string(),
                description: "Run a full automated QA suite on a finished video using FFmpeg signal analysis. Returns duration, resolution, FPS, audio presence, frozen frames, black frames, and scene change count. Use AFTER merge_videos and BEFORE presenting to the user.".to_string(),
                parameters: Parameters {
                    param_type: "object".to_string(),
                    properties: {
                        let mut props = HashMap::new();
                        props.insert("file_path".to_string(), PropertyDefinition {
                            prop_type: "string".to_string(),
                            description: "Path to the finished video file to QA".to_string(),
                            items: None,
                        });
                        props
                    },
                    required: vec!["file_path".to_string()],
                },
            },
            FunctionDeclaration {
                name: "auto_generate_video".to_string(),
                description: "Orchestrates automatic video generation from a topic/prompt. This high-level tool searches Pexels for stock footage, generates custom images with Nano Banana Pro, downloads clips, merges them, adds text overlays, music, and exports a complete video. Perfect for creating videos from scratch.".to_string(),
                parameters: Parameters {
                    param_type: "object".to_string(),
                    properties: {
                        let mut props = HashMap::new();
                        props.insert("topic".to_string(), PropertyDefinition {
                            prop_type: "string".to_string(),
                            description: "Topic or description of the video to create (e.g., 'A motivational video about success')".to_string(),
                            items: None,
                        });
                        props.insert("output_file".to_string(), PropertyDefinition {
                            prop_type: "string".to_string(),
                            description: "Path where the final video should be saved".to_string(),
                            items: None,
                        });
                        props.insert("duration".to_string(), PropertyDefinition {
                            prop_type: "number".to_string(),
                            description: "Target video duration in seconds (default: 30)".to_string(),
                            items: None,
                        });
                        props.insert("style".to_string(), PropertyDefinition {
                            prop_type: "string".to_string(),
                            description: "Video style: 'cinematic', 'minimal', 'energetic', 'calm', 'corporate' (default: 'cinematic')".to_string(),
                            items: None,
                        });
                        props.insert("include_text_overlays".to_string(), PropertyDefinition {
                            prop_type: "boolean".to_string(),
                            description: "Whether to add text overlays with key messages (default: true)".to_string(),
                            items: None,
                        });
                        props.insert("include_music".to_string(), PropertyDefinition {
                            prop_type: "boolean".to_string(),
                            description: "Whether to generate and add background music via ElevenLabs (default: true)".to_string(),
                            items: None,
                        });
                        props.insert("num_clips".to_string(), PropertyDefinition {
                            prop_type: "number".to_string(),
                            description: "Max number of video clips to use from Pexels (default: 3-5 based on duration)".to_string(),
                            items: None,
                        });
                        props.insert("aspect_ratio".to_string(), PropertyDefinition {
                            prop_type: "string".to_string(),
                            description: "Output aspect ratio: '16:9' (landscape, default), '9:16' (portrait/Shorts), '1:1' (square), '4:3'".to_string(),
                            items: None,
                        });
                        props.insert("video_source".to_string(), PropertyDefinition {
                            prop_type: "string".to_string(),
                            description: "Clip source: 'pexels' (default — stock footage), 'blender' (custom 3D renders via BlenderMCPServer), 'hybrid' (Pexels first, Blender fallback). Use 'blender' for fully custom visuals or when style='educational_math' (auto-routes to LaTeX animations).".to_string(),
                            items: None,
                        });
                        props
                    },
                    required: vec!["topic".to_string(), "output_file".to_string()],
                },
            },
            // =====================================================================
            // WEBSITE IMAGE EXTRACTION
            // =====================================================================

            FunctionDeclaration {
                name: "fetch_website_image".to_string(),
                description: "Fetch the hero/og:image from a website URL. Use this when a user provides a website URL and you need to extract its visual for use in a Blender scene or product mockup. Returns the image URL string that can be passed to blender_generate_scene_type's reference_image_url parameter.".to_string(),
                parameters: Parameters {
                    param_type: "object".to_string(),
                    properties: {
                        let mut props = HashMap::new();
                        props.insert("url".to_string(), PropertyDefinition {
                            prop_type: "string".to_string(),
                            description: "The website URL to extract the hero image from (e.g. 'https://netflix.com')".to_string(),
                            items: None,
                        });
                        props
                    },
                    required: vec!["url".to_string()],
                },
            },

            FunctionDeclaration {
                name: "read_website_content".to_string(),
                description: "Fetch and read a website URL, returning its title, description, and main text content. Use this to understand what a website is about before generating a video script, voiceover, or Blender animation about it. Chain with generate_video_script and blender tools to create a full promotional/informative video from a URL.".to_string(),
                parameters: Parameters {
                    param_type: "object".to_string(),
                    properties: {
                        let mut props = HashMap::new();
                        props.insert("url".to_string(), PropertyDefinition {
                            prop_type: "string".to_string(),
                            description: "The website URL to read (e.g. 'https://stripe.com')".to_string(),
                            items: None,
                        });
                        props
                    },
                    required: vec!["url".to_string()],
                },
            },

            // BROWSERBASE CRAWL — full website crawl with CSS info + subpages
            // =====================================================================

            FunctionDeclaration {
                name: "browserbase_crawl_website".to_string(),
                description: "Crawl an entire website using BrowserBase — fetches the homepage, extracts all internal links, fetches each subpage's markdown content, and extracts CSS design tokens (colors, fonts). Returns combined markdown with page titles and URLs, a design tokens summary, and a feature_tag. Use vectorize_crawled_content to store the results in Qdrant for semantic search, then use search_crawled_content to query specific pages.".to_string(),
                parameters: Parameters {
                    param_type: "object".to_string(),
                    properties: {
                        let mut props = HashMap::new();
                        props.insert("url".to_string(), PropertyDefinition {
                            prop_type: "string".to_string(),
                            description: "The website URL to crawl (e.g. 'https://stripe.com')".to_string(),
                            items: None,
                        });
                        props
                    },
                    required: vec!["url".to_string()],
                },
            },

            // VECTORIZE CRAWLED CONTENT — store pages in Qdrant for semantic search
            // =====================================================================

            FunctionDeclaration {
                name: "vectorize_crawled_content".to_string(),
                description: "Store crawled website pages in Qdrant vector database for semantic search. Takes the feature_tag and pages array returned by browserbase_crawl_website, embeds each page's content using Gemini Embedding 2, and stores it in Qdrant. After calling this, you can use search_crawled_content to find specific information across all crawled pages.".to_string(),
                parameters: Parameters {
                    param_type: "object".to_string(),
                    properties: {
                        let mut props = HashMap::new();
                        props.insert("feature_tag".to_string(), PropertyDefinition {
                            prop_type: "string".to_string(),
                            description: "The feature_tag returned by browserbase_crawl_website".to_string(),
                            items: None,
                        });
                        props.insert("pages".to_string(), PropertyDefinition {
                            prop_type: "array".to_string(),
                            description: "The 'pages' array from browserbase_crawl_website result. Each element must have 'url', 'title', and 'content' fields.".to_string(),
                            items: None,
                        });
                        props
                    },
                    required: vec!["feature_tag".to_string(), "pages".to_string()],
                },
            },

            // SEARCH CRAWLED CONTENT — semantic search over vectorized site content
            // =====================================================================

            FunctionDeclaration {
                name: "search_crawled_content".to_string(),
                description: "Search previously vectorized crawled website content via semantic search. Takes a natural language query and the feature_tag from browserbase_crawl_website. Embeds the query with Gemini Embedding 2 and returns ranked matching pages with content snippets and scores. Requires vectorize_crawled_content to have been called first on the crawl results.".to_string(),
                parameters: Parameters {
                    param_type: "object".to_string(),
                    properties: {
                        let mut props = HashMap::new();
                        props.insert("query".to_string(), PropertyDefinition {
                            prop_type: "string".to_string(),
                            description: "Natural language query about the website content (e.g. 'what are their pricing plans?', 'brand colors')".to_string(),
                            items: None,
                        });
                        props.insert("feature_tag".to_string(), PropertyDefinition {
                            prop_type: "string".to_string(),
                            description: "The feature_tag returned by browserbase_crawl_website".to_string(),
                            items: None,
                        });
                        props.insert("limit".to_string(), PropertyDefinition {
                            prop_type: "number".to_string(),
                            description: "Maximum number of results to return (default: 5)".to_string(),
                            items: None,
                        });
                        props
                    },
                    required: vec!["query".to_string(), "feature_tag".to_string()],
                },
            },

            // CLOUD STORAGE TOOL
            // =====================================================================

            FunctionDeclaration {
                name: "download_from_cloud".to_string(),
                description: "Download a file from a cloud storage URL (R2 presigned URL or any HTTP URL) to a local file in the outputs/ directory. Use this to retrieve previously generated videos, images, or assets from storage for re-editing, compositing, or quality review. The file is saved to outputs/ and can then be used with any video editing tool.".to_string(),
                parameters: Parameters {
                    param_type: "object".to_string(),
                    properties: {
                        let mut props = HashMap::new();
                        props.insert("url".to_string(), PropertyDefinition {
                            prop_type: "string".to_string(),
                            description: "The presigned URL to download from (R2 presigned URL or any HTTP URL)".to_string(),
                            items: None,
                        });
                        props.insert("output_path".to_string(), PropertyDefinition {
                            prop_type: "string".to_string(),
                            description: "Desired output filename inside outputs/ (e.g. 'video.mp4' or 'outputs/video.mp4')".to_string(),
                            items: None,
                        });
                        props
                    },
                    required: vec!["url".to_string(), "output_path".to_string()],
                },
            },

            // BLENDER MCP TOOLS — consolidated codegen-based rendering
            // =====================================================================
            // blender_generate_scene_type and manim_execute_script are defined
            // via their dedicated builder functions above (lines 1064-1219).
            // They replace all 28+ individual blender_generate_* tools.

            FunctionDeclaration {
                name: "set_chat_title".to_string(),
                description: "Sets a descriptive title for the current chat session. Use this to give the conversation a meaningful title based on the user's request or the work being done.".to_string(),
                parameters: Parameters {
                    param_type: "object".to_string(),
                    properties: {
                        let mut props = HashMap::new();
                        props.insert("title".to_string(), PropertyDefinition {
                            prop_type: "string".to_string(),
                            description: "A concise, descriptive title for this chat session (max 100 characters)".to_string(),
                            items: None,
                        });
                        props
                    },
                    required: vec!["title".to_string()],
                },
            },

            // =====================================================================
            // YOUTUBE INTEGRATION TOOLS (READ-ONLY RESEARCH & OPTIMIZATION)
            // =====================================================================

            FunctionDeclaration {
                name: "optimize_youtube_metadata".to_string(),
                description: "Analyzes a video file and generates SEO-optimized YouTube metadata (title, description, tags) to maximize discoverability and engagement. Returns suggestions only - does not upload or modify anything.".to_string(),
                parameters: Parameters {
                    param_type: "object".to_string(),
                    properties: {
                        let mut props = HashMap::new();
                        props.insert("video_path".to_string(), PropertyDefinition {
                            prop_type: "string".to_string(),
                            description: "Path to the video file to analyze for metadata optimization".to_string(),
                            items: None,
                        });
                        props.insert("target_audience".to_string(), PropertyDefinition {
                            prop_type: "string".to_string(),
                            description: "Target audience type: 'gaming', 'education', 'vlog', 'entertainment', 'tech', 'music', etc.".to_string(),
                            items: None,
                        });
                        props.insert("style".to_string(), PropertyDefinition {
                            prop_type: "string".to_string(),
                            description: "Metadata style: 'clickbait', 'professional', or 'casual'".to_string(),
                            items: None,
                        });
                        props
                    },
                    required: vec!["video_path".to_string()],
                },
            },
            FunctionDeclaration {
                name: "analyze_youtube_performance".to_string(),
                description: "Fetches analytics data for a YouTube video and provides AI-powered insights on performance, audience engagement, and optimization opportunities. READ-ONLY tool.".to_string(),
                parameters: Parameters {
                    param_type: "object".to_string(),
                    properties: {
                        let mut props = HashMap::new();
                        props.insert("video_id".to_string(), PropertyDefinition {
                            prop_type: "string".to_string(),
                            description: "YouTube video ID (the alphanumeric code from youtube.com/watch?v=VIDEO_ID)".to_string(),
                            items: None,
                        });
                        props.insert("date_range_days".to_string(), PropertyDefinition {
                            prop_type: "number".to_string(),
                            description: "Number of days to analyze (default: 30, max: 365)".to_string(),
                            items: None,
                        });
                        props
                    },
                    required: vec!["video_id".to_string()],
                },
            },
            // ── Director orchestrator agent ────────────────────────────────────
            FunctionDeclaration {
                name: "run_director".to_string(),
                description: "Run the Director agent with a high-level creative brief. The Director is an orchestrator agent that plans and calls multiple tools (3D scene rendering, thumbnails, title cards, data visualizations, UI mockups, LaTeX animations) to produce a complete set of video assets. Use this for complex multi-step production requests — the Director handles planning and calling blender_generate_scene_type / manim_execute_script internally. The Director returns a JSON list of produced asset URLs and a summary. After receiving assets, use FFmpeg tools (concat_videos, overlay_video, add_text_overlay, add_audio) for post-processing.".to_string(),
                parameters: Parameters {
                    param_type: "object".to_string(),
                    properties: {
                        let mut props = HashMap::new();
                        props.insert("brief".to_string(), PropertyDefinition {
                            prop_type: "string".to_string(),
                            description: "Natural language description of what to produce. Be descriptive and informative about the desired style, mood, content, duration, and any specific requirements — just like a customer brief. Do NOT hardcode tool names, as the Director handles tool selection internally.".to_string(),
                            items: None,
                        });
                        props.insert("feedback".to_string(), PropertyDefinition {
                            prop_type: "string".to_string(),
                            description: "Optional revision feedback from a previous Director run. If the Director's previous output needs changes, describe what to improve. The Director will incorporate this feedback.".to_string(),
                            items: None,
                        });
                        props
                    },
                    required: vec!["brief".to_string()],
                },
            },
            FunctionDeclaration {
                name: "suggest_content_ideas".to_string(),
                description: "Analyzes the user's YouTube channel performance and trending topics to suggest data-driven content ideas. Provides 5-10 specific video ideas with rationale. READ-ONLY research tool.".to_string(),
                parameters: Parameters {
                    param_type: "object".to_string(),
                    properties: {
                        let mut props = HashMap::new();
                        props.insert("channel_id".to_string(), PropertyDefinition {
                            prop_type: "number".to_string(),
                            description: "Internal channel ID from database (optional)".to_string(),
                            items: None,
                        });
                        props.insert("category".to_string(), PropertyDefinition {
                            prop_type: "string".to_string(),
                            description: "Content category: 'gaming', 'tech', 'education', etc.".to_string(),
                            items: None,
                        });
                        props.insert("num_ideas".to_string(), PropertyDefinition {
                            prop_type: "number".to_string(),
                            description: "Number of ideas to generate (default: 5, max: 10)".to_string(),
                            items: None,
                        });
                        props
                    },
                    required: vec![],
                },
            },
            FunctionDeclaration {
                name: "search_youtube_trends".to_string(),
                description: "Searches for trending YouTube videos to understand what content is performing well. Returns video titles, view counts, and engagement metrics. READ-ONLY research tool.".to_string(),
                parameters: Parameters {
                    param_type: "object".to_string(),
                    properties: {
                        let mut props = HashMap::new();
                        props.insert("query".to_string(), PropertyDefinition {
                            prop_type: "string".to_string(),
                            description: "Search query/keywords (optional)".to_string(),
                            items: None,
                        });
                        props.insert("region_code".to_string(), PropertyDefinition {
                            prop_type: "string".to_string(),
                            description: "Two-letter country code: 'US', 'GB', 'CA', etc.".to_string(),
                            items: None,
                        });
                        props.insert("category".to_string(), PropertyDefinition {
                            prop_type: "string".to_string(),
                            description: "Content category: 'gaming', 'music', 'education', etc.".to_string(),
                            items: None,
                        });
                        props.insert("max_results".to_string(), PropertyDefinition {
                            prop_type: "number".to_string(),
                            description: "Maximum results (default: 10, max: 50)".to_string(),
                            items: None,
                        });
                        props
                    },
                    required: vec![],
                },
            },
            FunctionDeclaration {
                name: "search_youtube_channels".to_string(),
                description: "Searches for YouTube channels by name or keywords. Returns channel names, descriptions, and subscriber information. READ-ONLY research tool.".to_string(),
                parameters: Parameters {
                    param_type: "object".to_string(),
                    properties: {
                        let mut props = HashMap::new();
                        props.insert("query".to_string(), PropertyDefinition {
                            prop_type: "string".to_string(),
                            description: "Channel name or keywords to search for".to_string(),
                            items: None,
                        });
                        props.insert("max_results".to_string(), PropertyDefinition {
                            prop_type: "number".to_string(),
                            description: "Maximum channels to return (default: 10, max: 50)".to_string(),
                            items: None,
                        });
                        props.insert("order".to_string(), PropertyDefinition {
                            prop_type: "string".to_string(),
                            description: "Sort order: 'relevance', 'viewCount', 'videoCount'".to_string(),
                            items: None,
                        });
                        props
                    },
                    required: vec!["query".to_string()],
                },
            },

            FunctionDeclaration {
                name: "submit_final_answer".to_string(),
                description: "**CRITICAL COMPLETION TOOL**: Call this tool ONLY when you have successfully completed ALL parts of the user's request. This signals that all operations are done and no more work is needed.".to_string(),
                parameters: Parameters {
                    param_type: "object".to_string(),
                    properties: {
                        let mut props = HashMap::new();
                        props.insert("summary".to_string(), PropertyDefinition {
                            prop_type: "string".to_string(),
                            description: "A natural, conversational summary of what was accomplished".to_string(),
                            items: None,
                        });
                        props.insert("output_files".to_string(), PropertyDefinition {
                            prop_type: "array".to_string(),
                            description: "Array of output file paths that were created during this request".to_string(),
                            items: Some(Box::new(PropertyDefinition {
                                prop_type: "string".to_string(),
                                description: "File path".to_string(),
                                items: None,
                            })),
                        });
                        props
                    },
                    required: vec!["summary".to_string()],
                },
            },
            FunctionDeclaration {
                name: "add_transition".to_string(),
                description: "Adds a transition effect between two video clips using FFmpeg xfade filter".to_string(),
                parameters: Parameters {
                    param_type: "object".to_string(),
                    properties: {
                        let mut props = HashMap::new();
                        props.insert("input_file1".to_string(), PropertyDefinition {
                            prop_type: "string".to_string(),
                            description: "Path to the first video file".to_string(),
                            items: None,
                        });
                        props.insert("input_file2".to_string(), PropertyDefinition {
                            prop_type: "string".to_string(),
                            description: "Path to the second video file".to_string(),
                            items: None,
                        });
                        props.insert("output_file".to_string(), PropertyDefinition {
                            prop_type: "string".to_string(),
                            description: "Path to save the output video with transition".to_string(),
                            items: None,
                        });
                        props.insert("transition_type".to_string(), PropertyDefinition {
                            prop_type: "string".to_string(),
                            description: "Transition type: fade, dissolve, wipeleft, wiperight, circleopen, circleclose, radial, pixelize, diagtl, diagtr".to_string(),
                            items: None,
                        });
                        props.insert("duration_seconds".to_string(), PropertyDefinition {
                            prop_type: "number".to_string(),
                            description: "Transition duration in seconds (0.5–3.0 recommended)".to_string(),
                            items: None,
                        });
                        props.insert("offset_seconds".to_string(), PropertyDefinition {
                            prop_type: "number".to_string(),
                            description: "Time offset in seconds where the transition starts".to_string(),
                            items: None,
                        });
                        props
                    },
                    required: vec![
                        "input_file1".to_string(), "input_file2".to_string(),
                        "output_file".to_string(), "transition_type".to_string(),
                        "duration_seconds".to_string(), "offset_seconds".to_string(),
                    ],
                },
            },
            FunctionDeclaration {
                name: "add_animated_text".to_string(),
                description: "Adds animated text to a video (fade_in, slide_in, or typewriter effects)".to_string(),
                parameters: Parameters {
                    param_type: "object".to_string(),
                    properties: {
                        let mut props = HashMap::new();
                        props.insert("input_file".to_string(), PropertyDefinition {
                            prop_type: "string".to_string(),
                            description: "Path to the input video file".to_string(),
                            items: None,
                        });
                        props.insert("output_file".to_string(), PropertyDefinition {
                            prop_type: "string".to_string(),
                            description: "Path to save the video with animated text".to_string(),
                            items: None,
                        });
                        props.insert("text".to_string(), PropertyDefinition {
                            prop_type: "string".to_string(),
                            description: "Text to display on the video".to_string(),
                            items: None,
                        });
                        props.insert("animation_type".to_string(), PropertyDefinition {
                            prop_type: "string".to_string(),
                            description: "Animation type: 'fade_in', 'slide_in', or 'typewriter'".to_string(),
                            items: None,
                        });
                        props.insert("start_time".to_string(), PropertyDefinition {
                            prop_type: "number".to_string(),
                            description: "Time in seconds when the text animation starts".to_string(),
                            items: None,
                        });
                        props.insert("duration".to_string(), PropertyDefinition {
                            prop_type: "number".to_string(),
                            description: "Duration in seconds for the text animation".to_string(),
                            items: None,
                        });
                        props
                    },
                    required: vec![
                        "input_file".to_string(), "output_file".to_string(),
                        "text".to_string(), "animation_type".to_string(),
                        "start_time".to_string(), "duration".to_string(),
                    ],
                },
            },
            FunctionDeclaration {
                name: "apply_filter_chain".to_string(),
                description: "Applies a chain of video filters (brightness, contrast, saturation, blur) in sequence".to_string(),
                parameters: Parameters {
                    param_type: "object".to_string(),
                    properties: {
                        let mut props = HashMap::new();
                        props.insert("input_file".to_string(), PropertyDefinition {
                            prop_type: "string".to_string(),
                            description: "Path to the input video file".to_string(),
                            items: None,
                        });
                        props.insert("output_file".to_string(), PropertyDefinition {
                            prop_type: "string".to_string(),
                            description: "Path to save the filtered video".to_string(),
                            items: None,
                        });
                        props.insert("filters".to_string(), PropertyDefinition {
                            prop_type: "array".to_string(),
                            description: "Array of filter objects with 'name' (brightness|contrast|saturation|blur) and 'value' (number) fields".to_string(),
                            items: Some(Box::new(PropertyDefinition {
                                prop_type: "object".to_string(),
                                description: "Filter object with name and value".to_string(),
                                items: None,
                            })),
                        });
                        props
                    },
                    required: vec!["input_file".to_string(), "output_file".to_string(), "filters".to_string()],
                },
            },
            FunctionDeclaration {
                name: "apply_audio_effect".to_string(),
                description: "Applies an audio effect (echo, reverb, chorus) to the video's audio track".to_string(),
                parameters: Parameters {
                    param_type: "object".to_string(),
                    properties: {
                        let mut props = HashMap::new();
                        props.insert("input_file".to_string(), PropertyDefinition {
                            prop_type: "string".to_string(),
                            description: "Path to the input video file".to_string(),
                            items: None,
                        });
                        props.insert("output_file".to_string(), PropertyDefinition {
                            prop_type: "string".to_string(),
                            description: "Path to save the video with audio effect applied".to_string(),
                            items: None,
                        });
                        props.insert("effect".to_string(), PropertyDefinition {
                            prop_type: "string".to_string(),
                            description: "Audio effect type: 'echo', 'reverb', or 'chorus'".to_string(),
                            items: None,
                        });
                        props.insert("intensity".to_string(), PropertyDefinition {
                            prop_type: "number".to_string(),
                            description: "Effect intensity from 0.0 (subtle) to 1.0 (strong)".to_string(),
                            items: None,
                        });
                        props
                    },
                    required: vec!["input_file".to_string(), "output_file".to_string(), "effect".to_string(), "intensity".to_string()],
                },
            },
            FunctionDeclaration {
                name: "deinterlace_video".to_string(),
                description: "Deinterlaces an interlaced video using the yadif filter for smoother playback".to_string(),
                parameters: Parameters {
                    param_type: "object".to_string(),
                    properties: {
                        let mut props = HashMap::new();
                        props.insert("input_file".to_string(), PropertyDefinition {
                            prop_type: "string".to_string(),
                            description: "Path to the interlaced input video file".to_string(),
                            items: None,
                        });
                        props.insert("output_file".to_string(), PropertyDefinition {
                            prop_type: "string".to_string(),
                            description: "Path to save the deinterlaced video".to_string(),
                            items: None,
                        });
                        props.insert("mode".to_string(), PropertyDefinition {
                            prop_type: "string".to_string(),
                            description: "Deinterlace mode: '0' (send_frame), '1' (send_field), '2' (send_frame_nospatial)".to_string(),
                            items: None,
                        });
                        props
                    },
                    required: vec!["input_file".to_string(), "output_file".to_string(), "mode".to_string()],
                },
            },
            // ================================================================
            // BATCH 1 — Wire existing Rust functions
            // ================================================================
            FunctionDeclaration {
                name: "create_thumbnail_hd".to_string(),
                description: "Creates an HD thumbnail at a custom resolution from a video frame".to_string(),
                parameters: Parameters {
                    param_type: "object".to_string(),
                    properties: {
                        let mut props = HashMap::new();
                        props.insert("input_file".to_string(), PropertyDefinition { prop_type: "string".to_string(), description: "Path to the input video file".to_string(), items: None });
                        props.insert("output_file".to_string(), PropertyDefinition { prop_type: "string".to_string(), description: "Path to save the thumbnail image".to_string(), items: None });
                        props.insert("timestamp".to_string(), PropertyDefinition { prop_type: "number".to_string(), description: "Time in seconds to extract the frame".to_string(), items: None });
                        props.insert("width".to_string(), PropertyDefinition { prop_type: "number".to_string(), description: "Target width in pixels".to_string(), items: None });
                        props.insert("height".to_string(), PropertyDefinition { prop_type: "number".to_string(), description: "Target height in pixels".to_string(), items: None });
                        props
                    },
                    required: vec!["input_file".to_string(), "output_file".to_string(), "timestamp".to_string(), "width".to_string(), "height".to_string()],
                },
            },
            FunctionDeclaration {
                name: "get_video_duration".to_string(),
                description: "Returns the duration of a video file in seconds".to_string(),
                parameters: Parameters {
                    param_type: "object".to_string(),
                    properties: {
                        let mut props = HashMap::new();
                        props.insert("input_file".to_string(), PropertyDefinition { prop_type: "string".to_string(), description: "Path to the video file".to_string(), items: None });
                        props
                    },
                    required: vec!["input_file".to_string()],
                },
            },
            // ================================================================
            // BATCH 2 — Color Grading
            // ================================================================
            FunctionDeclaration {
                name: "adjust_hue".to_string(),
                description: "Adjusts the hue and saturation of a video using the FFmpeg hue filter".to_string(),
                parameters: Parameters {
                    param_type: "object".to_string(),
                    properties: {
                        let mut props = HashMap::new();
                        props.insert("input_file".to_string(), PropertyDefinition { prop_type: "string".to_string(), description: "Path to the input video file".to_string(), items: None });
                        props.insert("output_file".to_string(), PropertyDefinition { prop_type: "string".to_string(), description: "Path to save the output video".to_string(), items: None });
                        props.insert("hue_degrees".to_string(), PropertyDefinition { prop_type: "number".to_string(), description: "Hue rotation in degrees (-180 to 180)".to_string(), items: None });
                        props.insert("saturation_factor".to_string(), PropertyDefinition { prop_type: "number".to_string(), description: "Saturation multiplier (0–3)".to_string(), items: None });
                        props
                    },
                    required: vec!["input_file".to_string(), "output_file".to_string(), "hue_degrees".to_string(), "saturation_factor".to_string()],
                },
            },
            FunctionDeclaration {
                name: "color_balance".to_string(),
                description: "Adjusts color balance in shadows, midtones, and highlights".to_string(),
                parameters: Parameters {
                    param_type: "object".to_string(),
                    properties: {
                        let mut props = HashMap::new();
                        props.insert("input_file".to_string(), PropertyDefinition { prop_type: "string".to_string(), description: "Path to the input video file".to_string(), items: None });
                        props.insert("output_file".to_string(), PropertyDefinition { prop_type: "string".to_string(), description: "Path to save the output video".to_string(), items: None });
                        props.insert("shadows_r".to_string(), PropertyDefinition { prop_type: "number".to_string(), description: "Shadow red adjustment (-1.0 to 1.0)".to_string(), items: None });
                        props.insert("shadows_g".to_string(), PropertyDefinition { prop_type: "number".to_string(), description: "Shadow green adjustment (-1.0 to 1.0)".to_string(), items: None });
                        props.insert("shadows_b".to_string(), PropertyDefinition { prop_type: "number".to_string(), description: "Shadow blue adjustment (-1.0 to 1.0)".to_string(), items: None });
                        props.insert("midtones_r".to_string(), PropertyDefinition { prop_type: "number".to_string(), description: "Midtone red adjustment (-1.0 to 1.0)".to_string(), items: None });
                        props.insert("midtones_g".to_string(), PropertyDefinition { prop_type: "number".to_string(), description: "Midtone green adjustment (-1.0 to 1.0)".to_string(), items: None });
                        props.insert("midtones_b".to_string(), PropertyDefinition { prop_type: "number".to_string(), description: "Midtone blue adjustment (-1.0 to 1.0)".to_string(), items: None });
                        props.insert("highlights_r".to_string(), PropertyDefinition { prop_type: "number".to_string(), description: "Highlight red adjustment (-1.0 to 1.0)".to_string(), items: None });
                        props.insert("highlights_g".to_string(), PropertyDefinition { prop_type: "number".to_string(), description: "Highlight green adjustment (-1.0 to 1.0)".to_string(), items: None });
                        props.insert("highlights_b".to_string(), PropertyDefinition { prop_type: "number".to_string(), description: "Highlight blue adjustment (-1.0 to 1.0)".to_string(), items: None });
                        props
                    },
                    required: vec!["input_file".to_string(), "output_file".to_string()],
                },
            },
            FunctionDeclaration {
                name: "normalize_video".to_string(),
                description: "Normalizes video brightness/luminance across frames".to_string(),
                parameters: Parameters {
                    param_type: "object".to_string(),
                    properties: {
                        let mut props = HashMap::new();
                        props.insert("input_file".to_string(), PropertyDefinition { prop_type: "string".to_string(), description: "Path to the input video file".to_string(), items: None });
                        props.insert("output_file".to_string(), PropertyDefinition { prop_type: "string".to_string(), description: "Path to save the output video".to_string(), items: None });
                        props.insert("smoothing".to_string(), PropertyDefinition { prop_type: "number".to_string(), description: "Temporal smoothing window size (0 = per-frame)".to_string(), items: None });
                        props
                    },
                    required: vec!["input_file".to_string(), "output_file".to_string()],
                },
            },
            FunctionDeclaration {
                name: "apply_lut".to_string(),
                description: "Applies a 3D LUT file to a video for color grading".to_string(),
                parameters: Parameters {
                    param_type: "object".to_string(),
                    properties: {
                        let mut props = HashMap::new();
                        props.insert("input_file".to_string(), PropertyDefinition { prop_type: "string".to_string(), description: "Path to the input video file".to_string(), items: None });
                        props.insert("output_file".to_string(), PropertyDefinition { prop_type: "string".to_string(), description: "Path to save the output video".to_string(), items: None });
                        props.insert("lut_file".to_string(), PropertyDefinition { prop_type: "string".to_string(), description: "Path to the .cube or .3dl LUT file".to_string(), items: None });
                        props.insert("interp".to_string(), PropertyDefinition { prop_type: "string".to_string(), description: "Interpolation: nearest, trilinear, or tetrahedral".to_string(), items: None });
                        props
                    },
                    required: vec!["input_file".to_string(), "output_file".to_string(), "lut_file".to_string()],
                },
            },
            // ================================================================
            // BATCH 3 — Denoising & Sharpening
            // ================================================================
            FunctionDeclaration {
                name: "denoise_video".to_string(),
                description: "Reduces video noise using the hqdn3d high-quality denoiser filter".to_string(),
                parameters: Parameters {
                    param_type: "object".to_string(),
                    properties: {
                        let mut props = HashMap::new();
                        props.insert("input_file".to_string(), PropertyDefinition { prop_type: "string".to_string(), description: "Path to the input video file".to_string(), items: None });
                        props.insert("output_file".to_string(), PropertyDefinition { prop_type: "string".to_string(), description: "Path to save the denoised video".to_string(), items: None });
                        props.insert("luma_spatial".to_string(), PropertyDefinition { prop_type: "number".to_string(), description: "Spatial luma denoising strength (0–10)".to_string(), items: None });
                        props.insert("luma_temporal".to_string(), PropertyDefinition { prop_type: "number".to_string(), description: "Temporal luma denoising strength (0–10)".to_string(), items: None });
                        props.insert("chroma_spatial".to_string(), PropertyDefinition { prop_type: "number".to_string(), description: "Spatial chroma denoising strength (0–10)".to_string(), items: None });
                        props.insert("chroma_temporal".to_string(), PropertyDefinition { prop_type: "number".to_string(), description: "Temporal chroma denoising strength (0–10)".to_string(), items: None });
                        props
                    },
                    required: vec!["input_file".to_string(), "output_file".to_string()],
                },
            },
            FunctionDeclaration {
                name: "unsharp_mask".to_string(),
                description: "Applies an unsharp mask to sharpen or blur a video".to_string(),
                parameters: Parameters {
                    param_type: "object".to_string(),
                    properties: {
                        let mut props = HashMap::new();
                        props.insert("input_file".to_string(), PropertyDefinition { prop_type: "string".to_string(), description: "Path to the input video file".to_string(), items: None });
                        props.insert("output_file".to_string(), PropertyDefinition { prop_type: "string".to_string(), description: "Path to save the output video".to_string(), items: None });
                        props.insert("luma_msize_x".to_string(), PropertyDefinition { prop_type: "number".to_string(), description: "Luma matrix horizontal size (3–23, must be odd)".to_string(), items: None });
                        props.insert("luma_msize_y".to_string(), PropertyDefinition { prop_type: "number".to_string(), description: "Luma matrix vertical size (3–23, must be odd)".to_string(), items: None });
                        props.insert("luma_amount".to_string(), PropertyDefinition { prop_type: "number".to_string(), description: "Sharpening amount (-1.5 to 1.5)".to_string(), items: None });
                        props
                    },
                    required: vec!["input_file".to_string(), "output_file".to_string(), "luma_msize_x".to_string(), "luma_msize_y".to_string(), "luma_amount".to_string()],
                },
            },
            FunctionDeclaration {
                name: "reduce_noise".to_string(),
                description: "Reduces noise using the non-local means (nlmeans) denoiser".to_string(),
                parameters: Parameters {
                    param_type: "object".to_string(),
                    properties: {
                        let mut props = HashMap::new();
                        props.insert("input_file".to_string(), PropertyDefinition { prop_type: "string".to_string(), description: "Path to the input video file".to_string(), items: None });
                        props.insert("output_file".to_string(), PropertyDefinition { prop_type: "string".to_string(), description: "Path to save the denoised video".to_string(), items: None });
                        props.insert("strength".to_string(), PropertyDefinition { prop_type: "number".to_string(), description: "Denoising strength (1–30)".to_string(), items: None });
                        props.insert("research_size".to_string(), PropertyDefinition { prop_type: "number".to_string(), description: "Research window size (9–45)".to_string(), items: None });
                        props.insert("patch_size".to_string(), PropertyDefinition { prop_type: "number".to_string(), description: "Patch size (3–21, must be odd)".to_string(), items: None });
                        props
                    },
                    required: vec!["input_file".to_string(), "output_file".to_string(), "strength".to_string()],
                },
            },
            // ================================================================
            // BATCH 4 — Audio Processing
            // ================================================================
            FunctionDeclaration {
                name: "compress_audio".to_string(),
                description: "Applies dynamic range compression to audio".to_string(),
                parameters: Parameters {
                    param_type: "object".to_string(),
                    properties: {
                        let mut props = HashMap::new();
                        props.insert("input_file".to_string(), PropertyDefinition { prop_type: "string".to_string(), description: "Path to the input file".to_string(), items: None });
                        props.insert("output_file".to_string(), PropertyDefinition { prop_type: "string".to_string(), description: "Path to save the output file".to_string(), items: None });
                        props.insert("threshold_db".to_string(), PropertyDefinition { prop_type: "number".to_string(), description: "Threshold in dB (-50 to 0)".to_string(), items: None });
                        props.insert("ratio".to_string(), PropertyDefinition { prop_type: "number".to_string(), description: "Compression ratio (1–20)".to_string(), items: None });
                        props.insert("attack_ms".to_string(), PropertyDefinition { prop_type: "number".to_string(), description: "Attack time in milliseconds".to_string(), items: None });
                        props.insert("release_ms".to_string(), PropertyDefinition { prop_type: "number".to_string(), description: "Release time in milliseconds".to_string(), items: None });
                        props.insert("makeup_gain_db".to_string(), PropertyDefinition { prop_type: "number".to_string(), description: "Makeup gain in dB".to_string(), items: None });
                        props
                    },
                    required: vec!["input_file".to_string(), "output_file".to_string(), "threshold_db".to_string(), "ratio".to_string()],
                },
            },
            FunctionDeclaration {
                name: "normalize_audio".to_string(),
                description: "Normalizes audio loudness to a target LUFS level".to_string(),
                parameters: Parameters {
                    param_type: "object".to_string(),
                    properties: {
                        let mut props = HashMap::new();
                        props.insert("input_file".to_string(), PropertyDefinition { prop_type: "string".to_string(), description: "Path to the input file".to_string(), items: None });
                        props.insert("output_file".to_string(), PropertyDefinition { prop_type: "string".to_string(), description: "Path to save the output file".to_string(), items: None });
                        props.insert("target_lufs".to_string(), PropertyDefinition { prop_type: "number".to_string(), description: "Target loudness in LUFS (e.g. -16)".to_string(), items: None });
                        props.insert("loudness_range_target".to_string(), PropertyDefinition { prop_type: "number".to_string(), description: "Loudness range target in LU (1–20)".to_string(), items: None });
                        props.insert("true_peak_dbtp".to_string(), PropertyDefinition { prop_type: "number".to_string(), description: "Maximum true peak in dBTP".to_string(), items: None });
                        props
                    },
                    required: vec!["input_file".to_string(), "output_file".to_string(), "target_lufs".to_string()],
                },
            },
            FunctionDeclaration {
                name: "equalize_audio".to_string(),
                description: "Applies parametric equalization to a frequency band in the audio".to_string(),
                parameters: Parameters {
                    param_type: "object".to_string(),
                    properties: {
                        let mut props = HashMap::new();
                        props.insert("input_file".to_string(), PropertyDefinition { prop_type: "string".to_string(), description: "Path to the input file".to_string(), items: None });
                        props.insert("output_file".to_string(), PropertyDefinition { prop_type: "string".to_string(), description: "Path to save the output file".to_string(), items: None });
                        props.insert("frequency_hz".to_string(), PropertyDefinition { prop_type: "number".to_string(), description: "Center frequency in Hz".to_string(), items: None });
                        props.insert("gain_db".to_string(), PropertyDefinition { prop_type: "number".to_string(), description: "Gain in dB (-20 to 20)".to_string(), items: None });
                        props.insert("bandwidth_hz".to_string(), PropertyDefinition { prop_type: "number".to_string(), description: "Bandwidth in Hz".to_string(), items: None });
                        props.insert("eq_type".to_string(), PropertyDefinition { prop_type: "string".to_string(), description: "EQ type: peak, lowshelf, or highshelf".to_string(), items: None });
                        props
                    },
                    required: vec!["input_file".to_string(), "output_file".to_string(), "frequency_hz".to_string(), "gain_db".to_string()],
                },
            },
            FunctionDeclaration {
                name: "gate_audio".to_string(),
                description: "Applies a noise gate to audio, silencing signals below a threshold".to_string(),
                parameters: Parameters {
                    param_type: "object".to_string(),
                    properties: {
                        let mut props = HashMap::new();
                        props.insert("input_file".to_string(), PropertyDefinition { prop_type: "string".to_string(), description: "Path to the input file".to_string(), items: None });
                        props.insert("output_file".to_string(), PropertyDefinition { prop_type: "string".to_string(), description: "Path to save the output file".to_string(), items: None });
                        props.insert("threshold_db".to_string(), PropertyDefinition { prop_type: "number".to_string(), description: "Gate threshold in dB".to_string(), items: None });
                        props.insert("ratio".to_string(), PropertyDefinition { prop_type: "number".to_string(), description: "Gate ratio".to_string(), items: None });
                        props.insert("attack_ms".to_string(), PropertyDefinition { prop_type: "number".to_string(), description: "Attack time in milliseconds".to_string(), items: None });
                        props.insert("release_ms".to_string(), PropertyDefinition { prop_type: "number".to_string(), description: "Release time in milliseconds".to_string(), items: None });
                        props
                    },
                    required: vec!["input_file".to_string(), "output_file".to_string(), "threshold_db".to_string()],
                },
            },
            FunctionDeclaration {
                name: "denoise_audio".to_string(),
                description: "Reduces background noise from audio using FFmpeg afftdn spectral denoiser".to_string(),
                parameters: Parameters {
                    param_type: "object".to_string(),
                    properties: {
                        let mut props = HashMap::new();
                        props.insert("input_file".to_string(), PropertyDefinition { prop_type: "string".to_string(), description: "Path to the input file".to_string(), items: None });
                        props.insert("output_file".to_string(), PropertyDefinition { prop_type: "string".to_string(), description: "Path to save the output file".to_string(), items: None });
                        props.insert("noise_floor_db".to_string(), PropertyDefinition { prop_type: "number".to_string(), description: "Estimated noise floor in dB".to_string(), items: None });
                        props.insert("noise_reduction_db".to_string(), PropertyDefinition { prop_type: "number".to_string(), description: "Amount of noise reduction in dB".to_string(), items: None });
                        props.insert("track_noise".to_string(), PropertyDefinition { prop_type: "boolean".to_string(), description: "Whether to adapt the noise profile over time".to_string(), items: None });
                        props
                    },
                    required: vec!["input_file".to_string(), "output_file".to_string()],
                },
            },
            // ================================================================
            // BATCH 5 — Video Composition & Layout
            // ================================================================
            FunctionDeclaration {
                name: "pad_video".to_string(),
                description: "Adds padding around the video to reach a target resolution".to_string(),
                parameters: Parameters {
                    param_type: "object".to_string(),
                    properties: {
                        let mut props = HashMap::new();
                        props.insert("input_file".to_string(), PropertyDefinition { prop_type: "string".to_string(), description: "Path to the input video file".to_string(), items: None });
                        props.insert("output_file".to_string(), PropertyDefinition { prop_type: "string".to_string(), description: "Path to save the padded video".to_string(), items: None });
                        props.insert("width".to_string(), PropertyDefinition { prop_type: "number".to_string(), description: "Total output width in pixels".to_string(), items: None });
                        props.insert("height".to_string(), PropertyDefinition { prop_type: "number".to_string(), description: "Total output height in pixels".to_string(), items: None });
                        props.insert("x_offset".to_string(), PropertyDefinition { prop_type: "number".to_string(), description: "Horizontal offset of the video within the frame".to_string(), items: None });
                        props.insert("y_offset".to_string(), PropertyDefinition { prop_type: "number".to_string(), description: "Vertical offset of the video within the frame".to_string(), items: None });
                        props.insert("color".to_string(), PropertyDefinition { prop_type: "string".to_string(), description: "Padding color (e.g. 'black', 'white')".to_string(), items: None });
                        props
                    },
                    required: vec!["input_file".to_string(), "output_file".to_string(), "width".to_string(), "height".to_string()],
                },
            },
            FunctionDeclaration {
                name: "blend_videos".to_string(),
                description: "Blends two video layers together using a compositing blend mode".to_string(),
                parameters: Parameters {
                    param_type: "object".to_string(),
                    properties: {
                        let mut props = HashMap::new();
                        props.insert("input_file1".to_string(), PropertyDefinition { prop_type: "string".to_string(), description: "Path to the base video file".to_string(), items: None });
                        props.insert("input_file2".to_string(), PropertyDefinition { prop_type: "string".to_string(), description: "Path to the overlay video file".to_string(), items: None });
                        props.insert("output_file".to_string(), PropertyDefinition { prop_type: "string".to_string(), description: "Path to save the blended video".to_string(), items: None });
                        props.insert("blend_mode".to_string(), PropertyDefinition { prop_type: "string".to_string(), description: "Blend mode: addition, multiply, screen, overlay, hardlight, softlight, difference, exclusion".to_string(), items: None });
                        props.insert("opacity".to_string(), PropertyDefinition { prop_type: "number".to_string(), description: "Opacity of the blend (0.0–1.0)".to_string(), items: None });
                        props
                    },
                    required: vec!["input_file1".to_string(), "input_file2".to_string(), "output_file".to_string(), "blend_mode".to_string()],
                },
            },
            FunctionDeclaration {
                name: "stack_videos".to_string(),
                description: "Stacks two videos side by side or top to bottom".to_string(),
                parameters: Parameters {
                    param_type: "object".to_string(),
                    properties: {
                        let mut props = HashMap::new();
                        props.insert("input_file1".to_string(), PropertyDefinition { prop_type: "string".to_string(), description: "Path to the first video file".to_string(), items: None });
                        props.insert("input_file2".to_string(), PropertyDefinition { prop_type: "string".to_string(), description: "Path to the second video file".to_string(), items: None });
                        props.insert("output_file".to_string(), PropertyDefinition { prop_type: "string".to_string(), description: "Path to save the stacked video".to_string(), items: None });
                        props.insert("direction".to_string(), PropertyDefinition { prop_type: "string".to_string(), description: "Stack direction: 'horizontal' or 'vertical'".to_string(), items: None });
                        props
                    },
                    required: vec!["input_file1".to_string(), "input_file2".to_string(), "output_file".to_string()],
                },
            },
            FunctionDeclaration {
                name: "add_vignette".to_string(),
                description: "Adds a vignette effect (darkened edges) to the video".to_string(),
                parameters: Parameters {
                    param_type: "object".to_string(),
                    properties: {
                        let mut props = HashMap::new();
                        props.insert("input_file".to_string(), PropertyDefinition { prop_type: "string".to_string(), description: "Path to the input video file".to_string(), items: None });
                        props.insert("output_file".to_string(), PropertyDefinition { prop_type: "string".to_string(), description: "Path to save the output video".to_string(), items: None });
                        props.insert("angle".to_string(), PropertyDefinition { prop_type: "number".to_string(), description: "Vignette angle in radians (0 to 1.5708)".to_string(), items: None });
                        props.insert("mode".to_string(), PropertyDefinition { prop_type: "string".to_string(), description: "Direction: 'forward' (darken edges) or 'backward' (brighten edges)".to_string(), items: None });
                        props
                    },
                    required: vec!["input_file".to_string(), "output_file".to_string()],
                },
            },
            FunctionDeclaration {
                name: "draw_box".to_string(),
                description: "Draws a rectangle on the video for highlighting areas or creating borders".to_string(),
                parameters: Parameters {
                    param_type: "object".to_string(),
                    properties: {
                        let mut props = HashMap::new();
                        props.insert("input_file".to_string(), PropertyDefinition { prop_type: "string".to_string(), description: "Path to the input video file".to_string(), items: None });
                        props.insert("output_file".to_string(), PropertyDefinition { prop_type: "string".to_string(), description: "Path to save the output video".to_string(), items: None });
                        props.insert("x".to_string(), PropertyDefinition { prop_type: "number".to_string(), description: "X coordinate of the box top-left corner".to_string(), items: None });
                        props.insert("y".to_string(), PropertyDefinition { prop_type: "number".to_string(), description: "Y coordinate of the box top-left corner".to_string(), items: None });
                        props.insert("width".to_string(), PropertyDefinition { prop_type: "number".to_string(), description: "Width of the box in pixels".to_string(), items: None });
                        props.insert("height".to_string(), PropertyDefinition { prop_type: "number".to_string(), description: "Height of the box in pixels".to_string(), items: None });
                        props.insert("color".to_string(), PropertyDefinition { prop_type: "string".to_string(), description: "Box color (e.g. 'red', 'white')".to_string(), items: None });
                        props.insert("thickness".to_string(), PropertyDefinition { prop_type: "number".to_string(), description: "Border thickness in pixels (0 = filled)".to_string(), items: None });
                        props
                    },
                    required: vec!["input_file".to_string(), "output_file".to_string(), "x".to_string(), "y".to_string(), "width".to_string(), "height".to_string(), "color".to_string()],
                },
            },
            // ================================================================
            // BATCH 6 — Motion, Time & Frame Effects
            // ================================================================
            FunctionDeclaration {
                name: "reverse_video".to_string(),
                description: "Reverses both video and audio of a clip to play it backwards".to_string(),
                parameters: Parameters {
                    param_type: "object".to_string(),
                    properties: {
                        let mut props = HashMap::new();
                        props.insert("input_file".to_string(), PropertyDefinition { prop_type: "string".to_string(), description: "Path to the input video file".to_string(), items: None });
                        props.insert("output_file".to_string(), PropertyDefinition { prop_type: "string".to_string(), description: "Path to save the reversed video".to_string(), items: None });
                        props
                    },
                    required: vec!["input_file".to_string(), "output_file".to_string()],
                },
            },
            FunctionDeclaration {
                name: "loop_video".to_string(),
                description: "Loops a video a specified number of times, capped to a maximum duration".to_string(),
                parameters: Parameters {
                    param_type: "object".to_string(),
                    properties: {
                        let mut props = HashMap::new();
                        props.insert("input_file".to_string(), PropertyDefinition { prop_type: "string".to_string(), description: "Path to the input video file".to_string(), items: None });
                        props.insert("output_file".to_string(), PropertyDefinition { prop_type: "string".to_string(), description: "Path to save the looped video".to_string(), items: None });
                        props.insert("loop_count".to_string(), PropertyDefinition { prop_type: "number".to_string(), description: "Number of loops (-1 for infinite, limited by duration)".to_string(), items: None });
                        props.insert("loop_duration_sec".to_string(), PropertyDefinition { prop_type: "number".to_string(), description: "Maximum output duration in seconds".to_string(), items: None });
                        props
                    },
                    required: vec!["input_file".to_string(), "output_file".to_string(), "loop_count".to_string(), "loop_duration_sec".to_string()],
                },
            },
            FunctionDeclaration {
                name: "zoompan".to_string(),
                description: "Applies a Ken Burns-style zoom and pan effect to a video or image".to_string(),
                parameters: Parameters {
                    param_type: "object".to_string(),
                    properties: {
                        let mut props = HashMap::new();
                        props.insert("input_file".to_string(), PropertyDefinition { prop_type: "string".to_string(), description: "Path to the input video or image file".to_string(), items: None });
                        props.insert("output_file".to_string(), PropertyDefinition { prop_type: "string".to_string(), description: "Path to save the output video".to_string(), items: None });
                        props.insert("zoom_factor".to_string(), PropertyDefinition { prop_type: "number".to_string(), description: "Zoom level (1.0 = no zoom, 2.0 = 2x zoom)".to_string(), items: None });
                        props.insert("x_expr".to_string(), PropertyDefinition { prop_type: "string".to_string(), description: "X pan expression".to_string(), items: None });
                        props.insert("y_expr".to_string(), PropertyDefinition { prop_type: "string".to_string(), description: "Y pan expression".to_string(), items: None });
                        props.insert("duration_frames".to_string(), PropertyDefinition { prop_type: "number".to_string(), description: "Duration in frames".to_string(), items: None });
                        props.insert("fps".to_string(), PropertyDefinition { prop_type: "number".to_string(), description: "Output frames per second (default 25)".to_string(), items: None });
                        props
                    },
                    required: vec!["input_file".to_string(), "output_file".to_string(), "zoom_factor".to_string(), "duration_frames".to_string()],
                },
            },
            FunctionDeclaration {
                name: "minterpolate".to_string(),
                description: "Increases video frame rate using motion interpolation".to_string(),
                parameters: Parameters {
                    param_type: "object".to_string(),
                    properties: {
                        let mut props = HashMap::new();
                        props.insert("input_file".to_string(), PropertyDefinition { prop_type: "string".to_string(), description: "Path to the input video file".to_string(), items: None });
                        props.insert("output_file".to_string(), PropertyDefinition { prop_type: "string".to_string(), description: "Path to save the interpolated video".to_string(), items: None });
                        props.insert("fps_target".to_string(), PropertyDefinition { prop_type: "number".to_string(), description: "Target frame rate (e.g. 60)".to_string(), items: None });
                        props.insert("mi_mode".to_string(), PropertyDefinition { prop_type: "string".to_string(), description: "Mode: dup, blend, or mci (default mci)".to_string(), items: None });
                        props
                    },
                    required: vec!["input_file".to_string(), "output_file".to_string(), "fps_target".to_string()],
                },
            },
            // ================================================================
            // BATCH 7 — Media Analysis Tools
            // ================================================================
            FunctionDeclaration {
                name: "export_custom_quality".to_string(),
                description: "Exports a video with custom quality, resolution, and bitrate settings".to_string(),
                parameters: Parameters {
                    param_type: "object".to_string(),
                    properties: {
                        let mut props = HashMap::new();
                        props.insert("input_file".to_string(), PropertyDefinition {
                            prop_type: "string".to_string(),
                            description: "Path to the input video file".to_string(),
                            items: None,
                        });
                        props.insert("output_file".to_string(), PropertyDefinition {
                            prop_type: "string".to_string(),
                            description: "Path to save the exported video".to_string(),
                            items: None,
                        });
                        props.insert("quality".to_string(), PropertyDefinition {
                            prop_type: "string".to_string(),
                            description: "Quality preset: 'low', 'medium', 'high', or 'ultra'".to_string(),
                            items: None,
                        });
                        props.insert("width".to_string(), PropertyDefinition {
                            prop_type: "number".to_string(),
                            description: "Optional output width in pixels (e.g. 1920)".to_string(),
                            items: None,
                        });
                        props.insert("height".to_string(), PropertyDefinition {
                            prop_type: "number".to_string(),
                            description: "Optional output height in pixels (e.g. 1080)".to_string(),
                            items: None,
                        });
                        props.insert("bitrate_kbps".to_string(), PropertyDefinition {
                            prop_type: "number".to_string(),
                            description: "Optional video bitrate in kbps (e.g. 8000). Overrides quality CRF.".to_string(),
                            items: None,
                        });
                        props
                    },
                    required: vec!["input_file".to_string(), "output_file".to_string(), "quality".to_string()],
                },
            },
            // ================================================================
            // BATCH 8 — Advanced Color Grading
            // ================================================================
            FunctionDeclaration {
                name: "adjust_curves".to_string(),
                description: "Adjusts color curves using the FFmpeg curves filter. Supports presets or custom per-channel control points.".to_string(),
                parameters: Parameters {
                    param_type: "object".to_string(),
                    properties: {
                        let mut props = HashMap::new();
                        props.insert("input_file".to_string(), PropertyDefinition { prop_type: "string".to_string(), description: "Path to the input video file".to_string(), items: None });
                        props.insert("output_file".to_string(), PropertyDefinition { prop_type: "string".to_string(), description: "Path to save the output video".to_string(), items: None });
                        props.insert("preset".to_string(), PropertyDefinition { prop_type: "string".to_string(), description: "Preset name: none/color_negative/cross_process/darker/increase_contrast/lighter/linear_contrast/medium_contrast/negative/strong_contrast/vintage".to_string(), items: None });
                        props.insert("master".to_string(), PropertyDefinition { prop_type: "string".to_string(), description: "Master curve control points e.g. '0/0 0.5/0.6 1/1'".to_string(), items: None });
                        props.insert("red_channel".to_string(), PropertyDefinition { prop_type: "string".to_string(), description: "Red channel curve control points".to_string(), items: None });
                        props.insert("green_channel".to_string(), PropertyDefinition { prop_type: "string".to_string(), description: "Green channel curve control points".to_string(), items: None });
                        props.insert("blue_channel".to_string(), PropertyDefinition { prop_type: "string".to_string(), description: "Blue channel curve control points".to_string(), items: None });
                        props
                    },
                    required: vec!["input_file".to_string(), "output_file".to_string()],
                },
            },
            FunctionDeclaration {
                name: "adjust_levels".to_string(),
                description: "Adjusts input/output levels per channel using the FFmpeg colorlevels filter.".to_string(),
                parameters: Parameters {
                    param_type: "object".to_string(),
                    properties: {
                        let mut props = HashMap::new();
                        props.insert("input_file".to_string(), PropertyDefinition { prop_type: "string".to_string(), description: "Path to the input video file".to_string(), items: None });
                        props.insert("output_file".to_string(), PropertyDefinition { prop_type: "string".to_string(), description: "Path to save the output video".to_string(), items: None });
                        props.insert("rimin".to_string(), PropertyDefinition { prop_type: "number".to_string(), description: "Red input black point (0.0–1.0, default 0.0)".to_string(), items: None });
                        props.insert("rimax".to_string(), PropertyDefinition { prop_type: "number".to_string(), description: "Red input white point (0.0–1.0, default 1.0)".to_string(), items: None });
                        props.insert("gimin".to_string(), PropertyDefinition { prop_type: "number".to_string(), description: "Green input black point (0.0–1.0, default 0.0)".to_string(), items: None });
                        props.insert("gimax".to_string(), PropertyDefinition { prop_type: "number".to_string(), description: "Green input white point (0.0–1.0, default 1.0)".to_string(), items: None });
                        props.insert("bimin".to_string(), PropertyDefinition { prop_type: "number".to_string(), description: "Blue input black point (0.0–1.0, default 0.0)".to_string(), items: None });
                        props.insert("bimax".to_string(), PropertyDefinition { prop_type: "number".to_string(), description: "Blue input white point (0.0–1.0, default 1.0)".to_string(), items: None });
                        props.insert("romin".to_string(), PropertyDefinition { prop_type: "number".to_string(), description: "Red output black point (0.0–1.0, default 0.0)".to_string(), items: None });
                        props.insert("romax".to_string(), PropertyDefinition { prop_type: "number".to_string(), description: "Red output white point (0.0–1.0, default 1.0)".to_string(), items: None });
                        props.insert("gomin".to_string(), PropertyDefinition { prop_type: "number".to_string(), description: "Green output black point (0.0–1.0, default 0.0)".to_string(), items: None });
                        props.insert("gomax".to_string(), PropertyDefinition { prop_type: "number".to_string(), description: "Green output white point (0.0–1.0, default 1.0)".to_string(), items: None });
                        props.insert("bomin".to_string(), PropertyDefinition { prop_type: "number".to_string(), description: "Blue output black point (0.0–1.0, default 0.0)".to_string(), items: None });
                        props.insert("bomax".to_string(), PropertyDefinition { prop_type: "number".to_string(), description: "Blue output white point (0.0–1.0, default 1.0)".to_string(), items: None });
                        props
                    },
                    required: vec!["input_file".to_string(), "output_file".to_string()],
                },
            },
            FunctionDeclaration {
                name: "split_tone".to_string(),
                description: "Applies split toning to shadows and highlights using the FFmpeg colorbalance filter.".to_string(),
                parameters: Parameters {
                    param_type: "object".to_string(),
                    properties: {
                        let mut props = HashMap::new();
                        props.insert("input_file".to_string(), PropertyDefinition { prop_type: "string".to_string(), description: "Path to the input video file".to_string(), items: None });
                        props.insert("output_file".to_string(), PropertyDefinition { prop_type: "string".to_string(), description: "Path to save the output video".to_string(), items: None });
                        props.insert("shadow_hue".to_string(), PropertyDefinition { prop_type: "number".to_string(), description: "Hue angle for shadows (0–360)".to_string(), items: None });
                        props.insert("shadow_saturation".to_string(), PropertyDefinition { prop_type: "number".to_string(), description: "Saturation for shadows (0.0–1.0)".to_string(), items: None });
                        props.insert("highlight_hue".to_string(), PropertyDefinition { prop_type: "number".to_string(), description: "Hue angle for highlights (0–360)".to_string(), items: None });
                        props.insert("highlight_saturation".to_string(), PropertyDefinition { prop_type: "number".to_string(), description: "Saturation for highlights (0.0–1.0)".to_string(), items: None });
                        props.insert("balance".to_string(), PropertyDefinition { prop_type: "number".to_string(), description: "Midtone bias (-1.0 to 1.0)".to_string(), items: None });
                        props
                    },
                    required: vec!["input_file".to_string(), "output_file".to_string()],
                },
            },
            FunctionDeclaration {
                name: "convert_colorspace".to_string(),
                description: "Converts video colorspace using the FFmpeg colorspace filter.".to_string(),
                parameters: Parameters {
                    param_type: "object".to_string(),
                    properties: {
                        let mut props = HashMap::new();
                        props.insert("input_file".to_string(), PropertyDefinition { prop_type: "string".to_string(), description: "Path to the input video file".to_string(), items: None });
                        props.insert("output_file".to_string(), PropertyDefinition { prop_type: "string".to_string(), description: "Path to save the output video".to_string(), items: None });
                        props.insert("colorspace".to_string(), PropertyDefinition { prop_type: "string".to_string(), description: "Target colorspace: bt709/bt2020/smpte170m/smpte240m".to_string(), items: None });
                        props.insert("transfer_characteristics".to_string(), PropertyDefinition { prop_type: "string".to_string(), description: "Transfer characteristics: bt709/bt2020-10/smpte2084/arib-std-b67".to_string(), items: None });
                        props.insert("color_primaries".to_string(), PropertyDefinition { prop_type: "string".to_string(), description: "Color primaries: bt709/bt2020/smpte170m".to_string(), items: None });
                        props
                    },
                    required: vec!["input_file".to_string(), "output_file".to_string(), "colorspace".to_string()],
                },
            },
            FunctionDeclaration {
                name: "apply_tonemap".to_string(),
                description: "Applies HDR to SDR tonemapping using the FFmpeg tonemap filter.".to_string(),
                parameters: Parameters {
                    param_type: "object".to_string(),
                    properties: {
                        let mut props = HashMap::new();
                        props.insert("input_file".to_string(), PropertyDefinition { prop_type: "string".to_string(), description: "Path to the input video file".to_string(), items: None });
                        props.insert("output_file".to_string(), PropertyDefinition { prop_type: "string".to_string(), description: "Path to save the output video".to_string(), items: None });
                        props.insert("algorithm".to_string(), PropertyDefinition { prop_type: "string".to_string(), description: "Tonemapping algorithm: none/linear/gamma/clip/reinhard/hable/mobius".to_string(), items: None });
                        props.insert("peak".to_string(), PropertyDefinition { prop_type: "number".to_string(), description: "Reference peak luminance (0 = auto-detect)".to_string(), items: None });
                        props.insert("desat".to_string(), PropertyDefinition { prop_type: "number".to_string(), description: "Desaturation strength (0.0–1.0, default 0.5)".to_string(), items: None });
                        props
                    },
                    required: vec!["input_file".to_string(), "output_file".to_string(), "algorithm".to_string()],
                },
            },
            // ================================================================
            // BATCH 9 — Audio Tone Shaping
            // ================================================================
            FunctionDeclaration {
                name: "filter_highpass".to_string(),
                description: "Applies a high-pass filter removing frequencies below the cutoff using FFmpeg highpass filter.".to_string(),
                parameters: Parameters {
                    param_type: "object".to_string(),
                    properties: {
                        let mut props = HashMap::new();
                        props.insert("input_file".to_string(), PropertyDefinition { prop_type: "string".to_string(), description: "Path to the input file".to_string(), items: None });
                        props.insert("output_file".to_string(), PropertyDefinition { prop_type: "string".to_string(), description: "Path to save the output file".to_string(), items: None });
                        props.insert("frequency_hz".to_string(), PropertyDefinition { prop_type: "number".to_string(), description: "Cutoff frequency in Hz".to_string(), items: None });
                        props.insert("poles".to_string(), PropertyDefinition { prop_type: "number".to_string(), description: "Number of poles (1 or 2, default 2)".to_string(), items: None });
                        props.insert("width_hz".to_string(), PropertyDefinition { prop_type: "number".to_string(), description: "Filter width in Hz (default 0.707)".to_string(), items: None });
                        props
                    },
                    required: vec!["input_file".to_string(), "output_file".to_string(), "frequency_hz".to_string()],
                },
            },
            FunctionDeclaration {
                name: "filter_lowpass".to_string(),
                description: "Applies a low-pass filter removing frequencies above the cutoff using FFmpeg lowpass filter.".to_string(),
                parameters: Parameters {
                    param_type: "object".to_string(),
                    properties: {
                        let mut props = HashMap::new();
                        props.insert("input_file".to_string(), PropertyDefinition { prop_type: "string".to_string(), description: "Path to the input file".to_string(), items: None });
                        props.insert("output_file".to_string(), PropertyDefinition { prop_type: "string".to_string(), description: "Path to save the output file".to_string(), items: None });
                        props.insert("frequency_hz".to_string(), PropertyDefinition { prop_type: "number".to_string(), description: "Cutoff frequency in Hz".to_string(), items: None });
                        props.insert("poles".to_string(), PropertyDefinition { prop_type: "number".to_string(), description: "Number of poles (1 or 2, default 2)".to_string(), items: None });
                        props.insert("width_hz".to_string(), PropertyDefinition { prop_type: "number".to_string(), description: "Filter width in Hz (default 0.707)".to_string(), items: None });
                        props
                    },
                    required: vec!["input_file".to_string(), "output_file".to_string(), "frequency_hz".to_string()],
                },
            },
            FunctionDeclaration {
                name: "adjust_bass".to_string(),
                description: "Boosts or cuts bass frequencies using the FFmpeg bass/lowshelf filter.".to_string(),
                parameters: Parameters {
                    param_type: "object".to_string(),
                    properties: {
                        let mut props = HashMap::new();
                        props.insert("input_file".to_string(), PropertyDefinition { prop_type: "string".to_string(), description: "Path to the input file".to_string(), items: None });
                        props.insert("output_file".to_string(), PropertyDefinition { prop_type: "string".to_string(), description: "Path to save the output file".to_string(), items: None });
                        props.insert("gain_db".to_string(), PropertyDefinition { prop_type: "number".to_string(), description: "Gain in dB (-20 to 20)".to_string(), items: None });
                        props.insert("frequency_hz".to_string(), PropertyDefinition { prop_type: "number".to_string(), description: "Center frequency in Hz (default 100)".to_string(), items: None });
                        props.insert("width_hz".to_string(), PropertyDefinition { prop_type: "number".to_string(), description: "Shelf width in Hz (default 0.5)".to_string(), items: None });
                        props
                    },
                    required: vec!["input_file".to_string(), "output_file".to_string(), "gain_db".to_string()],
                },
            },
            FunctionDeclaration {
                name: "adjust_treble".to_string(),
                description: "Boosts or cuts treble frequencies using the FFmpeg treble/highshelf filter.".to_string(),
                parameters: Parameters {
                    param_type: "object".to_string(),
                    properties: {
                        let mut props = HashMap::new();
                        props.insert("input_file".to_string(), PropertyDefinition { prop_type: "string".to_string(), description: "Path to the input file".to_string(), items: None });
                        props.insert("output_file".to_string(), PropertyDefinition { prop_type: "string".to_string(), description: "Path to save the output file".to_string(), items: None });
                        props.insert("gain_db".to_string(), PropertyDefinition { prop_type: "number".to_string(), description: "Gain in dB (-20 to 20)".to_string(), items: None });
                        props.insert("frequency_hz".to_string(), PropertyDefinition { prop_type: "number".to_string(), description: "Center frequency in Hz (default 3000)".to_string(), items: None });
                        props.insert("width_hz".to_string(), PropertyDefinition { prop_type: "number".to_string(), description: "Shelf width in Hz (default 0.5)".to_string(), items: None });
                        props
                    },
                    required: vec!["input_file".to_string(), "output_file".to_string(), "gain_db".to_string()],
                },
            },
            FunctionDeclaration {
                name: "audio_compand".to_string(),
                description: "Applies dynamic range compression/expansion using the FFmpeg compand filter.".to_string(),
                parameters: Parameters {
                    param_type: "object".to_string(),
                    properties: {
                        let mut props = HashMap::new();
                        props.insert("input_file".to_string(), PropertyDefinition { prop_type: "string".to_string(), description: "Path to the input file".to_string(), items: None });
                        props.insert("output_file".to_string(), PropertyDefinition { prop_type: "string".to_string(), description: "Path to save the output file".to_string(), items: None });
                        props.insert("attacks".to_string(), PropertyDefinition { prop_type: "string".to_string(), description: "Attack time(s) per channel comma-separated (default '0.3')".to_string(), items: None });
                        props.insert("decays".to_string(), PropertyDefinition { prop_type: "string".to_string(), description: "Decay time(s) per channel (default '0.8')".to_string(), items: None });
                        props.insert("points".to_string(), PropertyDefinition { prop_type: "string".to_string(), description: "Input/output level pairs (default '-70/-70 -60/-20 1/0')".to_string(), items: None });
                        props.insert("soft_knee_db".to_string(), PropertyDefinition { prop_type: "number".to_string(), description: "Soft knee width in dB (default 0.01)".to_string(), items: None });
                        props.insert("gain_db".to_string(), PropertyDefinition { prop_type: "number".to_string(), description: "Output gain in dB (default 0)".to_string(), items: None });
                        props
                    },
                    required: vec!["input_file".to_string(), "output_file".to_string()],
                },
            },
            FunctionDeclaration {
                name: "add_audio_delay".to_string(),
                description: "Adds delay to audio channels using the FFmpeg adelay filter.".to_string(),
                parameters: Parameters {
                    param_type: "object".to_string(),
                    properties: {
                        let mut props = HashMap::new();
                        props.insert("input_file".to_string(), PropertyDefinition { prop_type: "string".to_string(), description: "Path to the input file".to_string(), items: None });
                        props.insert("output_file".to_string(), PropertyDefinition { prop_type: "string".to_string(), description: "Path to save the output file".to_string(), items: None });
                        props.insert("delays_ms".to_string(), PropertyDefinition { prop_type: "string".to_string(), description: "Delay in ms per channel e.g. '500|500' or '1000'".to_string(), items: None });
                        props.insert("all_channels".to_string(), PropertyDefinition { prop_type: "boolean".to_string(), description: "Apply same delay to all channels (default true)".to_string(), items: None });
                        props
                    },
                    required: vec!["input_file".to_string(), "output_file".to_string(), "delays_ms".to_string()],
                },
            },
            FunctionDeclaration {
                name: "add_phaser".to_string(),
                description: "Adds a phaser effect to audio using the FFmpeg aphaser filter.".to_string(),
                parameters: Parameters {
                    param_type: "object".to_string(),
                    properties: {
                        let mut props = HashMap::new();
                        props.insert("input_file".to_string(), PropertyDefinition { prop_type: "string".to_string(), description: "Path to the input file".to_string(), items: None });
                        props.insert("output_file".to_string(), PropertyDefinition { prop_type: "string".to_string(), description: "Path to save the output file".to_string(), items: None });
                        props.insert("in_gain".to_string(), PropertyDefinition { prop_type: "number".to_string(), description: "Input gain (default 0.4)".to_string(), items: None });
                        props.insert("out_gain".to_string(), PropertyDefinition { prop_type: "number".to_string(), description: "Output gain (default 0.74)".to_string(), items: None });
                        props.insert("delay_ms".to_string(), PropertyDefinition { prop_type: "number".to_string(), description: "Delay in milliseconds (default 3.0)".to_string(), items: None });
                        props.insert("decay".to_string(), PropertyDefinition { prop_type: "number".to_string(), description: "Decay (0–1, default 0.4)".to_string(), items: None });
                        props.insert("speed_hz".to_string(), PropertyDefinition { prop_type: "number".to_string(), description: "Modulation speed in Hz (default 0.5)".to_string(), items: None });
                        props.insert("type".to_string(), PropertyDefinition { prop_type: "string".to_string(), description: "Waveform type: triangular/sinusoidal (default triangular)".to_string(), items: None });
                        props
                    },
                    required: vec!["input_file".to_string(), "output_file".to_string()],
                },
            },
            // ================================================================
            // BATCH 10 — Audio Restoration
            // ================================================================
            FunctionDeclaration {
                name: "remove_clicks".to_string(),
                description: "Removes clicks and pops from audio using the FFmpeg adeclick filter.".to_string(),
                parameters: Parameters {
                    param_type: "object".to_string(),
                    properties: {
                        let mut props = HashMap::new();
                        props.insert("input_file".to_string(), PropertyDefinition { prop_type: "string".to_string(), description: "Path to the input file".to_string(), items: None });
                        props.insert("output_file".to_string(), PropertyDefinition { prop_type: "string".to_string(), description: "Path to save the output file".to_string(), items: None });
                        props.insert("window_ms".to_string(), PropertyDefinition { prop_type: "number".to_string(), description: "Analysis window size in ms (55–100, default 55)".to_string(), items: None });
                        props.insert("overlap_pct".to_string(), PropertyDefinition { prop_type: "number".to_string(), description: "Window overlap percentage (50–95, default 75)".to_string(), items: None });
                        props.insert("ar_order".to_string(), PropertyDefinition { prop_type: "number".to_string(), description: "AR model order (default 2)".to_string(), items: None });
                        props.insert("threshold".to_string(), PropertyDefinition { prop_type: "number".to_string(), description: "Detection threshold (1–100, default 2)".to_string(), items: None });
                        props
                    },
                    required: vec!["input_file".to_string(), "output_file".to_string()],
                },
            },
            FunctionDeclaration {
                name: "restore_clipping".to_string(),
                description: "Restores clipped audio samples using the FFmpeg adeclip filter.".to_string(),
                parameters: Parameters {
                    param_type: "object".to_string(),
                    properties: {
                        let mut props = HashMap::new();
                        props.insert("input_file".to_string(), PropertyDefinition { prop_type: "string".to_string(), description: "Path to the input file".to_string(), items: None });
                        props.insert("output_file".to_string(), PropertyDefinition { prop_type: "string".to_string(), description: "Path to save the output file".to_string(), items: None });
                        props.insert("window_ms".to_string(), PropertyDefinition { prop_type: "number".to_string(), description: "Analysis window size in ms (default 55)".to_string(), items: None });
                        props.insert("overlap_pct".to_string(), PropertyDefinition { prop_type: "number".to_string(), description: "Window overlap percentage (default 75)".to_string(), items: None });
                        props.insert("ar_order".to_string(), PropertyDefinition { prop_type: "number".to_string(), description: "AR model order (default 8)".to_string(), items: None });
                        props.insert("threshold".to_string(), PropertyDefinition { prop_type: "number".to_string(), description: "Detection threshold (default 10)".to_string(), items: None });
                        props
                    },
                    required: vec!["input_file".to_string(), "output_file".to_string()],
                },
            },
            FunctionDeclaration {
                name: "remove_silence".to_string(),
                description: "Removes silence from audio using the FFmpeg silenceremove filter.".to_string(),
                parameters: Parameters {
                    param_type: "object".to_string(),
                    properties: {
                        let mut props = HashMap::new();
                        props.insert("input_file".to_string(), PropertyDefinition { prop_type: "string".to_string(), description: "Path to the input file".to_string(), items: None });
                        props.insert("output_file".to_string(), PropertyDefinition { prop_type: "string".to_string(), description: "Path to save the output file".to_string(), items: None });
                        props.insert("start_periods".to_string(), PropertyDefinition { prop_type: "number".to_string(), description: "Silence periods to remove at start (default 1)".to_string(), items: None });
                        props.insert("start_threshold_db".to_string(), PropertyDefinition { prop_type: "number".to_string(), description: "Start silence threshold in dB (default -50)".to_string(), items: None });
                        props.insert("stop_periods".to_string(), PropertyDefinition { prop_type: "number".to_string(), description: "Silence periods throughout (-1 = all, default -1)".to_string(), items: None });
                        props.insert("stop_threshold_db".to_string(), PropertyDefinition { prop_type: "number".to_string(), description: "Stop silence threshold in dB (default -50)".to_string(), items: None });
                        props.insert("stop_duration_sec".to_string(), PropertyDefinition { prop_type: "number".to_string(), description: "Minimum silence duration to remove in seconds (default 0.1)".to_string(), items: None });
                        props
                    },
                    required: vec!["input_file".to_string(), "output_file".to_string()],
                },
            },
            // ================================================================
            // BATCH 11 — Quality Metrics
            // ================================================================
            FunctionDeclaration {
                name: "analyze_audio_stats".to_string(),
                description: "Analyzes audio statistics (RMS, peak, crest factor) using the FFmpeg astats filter.".to_string(),
                parameters: Parameters {
                    param_type: "object".to_string(),
                    properties: {
                        let mut props = HashMap::new();
                        props.insert("input_file".to_string(), PropertyDefinition { prop_type: "string".to_string(), description: "Path to the input file".to_string(), items: None });
                        props.insert("reset_interval".to_string(), PropertyDefinition { prop_type: "number".to_string(), description: "Reset stats every N seconds (0 = whole file, default 0)".to_string(), items: None });
                        props
                    },
                    required: vec!["input_file".to_string()],
                },
            },
            FunctionDeclaration {
                name: "analyze_video_signal".to_string(),
                description: "Analyzes video signal statistics (luma/chroma min/max, saturation) using the FFmpeg signalstats filter.".to_string(),
                parameters: Parameters {
                    param_type: "object".to_string(),
                    properties: {
                        let mut props = HashMap::new();
                        props.insert("input_file".to_string(), PropertyDefinition { prop_type: "string".to_string(), description: "Path to the input video file".to_string(), items: None });
                        props
                    },
                    required: vec!["input_file".to_string()],
                },
            },
            // ================================================================
            // BATCH 12 — Geometric Transforms
            // ================================================================
            FunctionDeclaration {
                name: "correct_perspective".to_string(),
                description: "Corrects perspective distortion by mapping four corner points using the FFmpeg perspective filter.".to_string(),
                parameters: Parameters {
                    param_type: "object".to_string(),
                    properties: {
                        let mut props = HashMap::new();
                        props.insert("input_file".to_string(), PropertyDefinition { prop_type: "string".to_string(), description: "Path to the input video file".to_string(), items: None });
                        props.insert("output_file".to_string(), PropertyDefinition { prop_type: "string".to_string(), description: "Path to save the output video".to_string(), items: None });
                        props.insert("x0".to_string(), PropertyDefinition { prop_type: "number".to_string(), description: "Top-left X coordinate".to_string(), items: None });
                        props.insert("y0".to_string(), PropertyDefinition { prop_type: "number".to_string(), description: "Top-left Y coordinate".to_string(), items: None });
                        props.insert("x1".to_string(), PropertyDefinition { prop_type: "number".to_string(), description: "Top-right X coordinate".to_string(), items: None });
                        props.insert("y1".to_string(), PropertyDefinition { prop_type: "number".to_string(), description: "Top-right Y coordinate".to_string(), items: None });
                        props.insert("x2".to_string(), PropertyDefinition { prop_type: "number".to_string(), description: "Bottom-left X coordinate".to_string(), items: None });
                        props.insert("y2".to_string(), PropertyDefinition { prop_type: "number".to_string(), description: "Bottom-left Y coordinate".to_string(), items: None });
                        props.insert("x3".to_string(), PropertyDefinition { prop_type: "number".to_string(), description: "Bottom-right X coordinate".to_string(), items: None });
                        props.insert("y3".to_string(), PropertyDefinition { prop_type: "number".to_string(), description: "Bottom-right Y coordinate".to_string(), items: None });
                        props.insert("interpolation".to_string(), PropertyDefinition { prop_type: "string".to_string(), description: "Interpolation method: linear/cubic (default linear)".to_string(), items: None });
                        props
                    },
                    required: vec!["input_file".to_string(), "output_file".to_string()],
                },
            },
            FunctionDeclaration {
                name: "correct_lens".to_string(),
                description: "Corrects lens distortion (barrel/pincushion) using the FFmpeg lenscorrection filter.".to_string(),
                parameters: Parameters {
                    param_type: "object".to_string(),
                    properties: {
                        let mut props = HashMap::new();
                        props.insert("input_file".to_string(), PropertyDefinition { prop_type: "string".to_string(), description: "Path to the input video file".to_string(), items: None });
                        props.insert("output_file".to_string(), PropertyDefinition { prop_type: "string".to_string(), description: "Path to save the output video".to_string(), items: None });
                        props.insert("k1".to_string(), PropertyDefinition { prop_type: "number".to_string(), description: "Barrel distortion coefficient (-1.0–1.0, negative=barrel, positive=pincushion)".to_string(), items: None });
                        props.insert("k2".to_string(), PropertyDefinition { prop_type: "number".to_string(), description: "Secondary distortion coefficient (-1.0–1.0, default 0.0)".to_string(), items: None });
                        props.insert("center_x".to_string(), PropertyDefinition { prop_type: "number".to_string(), description: "Distortion center X (0.5 = center)".to_string(), items: None });
                        props.insert("center_y".to_string(), PropertyDefinition { prop_type: "number".to_string(), description: "Distortion center Y (0.5 = center)".to_string(), items: None });
                        props.insert("interpolation".to_string(), PropertyDefinition { prop_type: "string".to_string(), description: "Interpolation: nearest/bilinear (default bilinear)".to_string(), items: None });
                        props
                    },
                    required: vec!["input_file".to_string(), "output_file".to_string(), "k1".to_string()],
                },
            },
            // ================================================================
            // BATCH 13 — Temporal Frame Effects
            // ================================================================
            FunctionDeclaration {
                name: "blend_frames".to_string(),
                description: "Blends adjacent frames for motion blur or dream effects using the FFmpeg tblend filter.".to_string(),
                parameters: Parameters {
                    param_type: "object".to_string(),
                    properties: {
                        let mut props = HashMap::new();
                        props.insert("input_file".to_string(), PropertyDefinition { prop_type: "string".to_string(), description: "Path to the input video file".to_string(), items: None });
                        props.insert("output_file".to_string(), PropertyDefinition { prop_type: "string".to_string(), description: "Path to save the output video".to_string(), items: None });
                        props.insert("blend_mode".to_string(), PropertyDefinition { prop_type: "string".to_string(), description: "Blend mode: average/addition/multiply/screen/overlay/grainmerge (default average)".to_string(), items: None });
                        props.insert("opacity".to_string(), PropertyDefinition { prop_type: "number".to_string(), description: "Blend opacity (0.0–1.0, default 1.0)".to_string(), items: None });
                        props
                    },
                    required: vec!["input_file".to_string(), "output_file".to_string()],
                },
            },
            FunctionDeclaration {
                name: "temporal_median".to_string(),
                description: "Applies temporal median filtering to remove ghosting/flicker using the FFmpeg tmedian filter.".to_string(),
                parameters: Parameters {
                    param_type: "object".to_string(),
                    properties: {
                        let mut props = HashMap::new();
                        props.insert("input_file".to_string(), PropertyDefinition { prop_type: "string".to_string(), description: "Path to the input video file".to_string(), items: None });
                        props.insert("output_file".to_string(), PropertyDefinition { prop_type: "string".to_string(), description: "Path to save the output video".to_string(), items: None });
                        props.insert("radius".to_string(), PropertyDefinition { prop_type: "number".to_string(), description: "Number of frames on each side to sample (1–127, default 1)".to_string(), items: None });
                        props
                    },
                    required: vec!["input_file".to_string(), "output_file".to_string()],
                },
            },
            FunctionDeclaration {
                name: "convert_framerate".to_string(),
                description: "Converts video frame rate using the FFmpeg fps filter.".to_string(),
                parameters: Parameters {
                    param_type: "object".to_string(),
                    properties: {
                        let mut props = HashMap::new();
                        props.insert("input_file".to_string(), PropertyDefinition { prop_type: "string".to_string(), description: "Path to the input video file".to_string(), items: None });
                        props.insert("output_file".to_string(), PropertyDefinition { prop_type: "string".to_string(), description: "Path to save the output video".to_string(), items: None });
                        props.insert("target_fps".to_string(), PropertyDefinition { prop_type: "number".to_string(), description: "Target frame rate (e.g. 24, 25, 30, 60)".to_string(), items: None });
                        props.insert("round_mode".to_string(), PropertyDefinition { prop_type: "string".to_string(), description: "Rounding mode: near/up/down/zero/inf (default near)".to_string(), items: None });
                        props
                    },
                    required: vec!["input_file".to_string(), "output_file".to_string(), "target_fps".to_string()],
                },
            },
            FunctionDeclaration {
                name: "tile_frames".to_string(),
                description: "Arranges video frames as a grid/contact sheet image using the FFmpeg tile filter.".to_string(),
                parameters: Parameters {
                    param_type: "object".to_string(),
                    properties: {
                        let mut props = HashMap::new();
                        props.insert("input_file".to_string(), PropertyDefinition { prop_type: "string".to_string(), description: "Path to the input video file".to_string(), items: None });
                        props.insert("output_file".to_string(), PropertyDefinition { prop_type: "string".to_string(), description: "Path to save the output image (.jpg or .png)".to_string(), items: None });
                        props.insert("columns".to_string(), PropertyDefinition { prop_type: "number".to_string(), description: "Number of columns in the tile grid (default 4)".to_string(), items: None });
                        props.insert("rows".to_string(), PropertyDefinition { prop_type: "number".to_string(), description: "Number of rows in the tile grid (default 3)".to_string(), items: None });
                        props.insert("frame_gap".to_string(), PropertyDefinition { prop_type: "number".to_string(), description: "Pixels between tiles (default 2)".to_string(), items: None });
                        props
                    },
                    required: vec!["input_file".to_string(), "output_file".to_string(), "columns".to_string(), "rows".to_string()],
                },
            },
            // ================================================================
            // BATCH 14 — Spatial Audio
            // ================================================================
            FunctionDeclaration {
                name: "adjust_stereo_width".to_string(),
                description: "Adjusts stereo width and balance using the FFmpeg stereotools filter.".to_string(),
                parameters: Parameters {
                    param_type: "object".to_string(),
                    properties: {
                        let mut props = HashMap::new();
                        props.insert("input_file".to_string(), PropertyDefinition { prop_type: "string".to_string(), description: "Path to the input file".to_string(), items: None });
                        props.insert("output_file".to_string(), PropertyDefinition { prop_type: "string".to_string(), description: "Path to save the output file".to_string(), items: None });
                        props.insert("width".to_string(), PropertyDefinition { prop_type: "number".to_string(), description: "Stereo width (0=mono, 1=unchanged, 2=wide, range 0–4)".to_string(), items: None });
                        props.insert("balance".to_string(), PropertyDefinition { prop_type: "number".to_string(), description: "Balance left/right (-1.0 to 1.0, default 0.0)".to_string(), items: None });
                        props.insert("mode".to_string(), PropertyDefinition { prop_type: "string".to_string(), description: "Stereo mode: lr>lr/lr>ms/ms>lr/lr>ll/lr>rr (default lr>lr)".to_string(), items: None });
                        props
                    },
                    required: vec!["input_file".to_string(), "output_file".to_string()],
                },
            },
            FunctionDeclaration {
                name: "mix_audio_channels".to_string(),
                description: "Mixes and routes audio channels using the FFmpeg pan filter.".to_string(),
                parameters: Parameters {
                    param_type: "object".to_string(),
                    properties: {
                        let mut props = HashMap::new();
                        props.insert("input_file".to_string(), PropertyDefinition { prop_type: "string".to_string(), description: "Path to the input file".to_string(), items: None });
                        props.insert("output_file".to_string(), PropertyDefinition { prop_type: "string".to_string(), description: "Path to save the output file".to_string(), items: None });
                        props.insert("channel_layout".to_string(), PropertyDefinition { prop_type: "string".to_string(), description: "Output channel layout e.g. 'stereo', 'mono', '5.1'".to_string(), items: None });
                        props.insert("channel_mix".to_string(), PropertyDefinition { prop_type: "string".to_string(), description: "Pipe-separated channel expressions e.g. 'c0=0.5*c0+0.5*c1|c1=0.5*c0+0.5*c1'".to_string(), items: None });
                        props
                    },
                    required: vec!["input_file".to_string(), "output_file".to_string(), "channel_layout".to_string(), "channel_mix".to_string()],
                },
            },

            // ================================================================
            // PHASE D — Professional Finishing Tools
            // ================================================================

            FunctionDeclaration {
                name: "adjust_color_temperature".to_string(),
                description: "Adjusts the color temperature (white balance) of a video. Lower Kelvin = warmer/orange, higher = cooler/blue.".to_string(),
                parameters: Parameters {
                    param_type: "object".to_string(),
                    properties: {
                        let mut props = HashMap::new();
                        props.insert("input_file".to_string(), PropertyDefinition { prop_type: "string".to_string(), description: "Path to the input video file".to_string(), items: None });
                        props.insert("output_file".to_string(), PropertyDefinition { prop_type: "string".to_string(), description: "Path to save the output file".to_string(), items: None });
                        props.insert("temperature".to_string(), PropertyDefinition { prop_type: "number".to_string(), description: "Color temperature in Kelvin (1000–40000). Default 6500.".to_string(), items: None });
                        props.insert("mix".to_string(), PropertyDefinition { prop_type: "number".to_string(), description: "Blend factor with original (0–1). Default 1.0.".to_string(), items: None });
                        props
                    },
                    required: vec!["input_file".to_string(), "output_file".to_string()],
                },
            },

            FunctionDeclaration {
                name: "adjust_vibrance".to_string(),
                description: "Adjusts vibrance — a selective saturation boost that enhances muted colours without over-saturating already vivid ones.".to_string(),
                parameters: Parameters {
                    param_type: "object".to_string(),
                    properties: {
                        let mut props = HashMap::new();
                        props.insert("input_file".to_string(), PropertyDefinition { prop_type: "string".to_string(), description: "Path to the input video file".to_string(), items: None });
                        props.insert("output_file".to_string(), PropertyDefinition { prop_type: "string".to_string(), description: "Path to save the output file".to_string(), items: None });
                        props.insert("intensity".to_string(), PropertyDefinition { prop_type: "number".to_string(), description: "Vibrance intensity (-2.0–2.0). Default 0.".to_string(), items: None });
                        props.insert("red_balance".to_string(), PropertyDefinition { prop_type: "number".to_string(), description: "Red channel balance (-10–10). Default 1.0.".to_string(), items: None });
                        props.insert("green_balance".to_string(), PropertyDefinition { prop_type: "number".to_string(), description: "Green channel balance (-10–10). Default 1.0.".to_string(), items: None });
                        props.insert("blue_balance".to_string(), PropertyDefinition { prop_type: "number".to_string(), description: "Blue channel balance (-10–10). Default 1.0.".to_string(), items: None });
                        props
                    },
                    required: vec!["input_file".to_string(), "output_file".to_string()],
                },
            },

            FunctionDeclaration {
                name: "remove_flicker".to_string(),
                description: "Removes temporal flicker from video (old film, timelapse, fluorescent lighting) by averaging luminance over a window of frames.".to_string(),
                parameters: Parameters {
                    param_type: "object".to_string(),
                    properties: {
                        let mut props = HashMap::new();
                        props.insert("input_file".to_string(), PropertyDefinition { prop_type: "string".to_string(), description: "Path to the input video file".to_string(), items: None });
                        props.insert("output_file".to_string(), PropertyDefinition { prop_type: "string".to_string(), description: "Path to save the output file".to_string(), items: None });
                        props.insert("size".to_string(), PropertyDefinition { prop_type: "number".to_string(), description: "Number of frames to average (2–129). Default 5.".to_string(), items: None });
                        props.insert("mode".to_string(), PropertyDefinition { prop_type: "string".to_string(), description: "Averaging mode: am/gm/hm/qm/cm/pm/median. Default 'am'.".to_string(), items: None });
                        props
                    },
                    required: vec!["input_file".to_string(), "output_file".to_string()],
                },
            },

            FunctionDeclaration {
                name: "denoise_video_bm3d".to_string(),
                description: "Applies BM3D (Block-Matching 3D) high-quality spatial denoising to video, preserving detail while removing noise and grain.".to_string(),
                parameters: Parameters {
                    param_type: "object".to_string(),
                    properties: {
                        let mut props = HashMap::new();
                        props.insert("input_file".to_string(), PropertyDefinition { prop_type: "string".to_string(), description: "Path to the input video file".to_string(), items: None });
                        props.insert("output_file".to_string(), PropertyDefinition { prop_type: "string".to_string(), description: "Path to save the output file".to_string(), items: None });
                        props.insert("sigma".to_string(), PropertyDefinition { prop_type: "number".to_string(), description: "Noise standard deviation (0.1–999.9). Default 1.0.".to_string(), items: None });
                        props.insert("block_size".to_string(), PropertyDefinition { prop_type: "number".to_string(), description: "Block size (8–64). Default 16.".to_string(), items: None });
                        props.insert("mode".to_string(), PropertyDefinition { prop_type: "string".to_string(), description: "Mode: 'basic' or 'final' (two-pass, higher quality). Default 'basic'.".to_string(), items: None });
                        props
                    },
                    required: vec!["input_file".to_string(), "output_file".to_string()],
                },
            },

            FunctionDeclaration {
                name: "deshake_video".to_string(),
                description: "Stabilises shaky handheld video using the FFmpeg deshake filter. Compensates for camera shake by analysing and correcting inter-frame motion.".to_string(),
                parameters: Parameters {
                    param_type: "object".to_string(),
                    properties: {
                        let mut props = HashMap::new();
                        props.insert("input_file".to_string(), PropertyDefinition { prop_type: "string".to_string(), description: "Path to the input video file".to_string(), items: None });
                        props.insert("output_file".to_string(), PropertyDefinition { prop_type: "string".to_string(), description: "Path to save the stabilised output file".to_string(), items: None });
                        props.insert("x".to_string(), PropertyDefinition { prop_type: "number".to_string(), description: "X offset of ROI (-1 = auto). Default -1.".to_string(), items: None });
                        props.insert("y".to_string(), PropertyDefinition { prop_type: "number".to_string(), description: "Y offset of ROI (-1 = auto). Default -1.".to_string(), items: None });
                        props.insert("w".to_string(), PropertyDefinition { prop_type: "number".to_string(), description: "Width of ROI (-1 = auto). Default -1.".to_string(), items: None });
                        props.insert("h".to_string(), PropertyDefinition { prop_type: "number".to_string(), description: "Height of ROI (-1 = auto). Default -1.".to_string(), items: None });
                        props.insert("rx".to_string(), PropertyDefinition { prop_type: "number".to_string(), description: "Max horizontal compensation in pixels. Default 16.".to_string(), items: None });
                        props.insert("ry".to_string(), PropertyDefinition { prop_type: "number".to_string(), description: "Max vertical compensation in pixels. Default 16.".to_string(), items: None });
                        props
                    },
                    required: vec!["input_file".to_string(), "output_file".to_string()],
                },
            },


            FunctionDeclaration {
                name: "parametric_eq".to_string(),
                description: "Applies a multi-band parametric equalizer to audio using the FFmpeg anequalizer filter. Supports peak, shelf, notch, and pass filters per channel.".to_string(),
                parameters: Parameters {
                    param_type: "object".to_string(),
                    properties: {
                        let mut props = HashMap::new();
                        props.insert("input_file".to_string(), PropertyDefinition { prop_type: "string".to_string(), description: "Path to the input audio/video file".to_string(), items: None });
                        props.insert("output_file".to_string(), PropertyDefinition { prop_type: "string".to_string(), description: "Path to save the processed output".to_string(), items: None });
                        props.insert("eq_params".to_string(), PropertyDefinition { prop_type: "string".to_string(), description: "Pipe-separated band specs: 'c<ch> f=<hz> w=<bw> g=<db> t=<type>' (types: 0=LPF,1=HPF,5=Peak,7=LSF,8=HSF). E.g. 'c0 f=1000 w=200 g=6 t=5'".to_string(), items: None });
                        props
                    },
                    required: vec!["input_file".to_string(), "output_file".to_string(), "eq_params".to_string()],
                },
            },

            FunctionDeclaration {
                name: "audio_limiter".to_string(),
                description: "Applies a brickwall limiter to prevent audio peaks exceeding a ceiling. Essential for broadcast delivery and preventing clipping.".to_string(),
                parameters: Parameters {
                    param_type: "object".to_string(),
                    properties: {
                        let mut props = HashMap::new();
                        props.insert("input_file".to_string(), PropertyDefinition { prop_type: "string".to_string(), description: "Path to the input audio/video file".to_string(), items: None });
                        props.insert("output_file".to_string(), PropertyDefinition { prop_type: "string".to_string(), description: "Path to save the limited output".to_string(), items: None });
                        props.insert("limit_db".to_string(), PropertyDefinition { prop_type: "number".to_string(), description: "Peak ceiling in dBFS (-10 to 0). Default -1.0.".to_string(), items: None });
                        props.insert("attack_ms".to_string(), PropertyDefinition { prop_type: "number".to_string(), description: "Attack time ms. Default 5.".to_string(), items: None });
                        props.insert("release_ms".to_string(), PropertyDefinition { prop_type: "number".to_string(), description: "Release time ms. Default 50.".to_string(), items: None });
                        props.insert("auto_sc".to_string(), PropertyDefinition { prop_type: "boolean".to_string(), description: "Enable auto-level sidechain. Default false.".to_string(), items: None });
                        props
                    },
                    required: vec!["input_file".to_string(), "output_file".to_string()],
                },
            },

            FunctionDeclaration {
                name: "reduce_sibilance".to_string(),
                description: "Reduces harsh sibilance ('ess' sounds) in vocals using the FFmpeg deesser filter. Targets high-frequency sibilant energy without dulling the overall sound.".to_string(),
                parameters: Parameters {
                    param_type: "object".to_string(),
                    properties: {
                        let mut props = HashMap::new();
                        props.insert("input_file".to_string(), PropertyDefinition { prop_type: "string".to_string(), description: "Path to the input audio/video file".to_string(), items: None });
                        props.insert("output_file".to_string(), PropertyDefinition { prop_type: "string".to_string(), description: "Path to save the de-essed output".to_string(), items: None });
                        props.insert("split_hz".to_string(), PropertyDefinition { prop_type: "number".to_string(), description: "Crossover frequency Hz (default 8500).".to_string(), items: None });
                        props.insert("threshold".to_string(), PropertyDefinition { prop_type: "number".to_string(), description: "Detection threshold (0–1). Default 0.1.".to_string(), items: None });
                        props.insert("mode".to_string(), PropertyDefinition { prop_type: "string".to_string(), description: "'split' or 'wide'. Default 'split'.".to_string(), items: None });
                        props
                    },
                    required: vec!["input_file".to_string(), "output_file".to_string()],
                },
            },

            FunctionDeclaration {
                name: "denoise_speech_rnn".to_string(),
                description: "Removes background noise from speech using a neural RNN model (arnndn). Very effective for voice-overs, interviews, and dialogue cleanup.".to_string(),
                parameters: Parameters {
                    param_type: "object".to_string(),
                    properties: {
                        let mut props = HashMap::new();
                        props.insert("input_file".to_string(), PropertyDefinition { prop_type: "string".to_string(), description: "Path to the input audio/video file".to_string(), items: None });
                        props.insert("output_file".to_string(), PropertyDefinition { prop_type: "string".to_string(), description: "Path to save the denoised output".to_string(), items: None });
                        props.insert("model_file".to_string(), PropertyDefinition { prop_type: "string".to_string(), description: "Optional path to .rnnn model file. Empty = built-in model.".to_string(), items: None });
                        props.insert("mix".to_string(), PropertyDefinition { prop_type: "number".to_string(), description: "Mix ratio (0=original, 1=fully denoised). Default 1.0.".to_string(), items: None });
                        props
                    },
                    required: vec!["input_file".to_string(), "output_file".to_string()],
                },
            },

            // ================================================================
            // PHASE F — Niche/Specialised Tools
            // ================================================================

            FunctionDeclaration {
                name: "displace_video".to_string(),
                description: "Displaces pixels using x/y displacement map videos (displace filter). Creates warping and distortion effects.".to_string(),
                parameters: Parameters {
                    param_type: "object".to_string(),
                    properties: {
                        let mut props = HashMap::new();
                        props.insert("input_file".to_string(), PropertyDefinition { prop_type: "string".to_string(), description: "Path to the main input video".to_string(), items: None });
                        props.insert("xmap_file".to_string(), PropertyDefinition { prop_type: "string".to_string(), description: "Path to horizontal displacement map video".to_string(), items: None });
                        props.insert("ymap_file".to_string(), PropertyDefinition { prop_type: "string".to_string(), description: "Path to vertical displacement map video".to_string(), items: None });
                        props.insert("output_file".to_string(), PropertyDefinition { prop_type: "string".to_string(), description: "Path to save the output".to_string(), items: None });
                        props.insert("edge".to_string(), PropertyDefinition { prop_type: "string".to_string(), description: "Edge mode: smear (default), wrap, mirror, blank.".to_string(), items: None });
                        props
                    },
                    required: vec!["input_file".to_string(), "xmap_file".to_string(), "ymap_file".to_string(), "output_file".to_string()],
                },
            },

            FunctionDeclaration {
                name: "decimate_frames".to_string(),
                description: "Removes duplicate frames to reduce effective frame rate (decimate filter).".to_string(),
                parameters: Parameters {
                    param_type: "object".to_string(),
                    properties: {
                        let mut props = HashMap::new();
                        props.insert("input_file".to_string(), PropertyDefinition { prop_type: "string".to_string(), description: "Path to the input video".to_string(), items: None });
                        props.insert("output_file".to_string(), PropertyDefinition { prop_type: "string".to_string(), description: "Path to save the output".to_string(), items: None });
                        props.insert("cycle".to_string(), PropertyDefinition { prop_type: "number".to_string(), description: "Frames per analysis cycle (2–25). Default 5.".to_string(), items: None });
                        props.insert("dupthresh".to_string(), PropertyDefinition { prop_type: "number".to_string(), description: "Duplicate detection threshold. Default 1.1.".to_string(), items: None });
                        props.insert("scthresh".to_string(), PropertyDefinition { prop_type: "number".to_string(), description: "Scene change threshold. Default 15.".to_string(), items: None });
                        props
                    },
                    required: vec!["input_file".to_string(), "output_file".to_string()],
                },
            },

            FunctionDeclaration {
                name: "denoise_video_owden".to_string(),
                description: "Overcomplete Wavelet denoising (owdenoise). Good for heavy noise reduction preserving texture.".to_string(),
                parameters: Parameters {
                    param_type: "object".to_string(),
                    properties: {
                        let mut props = HashMap::new();
                        props.insert("input_file".to_string(), PropertyDefinition { prop_type: "string".to_string(), description: "Path to the input video".to_string(), items: None });
                        props.insert("output_file".to_string(), PropertyDefinition { prop_type: "string".to_string(), description: "Path to save the output".to_string(), items: None });
                        props.insert("luma_strength".to_string(), PropertyDefinition { prop_type: "number".to_string(), description: "Luma strength (0–1000). Default 10.".to_string(), items: None });
                        props.insert("chroma_strength".to_string(), PropertyDefinition { prop_type: "number".to_string(), description: "Chroma strength (0–1000). Default 10.".to_string(), items: None });
                        props
                    },
                    required: vec!["input_file".to_string(), "output_file".to_string()],
                },
            },

            FunctionDeclaration {
                name: "despill_video".to_string(),
                description: "Removes green/blue screen colour spill from a keyed subject.".to_string(),
                parameters: Parameters {
                    param_type: "object".to_string(),
                    properties: {
                        let mut props = HashMap::new();
                        props.insert("input_file".to_string(), PropertyDefinition { prop_type: "string".to_string(), description: "Path to the input video".to_string(), items: None });
                        props.insert("output_file".to_string(), PropertyDefinition { prop_type: "string".to_string(), description: "Path to save the output".to_string(), items: None });
                        props.insert("spill_type".to_string(), PropertyDefinition { prop_type: "string".to_string(), description: "'green' (default) or 'blue'.".to_string(), items: None });
                        props.insert("mix".to_string(), PropertyDefinition { prop_type: "number".to_string(), description: "Mix with original (0–1). Default 0.5.".to_string(), items: None });
                        props.insert("expand".to_string(), PropertyDefinition { prop_type: "number".to_string(), description: "Despill region expansion (0–1). Default 0.".to_string(), items: None });
                        props
                    },
                    required: vec!["input_file".to_string(), "output_file".to_string()],
                },
            },

            FunctionDeclaration {
                name: "remap_pixels".to_string(),
                description: "Remaps pixels using x/y coordinate map videos (remap filter). Creates custom geometric distortions.".to_string(),
                parameters: Parameters {
                    param_type: "object".to_string(),
                    properties: {
                        let mut props = HashMap::new();
                        props.insert("input_file".to_string(), PropertyDefinition { prop_type: "string".to_string(), description: "Path to the main input video".to_string(), items: None });
                        props.insert("xmap_file".to_string(), PropertyDefinition { prop_type: "string".to_string(), description: "Path to x-coordinate map video".to_string(), items: None });
                        props.insert("ymap_file".to_string(), PropertyDefinition { prop_type: "string".to_string(), description: "Path to y-coordinate map video".to_string(), items: None });
                        props.insert("output_file".to_string(), PropertyDefinition { prop_type: "string".to_string(), description: "Path to save the output".to_string(), items: None });
                        props.insert("fill".to_string(), PropertyDefinition { prop_type: "string".to_string(), description: "Fill colour for out-of-bounds pixels. Default 'black'.".to_string(), items: None });
                        props
                    },
                    required: vec!["input_file".to_string(), "xmap_file".to_string(), "ymap_file".to_string(), "output_file".to_string()],
                },
            },

            FunctionDeclaration {
                name: "adjust_exposure".to_string(),
                description: "Adjusts video exposure in EV stops and black level using the exposure filter.".to_string(),
                parameters: Parameters {
                    param_type: "object".to_string(),
                    properties: {
                        let mut props = HashMap::new();
                        props.insert("input_file".to_string(), PropertyDefinition { prop_type: "string".to_string(), description: "Path to the input video".to_string(), items: None });
                        props.insert("output_file".to_string(), PropertyDefinition { prop_type: "string".to_string(), description: "Path to save the output".to_string(), items: None });
                        props.insert("exposure".to_string(), PropertyDefinition { prop_type: "number".to_string(), description: "Exposure in EV stops (-3 to 3). Default 0.".to_string(), items: None });
                        props.insert("black".to_string(), PropertyDefinition { prop_type: "number".to_string(), description: "Black pedestal (0–1). Default 0.".to_string(), items: None });
                        props
                    },
                    required: vec!["input_file".to_string(), "output_file".to_string()],
                },
            },


            FunctionDeclaration {
                name: "shift_audio_frequency".to_string(),
                description: "Shifts all audio frequencies by a constant Hz offset (afreqshift). Creates pitch-shift without time stretching.".to_string(),
                parameters: Parameters {
                    param_type: "object".to_string(),
                    properties: {
                        let mut props = HashMap::new();
                        props.insert("input_file".to_string(), PropertyDefinition { prop_type: "string".to_string(), description: "Path to the input audio/video file".to_string(), items: None });
                        props.insert("output_file".to_string(), PropertyDefinition { prop_type: "string".to_string(), description: "Path to save the output".to_string(), items: None });
                        props.insert("shift".to_string(), PropertyDefinition { prop_type: "number".to_string(), description: "Frequency shift Hz. Positive = up, negative = down.".to_string(), items: None });
                        props
                    },
                    required: vec!["input_file".to_string(), "output_file".to_string(), "shift".to_string()],
                },
            },


            FunctionDeclaration {
                name: "enhance_dialogue".to_string(),
                description: "Enhances speech/dialogue clarity without affecting music or effects (dialoguenhance filter).".to_string(),
                parameters: Parameters {
                    param_type: "object".to_string(),
                    properties: {
                        let mut props = HashMap::new();
                        props.insert("input_file".to_string(), PropertyDefinition { prop_type: "string".to_string(), description: "Path to the input audio/video file".to_string(), items: None });
                        props.insert("output_file".to_string(), PropertyDefinition { prop_type: "string".to_string(), description: "Path to save the output".to_string(), items: None });
                        props.insert("original".to_string(), PropertyDefinition { prop_type: "number".to_string(), description: "Original signal level (0–1). Default 0.5.".to_string(), items: None });
                        props.insert("expand".to_string(), PropertyDefinition { prop_type: "number".to_string(), description: "Enhancement strength (1–3). Default 2.".to_string(), items: None });
                        props
                    },
                    required: vec!["input_file".to_string(), "output_file".to_string()],
                },
            },

            FunctionDeclaration {
                name: "split_audio_channels".to_string(),
                description: "Extracts a single named channel from multichannel audio to a mono file.".to_string(),
                parameters: Parameters {
                    param_type: "object".to_string(),
                    properties: {
                        let mut props = HashMap::new();
                        props.insert("input_file".to_string(), PropertyDefinition { prop_type: "string".to_string(), description: "Path to the input audio/video file".to_string(), items: None });
                        props.insert("output_file".to_string(), PropertyDefinition { prop_type: "string".to_string(), description: "Path to save the mono channel".to_string(), items: None });
                        props.insert("channel_layout".to_string(), PropertyDefinition { prop_type: "string".to_string(), description: "Input layout: 'stereo', '5.1' etc. Default 'stereo'.".to_string(), items: None });
                        props.insert("channel".to_string(), PropertyDefinition { prop_type: "string".to_string(), description: "Channel: FL, FR, FC, LFE, BL, BR. Default 'FL'.".to_string(), items: None });
                        props
                    },
                    required: vec!["input_file".to_string(), "output_file".to_string()],
                },
            },

            FunctionDeclaration {
                name: "map_audio_channels".to_string(),
                description: "Remaps audio channels to different output positions (channelmap filter).".to_string(),
                parameters: Parameters {
                    param_type: "object".to_string(),
                    properties: {
                        let mut props = HashMap::new();
                        props.insert("input_file".to_string(), PropertyDefinition { prop_type: "string".to_string(), description: "Path to the input audio/video file".to_string(), items: None });
                        props.insert("output_file".to_string(), PropertyDefinition { prop_type: "string".to_string(), description: "Path to save the output".to_string(), items: None });
                        props.insert("channel_map".to_string(), PropertyDefinition { prop_type: "string".to_string(), description: "Mapping e.g. 'FL-FL|FR-FR'. Default 'FL-FL|FR-FR'.".to_string(), items: None });
                        props.insert("channel_layout".to_string(), PropertyDefinition { prop_type: "string".to_string(), description: "Output layout e.g. 'stereo'. Default 'stereo'.".to_string(), items: None });
                        props
                    },
                    required: vec!["input_file".to_string(), "output_file".to_string()],
                },
            },

            FunctionDeclaration {
                name: "merge_audio_inputs".to_string(),
                description: "Merges multiple audio files into multichannel audio (amerge). Comma-separated input paths.".to_string(),
                parameters: Parameters {
                    param_type: "object".to_string(),
                    properties: {
                        let mut props = HashMap::new();
                        props.insert("input_files".to_string(), PropertyDefinition { prop_type: "string".to_string(), description: "Comma-separated audio/video file paths (minimum 2)".to_string(), items: None });
                        props.insert("output_file".to_string(), PropertyDefinition { prop_type: "string".to_string(), description: "Path to save the merged output".to_string(), items: None });
                        props
                    },
                    required: vec!["input_files".to_string(), "output_file".to_string()],
                },
            },





            FunctionDeclaration {
                name: "filter_bandpass".to_string(),
                description: "Passes frequencies within a band, attenuates those outside (bandpass filter).".to_string(),
                parameters: Parameters {
                    param_type: "object".to_string(),
                    properties: {
                        let mut props = HashMap::new();
                        props.insert("input_file".to_string(), PropertyDefinition { prop_type: "string".to_string(), description: "Path to the input audio/video file".to_string(), items: None });
                        props.insert("output_file".to_string(), PropertyDefinition { prop_type: "string".to_string(), description: "Path to save the output".to_string(), items: None });
                        props.insert("frequency".to_string(), PropertyDefinition { prop_type: "number".to_string(), description: "Centre frequency Hz. Default 3000.".to_string(), items: None });
                        props.insert("width".to_string(), PropertyDefinition { prop_type: "number".to_string(), description: "Bandwidth. Default 200.".to_string(), items: None });
                        props.insert("width_type".to_string(), PropertyDefinition { prop_type: "string".to_string(), description: "h=Hz (default), q=Q, o=octaves, s=slope.".to_string(), items: None });
                        props
                    },
                    required: vec!["input_file".to_string(), "output_file".to_string()],
                },
            },

            FunctionDeclaration {
                name: "filter_bandreject".to_string(),
                description: "Attenuates a specific frequency band (notch/band-reject filter).".to_string(),
                parameters: Parameters {
                    param_type: "object".to_string(),
                    properties: {
                        let mut props = HashMap::new();
                        props.insert("input_file".to_string(), PropertyDefinition { prop_type: "string".to_string(), description: "Path to the input audio/video file".to_string(), items: None });
                        props.insert("output_file".to_string(), PropertyDefinition { prop_type: "string".to_string(), description: "Path to save the output".to_string(), items: None });
                        props.insert("frequency".to_string(), PropertyDefinition { prop_type: "number".to_string(), description: "Centre frequency to reject Hz. Default 3000.".to_string(), items: None });
                        props.insert("width".to_string(), PropertyDefinition { prop_type: "number".to_string(), description: "Rejection bandwidth. Default 200.".to_string(), items: None });
                        props.insert("width_type".to_string(), PropertyDefinition { prop_type: "string".to_string(), description: "h=Hz (default), q=Q, o=octaves, s=slope.".to_string(), items: None });
                        props
                    },
                    required: vec!["input_file".to_string(), "output_file".to_string()],
                },
            },

            FunctionDeclaration {
                name: "boost_sub_bass".to_string(),
                description: "Boosts sub-bass frequencies using asubboost. Adds low-end warmth and weight.".to_string(),
                parameters: Parameters {
                    param_type: "object".to_string(),
                    properties: {
                        let mut props = HashMap::new();
                        props.insert("input_file".to_string(), PropertyDefinition { prop_type: "string".to_string(), description: "Path to the input audio/video file".to_string(), items: None });
                        props.insert("output_file".to_string(), PropertyDefinition { prop_type: "string".to_string(), description: "Path to save the output".to_string(), items: None });
                        props.insert("dry".to_string(), PropertyDefinition { prop_type: "number".to_string(), description: "Dry level (0–1). Default 1.".to_string(), items: None });
                        props.insert("wet".to_string(), PropertyDefinition { prop_type: "number".to_string(), description: "Wet level (0–1). Default 1.".to_string(), items: None });
                        props.insert("freq".to_string(), PropertyDefinition { prop_type: "number".to_string(), description: "Sub-bass cutoff Hz (10–200). Default 20.".to_string(), items: None });
                        props.insert("decay".to_string(), PropertyDefinition { prop_type: "number".to_string(), description: "Decay factor (0–1). Default 0.7.".to_string(), items: None });
                        props
                    },
                    required: vec!["input_file".to_string(), "output_file".to_string()],
                },
            },

            // ================================================================
            // PHASE I — Long-tail sweep, Batch 1
            // ================================================================

            FunctionDeclaration {
                name: "zoom_pan".to_string(),
                description: "Ken Burns zoom-and-pan effect via FFmpeg zoompan filter.".to_string(),
                parameters: Parameters {
                    param_type: "object".to_string(),
                    properties: {
                        let mut props = HashMap::new();
                        props.insert("input_file".to_string(), PropertyDefinition { prop_type: "string".to_string(), description: "Path to the input video or image file".to_string(), items: None });
                        props.insert("output_file".to_string(), PropertyDefinition { prop_type: "string".to_string(), description: "Path to save the zoompan output".to_string(), items: None });
                        props.insert("zoom".to_string(), PropertyDefinition { prop_type: "number".to_string(), description: "Zoom level (>1.0). Default 1.5.".to_string(), items: None });
                        props.insert("x_expr".to_string(), PropertyDefinition { prop_type: "string".to_string(), description: "X position expression. Default centres frame.".to_string(), items: None });
                        props.insert("y_expr".to_string(), PropertyDefinition { prop_type: "string".to_string(), description: "Y position expression. Default centres frame.".to_string(), items: None });
                        props.insert("duration_frames".to_string(), PropertyDefinition { prop_type: "number".to_string(), description: "Duration in frames. Default 125.".to_string(), items: None });
                        props.insert("fps".to_string(), PropertyDefinition { prop_type: "number".to_string(), description: "Output frame rate. Default 25.".to_string(), items: None });
                        props
                    },
                    required: vec!["input_file".to_string(), "output_file".to_string()],
                },
            },

            FunctionDeclaration {
                name: "chromatic_aberration".to_string(),
                description: "Shifts R/B channels for chromatic aberration/lens fringing via FFmpeg rgbashift.".to_string(),
                parameters: Parameters {
                    param_type: "object".to_string(),
                    properties: {
                        let mut props = HashMap::new();
                        props.insert("input_file".to_string(), PropertyDefinition { prop_type: "string".to_string(), description: "Path to the input video file".to_string(), items: None });
                        props.insert("output_file".to_string(), PropertyDefinition { prop_type: "string".to_string(), description: "Path to save the output".to_string(), items: None });
                        props.insert("rh".to_string(), PropertyDefinition { prop_type: "number".to_string(), description: "Red horizontal shift px. Default 5.".to_string(), items: None });
                        props.insert("rv".to_string(), PropertyDefinition { prop_type: "number".to_string(), description: "Red vertical shift px. Default 0.".to_string(), items: None });
                        props.insert("bh".to_string(), PropertyDefinition { prop_type: "number".to_string(), description: "Blue horizontal shift px. Default -5.".to_string(), items: None });
                        props.insert("bv".to_string(), PropertyDefinition { prop_type: "number".to_string(), description: "Blue vertical shift px. Default 0.".to_string(), items: None });
                        props
                    },
                    required: vec!["input_file".to_string(), "output_file".to_string()],
                },
            },

            FunctionDeclaration {
                name: "temporal_blend".to_string(),
                description: "Blends consecutive frames via FFmpeg tblend. Creates motion blur and painterly effects.".to_string(),
                parameters: Parameters {
                    param_type: "object".to_string(),
                    properties: {
                        let mut props = HashMap::new();
                        props.insert("input_file".to_string(), PropertyDefinition { prop_type: "string".to_string(), description: "Path to the input video file".to_string(), items: None });
                        props.insert("output_file".to_string(), PropertyDefinition { prop_type: "string".to_string(), description: "Path to save the blended output".to_string(), items: None });
                        props.insert("mode".to_string(), PropertyDefinition { prop_type: "string".to_string(), description: "Blend mode: average (default), addition, multiply, screen, overlay, difference.".to_string(), items: None });
                        props.insert("opacity".to_string(), PropertyDefinition { prop_type: "number".to_string(), description: "Blend opacity (0–1). Default 1.".to_string(), items: None });
                        props
                    },
                    required: vec!["input_file".to_string(), "output_file".to_string()],
                },
            },

            FunctionDeclaration {
                name: "motion_interpolate".to_string(),
                description: "Motion-compensated frame interpolation via FFmpeg minterpolate. Creates smooth slow-motion or high-FPS output.".to_string(),
                parameters: Parameters {
                    param_type: "object".to_string(),
                    properties: {
                        let mut props = HashMap::new();
                        props.insert("input_file".to_string(), PropertyDefinition { prop_type: "string".to_string(), description: "Path to the input video file".to_string(), items: None });
                        props.insert("output_file".to_string(), PropertyDefinition { prop_type: "string".to_string(), description: "Path to save the interpolated output".to_string(), items: None });
                        props.insert("target_fps".to_string(), PropertyDefinition { prop_type: "number".to_string(), description: "Target frame rate. Default 60.".to_string(), items: None });
                        props.insert("mi_mode".to_string(), PropertyDefinition { prop_type: "string".to_string(), description: "Mode: mci (default), blend, dup.".to_string(), items: None });
                        props
                    },
                    required: vec!["input_file".to_string(), "output_file".to_string()],
                },
            },

            FunctionDeclaration {
                name: "correct_lens_simple".to_string(),
                description: "Corrects barrel/pincushion lens distortion via FFmpeg lenscorrection.".to_string(),
                parameters: Parameters {
                    param_type: "object".to_string(),
                    properties: {
                        let mut props = HashMap::new();
                        props.insert("input_file".to_string(), PropertyDefinition { prop_type: "string".to_string(), description: "Path to the input video file".to_string(), items: None });
                        props.insert("output_file".to_string(), PropertyDefinition { prop_type: "string".to_string(), description: "Path to save the corrected output".to_string(), items: None });
                        props.insert("k1".to_string(), PropertyDefinition { prop_type: "number".to_string(), description: "Barrel coefficient (negative=barrel, positive=pincushion). Default -0.1.".to_string(), items: None });
                        props.insert("k2".to_string(), PropertyDefinition { prop_type: "number".to_string(), description: "Secondary coefficient. Default 0.".to_string(), items: None });
                        props
                    },
                    required: vec!["input_file".to_string(), "output_file".to_string()],
                },
            },

            FunctionDeclaration {
                name: "deinterlace_yadif".to_string(),
                description: "Removes interlacing from broadcast footage via FFmpeg yadif.".to_string(),
                parameters: Parameters {
                    param_type: "object".to_string(),
                    properties: {
                        let mut props = HashMap::new();
                        props.insert("input_file".to_string(), PropertyDefinition { prop_type: "string".to_string(), description: "Path to the interlaced input".to_string(), items: None });
                        props.insert("output_file".to_string(), PropertyDefinition { prop_type: "string".to_string(), description: "Path to save the deinterlaced output".to_string(), items: None });
                        props.insert("mode".to_string(), PropertyDefinition { prop_type: "number".to_string(), description: "Mode: 0=send frame (default), 1=send field.".to_string(), items: None });
                        props.insert("parity".to_string(), PropertyDefinition { prop_type: "number".to_string(), description: "Parity: -1=auto (default), 0=TFF, 1=BFF.".to_string(), items: None });
                        props
                    },
                    required: vec!["input_file".to_string(), "output_file".to_string()],
                },
            },

            FunctionDeclaration {
                name: "correct_perspective_linear".to_string(),
                description: "Fixes perspective/keystone distortion via FFmpeg perspective filter.".to_string(),
                parameters: Parameters {
                    param_type: "object".to_string(),
                    properties: {
                        let mut props = HashMap::new();
                        props.insert("input_file".to_string(), PropertyDefinition { prop_type: "string".to_string(), description: "Path to the input video".to_string(), items: None });
                        props.insert("output_file".to_string(), PropertyDefinition { prop_type: "string".to_string(), description: "Path to save the corrected output".to_string(), items: None });
                        props.insert("x0".to_string(), PropertyDefinition { prop_type: "number".to_string(), description: "Top-left X".to_string(), items: None });
                        props.insert("y0".to_string(), PropertyDefinition { prop_type: "number".to_string(), description: "Top-left Y".to_string(), items: None });
                        props.insert("x1".to_string(), PropertyDefinition { prop_type: "number".to_string(), description: "Top-right X".to_string(), items: None });
                        props.insert("y1".to_string(), PropertyDefinition { prop_type: "number".to_string(), description: "Top-right Y".to_string(), items: None });
                        props.insert("x2".to_string(), PropertyDefinition { prop_type: "number".to_string(), description: "Bottom-left X".to_string(), items: None });
                        props.insert("y2".to_string(), PropertyDefinition { prop_type: "number".to_string(), description: "Bottom-left Y".to_string(), items: None });
                        props.insert("x3".to_string(), PropertyDefinition { prop_type: "number".to_string(), description: "Bottom-right X".to_string(), items: None });
                        props.insert("y3".to_string(), PropertyDefinition { prop_type: "number".to_string(), description: "Bottom-right Y".to_string(), items: None });
                        props
                    },
                    required: vec!["input_file".to_string(), "output_file".to_string(), "x0".to_string(), "y0".to_string(), "x1".to_string(), "y1".to_string(), "x2".to_string(), "y2".to_string(), "x3".to_string(), "y3".to_string()],
                },
            },

            FunctionDeclaration {
                name: "colorize_video".to_string(),
                description: "Colorizes grayscale video with a colour tint via FFmpeg colorize filter.".to_string(),
                parameters: Parameters {
                    param_type: "object".to_string(),
                    properties: {
                        let mut props = HashMap::new();
                        props.insert("input_file".to_string(), PropertyDefinition { prop_type: "string".to_string(), description: "Path to the input video file".to_string(), items: None });
                        props.insert("output_file".to_string(), PropertyDefinition { prop_type: "string".to_string(), description: "Path to save the colorized output".to_string(), items: None });
                        props.insert("hue".to_string(), PropertyDefinition { prop_type: "number".to_string(), description: "Hue degrees (0–360). Default 210.".to_string(), items: None });
                        props.insert("saturation".to_string(), PropertyDefinition { prop_type: "number".to_string(), description: "Saturation (0–1). Default 0.5.".to_string(), items: None });
                        props.insert("lightness".to_string(), PropertyDefinition { prop_type: "number".to_string(), description: "Lightness (-1–1). Default 0.".to_string(), items: None });
                        props
                    },
                    required: vec!["input_file".to_string(), "output_file".to_string()],
                },
            },

            FunctionDeclaration {
                name: "denoise_hqdn3d".to_string(),
                description: "Fast video denoising via FFmpeg hqdn3d (High Quality 3D Denoiser).".to_string(),
                parameters: Parameters {
                    param_type: "object".to_string(),
                    properties: {
                        let mut props = HashMap::new();
                        props.insert("input_file".to_string(), PropertyDefinition { prop_type: "string".to_string(), description: "Path to the input video file".to_string(), items: None });
                        props.insert("output_file".to_string(), PropertyDefinition { prop_type: "string".to_string(), description: "Path to save the denoised output".to_string(), items: None });
                        props.insert("luma_spatial".to_string(), PropertyDefinition { prop_type: "number".to_string(), description: "Luma spatial strength. Default 4.".to_string(), items: None });
                        props.insert("chroma_spatial".to_string(), PropertyDefinition { prop_type: "number".to_string(), description: "Chroma spatial strength. Default 3.".to_string(), items: None });
                        props.insert("luma_tmp".to_string(), PropertyDefinition { prop_type: "number".to_string(), description: "Luma temporal strength. Default 6.".to_string(), items: None });
                        props.insert("chroma_tmp".to_string(), PropertyDefinition { prop_type: "number".to_string(), description: "Chroma temporal strength. Default 4.5.".to_string(), items: None });
                        props
                    },
                    required: vec!["input_file".to_string(), "output_file".to_string()],
                },
            },

            FunctionDeclaration {
                name: "add_echo".to_string(),
                description: "Echo/delay effect via FFmpeg aecho filter.".to_string(),
                parameters: Parameters {
                    param_type: "object".to_string(),
                    properties: {
                        let mut props = HashMap::new();
                        props.insert("input_file".to_string(), PropertyDefinition { prop_type: "string".to_string(), description: "Path to input audio/video".to_string(), items: None });
                        props.insert("output_file".to_string(), PropertyDefinition { prop_type: "string".to_string(), description: "Path to save the echo output".to_string(), items: None });
                        props.insert("in_gain".to_string(), PropertyDefinition { prop_type: "number".to_string(), description: "Input gain (0–1). Default 0.6.".to_string(), items: None });
                        props.insert("out_gain".to_string(), PropertyDefinition { prop_type: "number".to_string(), description: "Output gain (0–1). Default 0.3.".to_string(), items: None });
                        props.insert("delays".to_string(), PropertyDefinition { prop_type: "string".to_string(), description: "Delay ms, pipe-separated. Default '1000'.".to_string(), items: None });
                        props.insert("decays".to_string(), PropertyDefinition { prop_type: "string".to_string(), description: "Decay factors, pipe-separated. Default '0.5'.".to_string(), items: None });
                        props
                    },
                    required: vec!["input_file".to_string(), "output_file".to_string()],
                },
            },

            FunctionDeclaration {
                name: "noise_gate".to_string(),
                description: "Noise gate via FFmpeg agate. Silences audio below threshold to remove background hiss.".to_string(),
                parameters: Parameters {
                    param_type: "object".to_string(),
                    properties: {
                        let mut props = HashMap::new();
                        props.insert("input_file".to_string(), PropertyDefinition { prop_type: "string".to_string(), description: "Path to input audio/video".to_string(), items: None });
                        props.insert("output_file".to_string(), PropertyDefinition { prop_type: "string".to_string(), description: "Path to save the gated output".to_string(), items: None });
                        props.insert("threshold".to_string(), PropertyDefinition { prop_type: "number".to_string(), description: "Gate threshold (0–1). Default 0.01.".to_string(), items: None });
                        props.insert("range".to_string(), PropertyDefinition { prop_type: "number".to_string(), description: "Range factor (0–1). Default 0.06125.".to_string(), items: None });
                        props.insert("attack".to_string(), PropertyDefinition { prop_type: "number".to_string(), description: "Attack ms. Default 20.".to_string(), items: None });
                        props.insert("release".to_string(), PropertyDefinition { prop_type: "number".to_string(), description: "Release ms. Default 250.".to_string(), items: None });
                        props
                    },
                    required: vec!["input_file".to_string(), "output_file".to_string()],
                },
            },

            FunctionDeclaration {
                name: "compress_dynamics".to_string(),
                description: "Dynamic range compression via FFmpeg acompressor.".to_string(),
                parameters: Parameters {
                    param_type: "object".to_string(),
                    properties: {
                        let mut props = HashMap::new();
                        props.insert("input_file".to_string(), PropertyDefinition { prop_type: "string".to_string(), description: "Path to input audio/video".to_string(), items: None });
                        props.insert("output_file".to_string(), PropertyDefinition { prop_type: "string".to_string(), description: "Path to save the compressed output".to_string(), items: None });
                        props.insert("threshold".to_string(), PropertyDefinition { prop_type: "number".to_string(), description: "Threshold (0–1). Default 0.125.".to_string(), items: None });
                        props.insert("ratio".to_string(), PropertyDefinition { prop_type: "number".to_string(), description: "Compression ratio. Default 4.".to_string(), items: None });
                        props.insert("attack".to_string(), PropertyDefinition { prop_type: "number".to_string(), description: "Attack ms. Default 20.".to_string(), items: None });
                        props.insert("release".to_string(), PropertyDefinition { prop_type: "number".to_string(), description: "Release ms. Default 250.".to_string(), items: None });
                        props.insert("makeup".to_string(), PropertyDefinition { prop_type: "number".to_string(), description: "Makeup gain. Default 1.".to_string(), items: None });
                        props
                    },
                    required: vec!["input_file".to_string(), "output_file".to_string()],
                },
            },

            FunctionDeclaration {
                name: "add_chorus".to_string(),
                description: "Chorus effect via FFmpeg chorus filter. Adds shimmer and doubled-voice character.".to_string(),
                parameters: Parameters {
                    param_type: "object".to_string(),
                    properties: {
                        let mut props = HashMap::new();
                        props.insert("input_file".to_string(), PropertyDefinition { prop_type: "string".to_string(), description: "Path to input audio/video".to_string(), items: None });
                        props.insert("output_file".to_string(), PropertyDefinition { prop_type: "string".to_string(), description: "Path to save the chorus output".to_string(), items: None });
                        props.insert("in_gain".to_string(), PropertyDefinition { prop_type: "number".to_string(), description: "Input gain. Default 0.4.".to_string(), items: None });
                        props.insert("out_gain".to_string(), PropertyDefinition { prop_type: "number".to_string(), description: "Output gain. Default 0.4.".to_string(), items: None });
                        props.insert("delays".to_string(), PropertyDefinition { prop_type: "string".to_string(), description: "Delay ms. Default '55'.".to_string(), items: None });
                        props.insert("decays".to_string(), PropertyDefinition { prop_type: "string".to_string(), description: "Decay. Default '0.4'.".to_string(), items: None });
                        props.insert("speeds".to_string(), PropertyDefinition { prop_type: "string".to_string(), description: "Mod speed Hz. Default '0.25'.".to_string(), items: None });
                        props.insert("depths".to_string(), PropertyDefinition { prop_type: "string".to_string(), description: "Mod depth. Default '2'.".to_string(), items: None });
                        props
                    },
                    required: vec!["input_file".to_string(), "output_file".to_string()],
                },
            },

            FunctionDeclaration {
                name: "widen_stereo".to_string(),
                description: "Stereo widening via FFmpeg stereowiden filter.".to_string(),
                parameters: Parameters {
                    param_type: "object".to_string(),
                    properties: {
                        let mut props = HashMap::new();
                        props.insert("input_file".to_string(), PropertyDefinition { prop_type: "string".to_string(), description: "Path to input audio/video".to_string(), items: None });
                        props.insert("output_file".to_string(), PropertyDefinition { prop_type: "string".to_string(), description: "Path to save the widened output".to_string(), items: None });
                        props.insert("delay".to_string(), PropertyDefinition { prop_type: "number".to_string(), description: "Delay ms (0–90). Default 20.".to_string(), items: None });
                        props.insert("feedback".to_string(), PropertyDefinition { prop_type: "number".to_string(), description: "Feedback (0–1). Default 0.".to_string(), items: None });
                        props.insert("crossfeed".to_string(), PropertyDefinition { prop_type: "number".to_string(), description: "Crossfeed (0–1). Default 0.".to_string(), items: None });
                        props.insert("drymix".to_string(), PropertyDefinition { prop_type: "number".to_string(), description: "Dry mix (0–1). Default 0.8.".to_string(), items: None });
                        props
                    },
                    required: vec!["input_file".to_string(), "output_file".to_string()],
                },
            },

            FunctionDeclaration {
                name: "normalize_speech".to_string(),
                description: "Speech volume normalisation via FFmpeg speechnorm.".to_string(),
                parameters: Parameters {
                    param_type: "object".to_string(),
                    properties: {
                        let mut props = HashMap::new();
                        props.insert("input_file".to_string(), PropertyDefinition { prop_type: "string".to_string(), description: "Path to input audio/video".to_string(), items: None });
                        props.insert("output_file".to_string(), PropertyDefinition { prop_type: "string".to_string(), description: "Path to save the normalised output".to_string(), items: None });
                        props.insert("peak".to_string(), PropertyDefinition { prop_type: "number".to_string(), description: "Peak target (0–1). Default 0.95.".to_string(), items: None });
                        props.insert("strength".to_string(), PropertyDefinition { prop_type: "number".to_string(), description: "Normalisation strength (0–1). Default 0.8.".to_string(), items: None });
                        props
                    },
                    required: vec!["input_file".to_string(), "output_file".to_string()],
                },
            },

            FunctionDeclaration {
                name: "remove_silence_simple".to_string(),
                description: "Removes silent segments via FFmpeg silenceremove.".to_string(),
                parameters: Parameters {
                    param_type: "object".to_string(),
                    properties: {
                        let mut props = HashMap::new();
                        props.insert("input_file".to_string(), PropertyDefinition { prop_type: "string".to_string(), description: "Path to input audio/video".to_string(), items: None });
                        props.insert("output_file".to_string(), PropertyDefinition { prop_type: "string".to_string(), description: "Path to save the output".to_string(), items: None });
                        props.insert("threshold".to_string(), PropertyDefinition { prop_type: "number".to_string(), description: "Silence threshold (0–1). Default 0.02.".to_string(), items: None });
                        props.insert("duration".to_string(), PropertyDefinition { prop_type: "number".to_string(), description: "Min silence duration s. Default 0.5.".to_string(), items: None });
                        props.insert("periods".to_string(), PropertyDefinition { prop_type: "number".to_string(), description: "Start periods. Default 1.".to_string(), items: None });
                        props
                    },
                    required: vec!["input_file".to_string(), "output_file".to_string()],
                },
            },

            FunctionDeclaration {
                name: "soft_clip_audio".to_string(),
                description: "Soft audio clipping via FFmpeg asoftclip. Prevents harsh digital distortion.".to_string(),
                parameters: Parameters {
                    param_type: "object".to_string(),
                    properties: {
                        let mut props = HashMap::new();
                        props.insert("input_file".to_string(), PropertyDefinition { prop_type: "string".to_string(), description: "Path to input audio/video".to_string(), items: None });
                        props.insert("output_file".to_string(), PropertyDefinition { prop_type: "string".to_string(), description: "Path to save the output".to_string(), items: None });
                        props.insert("clip_type".to_string(), PropertyDefinition { prop_type: "string".to_string(), description: "Clip curve: tanh (default), atan, cubic, exp, alg, quintic, sin, erf.".to_string(), items: None });
                        props.insert("param".to_string(), PropertyDefinition { prop_type: "number".to_string(), description: "Clipping parameter. Default 1.".to_string(), items: None });
                        props
                    },
                    required: vec!["input_file".to_string(), "output_file".to_string()],
                },
            },

            FunctionDeclaration {
                name: "segment_video".to_string(),
                description: "Splits video into fixed-duration segments via FFmpeg segment muxer.".to_string(),
                parameters: Parameters {
                    param_type: "object".to_string(),
                    properties: {
                        let mut props = HashMap::new();
                        props.insert("input_file".to_string(), PropertyDefinition { prop_type: "string".to_string(), description: "Path to the input video file".to_string(), items: None });
                        props.insert("output_pattern".to_string(), PropertyDefinition { prop_type: "string".to_string(), description: "Output pattern with %03d e.g. 'segment_%03d.mp4'".to_string(), items: None });
                        props.insert("segment_time".to_string(), PropertyDefinition { prop_type: "number".to_string(), description: "Seconds per segment. Default 60.".to_string(), items: None });
                        props.insert("reset_timestamps".to_string(), PropertyDefinition { prop_type: "string".to_string(), description: "Reset timestamps: true or false. Default true.".to_string(), items: None });
                        props
                    },
                    required: vec!["input_file".to_string(), "output_pattern".to_string()],
                },
            },

            FunctionDeclaration {
                name: "pad_video_time".to_string(),
                description: "Adds padding frames at the start/end of a video via FFmpeg tpad filter.".to_string(),
                parameters: Parameters {
                    param_type: "object".to_string(),
                    properties: {
                        let mut props = HashMap::new();
                        props.insert("input_file".to_string(), PropertyDefinition { prop_type: "string".to_string(), description: "Path to the input video file".to_string(), items: None });
                        props.insert("output_file".to_string(), PropertyDefinition { prop_type: "string".to_string(), description: "Path to save the padded output".to_string(), items: None });
                        props.insert("start_duration".to_string(), PropertyDefinition { prop_type: "number".to_string(), description: "Seconds of padding before video. Default 0.".to_string(), items: None });
                        props.insert("stop_duration".to_string(), PropertyDefinition { prop_type: "number".to_string(), description: "Seconds of padding after video. Default 0.".to_string(), items: None });
                        props.insert("color".to_string(), PropertyDefinition { prop_type: "string".to_string(), description: "Pad colour: 'black' (default), 'white', '#ff0000'.".to_string(), items: None });
                        props
                    },
                    required: vec!["input_file".to_string(), "output_file".to_string()],
                },
            },

            // ================================================================
            // PHASE I BATCH 10 — stabilize_video_2pass, lut_rgb, hsvhold, convert_pixel_format,
            //                    setsar, random_frames, visualize_cqt, visualize_frequencies,
            //                    audio_iir, audio_expression, convert_audio_format,
            //                    cross_correlate, audio_multiply, audio_contrast, decode_hdcd
            // ================================================================
            FunctionDeclaration {
                name: "stabilize_video_2pass".to_string(),
                description: "Two-pass video stabilization using vidstabdetect + vidstabtransform. Pass 1 analyzes shake, pass 2 applies correction.".to_string(),
                parameters: {
                    let mut p = HashMap::new();
                    p.insert("input_file".to_string(), PropertyDefinition { prop_type: "string".to_string(), description: "Path to input video file".to_string(), items: None });
                    p.insert("output_file".to_string(), PropertyDefinition { prop_type: "string".to_string(), description: "Path to output stabilized video file".to_string(), items: None });
                    p.insert("shakiness".to_string(), PropertyDefinition { prop_type: "number".to_string(), description: "Shakiness level 1-10 (default 5)".to_string(), items: None });
                    p.insert("accuracy".to_string(), PropertyDefinition { prop_type: "number".to_string(), description: "Detection accuracy 1-15 (default 15)".to_string(), items: None });
                    p.insert("smoothing".to_string(), PropertyDefinition { prop_type: "number".to_string(), description: "Smoothing frames (default 10)".to_string(), items: None });
                    p.insert("zoom".to_string(), PropertyDefinition { prop_type: "number".to_string(), description: "Additional zoom percent, 0=auto (default 0)".to_string(), items: None });
                    Parameters { param_type: "object".to_string(), properties: p, required: vec!["input_file".to_string(), "output_file".to_string()] }
                },
            },
            FunctionDeclaration {
                name: "apply_lut_rgb".to_string(),
                description: "Apply per-pixel RGB expression LUT using lutrgb filter. Create custom color transformations using math expressions on R, G, B channels.".to_string(),
                parameters: {
                    let mut p = HashMap::new();
                    p.insert("input_file".to_string(), PropertyDefinition { prop_type: "string".to_string(), description: "Path to input video file".to_string(), items: None });
                    p.insert("output_file".to_string(), PropertyDefinition { prop_type: "string".to_string(), description: "Path to output video file".to_string(), items: None });
                    p.insert("r_expr".to_string(), PropertyDefinition { prop_type: "string".to_string(), description: "Expression for red channel (default: val)".to_string(), items: None });
                    p.insert("g_expr".to_string(), PropertyDefinition { prop_type: "string".to_string(), description: "Expression for green channel (default: val)".to_string(), items: None });
                    p.insert("b_expr".to_string(), PropertyDefinition { prop_type: "string".to_string(), description: "Expression for blue channel (default: val)".to_string(), items: None });
                    Parameters { param_type: "object".to_string(), properties: p, required: vec!["input_file".to_string(), "output_file".to_string()] }
                },
            },
            FunctionDeclaration {
                name: "apply_hsvhold".to_string(),
                description: "Selective color hold using HSV: keep only pixels near a specific hue, convert others to greyscale.".to_string(),
                parameters: {
                    let mut p = HashMap::new();
                    p.insert("input_file".to_string(), PropertyDefinition { prop_type: "string".to_string(), description: "Path to input video file".to_string(), items: None });
                    p.insert("output_file".to_string(), PropertyDefinition { prop_type: "string".to_string(), description: "Path to output video file".to_string(), items: None });
                    p.insert("hue".to_string(), PropertyDefinition { prop_type: "number".to_string(), description: "Target hue to keep, 0-360 degrees (default 0)".to_string(), items: None });
                    p.insert("white".to_string(), PropertyDefinition { prop_type: "number".to_string(), description: "White threshold 0-1 (default 0.01)".to_string(), items: None });
                    p.insert("black".to_string(), PropertyDefinition { prop_type: "number".to_string(), description: "Black threshold 0-1 (default 0.01)".to_string(), items: None });
                    p.insert("similarity".to_string(), PropertyDefinition { prop_type: "number".to_string(), description: "Hue similarity radius 0-1 (default 0.01)".to_string(), items: None });
                    p.insert("blend".to_string(), PropertyDefinition { prop_type: "number".to_string(), description: "Blend factor 0-1 for soft edge (default 0)".to_string(), items: None });
                    Parameters { param_type: "object".to_string(), properties: p, required: vec!["input_file".to_string(), "output_file".to_string()] }
                },
            },
            FunctionDeclaration {
                name: "convert_pixel_format".to_string(),
                description: "Convert video pixel format using FFmpeg format filter. Useful for compatibility, HDR→SDR, or codec requirements.".to_string(),
                parameters: {
                    let mut p = HashMap::new();
                    p.insert("input_file".to_string(), PropertyDefinition { prop_type: "string".to_string(), description: "Path to input video file".to_string(), items: None });
                    p.insert("output_file".to_string(), PropertyDefinition { prop_type: "string".to_string(), description: "Path to output video file".to_string(), items: None });
                    p.insert("pix_fmt".to_string(), PropertyDefinition { prop_type: "string".to_string(), description: "Target pixel format (default: yuv420p). Common: yuv444p, nv12, rgb24, gbrp, p010le".to_string(), items: None });
                    Parameters { param_type: "object".to_string(), properties: p, required: vec!["input_file".to_string(), "output_file".to_string()] }
                },
            },
            FunctionDeclaration {
                name: "visualize_cqt".to_string(),
                description: "Render Constant-Q Transform (CQT) spectrum visualization video from audio using showcqt.".to_string(),
                parameters: {
                    let mut p = HashMap::new();
                    p.insert("input_file".to_string(), PropertyDefinition { prop_type: "string".to_string(), description: "Path to input audio or video file".to_string(), items: None });
                    p.insert("output_file".to_string(), PropertyDefinition { prop_type: "string".to_string(), description: "Path to output video visualization file".to_string(), items: None });
                    p.insert("width".to_string(), PropertyDefinition { prop_type: "number".to_string(), description: "Output width (default 1920)".to_string(), items: None });
                    p.insert("height".to_string(), PropertyDefinition { prop_type: "number".to_string(), description: "Output height (default 1080)".to_string(), items: None });
                    p.insert("bar_h".to_string(), PropertyDefinition { prop_type: "number".to_string(), description: "Bar area height in pixels (default 20)".to_string(), items: None });
                    p.insert("axis_h".to_string(), PropertyDefinition { prop_type: "number".to_string(), description: "Axis area height in pixels (default 30)".to_string(), items: None });
                    Parameters { param_type: "object".to_string(), properties: p, required: vec!["input_file".to_string(), "output_file".to_string()] }
                },
            },
            FunctionDeclaration {
                name: "visualize_frequencies".to_string(),
                description: "Render frequency spectrum visualization video from audio using showfreqs.".to_string(),
                parameters: {
                    let mut p = HashMap::new();
                    p.insert("input_file".to_string(), PropertyDefinition { prop_type: "string".to_string(), description: "Path to input audio or video file".to_string(), items: None });
                    p.insert("output_file".to_string(), PropertyDefinition { prop_type: "string".to_string(), description: "Path to output video visualization file".to_string(), items: None });
                    p.insert("width".to_string(), PropertyDefinition { prop_type: "number".to_string(), description: "Output width (default 1024)".to_string(), items: None });
                    p.insert("height".to_string(), PropertyDefinition { prop_type: "number".to_string(), description: "Output height (default 512)".to_string(), items: None });
                    p.insert("mode".to_string(), PropertyDefinition { prop_type: "string".to_string(), description: "Display mode: line, bar, dot (default line)".to_string(), items: None });
                    p.insert("ascale".to_string(), PropertyDefinition { prop_type: "string".to_string(), description: "Amplitude scale: lin, sqrt, cbrt, log (default log)".to_string(), items: None });
                    Parameters { param_type: "object".to_string(), properties: p, required: vec!["input_file".to_string(), "output_file".to_string()] }
                },
            },
            FunctionDeclaration {
                name: "convert_audio_format".to_string(),
                description: "Force specific audio sample format, sample rate, and channel layout using aformat for codec compatibility.".to_string(),
                parameters: {
                    let mut p = HashMap::new();
                    p.insert("input_file".to_string(), PropertyDefinition { prop_type: "string".to_string(), description: "Path to input audio or video file".to_string(), items: None });
                    p.insert("output_file".to_string(), PropertyDefinition { prop_type: "string".to_string(), description: "Path to output file".to_string(), items: None });
                    p.insert("sample_fmts".to_string(), PropertyDefinition { prop_type: "string".to_string(), description: "Target sample format (e.g. s16, s32, fltp). Leave empty to keep current".to_string(), items: None });
                    p.insert("sample_rates".to_string(), PropertyDefinition { prop_type: "string".to_string(), description: "Target sample rate in Hz (e.g. 44100, 48000). Leave empty to keep current".to_string(), items: None });
                    p.insert("channel_layouts".to_string(), PropertyDefinition { prop_type: "string".to_string(), description: "Target channel layout (e.g. stereo, mono, 5.1). Leave empty to keep current".to_string(), items: None });
                    Parameters { param_type: "object".to_string(), properties: p, required: vec!["input_file".to_string(), "output_file".to_string()] }
                },
            },
            FunctionDeclaration {
                name: "decode_hdcd".to_string(),
                description: "Decode HDCD (High Definition Compatible Digital) encoded audio. Required for proper playback of HDCD masters with extended dynamic range.".to_string(),
                parameters: {
                    let mut p = HashMap::new();
                    p.insert("input_file".to_string(), PropertyDefinition { prop_type: "string".to_string(), description: "Path to input HDCD audio file".to_string(), items: None });
                    p.insert("output_file".to_string(), PropertyDefinition { prop_type: "string".to_string(), description: "Path to output decoded file".to_string(), items: None });
                    p.insert("disable_autoconvert".to_string(), PropertyDefinition { prop_type: "boolean".to_string(), description: "Disable automatic format conversion (default false)".to_string(), items: None });
                    p.insert("process_stereo".to_string(), PropertyDefinition { prop_type: "boolean".to_string(), description: "Process both channels as stereo pair (default false)".to_string(), items: None });
                    p.insert("force_pe".to_string(), PropertyDefinition { prop_type: "boolean".to_string(), description: "Force peak extend processing (default false)".to_string(), items: None });
                    Parameters { param_type: "object".to_string(), properties: p, required: vec!["input_file".to_string(), "output_file".to_string()] }
                },
            },
            // ================================================================
            // PHASE I BATCH 9 — scale_to_reference, fieldorder, optimize_gif_palette,
            //                   hsv_key, lut_yuv, freezeframes, draw_signal_graph, video_entropy,
            //                   compensation_delay, earwax, allpass, highshelf, lowshelf,
            //                   surround_upmix, detect_volume_levels
            // ================================================================

            FunctionDeclaration {
                name: "scale_to_reference".to_string(),
                description: "Scales a video to match the dimensions of a reference video using FFmpeg scale2ref.".to_string(),
                parameters: {
                    let mut p = HashMap::new();
                    p.insert("input_file".to_string(), PropertyDefinition { prop_type: "string".to_string(), description: "Path to video to scale".to_string(), items: None });
                    p.insert("ref_file".to_string(), PropertyDefinition { prop_type: "string".to_string(), description: "Path to reference video whose dimensions will be matched".to_string(), items: None });
                    p.insert("output_file".to_string(), PropertyDefinition { prop_type: "string".to_string(), description: "Path for output scaled video".to_string(), items: None });
                    p.insert("flags".to_string(), PropertyDefinition { prop_type: "string".to_string(), description: "Scaling algorithm (default: bilinear). Options: bilinear, bicubic, lanczos, area".to_string(), items: None });
                    Parameters { param_type: "object".to_string(), properties: p, required: vec!["input_file".to_string(), "ref_file".to_string(), "output_file".to_string()] }
                },
            },


            FunctionDeclaration {
                name: "optimize_gif_palette".to_string(),
                description: "Creates high-quality GIF using FFmpeg two-pass palettegen+paletteuse with dithering. Far better quality than simple GIF conversion.".to_string(),
                parameters: {
                    let mut p = HashMap::new();
                    p.insert("input_file".to_string(), PropertyDefinition { prop_type: "string".to_string(), description: "Path to input video file".to_string(), items: None });
                    p.insert("output_file".to_string(), PropertyDefinition { prop_type: "string".to_string(), description: "Path for output .gif file".to_string(), items: None });
                    p.insert("width".to_string(), PropertyDefinition { prop_type: "number".to_string(), description: "Output width pixels (default: 320)".to_string(), items: None });
                    p.insert("fps".to_string(), PropertyDefinition { prop_type: "number".to_string(), description: "Output frame rate (default: 10)".to_string(), items: None });
                    p.insert("stats_mode".to_string(), PropertyDefinition { prop_type: "string".to_string(), description: "Palette mode: diff (default), full, single".to_string(), items: None });
                    Parameters { param_type: "object".to_string(), properties: p, required: vec!["input_file".to_string(), "output_file".to_string()] }
                },
            },

            FunctionDeclaration {
                name: "apply_hsv_key".to_string(),
                description: "Keys out pixels based on HSV colour space using FFmpeg hsvkey. More precise than chroma key for complex or desaturated backgrounds.".to_string(),
                parameters: {
                    let mut p = HashMap::new();
                    p.insert("input_file".to_string(), PropertyDefinition { prop_type: "string".to_string(), description: "Path to input video file".to_string(), items: None });
                    p.insert("output_file".to_string(), PropertyDefinition { prop_type: "string".to_string(), description: "Path for output video file".to_string(), items: None });
                    p.insert("hue".to_string(), PropertyDefinition { prop_type: "number".to_string(), description: "Target hue 0–360 degrees (default: 0)".to_string(), items: None });
                    p.insert("saturation".to_string(), PropertyDefinition { prop_type: "number".to_string(), description: "Target saturation 0–1 (default: 0)".to_string(), items: None });
                    p.insert("value".to_string(), PropertyDefinition { prop_type: "number".to_string(), description: "Target value/brightness 0–1 (default: 0)".to_string(), items: None });
                    p.insert("similarity".to_string(), PropertyDefinition { prop_type: "number".to_string(), description: "HSV distance tolerance 0–1 (default: 0.1)".to_string(), items: None });
                    p.insert("blend".to_string(), PropertyDefinition { prop_type: "number".to_string(), description: "Edge blend factor 0–1 (default: 0.0)".to_string(), items: None });
                    Parameters { param_type: "object".to_string(), properties: p, required: vec!["input_file".to_string(), "output_file".to_string()] }
                },
            },

            FunctionDeclaration {
                name: "apply_lut_yuv".to_string(),
                description: "Per-pixel YUV colour transformation using FFmpeg lutyuv. Apply custom expressions per channel: y='negval' inverts luma, u='128' removes chroma.".to_string(),
                parameters: {
                    let mut p = HashMap::new();
                    p.insert("input_file".to_string(), PropertyDefinition { prop_type: "string".to_string(), description: "Path to input video file".to_string(), items: None });
                    p.insert("output_file".to_string(), PropertyDefinition { prop_type: "string".to_string(), description: "Path for output video file".to_string(), items: None });
                    p.insert("y_expr".to_string(), PropertyDefinition { prop_type: "string".to_string(), description: "Y (luma) expression (default: val). e.g. 'negval', 'val/2'".to_string(), items: None });
                    p.insert("u_expr".to_string(), PropertyDefinition { prop_type: "string".to_string(), description: "U (Cb) expression (default: val). '128' = zero chroma".to_string(), items: None });
                    p.insert("v_expr".to_string(), PropertyDefinition { prop_type: "string".to_string(), description: "V (Cr) expression (default: val)".to_string(), items: None });
                    Parameters { param_type: "object".to_string(), properties: p, required: vec!["input_file".to_string(), "output_file".to_string()] }
                },
            },


            FunctionDeclaration {
                name: "draw_signal_graph".to_string(),
                description: "Draws a scrolling graph of video signal statistics over time using FFmpeg signalstats+drawgraph. Visualises YAVG, YMAX, UAVG, VAVG etc. for QC monitoring.".to_string(),
                parameters: {
                    let mut p = HashMap::new();
                    p.insert("input_file".to_string(), PropertyDefinition { prop_type: "string".to_string(), description: "Path to input video file".to_string(), items: None });
                    p.insert("output_file".to_string(), PropertyDefinition { prop_type: "string".to_string(), description: "Path for output video with graph".to_string(), items: None });
                    p.insert("signal".to_string(), PropertyDefinition { prop_type: "string".to_string(), description: "Signal to graph (default: YAVG). Options: YAVG, YMAX, YMIN, UAVG, VAVG, SATAVG, HUEMED".to_string(), items: None });
                    p.insert("width".to_string(), PropertyDefinition { prop_type: "number".to_string(), description: "Graph width pixels (default: 1280)".to_string(), items: None });
                    p.insert("height".to_string(), PropertyDefinition { prop_type: "number".to_string(), description: "Graph height pixels (default: 256)".to_string(), items: None });
                    Parameters { param_type: "object".to_string(), properties: p, required: vec!["input_file".to_string(), "output_file".to_string()] }
                },
            },









            // ================================================================
            // PHASE I BATCH 8 — extract_alpha, merge_alpha, framestep, swaprect,
            //                   fillborders, chromanr, weave, interlace,
            //                   denoise_audio_fft, loop_audio, dc_shift, dynamic_range,
            //                   single_eq_band, stereotools, asetrate
            // ================================================================


            FunctionDeclaration {
                name: "merge_alpha_channel".to_string(),
                description: "Merges a greyscale video as the alpha channel into a colour video using FFmpeg alphamerge.".to_string(),
                parameters: {
                    let mut p = HashMap::new();
                    p.insert("input_file".to_string(), PropertyDefinition { prop_type: "string".to_string(), description: "Path to base colour video".to_string(), items: None });
                    p.insert("alpha_file".to_string(), PropertyDefinition { prop_type: "string".to_string(), description: "Path to greyscale video to use as alpha".to_string(), items: None });
                    p.insert("output_file".to_string(), PropertyDefinition { prop_type: "string".to_string(), description: "Path for output video with alpha".to_string(), items: None });
                    Parameters { param_type: "object".to_string(), properties: p, required: vec!["input_file".to_string(), "alpha_file".to_string(), "output_file".to_string()] }
                },
            },







            FunctionDeclaration {
                name: "denoise_audio_fft".to_string(),
                description: "Reduces background noise using FFmpeg afftdn (FFT-based denoiser). Works on consistent background noise (fan, hum, room tone) without a model file.".to_string(),
                parameters: {
                    let mut p = HashMap::new();
                    p.insert("input_file".to_string(), PropertyDefinition { prop_type: "string".to_string(), description: "Path to input audio/video file".to_string(), items: None });
                    p.insert("output_file".to_string(), PropertyDefinition { prop_type: "string".to_string(), description: "Path for output file".to_string(), items: None });
                    p.insert("noise_floor".to_string(), PropertyDefinition { prop_type: "number".to_string(), description: "Noise floor in dB -100–0 (default: -25.0)".to_string(), items: None });
                    p.insert("noise_reduction".to_string(), PropertyDefinition { prop_type: "number".to_string(), description: "Reduction in dB 0.01–97 (default: 12.0)".to_string(), items: None });
                    p.insert("track_noise".to_string(), PropertyDefinition { prop_type: "boolean".to_string(), description: "Track changing noise (default: false)".to_string(), items: None });
                    Parameters { param_type: "object".to_string(), properties: p, required: vec!["input_file".to_string(), "output_file".to_string()] }
                },
            },

            FunctionDeclaration {
                name: "loop_audio".to_string(),
                description: "Loops an audio stream N times using FFmpeg aloop. Use to extend background music or create repeating audio textures.".to_string(),
                parameters: {
                    let mut p = HashMap::new();
                    p.insert("input_file".to_string(), PropertyDefinition { prop_type: "string".to_string(), description: "Path to input audio/video file".to_string(), items: None });
                    p.insert("output_file".to_string(), PropertyDefinition { prop_type: "string".to_string(), description: "Path for output file".to_string(), items: None });
                    p.insert("loop_count".to_string(), PropertyDefinition { prop_type: "number".to_string(), description: "Loops: -1=infinite, 0=no loop, N=loop N times (default: 1)".to_string(), items: None });
                    p.insert("size".to_string(), PropertyDefinition { prop_type: "number".to_string(), description: "Samples per loop (0 = whole file, default: 0)".to_string(), items: None });
                    p.insert("start".to_string(), PropertyDefinition { prop_type: "number".to_string(), description: "Sample offset to start loop (default: 0)".to_string(), items: None });
                    Parameters { param_type: "object".to_string(), properties: p, required: vec!["input_file".to_string(), "output_file".to_string()] }
                },
            },






            // ================================================================
            // PHASE I BATCH 7 — xfade_transition, color_key, monochrome, maskedmerge,
            //                   convert_360_video, fix_banding, greyedge, fade_video,
            //                   normalize_loudness, dynamic_audio_normalize, resample_audio,
            //                   trim_audio, crystalizer, multiband_compress, super_equalizer
            // ================================================================

            FunctionDeclaration {
                name: "apply_xfade_transition".to_string(),
                description: "Cross-fades between two video clips using FFmpeg xfade. Many transition types: fade, dissolve, wipeleft/right, slideleft/right, circlecrop, fadeblack, fadewhite, radial, etc.".to_string(),
                parameters: {
                    let mut p = HashMap::new();
                    p.insert("input_file".to_string(), PropertyDefinition { prop_type: "string".to_string(), description: "Path to first input video file".to_string(), items: None });
                    p.insert("secondary_file".to_string(), PropertyDefinition { prop_type: "string".to_string(), description: "Path to second input video file".to_string(), items: None });
                    p.insert("output_file".to_string(), PropertyDefinition { prop_type: "string".to_string(), description: "Path for output video file".to_string(), items: None });
                    p.insert("transition".to_string(), PropertyDefinition { prop_type: "string".to_string(), description: "Transition type (default: fade). e.g. fade, dissolve, wipeleft, wiperight, slideleft, slideright, circleopen, circleclose, fadeblack, fadewhite, radial".to_string(), items: None });
                    p.insert("duration".to_string(), PropertyDefinition { prop_type: "number".to_string(), description: "Duration of transition in seconds (default: 1.0)".to_string(), items: None });
                    p.insert("offset".to_string(), PropertyDefinition { prop_type: "number".to_string(), description: "Seconds into first clip where transition starts (default: 0.0)".to_string(), items: None });
                    Parameters { param_type: "object".to_string(), properties: p, required: vec!["input_file".to_string(), "secondary_file".to_string(), "output_file".to_string()] }
                },
            },



            FunctionDeclaration {
                name: "apply_maskedmerge".to_string(),
                description: "Merges two video streams using a third mask stream using FFmpeg maskedmerge. White mask = show overlay, black mask = show base.".to_string(),
                parameters: {
                    let mut p = HashMap::new();
                    p.insert("input_file".to_string(), PropertyDefinition { prop_type: "string".to_string(), description: "Path to base video file".to_string(), items: None });
                    p.insert("overlay_file".to_string(), PropertyDefinition { prop_type: "string".to_string(), description: "Path to overlay video file".to_string(), items: None });
                    p.insert("mask_file".to_string(), PropertyDefinition { prop_type: "string".to_string(), description: "Path to mask video file (white = overlay, black = base)".to_string(), items: None });
                    p.insert("output_file".to_string(), PropertyDefinition { prop_type: "string".to_string(), description: "Path for output video file".to_string(), items: None });
                    p.insert("planes".to_string(), PropertyDefinition { prop_type: "number".to_string(), description: "Bitmask of planes to process (default: 15 = all)".to_string(), items: None });
                    Parameters { param_type: "object".to_string(), properties: p, required: vec!["input_file".to_string(), "overlay_file".to_string(), "mask_file".to_string(), "output_file".to_string()] }
                },
            },

            FunctionDeclaration {
                name: "convert_360_video".to_string(),
                description: "Converts 360° video between projection formats using FFmpeg v360 (equirectangular, cubemap, flat, fisheye, etc.). Can extract a normal crop from 360° footage.".to_string(),
                parameters: {
                    let mut p = HashMap::new();
                    p.insert("input_file".to_string(), PropertyDefinition { prop_type: "string".to_string(), description: "Path to input 360° video file".to_string(), items: None });
                    p.insert("output_file".to_string(), PropertyDefinition { prop_type: "string".to_string(), description: "Path for output video file".to_string(), items: None });
                    p.insert("input_fmt".to_string(), PropertyDefinition { prop_type: "string".to_string(), description: "Input projection format (default: equirect). Options: equirect, c3x2, c6x1, eac, flat, fisheye, cylindrical, stereographic, mercator, healpix".to_string(), items: None });
                    p.insert("output_fmt".to_string(), PropertyDefinition { prop_type: "string".to_string(), description: "Output projection format (default: flat)".to_string(), items: None });
                    p.insert("width".to_string(), PropertyDefinition { prop_type: "number".to_string(), description: "Output width in pixels (default: 1920)".to_string(), items: None });
                    p.insert("height".to_string(), PropertyDefinition { prop_type: "number".to_string(), description: "Output height in pixels (default: 1080)".to_string(), items: None });
                    p.insert("h_fov".to_string(), PropertyDefinition { prop_type: "number".to_string(), description: "Horizontal field of view in degrees (default: 90.0)".to_string(), items: None });
                    p.insert("v_fov".to_string(), PropertyDefinition { prop_type: "number".to_string(), description: "Vertical field of view in degrees (default: 90.0)".to_string(), items: None });
                    Parameters { param_type: "object".to_string(), properties: p, required: vec!["input_file".to_string(), "output_file".to_string()] }
                },
            },

            FunctionDeclaration {
                name: "fix_banding".to_string(),
                description: "Fixes colour banding in gradients using FFmpeg gradfun (gradient dithering). Adds subtle noise to smooth areas to break up visible colour bands.".to_string(),
                parameters: {
                    let mut p = HashMap::new();
                    p.insert("input_file".to_string(), PropertyDefinition { prop_type: "string".to_string(), description: "Path to input video file".to_string(), items: None });
                    p.insert("output_file".to_string(), PropertyDefinition { prop_type: "string".to_string(), description: "Path for output video file".to_string(), items: None });
                    p.insert("strength".to_string(), PropertyDefinition { prop_type: "number".to_string(), description: "Dithering strength 0.51–65.0 (default: 1.2)".to_string(), items: None });
                    p.insert("radius".to_string(), PropertyDefinition { prop_type: "number".to_string(), description: "Gradient detection radius 4–32 (default: 16)".to_string(), items: None });
                    Parameters { param_type: "object".to_string(), properties: p, required: vec!["input_file".to_string(), "output_file".to_string()] }
                },
            },



            FunctionDeclaration {
                name: "normalize_loudness".to_string(),
                description: "Normalises audio to target integrated loudness using FFmpeg loudnorm (EBU R128). Broadcast=-23 LUFS, streaming=-14 LUFS, podcasts=-16 LUFS.".to_string(),
                parameters: {
                    let mut p = HashMap::new();
                    p.insert("input_file".to_string(), PropertyDefinition { prop_type: "string".to_string(), description: "Path to input audio/video file".to_string(), items: None });
                    p.insert("output_file".to_string(), PropertyDefinition { prop_type: "string".to_string(), description: "Path for output file".to_string(), items: None });
                    p.insert("i".to_string(), PropertyDefinition { prop_type: "number".to_string(), description: "Target integrated loudness LUFS (default: -23.0)".to_string(), items: None });
                    p.insert("lra".to_string(), PropertyDefinition { prop_type: "number".to_string(), description: "Target loudness range LU (default: 7.0, range 1–20)".to_string(), items: None });
                    p.insert("tp".to_string(), PropertyDefinition { prop_type: "number".to_string(), description: "Max true peak dBFS (default: -2.0)".to_string(), items: None });
                    Parameters { param_type: "object".to_string(), properties: p, required: vec!["input_file".to_string(), "output_file".to_string()] }
                },
            },

            FunctionDeclaration {
                name: "dynamic_audio_normalize".to_string(),
                description: "Per-frame dynamic normalisation using FFmpeg dynaudnorm. Levels out wildly varying volume while preserving dynamics.".to_string(),
                parameters: {
                    let mut p = HashMap::new();
                    p.insert("input_file".to_string(), PropertyDefinition { prop_type: "string".to_string(), description: "Path to input audio/video file".to_string(), items: None });
                    p.insert("output_file".to_string(), PropertyDefinition { prop_type: "string".to_string(), description: "Path for output file".to_string(), items: None });
                    p.insert("frame_len".to_string(), PropertyDefinition { prop_type: "number".to_string(), description: "Frame length ms 10–8000 (default: 500)".to_string(), items: None });
                    p.insert("gausssize".to_string(), PropertyDefinition { prop_type: "number".to_string(), description: "Gaussian window size odd 3–301 (default: 31)".to_string(), items: None });
                    p.insert("peak".to_string(), PropertyDefinition { prop_type: "number".to_string(), description: "Target peak 0.0–1.0 (default: 0.95)".to_string(), items: None });
                    p.insert("max_gain".to_string(), PropertyDefinition { prop_type: "number".to_string(), description: "Max gain factor 1.0–100.0 (default: 10.0)".to_string(), items: None });
                    p.insert("rms".to_string(), PropertyDefinition { prop_type: "number".to_string(), description: "RMS target 0.0–1.0 (default: 0.0 = peak mode)".to_string(), items: None });
                    p.insert("coupling".to_string(), PropertyDefinition { prop_type: "boolean".to_string(), description: "Couple channels together (default: true)".to_string(), items: None });
                    Parameters { param_type: "object".to_string(), properties: p, required: vec!["input_file".to_string(), "output_file".to_string()] }
                },
            },

            FunctionDeclaration {
                name: "resample_audio".to_string(),
                description: "Resamples audio to a different sample rate using FFmpeg aresample. Converts between 44.1kHz and 48kHz or fixes mismatched rates.".to_string(),
                parameters: {
                    let mut p = HashMap::new();
                    p.insert("input_file".to_string(), PropertyDefinition { prop_type: "string".to_string(), description: "Path to input audio/video file".to_string(), items: None });
                    p.insert("output_file".to_string(), PropertyDefinition { prop_type: "string".to_string(), description: "Path for output file".to_string(), items: None });
                    p.insert("sample_rate".to_string(), PropertyDefinition { prop_type: "number".to_string(), description: "Target sample rate Hz (default: 44100). Common: 22050, 44100, 48000, 96000".to_string(), items: None });
                    p.insert("resampler".to_string(), PropertyDefinition { prop_type: "string".to_string(), description: "Resampler: swr (default, fast) or soxr (high quality)".to_string(), items: None });
                    Parameters { param_type: "object".to_string(), properties: p, required: vec!["input_file".to_string(), "output_file".to_string()] }
                },
            },

            FunctionDeclaration {
                name: "trim_audio".to_string(),
                description: "Trims an audio stream to a time range using FFmpeg atrim. Resets timestamps to zero. Precise frame-accurate audio cutting.".to_string(),
                parameters: {
                    let mut p = HashMap::new();
                    p.insert("input_file".to_string(), PropertyDefinition { prop_type: "string".to_string(), description: "Path to input audio/video file".to_string(), items: None });
                    p.insert("output_file".to_string(), PropertyDefinition { prop_type: "string".to_string(), description: "Path for output file".to_string(), items: None });
                    p.insert("start".to_string(), PropertyDefinition { prop_type: "number".to_string(), description: "Start time in seconds (default: 0.0)".to_string(), items: None });
                    p.insert("end".to_string(), PropertyDefinition { prop_type: "number".to_string(), description: "End time in seconds (0 = use duration)".to_string(), items: None });
                    p.insert("duration".to_string(), PropertyDefinition { prop_type: "number".to_string(), description: "Duration from start in seconds (0 = use end)".to_string(), items: None });
                    Parameters { param_type: "object".to_string(), properties: p, required: vec!["input_file".to_string(), "output_file".to_string()] }
                },
            },


            FunctionDeclaration {
                name: "multiband_compress".to_string(),
                description: "Multiband dynamic range compression using FFmpeg mcompand. Compresses each frequency band independently for transparent mastering.".to_string(),
                parameters: {
                    let mut p = HashMap::new();
                    p.insert("input_file".to_string(), PropertyDefinition { prop_type: "string".to_string(), description: "Path to input audio/video file".to_string(), items: None });
                    p.insert("output_file".to_string(), PropertyDefinition { prop_type: "string".to_string(), description: "Path for output file".to_string(), items: None });
                    p.insert("params".to_string(), PropertyDefinition { prop_type: "string".to_string(), description: "mcompand band spec. Format: 'attacks decays points gain [crossover attacks decays points gain ...]'".to_string(), items: None });
                    Parameters { param_type: "object".to_string(), properties: p, required: vec!["input_file".to_string(), "output_file".to_string()] }
                },
            },


            // ================================================================
            // PHASE I BATCH 6 — colormatrix, chromashift, cas, nlmeans_video, spp, pp,
            //                   mestimate, midequalizer, median_spatial,
            //                   acrusher, atempo, asetnsamples, apad, asubcut, asupercut
            // ================================================================



            FunctionDeclaration {
                name: "apply_cas".to_string(),
                description: "Applies Contrast Adaptive Sharpening (CAS) — AMD FidelityFX-style adaptive sharpening.".to_string(),
                parameters: Parameters { param_type: "object".to_string(), properties: { let mut p = HashMap::new(); p.insert("input_file".to_string(), PropertyDefinition { prop_type: "string".to_string(), description: "Input video".to_string(), items: None }); p.insert("output_file".to_string(), PropertyDefinition { prop_type: "string".to_string(), description: "Output path".to_string(), items: None }); p.insert("strength".to_string(), PropertyDefinition { prop_type: "number".to_string(), description: "Sharpening strength 0.0-1.0 (default 0.0)".to_string(), items: None }); p.insert("planes".to_string(), PropertyDefinition { prop_type: "number".to_string(), description: "Bitmask of planes (default 7)".to_string(), items: None }); p }, required: vec!["input_file".to_string(), "output_file".to_string()] },
            },

            FunctionDeclaration {
                name: "apply_nlmeans_video".to_string(),
                description: "Applies Non-Local Means denoising for high-quality video noise reduction via patch similarity comparison.".to_string(),
                parameters: Parameters {
                    param_type: "object".to_string(),
                    properties: {
                        let mut p = HashMap::new();
                        p.insert("input_file".to_string(), PropertyDefinition { prop_type: "string".to_string(), description: "Input video".to_string(), items: None });
                        p.insert("output_file".to_string(), PropertyDefinition { prop_type: "string".to_string(), description: "Output path".to_string(), items: None });
                        p.insert("s".to_string(), PropertyDefinition { prop_type: "number".to_string(), description: "Denoising strength (default 1.0)".to_string(), items: None });
                        p.insert("p".to_string(), PropertyDefinition { prop_type: "number".to_string(), description: "Patch radius px (default 3)".to_string(), items: None });
                        p.insert("pc".to_string(), PropertyDefinition { prop_type: "number".to_string(), description: "Chroma patch radius (default = p)".to_string(), items: None });
                        p.insert("r".to_string(), PropertyDefinition { prop_type: "number".to_string(), description: "Research window radius (default 7)".to_string(), items: None });
                        p.insert("rc".to_string(), PropertyDefinition { prop_type: "number".to_string(), description: "Chroma research radius (default = r)".to_string(), items: None });
                        p
                    },
                    required: vec!["input_file".to_string(), "output_file".to_string()],
                },
            },

            FunctionDeclaration {
                name: "apply_spp".to_string(),
                description: "Applies Simple Post-Processing (spp) DCT-based deblocking and denoising for compressed video artefact removal.".to_string(),
                parameters: Parameters { param_type: "object".to_string(), properties: { let mut p = HashMap::new(); p.insert("input_file".to_string(), PropertyDefinition { prop_type: "string".to_string(), description: "Input video".to_string(), items: None }); p.insert("output_file".to_string(), PropertyDefinition { prop_type: "string".to_string(), description: "Output path".to_string(), items: None }); p.insert("quality".to_string(), PropertyDefinition { prop_type: "number".to_string(), description: "Quality 1-6 (default 3)".to_string(), items: None }); p.insert("qp".to_string(), PropertyDefinition { prop_type: "number".to_string(), description: "Fixed QP for strength (0=from stream, default 0)".to_string(), items: None }); p.insert("mode".to_string(), PropertyDefinition { prop_type: "string".to_string(), description: "hard or soft (default hard)".to_string(), items: None }); p }, required: vec!["input_file".to_string(), "output_file".to_string()] },
            },

            FunctionDeclaration {
                name: "apply_pp".to_string(),
                description: "Applies FFmpeg pp postprocess filter with deblocking/deringing subfilters like hb/vb/dr or 'default'.".to_string(),
                parameters: Parameters { param_type: "object".to_string(), properties: { let mut p = HashMap::new(); p.insert("input_file".to_string(), PropertyDefinition { prop_type: "string".to_string(), description: "Input video".to_string(), items: None }); p.insert("output_file".to_string(), PropertyDefinition { prop_type: "string".to_string(), description: "Output path".to_string(), items: None }); p.insert("subfilters".to_string(), PropertyDefinition { prop_type: "string".to_string(), description: "Subfilter string e.g. hb/vb/dr or default".to_string(), items: None }); p }, required: vec!["input_file".to_string(), "output_file".to_string()] },
            },

            FunctionDeclaration {
                name: "apply_mestimate".to_string(),
                description: "Estimates and visualises motion vectors using mestimate filter for motion analysis.".to_string(),
                parameters: Parameters { param_type: "object".to_string(), properties: { let mut p = HashMap::new(); p.insert("input_file".to_string(), PropertyDefinition { prop_type: "string".to_string(), description: "Input video".to_string(), items: None }); p.insert("output_file".to_string(), PropertyDefinition { prop_type: "string".to_string(), description: "Output path".to_string(), items: None }); p.insert("method".to_string(), PropertyDefinition { prop_type: "string".to_string(), description: "ME method: esa,tss,tdls,ntss,fss,ds,hexbs,epzs,umh (default esa)".to_string(), items: None }); p.insert("mb_size".to_string(), PropertyDefinition { prop_type: "number".to_string(), description: "Macroblock size (default 16)".to_string(), items: None }); p.insert("search_param".to_string(), PropertyDefinition { prop_type: "number".to_string(), description: "Search parameter (default 7)".to_string(), items: None }); p }, required: vec!["input_file".to_string(), "output_file".to_string()] },
            },

            FunctionDeclaration {
                name: "apply_midequalizer".to_string(),
                description: "Matches midtone exposure between two video streams using midequalizer for colour-matching shots.".to_string(),
                parameters: Parameters { param_type: "object".to_string(), properties: { let mut p = HashMap::new(); p.insert("input_file".to_string(), PropertyDefinition { prop_type: "string".to_string(), description: "Reference video".to_string(), items: None }); p.insert("secondary_file".to_string(), PropertyDefinition { prop_type: "string".to_string(), description: "Video to be matched".to_string(), items: None }); p.insert("output_file".to_string(), PropertyDefinition { prop_type: "string".to_string(), description: "Output path".to_string(), items: None }); p.insert("planes".to_string(), PropertyDefinition { prop_type: "number".to_string(), description: "Bitmask of planes (default 15)".to_string(), items: None }); p }, required: vec!["input_file".to_string(), "secondary_file".to_string(), "output_file".to_string()] },
            },

            FunctionDeclaration {
                name: "apply_median_spatial".to_string(),
                description: "Applies spatio-temporal median filter to remove outlier pixels and impulse noise across frames.".to_string(),
                parameters: Parameters { param_type: "object".to_string(), properties: { let mut p = HashMap::new(); p.insert("input_file".to_string(), PropertyDefinition { prop_type: "string".to_string(), description: "Input video".to_string(), items: None }); p.insert("output_file".to_string(), PropertyDefinition { prop_type: "string".to_string(), description: "Output path".to_string(), items: None }); p.insert("radius".to_string(), PropertyDefinition { prop_type: "number".to_string(), description: "Spatial radius 1-127 (default 1)".to_string(), items: None }); p.insert("radiusV".to_string(), PropertyDefinition { prop_type: "number".to_string(), description: "Vertical radius (default = radius)".to_string(), items: None }); p.insert("percentile".to_string(), PropertyDefinition { prop_type: "number".to_string(), description: "Percentile 0.0-1.0 (default 0.5=median)".to_string(), items: None }); p.insert("planes".to_string(), PropertyDefinition { prop_type: "number".to_string(), description: "Bitmask of planes (default 15)".to_string(), items: None }); p }, required: vec!["input_file".to_string(), "output_file".to_string()] },
            },







            // ================================================================
            // PHASE I BATCH 5 — threshold, maskedclamp, roberts, sobel, prewitt, kirsch,
            //                   video_limiter, bilateral, unsharp_mask, lagfun, tinterlace,
            //                   datascope, fspp, haas, aemphasis
            // ================================================================
















            // ================================================================
            // PHASE I BATCH 4 — negate, pixelize, colorlevels, pseudocolor, colorhold, shuffleplanes,
            //                   blackdetect, idet, vstack, hstack, setdar, stereo3d, telecine, pullup, thumbnail
            // ================================================================















            FunctionDeclaration {
                name: "select_thumbnail_frame".to_string(),
                description: "Selects the best representative thumbnail frame via FFmpeg thumbnail filter. Analyses N-frame batches for the most representative frame.".to_string(),
                parameters: Parameters { param_type: "object".to_string(), properties: { let mut p = HashMap::new(); p.insert("input_file".to_string(), PropertyDefinition { prop_type: "string".to_string(), description: "Input video file".to_string(), items: None }); p.insert("output_file".to_string(), PropertyDefinition { prop_type: "string".to_string(), description: "Output image file (e.g. thumb.jpg)".to_string(), items: None }); p.insert("n".to_string(), PropertyDefinition { prop_type: "number".to_string(), description: "Frames per batch. Default 100.".to_string(), items: None }); p }, required: vec!["input_file".to_string(), "output_file".to_string()] },
            },

            // ================================================================
            // PHASE I BATCH 3 — Blur variants, grain, rotation, geq, CCM, denoisers, LUT3D, SITI, amplify
            // ================================================================




            FunctionDeclaration {
                name: "add_film_grain".to_string(),
                description: "Adds analog film grain via FFmpeg noise filter. Simulates film texture.".to_string(),
                parameters: Parameters {
                    param_type: "object".to_string(),
                    properties: {
                        let mut props = HashMap::new();
                        props.insert("input_file".to_string(), PropertyDefinition { prop_type: "string".to_string(), description: "Input video file".to_string(), items: None });
                        props.insert("output_file".to_string(), PropertyDefinition { prop_type: "string".to_string(), description: "Output file path".to_string(), items: None });
                        props.insert("all_strength".to_string(), PropertyDefinition { prop_type: "number".to_string(), description: "Grain intensity 1–100. Default 8.".to_string(), items: None });
                        props.insert("flags".to_string(), PropertyDefinition { prop_type: "string".to_string(), description: "Grain type: a=additive (default), u=uniform, p=temporal animated.".to_string(), items: None });
                        props
                    },
                    required: vec!["input_file".to_string(), "output_file".to_string()],
                },
            },







            FunctionDeclaration {
                name: "generate_waveform_video".to_string(),
                description: "Renders audio amplitude waveform as video via FFmpeg showwaves. Audio visualisation and editing guide.".to_string(),
                parameters: Parameters {
                    param_type: "object".to_string(),
                    properties: {
                        let mut props = HashMap::new();
                        props.insert("input_file".to_string(), PropertyDefinition { prop_type: "string".to_string(), description: "Input audio/video file".to_string(), items: None });
                        props.insert("output_file".to_string(), PropertyDefinition { prop_type: "string".to_string(), description: "Output waveform video path".to_string(), items: None });
                        props.insert("width".to_string(), PropertyDefinition { prop_type: "number".to_string(), description: "Width px. Default 1280.".to_string(), items: None });
                        props.insert("height".to_string(), PropertyDefinition { prop_type: "number".to_string(), description: "Height px. Default 240.".to_string(), items: None });
                        props.insert("mode".to_string(), PropertyDefinition { prop_type: "string".to_string(), description: "Mode: line (default), point, p2p, cline.".to_string(), items: None });
                        props.insert("color".to_string(), PropertyDefinition { prop_type: "string".to_string(), description: "Waveform colour. Default 'white'.".to_string(), items: None });
                        props
                    },
                    required: vec!["input_file".to_string(), "output_file".to_string()],
                },
            },

            FunctionDeclaration {
                name: "apply_lut3d".to_string(),
                description: "Applies a 3D .cube LUT via FFmpeg lut3d filter. More precise than haldclut for colour grading.".to_string(),
                parameters: Parameters {
                    param_type: "object".to_string(),
                    properties: {
                        let mut props = HashMap::new();
                        props.insert("input_file".to_string(), PropertyDefinition { prop_type: "string".to_string(), description: "Input video file".to_string(), items: None });
                        props.insert("output_file".to_string(), PropertyDefinition { prop_type: "string".to_string(), description: "Output file path".to_string(), items: None });
                        props.insert("lut_file".to_string(), PropertyDefinition { prop_type: "string".to_string(), description: "Path to .cube LUT file".to_string(), items: None });
                        props.insert("interp".to_string(), PropertyDefinition { prop_type: "string".to_string(), description: "Interpolation: tetrahedral (default), trilinear, nearest.".to_string(), items: None });
                        props
                    },
                    required: vec!["input_file".to_string(), "output_file".to_string(), "lut_file".to_string()],
                },
            },


            FunctionDeclaration {
                name: "create_test_pattern".to_string(),
                description: "Generates a test pattern video via FFmpeg lavfi (smptebars, testsrc). No input file required.".to_string(),
                parameters: Parameters {
                    param_type: "object".to_string(),
                    properties: {
                        let mut props = HashMap::new();
                        props.insert("output_file".to_string(), PropertyDefinition { prop_type: "string".to_string(), description: "Output file path".to_string(), items: None });
                        props.insert("width".to_string(), PropertyDefinition { prop_type: "number".to_string(), description: "Width px. Default 1920.".to_string(), items: None });
                        props.insert("height".to_string(), PropertyDefinition { prop_type: "number".to_string(), description: "Height px. Default 1080.".to_string(), items: None });
                        props.insert("duration".to_string(), PropertyDefinition { prop_type: "number".to_string(), description: "Duration seconds. Default 10.".to_string(), items: None });
                        props.insert("pattern".to_string(), PropertyDefinition { prop_type: "string".to_string(), description: "Pattern: smptebars (default), smptehdbars, testsrc, testsrc2.".to_string(), items: None });
                        props.insert("framerate".to_string(), PropertyDefinition { prop_type: "number".to_string(), description: "Frame rate. Default 25.".to_string(), items: None });
                        props
                    },
                    required: vec!["output_file".to_string()],
                },
            },


            // ================================================================
            // PHASE I BATCH 2 — Long-tail: morphology, histogram, convolution
            // ================================================================

            FunctionDeclaration {
                name: "select_frames".to_string(),
                description: "Selects specific frames from video using FFmpeg select+setpts. Extract keyframes, sample at intervals, or filter by frame type.".to_string(),
                parameters: Parameters {
                    param_type: "object".to_string(),
                    properties: {
                        let mut props = HashMap::new();
                        props.insert("input_file".to_string(), PropertyDefinition { prop_type: "string".to_string(), description: "Path to the input video file".to_string(), items: None });
                        props.insert("output_file".to_string(), PropertyDefinition { prop_type: "string".to_string(), description: "Path to save the selected-frames output".to_string(), items: None });
                        props.insert("expr".to_string(), PropertyDefinition { prop_type: "string".to_string(), description: "FFmpeg select expression. Default: keyframes. e.g. 'not(mod(n,30))' = every 30th frame.".to_string(), items: None });
                        props.insert("fps".to_string(), PropertyDefinition { prop_type: "number".to_string(), description: "Output FPS. 0 = original timing. Default 0.".to_string(), items: None });
                        props
                    },
                    required: vec!["input_file".to_string(), "output_file".to_string()],
                },
            },

            FunctionDeclaration {
                name: "posterize_video".to_string(),
                description: "Posterizes video to N colour levels via FFmpeg posterize. Creates graphic-novel/stencil look.".to_string(),
                parameters: Parameters {
                    param_type: "object".to_string(),
                    properties: {
                        let mut props = HashMap::new();
                        props.insert("input_file".to_string(), PropertyDefinition { prop_type: "string".to_string(), description: "Path to the input video file".to_string(), items: None });
                        props.insert("output_file".to_string(), PropertyDefinition { prop_type: "string".to_string(), description: "Path to save the posterized output".to_string(), items: None });
                        props.insert("levels".to_string(), PropertyDefinition { prop_type: "number".to_string(), description: "Colour levels per channel (2–64). Default 5.".to_string(), items: None });
                        props
                    },
                    required: vec!["input_file".to_string(), "output_file".to_string()],
                },
            },

            FunctionDeclaration {
                name: "solarize_video".to_string(),
                description: "Solarize effect: pixels above threshold are inverted via FFmpeg solarize. Classic darkroom/psychedelic look.".to_string(),
                parameters: Parameters {
                    param_type: "object".to_string(),
                    properties: {
                        let mut props = HashMap::new();
                        props.insert("input_file".to_string(), PropertyDefinition { prop_type: "string".to_string(), description: "Path to the input video file".to_string(), items: None });
                        props.insert("output_file".to_string(), PropertyDefinition { prop_type: "string".to_string(), description: "Path to save the solarized output".to_string(), items: None });
                        props.insert("threshold".to_string(), PropertyDefinition { prop_type: "number".to_string(), description: "Luminance threshold 0–255. Default 128.".to_string(), items: None });
                        props
                    },
                    required: vec!["input_file".to_string(), "output_file".to_string()],
                },
            },







            FunctionDeclaration {
                name: "adjust_hue_saturation".to_string(),
                description: "Precise hue, saturation, intensity and lightness control via FFmpeg huesaturation filter.".to_string(),
                parameters: Parameters {
                    param_type: "object".to_string(),
                    properties: {
                        let mut props = HashMap::new();
                        props.insert("input_file".to_string(), PropertyDefinition { prop_type: "string".to_string(), description: "Path to the input video file".to_string(), items: None });
                        props.insert("output_file".to_string(), PropertyDefinition { prop_type: "string".to_string(), description: "Path to save the adjusted output".to_string(), items: None });
                        props.insert("hue".to_string(), PropertyDefinition { prop_type: "number".to_string(), description: "Hue rotation degrees (-180 to 180). Default 0.".to_string(), items: None });
                        props.insert("saturation".to_string(), PropertyDefinition { prop_type: "number".to_string(), description: "Saturation (-3 to 3). Default 0.".to_string(), items: None });
                        props.insert("intensity".to_string(), PropertyDefinition { prop_type: "number".to_string(), description: "Intensity (-1 to 1). Default 0.".to_string(), items: None });
                        props.insert("lightness".to_string(), PropertyDefinition { prop_type: "number".to_string(), description: "Lightness (-1 to 1). Default 0.".to_string(), items: None });
                        props
                    },
                    required: vec!["input_file".to_string(), "output_file".to_string()],
                },
            },


            FunctionDeclaration {
                name: "reverse_audio".to_string(),
                description: "Reverses the audio stream via FFmpeg areverse filter. Creates backwards/reversed audio.".to_string(),
                parameters: Parameters {
                    param_type: "object".to_string(),
                    properties: {
                        let mut props = HashMap::new();
                        props.insert("input_file".to_string(), PropertyDefinition { prop_type: "string".to_string(), description: "Path to the input audio/video file".to_string(), items: None });
                        props.insert("output_file".to_string(), PropertyDefinition { prop_type: "string".to_string(), description: "Path to save the reversed audio output".to_string(), items: None });
                        props
                    },
                    required: vec!["input_file".to_string(), "output_file".to_string()],
                },
            },

            FunctionDeclaration {
                name: "blend_audio_streams".to_string(),
                description: "Mixes two audio inputs via FFmpeg amix filter. Blends primary and secondary into a single output.".to_string(),
                parameters: Parameters {
                    param_type: "object".to_string(),
                    properties: {
                        let mut props = HashMap::new();
                        props.insert("input_file".to_string(), PropertyDefinition { prop_type: "string".to_string(), description: "Path to the primary audio/video file".to_string(), items: None });
                        props.insert("secondary_file".to_string(), PropertyDefinition { prop_type: "string".to_string(), description: "Path to the secondary audio file".to_string(), items: None });
                        props.insert("output_file".to_string(), PropertyDefinition { prop_type: "string".to_string(), description: "Path to save the mixed output".to_string(), items: None });
                        props.insert("duration".to_string(), PropertyDefinition { prop_type: "string".to_string(), description: "Output duration: longest (default), shortest, first.".to_string(), items: None });
                        props.insert("dropout_transition".to_string(), PropertyDefinition { prop_type: "number".to_string(), description: "Fade-out seconds when stream ends. Default 2.0.".to_string(), items: None });
                        props
                    },
                    required: vec!["input_file".to_string(), "secondary_file".to_string(), "output_file".to_string()],
                },
            },



            // ================================================================
            // PHASE H — Codec / Format Depth
            // ================================================================













            // ================================================================
            // PHASE G — AI/ML Filters
            // ================================================================


            FunctionDeclaration {
                name: "classify_frames_dnn".to_string(),
                description: "Runs DNN-based image classification on video frames using FFmpeg dnn_classify. Overlays predicted class label on each frame.".to_string(),
                parameters: Parameters {
                    param_type: "object".to_string(),
                    properties: {
                        let mut props = HashMap::new();
                        props.insert("input_file".to_string(), PropertyDefinition { prop_type: "string".to_string(), description: "Path to the input video file".to_string(), items: None });
                        props.insert("output_file".to_string(), PropertyDefinition { prop_type: "string".to_string(), description: "Path to save the classified output video".to_string(), items: None });
                        props.insert("model".to_string(), PropertyDefinition { prop_type: "string".to_string(), description: "Path to the DNN classification model file".to_string(), items: None });
                        props.insert("backend".to_string(), PropertyDefinition { prop_type: "string".to_string(), description: "DNN backend: native, openvino, tensorflow, pytorch, onnx. Default 'native'.".to_string(), items: None });
                        props.insert("labels".to_string(), PropertyDefinition { prop_type: "string".to_string(), description: "Optional path to labels file".to_string(), items: None });
                        props
                    },
                    required: vec!["input_file".to_string(), "output_file".to_string(), "model".to_string()],
                },
            },

            FunctionDeclaration {
                name: "upscale_super_resolution".to_string(),
                description: "AI-powered video upscaling using FFmpeg sr (super-resolution) filter. Increases resolution using DNN-based super-resolution models.".to_string(),
                parameters: Parameters {
                    param_type: "object".to_string(),
                    properties: {
                        let mut props = HashMap::new();
                        props.insert("input_file".to_string(), PropertyDefinition { prop_type: "string".to_string(), description: "Path to the input video file".to_string(), items: None });
                        props.insert("output_file".to_string(), PropertyDefinition { prop_type: "string".to_string(), description: "Path to save the upscaled output video".to_string(), items: None });
                        props.insert("scale_factor".to_string(), PropertyDefinition { prop_type: "number".to_string(), description: "Upscale multiplier: 2 or 4. Default 2.".to_string(), items: None });
                        props.insert("model".to_string(), PropertyDefinition { prop_type: "string".to_string(), description: "Optional path to a custom super-resolution model".to_string(), items: None });
                        props.insert("backend".to_string(), PropertyDefinition { prop_type: "string".to_string(), description: "DNN backend: native, openvino, tensorflow. Default 'native'.".to_string(), items: None });
                        props
                    },
                    required: vec!["input_file".to_string(), "output_file".to_string()],
                },
            },

            FunctionDeclaration {
                name: "remove_rain_ai".to_string(),
                description: "Removes rain streaks from video using FFmpeg derain AI filter (DNN-based). Requires a trained derain model.".to_string(),
                parameters: Parameters {
                    param_type: "object".to_string(),
                    properties: {
                        let mut props = HashMap::new();
                        props.insert("input_file".to_string(), PropertyDefinition { prop_type: "string".to_string(), description: "Path to the input video file".to_string(), items: None });
                        props.insert("output_file".to_string(), PropertyDefinition { prop_type: "string".to_string(), description: "Path to save the derrained output video".to_string(), items: None });
                        props.insert("model".to_string(), PropertyDefinition { prop_type: "string".to_string(), description: "Path to the derain DNN model file".to_string(), items: None });
                        props.insert("backend".to_string(), PropertyDefinition { prop_type: "string".to_string(), description: "DNN backend: native, openvino, tensorflow. Default 'native'.".to_string(), items: None });
                        props
                    },
                    required: vec!["input_file".to_string(), "output_file".to_string(), "model".to_string()],
                },
            },



            // ================================================================
            // PHASE E — Vectorscope, Waveform, Grid, LumaKey, Binaural, Modulation
            // ================================================================

            FunctionDeclaration {
                name: "analyze_vectorscope".to_string(),
                description: "Renders a vectorscope visualisation frame from a video for colour grading analysis.".to_string(),
                parameters: Parameters {
                    param_type: "object".to_string(),
                    properties: {
                        let mut props = HashMap::new();
                        props.insert("input_file".to_string(), PropertyDefinition { prop_type: "string".to_string(), description: "Path to the input video file".to_string(), items: None });
                        props.insert("output_file".to_string(), PropertyDefinition { prop_type: "string".to_string(), description: "Path to save the vectorscope image".to_string(), items: None });
                        props.insert("mode".to_string(), PropertyDefinition { prop_type: "string".to_string(), description: "Display mode: color/color2/color3/color4/color5/gray/tint/phase. Default 'color'.".to_string(), items: None });
                        props
                    },
                    required: vec!["input_file".to_string(), "output_file".to_string()],
                },
            },

            FunctionDeclaration {
                name: "analyze_waveform".to_string(),
                description: "Renders a waveform monitor frame from a video for luma/chroma levels and exposure analysis.".to_string(),
                parameters: Parameters {
                    param_type: "object".to_string(),
                    properties: {
                        let mut props = HashMap::new();
                        props.insert("input_file".to_string(), PropertyDefinition { prop_type: "string".to_string(), description: "Path to the input video file".to_string(), items: None });
                        props.insert("output_file".to_string(), PropertyDefinition { prop_type: "string".to_string(), description: "Path to save the waveform image".to_string(), items: None });
                        props.insert("mode".to_string(), PropertyDefinition { prop_type: "string".to_string(), description: "Layout mode: row (default) or column.".to_string(), items: None });
                        props.insert("filter_type".to_string(), PropertyDefinition { prop_type: "string".to_string(), description: "Component: lowpass (default), flat, aflat, chroma, color, acolor.".to_string(), items: None });
                        props
                    },
                    required: vec!["input_file".to_string(), "output_file".to_string()],
                },
            },

            FunctionDeclaration {
                name: "draw_grid".to_string(),
                description: "Draws a regular grid over a video using the FFmpeg drawgrid filter. Useful for composition guides and rule-of-thirds overlays.".to_string(),
                parameters: Parameters {
                    param_type: "object".to_string(),
                    properties: {
                        let mut props = HashMap::new();
                        props.insert("input_file".to_string(), PropertyDefinition { prop_type: "string".to_string(), description: "Path to the input video file".to_string(), items: None });
                        props.insert("output_file".to_string(), PropertyDefinition { prop_type: "string".to_string(), description: "Path to save the output file".to_string(), items: None });
                        props.insert("width".to_string(), PropertyDefinition { prop_type: "number".to_string(), description: "Cell width in pixels. Default 100.".to_string(), items: None });
                        props.insert("height".to_string(), PropertyDefinition { prop_type: "number".to_string(), description: "Cell height in pixels. Default 100.".to_string(), items: None });
                        props.insert("thickness".to_string(), PropertyDefinition { prop_type: "number".to_string(), description: "Line thickness in pixels. Default 1.".to_string(), items: None });
                        props.insert("color".to_string(), PropertyDefinition { prop_type: "string".to_string(), description: "Line colour. Default 'white@0.5'.".to_string(), items: None });
                        props
                    },
                    required: vec!["input_file".to_string(), "output_file".to_string()],
                },
            },

            FunctionDeclaration {
                name: "grid_stack_videos".to_string(),
                description: "Stacks multiple videos in a grid layout using xstack. Provide comma-separated input file paths. Supports 2-input side-by-side, 4-input 2×2, or custom layouts.".to_string(),
                parameters: Parameters {
                    param_type: "object".to_string(),
                    properties: {
                        let mut props = HashMap::new();
                        props.insert("input_files".to_string(), PropertyDefinition { prop_type: "string".to_string(), description: "Comma-separated list of input video file paths (minimum 2)".to_string(), items: None });
                        props.insert("output_file".to_string(), PropertyDefinition { prop_type: "string".to_string(), description: "Path to save the stacked output video".to_string(), items: None });
                        props.insert("layout".to_string(), PropertyDefinition { prop_type: "string".to_string(), description: "xstack layout e.g. '0_0|w0_0|0_h0|w0_h0' for 2×2. Empty = auto.".to_string(), items: None });
                        props
                    },
                    required: vec!["input_files".to_string(), "output_file".to_string()],
                },
            },

            FunctionDeclaration {
                name: "luma_key".to_string(),
                description: "Keys out dark or bright regions of video by luma value, making them transparent. Useful for compositing title cards over backgrounds.".to_string(),
                parameters: Parameters {
                    param_type: "object".to_string(),
                    properties: {
                        let mut props = HashMap::new();
                        props.insert("input_file".to_string(), PropertyDefinition { prop_type: "string".to_string(), description: "Path to the input video file".to_string(), items: None });
                        props.insert("output_file".to_string(), PropertyDefinition { prop_type: "string".to_string(), description: "Path to save the output with alpha".to_string(), items: None });
                        props.insert("threshold".to_string(), PropertyDefinition { prop_type: "number".to_string(), description: "Luma threshold (0–1). Default 0.1.".to_string(), items: None });
                        props.insert("tolerance".to_string(), PropertyDefinition { prop_type: "number".to_string(), description: "Tolerance (0–1). Default 0.1.".to_string(), items: None });
                        props.insert("softness".to_string(), PropertyDefinition { prop_type: "number".to_string(), description: "Edge softness (0–1). Default 0.".to_string(), items: None });
                        props
                    },
                    required: vec!["input_file".to_string(), "output_file".to_string()],
                },
            },

            FunctionDeclaration {
                name: "render_binaural".to_string(),
                description: "Virtualises audio for headphone playback using HRTF-based binaural rendering (FFmpeg headphone filter).".to_string(),
                parameters: Parameters {
                    param_type: "object".to_string(),
                    properties: {
                        let mut props = HashMap::new();
                        props.insert("input_file".to_string(), PropertyDefinition { prop_type: "string".to_string(), description: "Path to the input audio/video file".to_string(), items: None });
                        props.insert("output_file".to_string(), PropertyDefinition { prop_type: "string".to_string(), description: "Path to save the binaural output".to_string(), items: None });
                        props.insert("hrir_type".to_string(), PropertyDefinition { prop_type: "string".to_string(), description: "'stereo' (default, built-in) or 'multich' (multichannel HRIR).".to_string(), items: None });
                        props
                    },
                    required: vec!["input_file".to_string(), "output_file".to_string()],
                },
            },

            FunctionDeclaration {
                name: "add_vibrato".to_string(),
                description: "Adds a vibrato (periodic pitch modulation) effect to audio using the FFmpeg vibrato filter.".to_string(),
                parameters: Parameters {
                    param_type: "object".to_string(),
                    properties: {
                        let mut props = HashMap::new();
                        props.insert("input_file".to_string(), PropertyDefinition { prop_type: "string".to_string(), description: "Path to the input audio/video file".to_string(), items: None });
                        props.insert("output_file".to_string(), PropertyDefinition { prop_type: "string".to_string(), description: "Path to save the output".to_string(), items: None });
                        props.insert("frequency".to_string(), PropertyDefinition { prop_type: "number".to_string(), description: "Modulation frequency Hz (0.1–20000). Default 5.".to_string(), items: None });
                        props.insert("depth".to_string(), PropertyDefinition { prop_type: "number".to_string(), description: "Modulation depth (0–1). Default 0.5.".to_string(), items: None });
                        props
                    },
                    required: vec!["input_file".to_string(), "output_file".to_string()],
                },
            },

            FunctionDeclaration {
                name: "add_tremolo".to_string(),
                description: "Adds a tremolo (periodic amplitude modulation) effect to audio using the FFmpeg tremolo filter.".to_string(),
                parameters: Parameters {
                    param_type: "object".to_string(),
                    properties: {
                        let mut props = HashMap::new();
                        props.insert("input_file".to_string(), PropertyDefinition { prop_type: "string".to_string(), description: "Path to the input audio/video file".to_string(), items: None });
                        props.insert("output_file".to_string(), PropertyDefinition { prop_type: "string".to_string(), description: "Path to save the output".to_string(), items: None });
                        props.insert("frequency".to_string(), PropertyDefinition { prop_type: "number".to_string(), description: "Modulation frequency Hz (0.1–20000). Default 5.".to_string(), items: None });
                        props.insert("depth".to_string(), PropertyDefinition { prop_type: "number".to_string(), description: "Modulation depth (0–1). Default 0.5.".to_string(), items: None });
                        props
                    },
                    required: vec!["input_file".to_string(), "output_file".to_string()],
                },
            },

            FunctionDeclaration {
                name: "add_flanger".to_string(),
                description: "Adds a flanger (comb-filtering modulation) effect to audio using the FFmpeg flanger filter.".to_string(),
                parameters: Parameters {
                    param_type: "object".to_string(),
                    properties: {
                        let mut props = HashMap::new();
                        props.insert("input_file".to_string(), PropertyDefinition { prop_type: "string".to_string(), description: "Path to the input audio/video file".to_string(), items: None });
                        props.insert("output_file".to_string(), PropertyDefinition { prop_type: "string".to_string(), description: "Path to save the output".to_string(), items: None });
                        props.insert("delay".to_string(), PropertyDefinition { prop_type: "number".to_string(), description: "Base delay ms (0–30). Default 0.".to_string(), items: None });
                        props.insert("depth".to_string(), PropertyDefinition { prop_type: "number".to_string(), description: "Sweep depth ms (0–10). Default 2.".to_string(), items: None });
                        props.insert("speed".to_string(), PropertyDefinition { prop_type: "number".to_string(), description: "Sweep speed Hz (0.1–10). Default 0.5.".to_string(), items: None });
                        props.insert("shape".to_string(), PropertyDefinition { prop_type: "string".to_string(), description: "'sinusoidal' (default) or 'triangular'.".to_string(), items: None });
                        props
                    },
                    required: vec!["input_file".to_string(), "output_file".to_string()],
                },
            },

            FunctionDeclaration {
                name: "denoise_audio_nlm".to_string(),
                description: "Denoises audio using Non-Local Means (NLM) algorithm (anlmdn). Effective for broadband noise without musical noise artifacts.".to_string(),
                parameters: Parameters {
                    param_type: "object".to_string(),
                    properties: {
                        let mut props = HashMap::new();
                        props.insert("input_file".to_string(), PropertyDefinition { prop_type: "string".to_string(), description: "Path to the input audio/video file".to_string(), items: None });
                        props.insert("output_file".to_string(), PropertyDefinition { prop_type: "string".to_string(), description: "Path to save the denoised output".to_string(), items: None });
                        props.insert("strength".to_string(), PropertyDefinition { prop_type: "number".to_string(), description: "Denoising strength (0.00001–10). Default 0.0001.".to_string(), items: None });
                        props.insert("patch_size".to_string(), PropertyDefinition { prop_type: "number".to_string(), description: "Patch size in seconds (0–100). Default 0.002.".to_string(), items: None });
                        props.insert("research_size".to_string(), PropertyDefinition { prop_type: "number".to_string(), description: "Research window in seconds (0–100). Default 0.002.".to_string(), items: None });
                        props
                    },
                    required: vec!["input_file".to_string(), "output_file".to_string()],
                },
            },

            // ── Workflow Recipes ─────────────────────────────────────────
            FunctionDeclaration {
                name: "youtube_ready_export".to_string(),
                description: "Multi-step YouTube export pipeline: stabilize → normalize color → loudnorm to −14 LUFS → convert to yuv420p. Use when the user wants their video ready for YouTube in one shot.".to_string(),
                parameters: { let mut p = HashMap::new(); p.insert("input_file".to_string(), PropertyDefinition { prop_type: "string".to_string(), description: "Path to the input video file".to_string(), items: None }); p.insert("output_file".to_string(), PropertyDefinition { prop_type: "string".to_string(), description: "Path to save the YouTube-ready output".to_string(), items: None }); Parameters { param_type: "object".to_string(), properties: p, required: vec!["input_file".to_string(), "output_file".to_string()] } },
            },
            FunctionDeclaration {
                name: "podcast_cleanup".to_string(),
                description: "Multi-step podcast audio cleanup: denoise → de-ess sibilance → limit peaks → loudnorm to −16 LUFS. Use when the user wants professional speech audio.".to_string(),
                parameters: { let mut p = HashMap::new(); p.insert("input_file".to_string(), PropertyDefinition { prop_type: "string".to_string(), description: "Path to the input audio or video file".to_string(), items: None }); p.insert("output_file".to_string(), PropertyDefinition { prop_type: "string".to_string(), description: "Path to save the cleaned audio".to_string(), items: None }); Parameters { param_type: "object".to_string(), properties: p, required: vec!["input_file".to_string(), "output_file".to_string()] } },
            },
            FunctionDeclaration {
                name: "cinematic_grade".to_string(),
                description: "Multi-step cinematic color grade: vintage curves → vibrance → vignette → film grain. Use when the user wants a cinematic look for trailers or highlight reels.".to_string(),
                parameters: { let mut p = HashMap::new(); p.insert("input_file".to_string(), PropertyDefinition { prop_type: "string".to_string(), description: "Path to the input video file".to_string(), items: None }); p.insert("output_file".to_string(), PropertyDefinition { prop_type: "string".to_string(), description: "Path to save the graded video".to_string(), items: None }); Parameters { param_type: "object".to_string(), properties: p, required: vec!["input_file".to_string(), "output_file".to_string()] } },
            },
            FunctionDeclaration {
                name: "create_gif_workflow".to_string(),
                description: "Creates a high-quality optimised GIF: trim segment → scale → palette-optimised GIF. Use when the user wants a GIF from a video clip.".to_string(),
                parameters: { let mut p = HashMap::new(); p.insert("input_file".to_string(), PropertyDefinition { prop_type: "string".to_string(), description: "Path to the input video file".to_string(), items: None }); p.insert("output_file".to_string(), PropertyDefinition { prop_type: "string".to_string(), description: "Path to save the GIF".to_string(), items: None }); p.insert("start_seconds".to_string(), PropertyDefinition { prop_type: "number".to_string(), description: "Start time in seconds. Default 0.".to_string(), items: None }); p.insert("duration_seconds".to_string(), PropertyDefinition { prop_type: "number".to_string(), description: "Duration in seconds. Default 5.".to_string(), items: None }); p.insert("width".to_string(), PropertyDefinition { prop_type: "number".to_string(), description: "Output width in pixels. Default 480.".to_string(), items: None }); p.insert("fps".to_string(), PropertyDefinition { prop_type: "number".to_string(), description: "Frames per second. Default 15.".to_string(), items: None }); Parameters { param_type: "object".to_string(), properties: p, required: vec!["input_file".to_string(), "output_file".to_string()] } },
            },
            FunctionDeclaration {
                name: "talking_head_cleanup".to_string(),
                description: "Multi-step talking head video cleanup: stabilize → denoise speech → de-ess sibilance → loudnorm to −16 LUFS. Use for YouTube talking head footage, interviews, or screen recordings.".to_string(),
                parameters: { let mut p = HashMap::new(); p.insert("input_file".to_string(), PropertyDefinition { prop_type: "string".to_string(), description: "Path to the input video file".to_string(), items: None }); p.insert("output_file".to_string(), PropertyDefinition { prop_type: "string".to_string(), description: "Path to save the cleaned video".to_string(), items: None }); Parameters { param_type: "object".to_string(), properties: p, required: vec!["input_file".to_string(), "output_file".to_string()] } },
            },
            
            // =====================================================================
            // QUERY TOOLS — read-only DB lookups for re-editing across pipeline runs
            // =====================================================================
            FunctionDeclaration {
                name: "get_output_videos".to_string(),
                description: "Queries the output_videos table. Returns video outputs matching the search query with pagination support. Use this to discover previously generated video files for re-editing.".to_string(),
                parameters: Parameters {
                    param_type: "object".to_string(),
                    properties: {
                        let mut props = HashMap::new();
                        props.insert("query".to_string(), PropertyDefinition {
                            prop_type: "string".to_string(),
                            description: "Search term to filter by filename, tool name, or session ID (optional)".to_string(),
                            items: None,
                        });
                        props.insert("session_filter".to_string(), PropertyDefinition {
                            prop_type: "string".to_string(),
                            description: "Session UUID to filter by (optional; defaults to current session)".to_string(),
                            items: None,
                        });
                        props.insert("limit".to_string(), PropertyDefinition {
                            prop_type: "integer".to_string(),
                            description: "Maximum number of results to return (default: 20, max: 100)".to_string(),
                            items: None,
                        });
                        props.insert("offset".to_string(), PropertyDefinition {
                            prop_type: "integer".to_string(),
                            description: "Number of results to skip for pagination (default: 0)".to_string(),
                            items: None,
                        });
                        props
                    },
                    required: vec![],
                },
            },
            FunctionDeclaration {
                name: "get_generated_artifacts".to_string(),
                description: "Queries the generated_artifacts table. Returns generated assets (images, audio, files) matching the search query, optionally filtered by artifact kind. Use this to discover previously generated images, audio files, and other assets.".to_string(),
                parameters: Parameters {
                    param_type: "object".to_string(),
                    properties: {
                        let mut props = HashMap::new();
                        props.insert("query".to_string(), PropertyDefinition {
                            prop_type: "string".to_string(),
                            description: "Search term to filter by filename, source, or session (optional)".to_string(),
                            items: None,
                        });
                        props.insert("kind".to_string(), PropertyDefinition {
                            prop_type: "string".to_string(),
                            description: "Filter by artifact kind: 'generated_image', 'generated_audio', 'generated_file', or leave blank for all".to_string(),
                            items: None,
                        });
                        props.insert("session_filter".to_string(), PropertyDefinition {
                            prop_type: "string".to_string(),
                            description: "Session UUID to filter by (optional; defaults to current session)".to_string(),
                            items: None,
                        });
                        props.insert("limit".to_string(), PropertyDefinition {
                            prop_type: "integer".to_string(),
                            description: "Maximum number of results to return (default: 20, max: 100)".to_string(),
                            items: None,
                        });
                        props.insert("offset".to_string(), PropertyDefinition {
                            prop_type: "integer".to_string(),
                            description: "Number of results to skip for pagination (default: 0)".to_string(),
                            items: None,
                        });
                        props
                    },
                    required: vec![],
                },
            },
            FunctionDeclaration {
                name: "get_extracted_clips".to_string(),
                description: "Queries the extracted_clips table (from YouTube channels). Returns clips associated with a specific clipping job ID with pagination support.".to_string(),
                parameters: Parameters {
                    param_type: "object".to_string(),
                    properties: {
                        let mut props = HashMap::new();
                        props.insert("job_id".to_string(), PropertyDefinition {
                            prop_type: "string".to_string(),
                            description: "The clipping job ID (UUID string) to filter clips by".to_string(),
                            items: None,
                        });
                        props.insert("limit".to_string(), PropertyDefinition {
                            prop_type: "integer".to_string(),
                            description: "Maximum number of results to return (default: 50, max: 200)".to_string(),
                            items: None,
                        });
                        props.insert("offset".to_string(), PropertyDefinition {
                            prop_type: "integer".to_string(),
                            description: "Number of results to skip for pagination (default: 0)".to_string(),
                            items: None,
                        });
                        props
                    },
                    required: vec!["job_id".to_string()],
                },
            },
        ]
    }

    /// Filter tools by name (for dynamic tool selection)
    /// Returns only the tools whose names are in the provided list
    pub fn filter_tools_by_name(tool_names: &[String]) -> Vec<FunctionDeclaration> {
        crate::tool_registry::ToolRegistry::filter_gemini_tools_for_profile(
            crate::tool_registry::AgentExecutionProfile::FullProduction,
            tool_names,
        )
    }

    /// Analyze an image from bytes using Gemini's vision capabilities
    pub async fn analyze_image_bytes(
        &self,
        image_bytes: &[u8],
        analysis_prompt: &str,
    ) -> Result<String, Box<dyn std::error::Error + Send + Sync>> {
        let encoded_data = BASE64_STANDARD.encode(image_bytes);

        // Determine MIME type from image signature
        let mime_type = if image_bytes.starts_with(&[0xFF, 0xD8, 0xFF]) {
            "image/jpeg"
        } else if image_bytes.starts_with(&[0x89, 0x50, 0x4E, 0x47]) {
            "image/png"
        } else if image_bytes.starts_with(&[0x47, 0x49, 0x46]) {
            "image/gif"
        } else if image_bytes.starts_with(&[0x52, 0x49, 0x46, 0x46]) {
            "image/webp"
        } else {
            "image/png" // default
        };

        let request = GenerateContentRequest {
            contents: vec![Content {
                parts: vec![
                    Part::Text {
                        text: analysis_prompt.to_string(),
                    },
                    Part::InlineData {
                        inline_data: InlineData {
                            mime_type: mime_type.to_string(),
                            data: encoded_data,
                        },
                    },
                ],
                role: Some("user".to_string()),
            }],
            tools: None,
            generation_config: Some(GenerationConfig {
                temperature: 0.3,
                top_k: 40,
                top_p: 0.9,
                max_output_tokens: 2048,
            }),
            tool_config: None,
            system_instruction: None,
        };

        let response = self.generate_content(request).await?;

        // Extract text from response
        if let Some(candidate) = response.candidates.first() {
            if let Some(ref content) = candidate.content {
                for part in &content.parts {
                    if let Part::Text { text } = part {
                        return Ok(text.clone());
                    }
                }
            }
        }

        Err("No valid response received from image analysis".into())
    }

    /// Analyze an audio file from bytes using Gemini multimodal capabilities.
    pub async fn analyze_audio_bytes(
        &self,
        audio_bytes: &[u8],
        mime_type: &str,
        analysis_prompt: &str,
    ) -> Result<String, Box<dyn std::error::Error + Send + Sync>> {
        let encoded_data = BASE64_STANDARD.encode(audio_bytes);

        let request = GenerateContentRequest {
            contents: vec![Content {
                parts: vec![
                    Part::Text {
                        text: analysis_prompt.to_string(),
                    },
                    Part::InlineData {
                        inline_data: InlineData {
                            mime_type: mime_type.to_string(),
                            data: encoded_data,
                        },
                    },
                ],
                role: Some("user".to_string()),
            }],
            tools: None,
            generation_config: Some(GenerationConfig {
                temperature: 0.2,
                top_k: 40,
                top_p: 0.9,
                max_output_tokens: 2048,
            }),
            tool_config: None,
            system_instruction: None,
        };

        let response = self.generate_content(request).await?;

        if let Some(candidate) = response.candidates.first() {
            if let Some(ref content) = candidate.content {
                for part in &content.parts {
                    if let Part::Text { text } = part {
                        return Ok(text.clone());
                    }
                }
            }
        }

        Err("No valid response received from audio analysis".into())
    }

    #[allow(dead_code)]
    async fn generate_image_with_gemini(
        &self,
        prompt: &str,
    ) -> Result<Vec<u8>, Box<dyn std::error::Error + Send + Sync>> {
        // Use Nano Banana Pro (Gemini 3 Pro Image) - Latest image generation model
        let url = format!(
            "{}/models/gemini-2.5-flash:generateContent?key={}",
            self.base_url, self.api_key
        );

        let enhanced_prompt = format!(
            "Generate a professional, abstract background image for a video editing application. Style: {}. Requirements: Dark theme with deep blues, purples, and blacks. Include subtle video editing elements like timeline bars, waveforms, or geometric shapes. Make it modern, clean, and suitable as a subtle background overlay. Resolution should be wide-screen format.",
            prompt
        );

        let request = GenerateContentRequest {
            contents: vec![Content {
                parts: vec![Part::Text {
                    text: enhanced_prompt,
                }],
                role: Some("user".to_string()),
            }],
            tools: None,
            generation_config: Some(GenerationConfig {
                temperature: 0.7,
                top_k: 32,
                top_p: 0.9,
                max_output_tokens: 4096,
            }),
            tool_config: None,
            system_instruction: None,
        };

        // Add the response modalities for image generation
        let mut request_json = serde_json::to_value(&request)?;
        if let Some(config) = request_json.get_mut("generationConfig") {
            config["responseModalities"] = serde_json::json!(["TEXT", "IMAGE"]);
        }

        let response = self
            .client
            .post(&url)
            .header("Content-Type", "application/json")
            .json(&request_json)
            .send()
            .await?;

        if response.status().is_success() {
            let response_text = response.text().await?;
            tracing::debug!("Gemini image response: {}", response_text);

            // Parse the response to extract image data
            let response_json: serde_json::Value = serde_json::from_str(&response_text)?;

            if let Some(candidates) = response_json.get("candidates").and_then(|c| c.as_array()) {
                for candidate in candidates {
                    if let Some(content) = candidate.get("content") {
                        if let Some(parts) = content.get("parts").and_then(|p| p.as_array()) {
                            for part in parts {
                                if let Some(inline_data) = part.get("inlineData") {
                                    if let Some(data) =
                                        inline_data.get("data").and_then(|d| d.as_str())
                                    {
                                        // Decode base64 image data
                                        use base64::{engine::general_purpose, Engine as _};
                                        let image_bytes = general_purpose::STANDARD.decode(data)?;
                                        tracing::info!("Successfully decoded Gemini-generated image ({} bytes)", image_bytes.len());
                                        return Ok(image_bytes);
                                    }
                                }
                            }
                        }
                    }
                }
            }

            Err("No image data found in Gemini response".into())
        } else {
            let status = response.status();
            let error_text = response
                .text()
                .await
                .unwrap_or_else(|_| "Unknown error".to_string());
            Err(format!("Gemini Image API error ({}): {}", status, error_text).into())
        }
    }

    #[allow(dead_code)]
    async fn generate_svg_placeholder(
        &self,
        prompt: &str,
    ) -> Result<String, Box<dyn std::error::Error + Send + Sync>> {
        // Generate a description using Gemini that we can use for the SVG
        let description_prompt = format!(
            "Based on this image prompt: '{}', create a brief description (max 50 words) of colors, shapes, and visual elements that would make a good abstract background for a video editing app. Focus on: colors (using hex codes), geometric shapes, and overall mood.",
            prompt
        );

        let request = GenerateContentRequest {
            contents: vec![Content {
                parts: vec![Part::Text {
                    text: description_prompt,
                }],
                role: Some("user".to_string()),
            }],
            tools: None,
            generation_config: Some(GenerationConfig {
                temperature: 0.7,
                top_k: 32,
                top_p: 0.9,
                max_output_tokens: 150,
            }),
            tool_config: None,
            system_instruction: None,
        };

        let url = format!(
            "{}/models/gemini-2.5-flash:generateContent?key={}",
            self.base_url, self.api_key
        );

        let response = self
            .client
            .post(&url)
            .header("Content-Type", "application/json")
            .json(&request)
            .send()
            .await?;

        if response.status().is_success() {
            match response.json::<GenerateContentResponse>().await {
                Ok(result) => {
                    if let Some(candidate) = result.candidates.first() {
                        if let Some(ref content) = candidate.content {
                            if let Some(Part::Text { text }) = content.parts.first() {
                                tracing::info!("Generated description from Gemini: {}", text);
                                return Ok(self.create_svg_from_description(text));
                            }
                        }
                    }
                    tracing::warn!("No valid text content in Gemini response, using default SVG");
                }
                Err(e) => {
                    tracing::error!("Failed to parse Gemini response: {}", e);
                }
            }
        } else {
            let status = response.status();
            let error_text = response
                .text()
                .await
                .unwrap_or_else(|_| "Unknown error".to_string());
            tracing::error!("Gemini API error ({}): {}", status, error_text);
        }

        // Fallback SVG if API call fails
        tracing::info!("Using fallback default SVG");
        Ok(self.create_default_svg())
    }

    #[allow(dead_code)]
    fn create_svg_from_description(&self, description: &str) -> String {
        let mut rng = rand::thread_rng();

        // Extract colors from description or use defaults
        let colors = if description.contains("#") {
            vec!["#667eea", "#764ba2", "#3498db", "#2980b9"]
        } else {
            vec![
                "#667eea", "#764ba2", "#3498db", "#2980b9", "#8e44ad", "#2c3e50",
            ]
        };

        let primary_color = colors[rng.gen_range(0..colors.len())];
        let secondary_color = colors[rng.gen_range(0..colors.len())];

        // Generate random shapes and positions
        let circles = (0..5)
            .map(|_| {
                format!(
                    r#"<circle cx="{}" cy="{}" r="{}" fill="{}" opacity="0.{}"/>"#,
                    rng.gen_range(0..1920),
                    rng.gen_range(0..1080),
                    rng.gen_range(50..200),
                    colors[rng.gen_range(0..colors.len())],
                    rng.gen_range(1..4)
                )
            })
            .collect::<Vec<_>>()
            .join("\n        ");

        let rectangles = (0..3).map(|_| {
            format!(
                r#"<rect x="{}" y="{}" width="{}" height="{}" fill="{}" opacity="0.{}" transform="rotate({} {} {})"/>"#,
                rng.gen_range(0..1920),
                rng.gen_range(0..1080),
                rng.gen_range(100..400),
                rng.gen_range(50..200),
                colors[rng.gen_range(0..colors.len())],
                rng.gen_range(1..3),
                rng.gen_range(0..360),
                rng.gen_range(0..1920),
                rng.gen_range(0..1080)
            )
        }).collect::<Vec<_>>().join("\n        ");

        format!(
            r#"<svg width="1920" height="1080" xmlns="http://www.w3.org/2000/svg">
    <defs>
        <linearGradient id="bg" x1="0%" y1="0%" x2="100%" y2="100%">
            <stop offset="0%" style="stop-color:{};stop-opacity:1" />
            <stop offset="100%" style="stop-color:{};stop-opacity:1" />
        </linearGradient>
        <filter id="blur">
            <feGaussianBlur in="SourceGraphic" stdDeviation="3"/>
        </filter>
    </defs>
    <rect width="100%" height="100%" fill="url(#bg)"/>
    <g filter="url(#blur)">
        {}
        {}
    </g>
    <!-- Video editing themed elements -->
    <rect x="100" y="100" width="200" height="20" fill="white" opacity="0.1" rx="10"/>
    <rect x="100" y="130" width="150" height="20" fill="white" opacity="0.1" rx="10"/>
    <rect x="100" y="160" width="180" height="20" fill="white" opacity="0.1" rx="10"/>
    
    <circle cx="1720" cy="200" r="30" fill="white" opacity="0.1"/>
    <polygon points="1710,190 1730,200 1710,210" fill="white" opacity="0.2"/>
</svg>"#,
            primary_color, secondary_color, circles, rectangles
        )
    }

    #[allow(dead_code)]
    fn create_random_svg(&self) -> String {
        let mut rng = rand::thread_rng();

        // Predefined dark color palettes for video editing theme
        let color_palettes = vec![
            vec!["#1a1a2e", "#16213e", "#3b82f6", "#1d4ed8"],
            vec!["#0f1419", "#1e40af", "#3b82f6", "#2563eb"],
            vec!["#1f2937", "#374151", "#6366f1", "#4f46e5"],
            vec!["#111827", "#1f2937", "#3730a3", "#312e81"],
            vec!["#0c0e16", "#1a1a2e", "#4338ca", "#3730a3"],
        ];

        let palette = &color_palettes[rng.gen_range(0..color_palettes.len())];
        let primary_color = palette[0];
        let secondary_color = palette[1];

        // Generate varied shapes
        let circles = (0..rng.gen_range(3..7))
            .map(|_| {
                format!(
                    r#"<circle cx="{}" cy="{}" r="{}" fill="{}" opacity="0.{}"/>"#,
                    rng.gen_range(100..1820),
                    rng.gen_range(100..980),
                    rng.gen_range(40..150),
                    palette[rng.gen_range(0..palette.len())],
                    rng.gen_range(1..4)
                )
            })
            .collect::<Vec<_>>()
            .join("\n        ");

        let rectangles = (0..rng.gen_range(2..5)).map(|_| {
            format!(
                r#"<rect x="{}" y="{}" width="{}" height="{}" fill="{}" opacity="0.{}" transform="rotate({} {} {})"/>"#,
                rng.gen_range(100..1600),
                rng.gen_range(100..900),
                rng.gen_range(80..300),
                rng.gen_range(30..150),
                palette[rng.gen_range(0..palette.len())],
                rng.gen_range(1..3),
                rng.gen_range(0..45),
                rng.gen_range(100..1820),
                rng.gen_range(100..980)
            )
        }).collect::<Vec<_>>().join("\n        ");

        format!(
            r#"<svg width="1920" height="1080" xmlns="http://www.w3.org/2000/svg">
    <defs>
        <linearGradient id="bg" x1="{}%" y1="{}%" x2="{}%" y2="{}%">
            <stop offset="0%" style="stop-color:{};stop-opacity:1" />
            <stop offset="{}%" style="stop-color:{};stop-opacity:1" />
            <stop offset="100%" style="stop-color:{};stop-opacity:1" />
        </linearGradient>
        <filter id="blur">
            <feGaussianBlur in="SourceGraphic" stdDeviation="{}"/>
        </filter>
    </defs>
    <rect width="100%" height="100%" fill="url(#bg)"/>
    <g filter="url(#blur)">
        {}
        {}
    </g>
    <!-- Dynamic video editing themed elements -->
    <rect x="{}" y="{}" width="200" height="15" fill="white" opacity="0.1" rx="8"/>
    <rect x="{}" y="{}" width="150" height="15" fill="white" opacity="0.1" rx="8"/>
    <rect x="{}" y="{}" width="180" height="15" fill="white" opacity="0.1" rx="8"/>
    
    <circle cx="{}" cy="{}" r="25" fill="white" opacity="0.1"/>
    <polygon points="{},{} {},{} {},{}" fill="white" opacity="0.2"/>
    
    <!-- Waveform-like pattern -->
    <rect x="{}" y="{}" width="4" height="{}" fill="white" opacity="0.1"/>
    <rect x="{}" y="{}" width="4" height="{}" fill="white" opacity="0.1"/>
    <rect x="{}" y="{}" width="4" height="{}" fill="white" opacity="0.1"/>
</svg>"#,
            rng.gen_range(0..30),
            rng.gen_range(0..30), // gradient start
            rng.gen_range(70..100),
            rng.gen_range(70..100), // gradient end
            primary_color,
            rng.gen_range(40..60), // middle stop
            palette[rng.gen_range(0..palette.len())],
            secondary_color,
            rng.gen_range(2..5), // blur amount
            circles,
            rectangles,
            // Timeline elements
            rng.gen_range(80..200),
            rng.gen_range(80..200),
            rng.gen_range(80..200),
            rng.gen_range(110..230),
            rng.gen_range(80..200),
            rng.gen_range(140..260),
            // Play button
            rng.gen_range(1600..1800),
            rng.gen_range(150..300),
            // Play triangle
            rng.gen_range(1590..1790),
            rng.gen_range(140..290),
            rng.gen_range(1610..1810),
            rng.gen_range(150..300),
            rng.gen_range(1590..1790),
            rng.gen_range(160..310),
            // Waveform
            rng.gen_range(1400..1500),
            rng.gen_range(800..900),
            rng.gen_range(20..60),
            rng.gen_range(1410..1510),
            rng.gen_range(820..920),
            rng.gen_range(15..45),
            rng.gen_range(1420..1520),
            rng.gen_range(810..910),
            rng.gen_range(25..65),
        )
    }

    #[allow(dead_code)]
    fn create_default_svg(&self) -> String {
        r#"<svg width="1920" height="1080" xmlns="http://www.w3.org/2000/svg">
    <defs>
        <linearGradient id="bg" x1="0%" y1="0%" x2="100%" y2="100%">
            <stop offset="0%" style="stop-color:#1a1a2e;stop-opacity:1" />
            <stop offset="50%" style="stop-color:#16213e;stop-opacity:1" />
            <stop offset="100%" style="stop-color:#0f1419;stop-opacity:1" />
        </linearGradient>
    </defs>
    <rect width="100%" height="100%" fill="url(#bg)"/>
    <circle cx="300" cy="300" r="100" fill="white" opacity="0.08"/>
    <circle cx="1620" cy="780" r="80" fill="white" opacity="0.1"/>
    <rect x="100" y="100" width="200" height="20" fill="white" opacity="0.08" rx="10"/>
    <rect x="100" y="130" width="150" height="20" fill="white" opacity="0.08" rx="10"/>
    <rect x="100" y="160" width="180" height="20" fill="white" opacity="0.08" rx="10"/>
</svg>"#
            .to_string()
    }

    pub fn create_background_image_prompt(theme: &str) -> String {
        let prompts = vec![
            format!("Create a modern, abstract background image for a video editing application with {} theme. Include subtle geometric shapes, gradients in purple and blue tones, and video-related iconography like film strips, play buttons, or waveforms. Make it professional and clean with a tech aesthetic.", theme),
            format!("Design a creative background with {} style showing video editing concepts. Include abstract representations of timelines, video frames, color grading elements, and modern UI elements. Use a color palette of deep blues, purples, and subtle accents. Keep it minimalist and sophisticated.", theme),
            format!("Generate a {} themed background for a video editing platform. Show artistic representations of creativity tools like cameras, editing interfaces, sound waves, and light effects. Use gradients and modern design elements with a professional color scheme of blues and purples.", theme),
            format!("Create a {} style background featuring video production elements. Include abstract film reels, digital effects, color gradients, and modern tech aesthetics. Make it suitable for a professional video editing application with clean, contemporary design.", theme),
        ];

        let themes = vec![
            "cinematic",
            "creative",
            "professional",
            "artistic",
            "modern",
            "tech-focused",
            "minimalist",
            "dynamic",
            "elegant",
            "innovative",
        ];

        let mut rng = rand::thread_rng();
        let selected_theme = themes[rng.gen_range(0..themes.len())];
        let selected_prompt = &prompts[rng.gen_range(0..prompts.len())];

        selected_prompt.replace("{}", selected_theme)
    }

    /// Analyze a video file using Gemini 2.5 Flash multimodal capabilities
    /// This provides true "video watching" functionality for the AI agent
    pub async fn analyze_video_content(
        &self,
        video_file_path: &str,
        analysis_prompt: Option<String>,
    ) -> Result<String, Box<dyn std::error::Error + Send + Sync>> {
        // Read and encode the video file
        let video_data = match std::fs::read(video_file_path) {
            Ok(data) => data,
            Err(e) => return Err(format!("Failed to read video file: {}", e).into()),
        };

        let encoded_data = BASE64_STANDARD.encode(&video_data);

        // Determine MIME type based on file extension
        let mime_type = match std::path::Path::new(video_file_path)
            .extension()
            .and_then(|ext| ext.to_str())
            .map(|s| s.to_lowercase())
            .as_deref()
        {
            Some("mp4") => "video/mp4",
            Some("avi") => "video/avi",
            Some("mov") => "video/quicktime",
            Some("mkv") => "video/x-matroska",
            Some("webm") => "video/webm",
            Some("3gpp") => "video/3gpp",
            Some("wmv") => "video/x-ms-wmv",
            _ => "video/mp4", // default
        };

        let prompt = analysis_prompt.unwrap_or_else(|| {
            "Watch this video carefully and provide a detailed analysis. Describe:\n\
            1. Visual content: What objects, people, scenery, and activities do you see?\n\
            2. Audio content: Describe any speech, music, sound effects, or ambient audio\n\
            3. Scene changes: How many different scenes or segments are there?\n\
            4. Motion and action: Describe movement, camera work, and transitions\n\
            5. Technical quality: Comment on video quality, lighting, and production value\n\
            6. Context and purpose: What appears to be the purpose or genre of this video?\n\
            7. Timestamps: If possible, provide key moments with approximate timestamps\n\
            8. Editing opportunities: Suggest what editing operations might improve this video\n\
            \n\
            Be specific and detailed in your analysis as if you're truly watching the video."
                .to_string()
        });

        let request = GenerateContentRequest {
            contents: vec![Content {
                parts: vec![
                    Part::Text { text: prompt },
                    Part::InlineData {
                        inline_data: InlineData {
                            mime_type: mime_type.to_string(),
                            data: encoded_data,
                        },
                    },
                ],
                role: Some("user".to_string()),
            }],
            tools: None,
            generation_config: Some(GenerationConfig {
                temperature: 0.7,
                top_p: 0.8,
                top_k: 40,
                max_output_tokens: 2048, // Increased for detailed analysis
            }),
            tool_config: None,
            system_instruction: None,
        };

        let url = format!(
            "{}/models/gemini-2.5-flash:generateContent?key={}",
            self.base_url, self.api_key
        );

        let response = self
            .client
            .post(&url)
            .header("Content-Type", "application/json")
            .json(&request)
            .send()
            .await?;

        if !response.status().is_success() {
            let error_body = response.text().await?;
            return Err(format!("API request failed: {}", error_body).into());
        }

        let response_body: GenerateContentResponse = response.json().await?;

        if let Some(candidate) = response_body.candidates.first() {
            if let Some(ref content) = candidate.content {
                if let Some(part) = content.parts.first() {
                    if let Part::Text { text } = part {
                        tracing::info!(
                            "Successfully analyzed video content using Gemini 2.5 Flash"
                        );
                        return Ok(text.clone());
                    }
                }
            }
        }

        Err("No valid response received from video analysis".into())
    }

    /// Extract video frames and analyze them individually
    /// This can be used for detailed frame-by-frame analysis or for vectorization
    pub async fn analyze_video_frames(
        &self,
        video_file_path: &str,
        _frame_interval_seconds: Option<f64>,
    ) -> Result<Vec<String>, Box<dyn std::error::Error + Send + Sync>> {
        // For now, we'll analyze the whole video as a single unit
        // In the future, this could be enhanced to extract individual frames using FFmpeg
        let analysis = self.analyze_video_content(
            video_file_path,
            Some("Analyze this video frame by frame. Describe key visual elements, objects, text, and scene changes throughout the video. Focus on content that would be useful for video editing decisions.".to_string())
        ).await?;

        Ok(vec![analysis])
    }

    /// Create embeddings for video content that can be stored in Qdrant
    /// This enables semantic search and context building for video files
    pub async fn create_video_embeddings(
        &self,
        video_analysis: &str,
        video_file_path: &str,
    ) -> Result<Vec<f32>, Box<dyn std::error::Error + Send + Sync>> {
        // Create rich text for embedding that includes both analysis and metadata
        let embedding_text = format!(
            "Video file: {} | Analysis: {} | Content type: video | Capabilities: visual analysis, editing, processing",
            video_file_path, video_analysis
        );

        // Use the existing embed_content method
        self.embed_content(&embedding_text).await
    }

    /// Generate speech audio from text using Gemini TTS API
    pub async fn generate_speech(
        &self,
        text: &str,
        voice: Option<&str>,
        language: Option<&str>,
        style_prompt: Option<&str>,
    ) -> Result<Vec<u8>, Box<dyn std::error::Error + Send + Sync>> {
        let voice = voice.unwrap_or("Zephyr"); // Default professional voice
        let language_code = language.unwrap_or("en");

        // Build the TTS request
        let mut request_body = serde_json::json!({
            "contents": [{
                "parts": [{
                    "text": text
                }]
            }],
            "generationConfig": {
                "responseModalities": ["AUDIO"],
                "speechConfig": {
                    "voiceConfig": {
                        "prebuiltVoiceConfig": {
                            "voiceName": voice
                        }
                    }
                }
            }
        });

        // Add language if specified
        if language_code != "en" {
            request_body["generationConfig"]["speechConfig"]["languageCode"] =
                serde_json::Value::String(language_code.to_string());
        }

        // Add style prompt if provided
        if let Some(prompt) = style_prompt {
            request_body["systemInstruction"] = serde_json::json!({
                "parts": [{
                    "text": format!("Generate speech with the following style and tone: {}", prompt)
                }]
            });
        }

        tracing::info!(
            "🎵 Generating speech audio for text: '{}' with voice: {}",
            &text[..text.len().min(100)],
            voice
        );

        let response = self
            .client
            .post(&format!(
                "{}/v1beta/models/gemini-2.5-flash:generateContent",
                self.base_url
            ))
            .header("Authorization", format!("Bearer {}", self.api_key))
            .header("Content-Type", "application/json")
            .json(&request_body)
            .send()
            .await?;

        if !response.status().is_success() {
            let error_text = response.text().await?;
            tracing::error!("Gemini TTS API error: {}", error_text);
            return Err(format!("Gemini TTS API error: {}", error_text).into());
        }

        let response_json: serde_json::Value = response.json().await?;
        tracing::debug!(
            "Gemini TTS response: {}",
            serde_json::to_string_pretty(&response_json)?
        );

        // Extract audio data from response
        if let Some(candidates) = response_json["candidates"].as_array() {
            if let Some(candidate) = candidates.get(0) {
                if let Some(content) = candidate.get("content") {
                    if let Some(parts) = content["parts"].as_array() {
                        for part in parts {
                            if let Some(inline_data) = part.get("inlineData") {
                                if let Some(data) = inline_data["data"].as_str() {
                                    // Decode base64 audio data
                                    let audio_data =
                                        base64::prelude::BASE64_STANDARD.decode(data).map_err(
                                            |e| format!("Failed to decode audio data: {}", e),
                                        )?;

                                    tracing::info!(
                                        "✅ Generated {} bytes of audio data",
                                        audio_data.len()
                                    );
                                    return Ok(audio_data);
                                }
                            }
                        }
                    }
                }
            }
        }

        Err("No audio data found in TTS response".into())
    }

    /// Generate an advertisement script based on company and requirements
    pub async fn generate_ad_script(
        &self,
        company_name: &str,
        product_description: &str,
        duration_seconds: u32,
        target_audience: Option<&str>,
        style: Option<&str>,
    ) -> Result<String, Box<dyn std::error::Error + Send + Sync>> {
        let audience = target_audience.unwrap_or("general consumers");
        let ad_style = style.unwrap_or("professional and engaging");

        let prompt = format!(
            "Create a {}-second advertisement script for {}, a company that specializes in {}. 

The script should be:
- Targeted at {}
- Written in a {} style
- Exactly {} seconds when read aloud (approximately {} words)
- Include a compelling hook, key benefits, and call to action
- Sound natural when spoken as voiceover

Company: {}
Product: {}
Duration: {} seconds
Target Audience: {}
Style: {}

Generate ONLY the script text that will be spoken, no stage directions or formatting.",
            duration_seconds,
            company_name,
            product_description,
            audience,
            ad_style,
            duration_seconds,
            duration_seconds * 3, // ~3 words per second
            company_name,
            product_description,
            duration_seconds,
            audience,
            ad_style
        );

        let request = GenerateContentRequest {
            contents: vec![Content {
                parts: vec![Part::Text { text: prompt }],
                role: Some("user".to_string()),
            }],
            tools: None,
            generation_config: Some(GenerationConfig {
                temperature: 0.7,
                top_k: 40,
                top_p: 0.9,
                max_output_tokens: 1024,
            }),
            tool_config: None,
            system_instruction: None,
        };

        tracing::info!(
            "🎬 Generating advertisement script for {} ({}s duration)",
            company_name,
            duration_seconds
        );

        let response = self.generate_content(request).await?;

        // Extract the script text from response
        if let Some(candidate) = response.candidates.first() {
            if let Some(ref content) = candidate.content {
                if let Some(part) = content.parts.first() {
                    if let Part::Text { text } = part {
                        tracing::info!(
                            "✅ Generated {}-word script for {} second ad",
                            text.split_whitespace().count(),
                            duration_seconds
                        );
                        return Ok(text.clone());
                    }
                }
            }
        }

        Err("Failed to generate advertisement script".into())
    }

    /// Generate a script for any type of video
    pub async fn generate_video_script(
        &self,
        video_type: &str,
        subject: &str,
        description: &str,
        duration_seconds: u32,
        target_audience: Option<&str>,
        style: Option<&str>,
    ) -> Result<String, Box<dyn std::error::Error + Send + Sync>> {
        let audience = target_audience.unwrap_or("general audience");
        let video_style = style.unwrap_or("professional and engaging");

        let prompt = match video_type.to_lowercase().as_str() {
            "music_video" | "music video" => {
                format!(
                    "Create a {}-second music video concept script for '{}'. 

The music video should:
- Complement the music: {}
- Target audience: {}
- Visual style: {}
- Duration: {} seconds (approximately {} words for narration/lyrics)

Include:
- Visual concept and scenes
- Key moments and transitions
- Any spoken elements or voice-over
- Creative direction notes

Subject: {}
Description: {}
Duration: {} seconds
Target Audience: {}
Style: {}

Generate a creative music video script with scene descriptions and any narrative elements.",
                    duration_seconds,
                    subject,
                    description,
                    audience,
                    video_style,
                    duration_seconds,
                    duration_seconds * 2,
                    subject,
                    description,
                    duration_seconds,
                    audience,
                    video_style
                )
            }
            "documentary" => {
                format!(
                    "Create a {}-second documentary script about '{}'. 

The documentary should:
- Inform and educate about: {}
- Target audience: {}
- Style: {}
- Duration: {} seconds (approximately {} words for narration)

Include:
- Compelling opening hook
- Key facts and information
- Interview questions or talking points
- Narrative structure
- Strong conclusion

Subject: {}
Description: {}
Duration: {} seconds
Target Audience: {}
Style: {}

Generate a documentary script with narration and scene direction.",
                    duration_seconds,
                    subject,
                    description,
                    audience,
                    video_style,
                    duration_seconds,
                    duration_seconds * 3,
                    subject,
                    description,
                    duration_seconds,
                    audience,
                    video_style
                )
            }
            "tutorial" | "educational" => {
                format!(
                    "Create a {}-second tutorial script for '{}'. 

The tutorial should:
- Teach viewers about: {}
- Target audience: {}
- Teaching style: {}
- Duration: {} seconds (approximately {} words)

Include:
- Clear introduction and learning objectives
- Step-by-step instructions
- Tips and best practices
- Common mistakes to avoid
- Summary and next steps

Subject: {}
Description: {}
Duration: {} seconds
Target Audience: {}
Style: {}

Generate a clear, educational tutorial script.",
                    duration_seconds,
                    subject,
                    description,
                    audience,
                    video_style,
                    duration_seconds,
                    duration_seconds * 3,
                    subject,
                    description,
                    duration_seconds,
                    audience,
                    video_style
                )
            }
            "promotional" | "promo" => {
                format!(
                    "Create a {}-second promotional video script for '{}'. 

The promotional video should:
- Highlight: {}
- Target audience: {}
- Promotional style: {}
- Duration: {} seconds (approximately {} words)

Include:
- Attention-grabbing opening
- Key benefits and features
- Social proof or testimonials
- Clear call-to-action
- Memorable closing

Subject: {}
Description: {}
Duration: {} seconds
Target Audience: {}
Style: {}

Generate an engaging promotional script.",
                    duration_seconds,
                    subject,
                    description,
                    audience,
                    video_style,
                    duration_seconds,
                    duration_seconds * 3,
                    subject,
                    description,
                    duration_seconds,
                    audience,
                    video_style
                )
            }
            _ => {
                // Generic video script
                format!(
                    "Create a {}-second {} video script for '{}'. 

The video should:
- Focus on: {}
- Target audience: {}
- Style: {}
- Duration: {} seconds (approximately {} words)

Include:
- Engaging opening
- Clear main content
- Appropriate tone and pacing
- Strong conclusion
- Any necessary call-to-action

Subject: {}
Description: {}
Video Type: {}
Duration: {} seconds
Target Audience: {}
Style: {}

Generate a well-structured script appropriate for this type of video.",
                    duration_seconds,
                    video_type,
                    subject,
                    description,
                    audience,
                    video_style,
                    duration_seconds,
                    duration_seconds * 3,
                    subject,
                    description,
                    video_type,
                    duration_seconds,
                    audience,
                    video_style
                )
            }
        };

        let request = GenerateContentRequest {
            contents: vec![Content {
                parts: vec![Part::Text { text: prompt }],
                role: Some("user".to_string()),
            }],
            tools: None,
            generation_config: Some(GenerationConfig {
                temperature: 0.7,
                top_k: 40,
                top_p: 0.9,
                max_output_tokens: 2048,
            }),
            tool_config: None,
            system_instruction: None,
        };

        tracing::info!(
            "🎬 Generating {} video script for '{}' ({}s duration)",
            video_type,
            subject,
            duration_seconds
        );

        let response = self.generate_content(request).await?;

        // Extract the script text from response
        if let Some(candidate) = response.candidates.first() {
            if let Some(ref content) = candidate.content {
                if let Some(part) = content.parts.first() {
                    if let Part::Text { text } = part {
                        tracing::info!(
                            "✅ Generated {}-word {} script for {} second video",
                            text.split_whitespace().count(),
                            video_type,
                            duration_seconds
                        );
                        return Ok(text.clone());
                    }
                }
            }
        }

        Err("Failed to generate video script".into())
    }

    /// Analyze a YouTube video by URL using Gemini's native video understanding.
    ///
    /// This replaces the entire frame-by-frame pipeline:
    /// - ONE API call instead of 100+ sequential frame analyses
    /// - Gemini sees the full video including motion, audio, and pacing
    /// - `mediaResolution: MEDIA_RESOLUTION_LOW` processes at ~100 tokens/sec (~60k tokens per 10 min video)
    /// - Returns structured JSON with viral moments, timestamps, and quality scores
    ///
    /// Returns Err if no viral moments meet the quality threshold (fast-fail — skip download)
    pub async fn analyze_video_from_url(
        &self,
        youtube_url: &str,
        clips_per_video: usize,
        min_duration_secs: f64,
        max_duration_secs: f64,
        high_performing_factors: &[String],
    ) -> Result<
        crate::clipping::gemini_video_analyzer::VideoAnalysis,
        Box<dyn std::error::Error + Send + Sync>,
    > {
        // Acquire concurrency permit for this expensive video analysis call.
        let _permit = self
            .semaphore
            .acquire()
            .await
            .map_err(|e| format!("Gemini semaphore error: {}", e))?;

        tracing::info!(
            "🎬 Analyzing video via YouTube URL (1 Gemini call): {}",
            youtube_url
        );

        let learned_factors_hint = if !high_performing_factors.is_empty() {
            format!(
                "\nLEARNED HIGH-PERFORMING FACTORS (prioritize moments containing these): {}\n",
                high_performing_factors.join(", ")
            )
        } else {
            String::new()
        };

        let prompt = format!(
            r#"Analyze this YouTube video and identify exactly {clips_per_video} viral clip opportunities for YouTube Shorts.

REQUIREMENTS:
- Each clip must be between {min_dur:.0} and {max_dur:.0} seconds (HARD LIMIT — never exceed {max_dur:.0}s)
- Clips will be published as YouTube Shorts (vertical 9:16 portrait format, center-cropped from landscape)
- Prioritize moments where the subject is centered in frame (survives a center-crop to portrait)
- Focus on: dramatic hooks, surprising moments, emotional peaks, action sequences, plot twists
- Clips should work as standalone content without needing context
- Prefer moments with clear speech or impactful audio (loudness is normalized in post){learned_hint}

Return ONLY a valid JSON object matching this exact schema:
{{
  "video_summary": "<comprehensive plain-text summary of the entire video for search indexing>",
  "content_type": "<one of: entertainment, tutorial, news, gaming, sports, music, vlog, other>",
  "overall_quality": <float 0.0-1.0>,
  "viral_moments": [
    {{
      "start_sec": <float seconds>,
      "end_sec": <float seconds>,
      "title": "<engaging YouTube Short title, max 60 chars>",
      "hook": "<first sentence that grabs attention, used as video description opener>",
      "quality_score": <float 0.0-1.0>,
      "viral_factors": ["<factor1>", "<factor2>"],
      "thumbnail_sec": <float seconds — best frame for thumbnail within the clip>,
      "reason": "<why this moment is viral/engaging>"
    }}
  ]
}}

Provide ONLY the JSON object, no markdown, no code blocks, no other text."#,
            clips_per_video = clips_per_video,
            min_dur = min_duration_secs,
            max_dur = max_duration_secs,
            learned_hint = learned_factors_hint,
        );

        // Build request with YouTube fileData part and mediaResolution: MEDIA_RESOLUTION_LOW
        // thinkingBudget: 0 disables hidden thinking tokens on gemini-2.5-flash (saves 1k-5k tokens/call)
        let request_body = serde_json::json!({
            "contents": [{
                "role": "user",
                "parts": [
                    {
                        "fileData": {
                            "mimeType": "video/*",
                            "fileUri": youtube_url
                        }
                    },
                    {
                        "text": prompt
                    }
                ]
            }],
            "generationConfig": {
                "temperature": 0.3,
                "maxOutputTokens": 8192,
                "responseMimeType": "application/json",
                "mediaResolution": "MEDIA_RESOLUTION_LOW",
                "thinkingConfig": {
                    "thinkingBudget": 0
                }
            }
        });

        let url = format!(
            "{}/models/gemini-2.5-flash:generateContent?key={}",
            self.base_url, self.api_key
        );

        // Retry up to 3 times on transient errors
        let max_attempts = 3u32;
        let mut last_error: Box<dyn std::error::Error + Send + Sync> = "No attempts made".into();

        for attempt in 0..max_attempts {
            let response = self
                .client
                .post(&url)
                .header("Content-Type", "application/json")
                .json(&request_body)
                .send()
                .await?;

            let status = response.status();

            if status.is_success() {
                let response_text = response.text().await?;
                tracing::debug!(
                    "Gemini video analysis response (first 1000 chars): {}",
                    &response_text[..response_text.len().min(1000)]
                );

                // Parse Gemini response wrapper
                let response_json: serde_json::Value = serde_json::from_str(&response_text)
                    .map_err(|e| {
                        format!(
                            "Failed to parse Gemini response: {} — body: {}",
                            e,
                            &response_text[..response_text.len().min(500)]
                        )
                    })?;

                // Extract text from candidates[0].content.parts[0].text
                let text = response_json["candidates"][0]["content"]["parts"][0]["text"]
                    .as_str()
                    .ok_or("Gemini response missing text content")?;

                // Parse the JSON text content as VideoAnalysis
                let analysis: crate::clipping::gemini_video_analyzer::VideoAnalysis =
                    serde_json::from_str(text).map_err(|e| {
                        format!(
                            "Failed to parse VideoAnalysis JSON: {} — text: {}",
                            e,
                            &text[..text.len().min(1000)]
                        )
                    })?;

                tracing::info!(
                    "✅ Video analysis complete: {} viral moments identified (overall quality: {:.2})",
                    analysis.viral_moments.len(),
                    analysis.overall_quality
                );

                return Ok(analysis);
            }

            let error_text = response
                .text()
                .await
                .unwrap_or_else(|_| "Unknown error".to_string());

            // On 429 rate limit, back off and retry
            if status.as_u16() == 429 && attempt < max_attempts - 1 {
                let retry_secs = parse_gemini_retry_delay(&error_text, 30.0);
                let wait_secs = (retry_secs + 5.0) as u64;
                tracing::warn!(
                    "⏳ Gemini rate limited (429, attempt {}/{}). Waiting {}s…",
                    attempt + 1,
                    max_attempts,
                    wait_secs
                );
                tokio::time::sleep(tokio::time::Duration::from_secs(wait_secs)).await;
                last_error = format!("Gemini rate limited: {}", error_text).into();
                continue;
            }

            last_error = format!("Gemini API error (HTTP {}): {}", status, error_text).into();

            if attempt < max_attempts - 1 {
                tracing::warn!(
                    "Gemini attempt {}/{} failed, retrying: {}",
                    attempt + 1,
                    max_attempts,
                    last_error
                );
                tokio::time::sleep(tokio::time::Duration::from_secs(5)).await;
            }
        }

        Err(last_error)
    }

    /// Analyze a video from any source (local path or cloud URL) by sending the full
    /// video file as base64 inlineData to Gemini for NATIVE video understanding with
    /// full motion, audio, pacing, and temporal analysis.
    ///
    /// This replaces the old frame-extraction approach with true full-video processing.
    /// Gemini's base64 inlineData works well for files up to ~10MB (typical edit outputs).
    /// For larger videos without Gemini File API, the caller should use the Claude or
    /// Bedrock path instead (both support direct URL video input).
    ///
    /// Produces the same `VideoAnalysis` schema as `analyze_video_from_url`.
    pub async fn analyze_video_from_local_file(
        &self,
        video_source: &str,
        clips_per_video: usize,
        min_duration_secs: f64,
        max_duration_secs: f64,
        high_performing_factors: &[String],
    ) -> Result<
        crate::clipping::gemini_video_analyzer::VideoAnalysis,
        Box<dyn std::error::Error + Send + Sync>,
    > {
        let _permit = self
            .semaphore
            .acquire()
            .await
            .map_err(|e| format!("Gemini semaphore error: {}", e))?;

        tracing::info!(
            "Analyzing video via Gemini native full-video API: {}",
            video_source
        );

        // --- Step 1: Get the video bytes ---
        // Download from cloud URL if needed, or read local file directly.
        let video_bytes = if video_source.starts_with("http://")
            || video_source.starts_with("https://")
        {
            tracing::debug!("Downloading cloud video for Gemini analysis: {}", video_source);
            let response = reqwest::get(video_source)
                .await
                .map_err(|e| format!("Failed to download video for analysis: {}", e))?;
            response.bytes().await?.to_vec()
        } else {
            tracing::debug!("Reading local video for Gemini analysis: {}", video_source);
            tokio::fs::read(video_source)
                .await
                .map_err(|e| format!("Failed to read video file '{}': {}", video_source, e))?
        };

        if video_bytes.is_empty() {
            return Err(format!("Video source '{}' produced zero bytes", video_source).into());
        }

        let total_len_mb = video_bytes.len() as f64 / 1_048_576.0;
        if total_len_mb > 12.0 {
            return Err(format!(
                "Video too large for Gemini inline analysis ({:.1}MB > 10MB limit). \
                 Use Claude (direct URL) or Bedrock (S3 reference) for large video analysis.",
                total_len_mb
            ).into());
        }

        // Get total duration via ffprobe (download to temp, probe, clean up)
        let total_dur = {
            let temp_probe = std::env::temp_dir().join(format!("gemini_probe_{}.mp4", uuid::Uuid::new_v4()));
            tokio::fs::write(&temp_probe, &video_bytes).await?;
            let dur = crate::core::get_video_duration(&temp_probe.to_string_lossy())
                .unwrap_or(30.0);
            let _ = tokio::fs::remove_file(&temp_probe).await;
            dur
        };

        // Detect MIME type from extension
        let ext = video_source.rsplit('.').next().unwrap_or("mp4").to_lowercase();
        let mime_type = match ext.as_str() {
            "avi" => "video/avi",
            "mov" | "qt" => "video/quicktime",
            "mkv" => "video/x-matroska",
            "webm" => "video/webm",
            _ => "video/mp4",
        };

        let learned_factors_hint = if !high_performing_factors.is_empty() {
            format!(
                "\nLEARNED HIGH-PERFORMING FACTORS (prioritize moments containing these): {}\n",
                high_performing_factors.join(", ")
            )
        } else {
            String::new()
        };

        // --- Step 2: Send full video as base64 inlineData for native Gemini video analysis ---
        let prompt = format!(
            r#"Analyze this video natively — you see the FULL video with motion, audio, pacing, and timing.

Total duration: {total_dur:.0}s ({total_min:.1} minutes).

Identify exactly {clips_per_video} viral clip opportunities for YouTube Shorts.

REQUIREMENTS:
- Each clip must be between {min_dur:.0}s and {max_dur:.0}s (HARD LIMIT — never exceed {max_dur:.0}s)
- Clips will be published as YouTube Shorts (vertical 9:16 portrait format, center-cropped from landscape)
- Prioritize moments where the subject is centered in frame
- Focus on: dramatic hooks, surprising moments, emotional peaks, action sequences, plot twists
- Clips should work as standalone content without needing context{learned_hint}

Return ONLY a valid JSON object matching this exact schema:
{{
  "video_summary": "<comprehensive plain-text summary of the entire video>",
  "content_type": "<one of: entertainment, tutorial, news, gaming, sports, music, vlog, other>",
  "overall_quality": <float 0.0-1.0>,
  "viral_moments": [
    {{
      "start_sec": <float seconds>,
      "end_sec": <float seconds>,
      "title": "<engaging YouTube Short title, max 60 chars>",
      "hook": "<first sentence that grabs attention>",
      "quality_score": <float 0.0-1.0>,
      "viral_factors": ["<factor1>", "<factor2>"],
      "thumbnail_sec": <float seconds — best frame for thumbnail within the clip>,
      "reason": "<why this moment is viral/engaging>"
    }}
  ]
}}

Provide ONLY the JSON object, no markdown, no code blocks, no other text."#,
            total_dur = total_dur,
            total_min = total_dur / 60.0,
            clips_per_video = clips_per_video,
            min_dur = min_duration_secs,
            max_dur = max_duration_secs,
            learned_hint = learned_factors_hint,
        );

        let encoded_data = BASE64_STANDARD.encode(&video_bytes);

        let request_body = serde_json::json!({
            "contents": [{
                "role": "user",
                "parts": [
                    {"text": prompt},
                    {"inlineData": {"mimeType": mime_type, "data": encoded_data}}
                ]
            }],
            "generationConfig": {
                "temperature": 0.3,
                "maxOutputTokens": 8192,
                "responseMimeType": "application/json",
                "thinkingConfig": {"thinkingBudget": 0}
            }
        });

        let url = format!(
            "{}/models/gemini-2.5-flash:generateContent?key={}",
            self.base_url, self.api_key
        );

        let max_attempts = 3u32;
        let mut last_error: Box<dyn std::error::Error + Send + Sync> = "No attempts made".into();

        for attempt in 0..max_attempts {
            let response = self
                .client
                .post(&url)
                .header("Content-Type", "application/json")
                .json(&request_body)
                .send()
                .await?;

            let status = response.status();

            if status.is_success() {
                let response_text = response.text().await?;
                let response_json: serde_json::Value = serde_json::from_str(&response_text)
                    .map_err(|e| format!("Failed to parse Gemini response: {}", e))?;

                let text = response_json["candidates"][0]["content"]["parts"][0]["text"]
                    .as_str()
                    .ok_or("Gemini response missing text content")?;

                let analysis: crate::clipping::gemini_video_analyzer::VideoAnalysis =
                    serde_json::from_str(text).map_err(|e| {
                        format!(
                            "Failed to parse VideoAnalysis JSON from native video analysis: {} — text: {}",
                            e,
                            &text[..text.len().min(500)]
                        )
                    })?;

                tracing::info!(
                    "✅ Native video analysis complete: {} viral moments (quality: {:.2})",
                    analysis.viral_moments.len(),
                    analysis.overall_quality
                );

                return Ok(analysis);
            }

            let error_text = response
                .text()
                .await
                .unwrap_or_else(|_| "Unknown error".to_string());

            if status.as_u16() == 429 && attempt < max_attempts - 1 {
                let wait_secs = (parse_gemini_retry_delay(&error_text, 30.0) + 5.0) as u64;
                tracing::warn!(
                    "⏳ Gemini rate limited (native video analysis, attempt {}/{}). Waiting {}s…",
                    attempt + 1,
                    max_attempts,
                    wait_secs
                );
                tokio::time::sleep(tokio::time::Duration::from_secs(wait_secs)).await;
                last_error = format!("Rate limited: {}", error_text).into();
                continue;
            }

            last_error = format!("Gemini API error (HTTP {}): {}", status, error_text).into();

            if attempt < max_attempts - 1 {
                tracing::warn!(
                    "Native video analysis attempt {}/{} failed, retrying: {}",
                    attempt + 1,
                    max_attempts,
                    last_error
                );
                tokio::time::sleep(tokio::time::Duration::from_secs(5)).await;
            }
        }

        Err(last_error)
    }

    /// Simple text generation — used by Twitch mapper and other plain-text callers.
    /// Uses raw JSON with thinkingBudget:0 to avoid burning thinking-token quota.
    /// Retries up to 3 times on 429 RESOURCE_EXHAUSTED with back-off.
    pub async fn generate_text(
        &self,
        prompt: &str,
    ) -> Result<String, Box<dyn std::error::Error + Send + Sync>> {
        let _permit = self
            .semaphore
            .acquire()
            .await
            .map_err(|e| format!("Gemini semaphore error: {}", e))?;

        let url = format!(
            "{}/models/{}:generateContent?key={}",
            self.base_url, self.text_model, self.api_key
        );

        let request_body = serde_json::json!({
            "contents": [{"role": "user", "parts": [{"text": prompt}]}],
            "generationConfig": {
                "temperature": 0.1,
                "maxOutputTokens": 512,
                "thinkingConfig": {"thinkingBudget": 0}
            }
        });

        let max_attempts = 3u32;
        let mut last_err: Box<dyn std::error::Error + Send + Sync> =
            "generate_text: no attempts made".into();

        for attempt in 0..max_attempts {
            let response = self
                .client
                .post(&url)
                .header("Content-Type", "application/json")
                .json(&request_body)
                .send()
                .await?;

            let status = response.status();

            if status.is_success() {
                let json: serde_json::Value = response.json().await?;
                let text = json["candidates"][0]["content"]["parts"][0]["text"]
                    .as_str()
                    .ok_or("Gemini generate_text: no text in response")?
                    .to_string();
                return Ok(text);
            }

            let err_body = response.text().await.unwrap_or_default();

            if status.as_u16() == 429 && attempt < max_attempts - 1 {
                let wait = (parse_gemini_retry_delay(&err_body, 60.0) + 5.0) as u64;
                tracing::warn!(
                    "⏳ generate_text: Gemini 429 (attempt {}/{}). Waiting {}s…",
                    attempt + 1,
                    max_attempts,
                    wait
                );
                tokio::time::sleep(tokio::time::Duration::from_secs(wait)).await;
                last_err = format!("Gemini API error: {}", err_body).into();
                continue;
            }

            last_err = format!("Gemini API error: {}", err_body).into();
            if attempt < max_attempts - 1 {
                tokio::time::sleep(tokio::time::Duration::from_secs(5)).await;
            }
        }

        Err(last_err)
    }
}
