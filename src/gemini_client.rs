use reqwest::Client;
use serde::{Deserialize, Serialize};
use serde_json::Value;
use std::collections::HashMap;
use std::sync::Arc;
use rand::Rng;
use base64::prelude::*;

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
    Text { text: String },
    FunctionCall { 
        #[serde(rename = "functionCall")]
        function_call: FunctionCall 
    },
    FunctionResponse { 
        #[serde(rename = "functionResponse")]
        function_response: FunctionResponse 
    },
    InlineData { 
        #[serde(rename = "inlineData")]
        inline_data: InlineData 
    },
    FileData { 
        #[serde(rename = "fileData")]
        file_data: FileData 
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
    #[serde(rename = "thoughtSignature", skip_serializing_if = "Option::is_none", default)]
    pub thought_signature: Option<String>,
}

#[derive(Debug, Serialize, Deserialize, Clone)]
pub struct FunctionResponse {
    pub name: String,
    pub response: HashMap<String, Value>,
    #[serde(rename = "thoughtSignature", skip_serializing_if = "Option::is_none", default)]
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
        Self {
            client: Client::new(),
            api_key,
            base_url: "https://generativelanguage.googleapis.com/v1beta".to_string(),
            text_model: "gemini-2.5-flash".to_string(),
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
        let _permit = self.semaphore.acquire().await
            .map_err(|e| format!("Gemini semaphore error: {}", e))?;

        let url = format!(
            "{}/models/{}:generateContent?key={}",
            self.base_url, self.text_model, self.api_key
        );

        // Debug: Log the request to see if thought signatures are present
        if let Ok(_request_json) = serde_json::to_string_pretty(&request) {
            tracing::debug!("Gemini API Request contents count: {}", request.contents.len());
            for (i, content) in request.contents.iter().enumerate() {
                tracing::debug!("Content[{}]: role={:?}, parts_count={}", i, content.role, content.parts.len());
                for (j, part) in content.parts.iter().enumerate() {
                    match part {
                        Part::FunctionCall { function_call } => {
                            tracing::warn!("Content[{}].Part[{}]: FunctionCall name={}, has_signature={}",
                                i, j, function_call.name, function_call.thought_signature.is_some());
                        }
                        Part::FunctionResponse { function_response } => {
                            tracing::debug!("Content[{}].Part[{}]: FunctionResponse name={}, has_signature={}",
                                i, j, function_response.name, function_response.thought_signature.is_some());
                        }
                        _ => {}
                    }
                }
            }
        }

        // Retry loop: up to 4 attempts, respecting Gemini rate-limit (429) responses.
        let max_attempts = 4u32;
        let mut last_error: Box<dyn std::error::Error + Send + Sync> =
            "No attempts made".into();

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
                tracing::debug!("Gemini API response (truncated): {}...", &response_text[..response_text.len().min(500)]);

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
                    },
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
                    attempt + 1, max_attempts, wait_secs
                );
                tokio::time::sleep(tokio::time::Duration::from_secs(wait_secs)).await;
                last_error = format!("Gemini API error (rate limited): {}", error_text).into();
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
        let url = format!(
            "{}/models/text-embedding-004:embedContent?key={}",
            self.base_url, self.api_key
        );

        let request = EmbedContentRequest {
            model: "models/text-embedding-004".to_string(),
            content: EmbedContent {
                parts: vec![Part::Text {
                    text: text.to_string(),
                }],
            },
            output_dimensionality: Some(768), // Using smaller dimension for efficiency
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

    /// Resolve a user-supplied model alias to a concrete Gemini model ID.
    fn resolve_image_model(model: Option<&str>) -> &str {
        match model {
            Some("fast") | Some("nano") => "gemini-2.0-flash-preview-image-generation",
            Some("quality") | Some("pro") | None => "gemini-3-pro-image-preview",
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
        let _permit = self.semaphore.acquire().await
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
        config_map.insert("imageConfig".to_string(), serde_json::Value::Object(image_config));

        config_map.insert(
            "responseModalities".to_string(),
            serde_json::Value::Array(vec![
                serde_json::Value::String("IMAGE".to_string()),
            ]),
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

        tracing::debug!("generate_image ({}) request: {}", model_id, serde_json::to_string_pretty(&request)?);

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
                                        let image_bytes = BASE64_STANDARD.decode(data)
                                            .map_err(|e| format!("Failed to decode base64 image: {}", e))?;
                                        return Ok(image_bytes);
                                    }
                                }
                            }
                        }
                    }
                }
            }

            Err("No image data found in response".into())
        } else {
            let error_text = response.text().await?;
            Err(format!("Gemini image generation API error ({}): {}", model_id, error_text).into())
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
        let _permit = self.semaphore.acquire().await
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
        self.edit_image(prompt, image_bytes, aspect_ratio, None).await
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
                name: "extract_audio".to_string(),
                description: "Extracts audio track from a video file".to_string(),
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
                            description: "Path to save the extracted audio".to_string(),
                            items: None,
                        });
                        props.insert("format".to_string(), PropertyDefinition {
                            prop_type: "string".to_string(),
                            description: "Audio format (mp3, wav, aac, etc.)".to_string(),
                            items: None,
                        });
                        props
                    },
                    required: vec!["input_file".to_string(), "output_file".to_string(), "format".to_string()],
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
                name: "extract_frames".to_string(),
                description: "Extracts individual frames from a video as image files".to_string(),
                parameters: Parameters {
                    param_type: "object".to_string(),
                    properties: {
                        let mut props = HashMap::new();
                        props.insert("input_file".to_string(), PropertyDefinition {
                            prop_type: "string".to_string(),
                            description: "Path to the input video file".to_string(),
                            items: None,
                        });
                        props.insert("output_dir".to_string(), PropertyDefinition {
                            prop_type: "string".to_string(),
                            description: "Directory to save extracted frames".to_string(),
                            items: None,
                        });
                        props.insert("frame_rate".to_string(), PropertyDefinition {
                            prop_type: "number".to_string(),
                            description: "Extract one frame every N seconds (default: 1)".to_string(),
                            items: None,
                        });
                        props.insert("format".to_string(), PropertyDefinition {
                            prop_type: "string".to_string(),
                            description: "Image format for frames (png, jpg, etc.)".to_string(),
                            items: None,
                        });
                        props
                    },
                    required: vec!["input_file".to_string(), "output_dir".to_string()],
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
                description: "Views/analyzes a video by retrieving its vectorized embeddings from the database. This allows you to 'see' what's in a video without re-processing it. Use this to understand video content, verify edits, or check what a previously generated video contains. Returns detailed frame-by-frame analysis and overall summary.".to_string(),
                parameters: Parameters {
                    param_type: "object".to_string(),
                    properties: {
                        let mut props = HashMap::new();
                        props.insert("video_path".to_string(), PropertyDefinition {
                            prop_type: "string".to_string(),
                            description: "Path to the video file to view/analyze (e.g., 'outputs/edited_video.mp4')".to_string(),
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
                            description: "Path to the output video to review".to_string(),
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
                description: "Views/analyzes an image file using AI vision. Use this to verify generated images, inspect stock photos from Pexels, or check overlay images before using them in videos. Returns detailed analysis of content, colors, composition, style, and suitability for video use.".to_string(),
                parameters: Parameters {
                    param_type: "object".to_string(),
                    properties: {
                        let mut props = HashMap::new();
                        props.insert("image_path".to_string(), PropertyDefinition {
                            prop_type: "string".to_string(),
                            description: "Path to the image file to view/analyze (e.g., 'outputs/generated_logo.png' or 'outputs/stock_photo.jpg')".to_string(),
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
                description: "Fetch the hero/og:image from a website URL. Use this when a user provides a website URL and you need to extract its visual for use in a Blender scene or product mockup. Returns the image URL string that can be passed to blender_generate_scene's reference_image_url parameter.".to_string(),
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

            // BLENDER MCP TOOLS — 3D rendering, Manim, thumbnails, data viz
            // =====================================================================

            FunctionDeclaration {
                name: "blender_generate_scene".to_string(),
                description: "Generate a procedural 3D Blender scene as an MP4 clip from a natural language description. Use this to create custom cinematic B-roll footage, abstract backgrounds, or any visual scene that stock footage cannot provide. Returns a local video file path.".to_string(),
                parameters: Parameters {
                    param_type: "object".to_string(),
                    properties: {
                        let mut props = HashMap::new();
                        props.insert("prompt".to_string(), PropertyDefinition {
                            prop_type: "string".to_string(),
                            description: "Natural language description of the scene (e.g. 'cinematic ocean at sunset, calm mood, 10 seconds')".to_string(),
                            items: None,
                        });
                        props.insert("duration".to_string(), PropertyDefinition {
                            prop_type: "number".to_string(),
                            description: "Target clip duration in seconds (default: 10)".to_string(),
                            items: None,
                        });
                        props.insert("style".to_string(), PropertyDefinition {
                            prop_type: "string".to_string(),
                            description: "Visual style: 'cinematic' (default), 'minimal', 'energetic', or 'calm'".to_string(),
                            items: None,
                        });
                        props.insert("reference_image_url".to_string(), PropertyDefinition {
                            prop_type: "string".to_string(),
                            description: "Optional URL of a reference/inspiration image to guide the scene aesthetics".to_string(),
                            items: None,
                        });
                        props.insert("include_narration".to_string(), PropertyDefinition {
                            prop_type: "boolean".to_string(),
                            description: "Optional. If true, generate narration audio and a narrated video variant when VibeVoice is configured".to_string(),
                            items: None,
                        });
                        props.insert("narration_text".to_string(), PropertyDefinition {
                            prop_type: "string".to_string(),
                            description: "Optional narration script to speak over the rendered scene".to_string(),
                            items: None,
                        });
                        props.insert("narration_speaker".to_string(), PropertyDefinition {
                            prop_type: "string".to_string(),
                            description: "Optional VibeVoice speaker preset name, e.g. 'Emma' or 'Carter'".to_string(),
                            items: None,
                        });
                        props
                    },
                    required: vec!["prompt".to_string()],
                },
            },
            FunctionDeclaration {
                name: "blender_generate_thumbnail".to_string(),
                description: "Generate a 3D rendered YouTube thumbnail image (1280x720 PNG) using Blender. Creates professional-grade thumbnails with 3D elements, dramatic lighting, and optional text overlays.".to_string(),
                parameters: Parameters {
                    param_type: "object".to_string(),
                    properties: {
                        let mut props = HashMap::new();
                        props.insert("prompt".to_string(), PropertyDefinition {
                            prop_type: "string".to_string(),
                            description: "Scene description for the thumbnail (e.g. 'tech startup success, dark background, neon blue accents')".to_string(),
                            items: None,
                        });
                        props.insert("title_text".to_string(), PropertyDefinition {
                            prop_type: "string".to_string(),
                            description: "Optional text to overlay on the thumbnail".to_string(),
                            items: None,
                        });
                        props.insert("style".to_string(), PropertyDefinition {
                            prop_type: "string".to_string(),
                            description: "Thumbnail style: 'youtube' (default), 'cinematic', or 'minimal'".to_string(),
                            items: None,
                        });
                        props
                    },
                    required: vec!["prompt".to_string()],
                },
            },
            FunctionDeclaration {
                name: "blender_generate_title_card".to_string(),
                description: "Generate an animated 3D title card as an MP4 clip (typically 3-8 seconds). Perfect for branded intros, section dividers, or professional chapter headings.".to_string(),
                parameters: Parameters {
                    param_type: "object".to_string(),
                    properties: {
                        let mut props = HashMap::new();
                        props.insert("title".to_string(), PropertyDefinition {
                            prop_type: "string".to_string(),
                            description: "Main title text to animate".to_string(),
                            items: None,
                        });
                        props.insert("subtitle".to_string(), PropertyDefinition {
                            prop_type: "string".to_string(),
                            description: "Secondary/tagline text (optional)".to_string(),
                            items: None,
                        });
                        props.insert("duration".to_string(), PropertyDefinition {
                            prop_type: "number".to_string(),
                            description: "Clip length in seconds, 3-8 recommended (default: 5)".to_string(),
                            items: None,
                        });
                        props.insert("style".to_string(), PropertyDefinition {
                            prop_type: "string".to_string(),
                            description: "Visual style description (e.g. 'minimalist dark', 'corporate blue', 'energetic neon')".to_string(),
                            items: None,
                        });
                        props
                    },
                    required: vec!["title".to_string()],
                },
            },
            FunctionDeclaration {
                name: "blender_generate_data_viz".to_string(),
                description: "Generate an animated 3D data visualisation clip from JSON data. Creates animated bar charts, line graphs, pie charts, or globe visualisations for educational and business videos.".to_string(),
                parameters: Parameters {
                    param_type: "object".to_string(),
                    properties: {
                        let mut props = HashMap::new();
                        props.insert("data_json".to_string(), PropertyDefinition {
                            prop_type: "string".to_string(),
                            description: r#"JSON array of data points, e.g. '[{"label":"Q1","value":42},{"label":"Q2","value":78}]'"#.to_string(),
                            items: None,
                        });
                        props.insert("chart_type".to_string(), PropertyDefinition {
                            prop_type: "string".to_string(),
                            description: "Chart type: 'bar' (default), 'line', 'pie', or 'globe'".to_string(),
                            items: None,
                        });
                        props.insert("title".to_string(), PropertyDefinition {
                            prop_type: "string".to_string(),
                            description: "Chart title overlay text".to_string(),
                            items: None,
                        });
                        props.insert("duration".to_string(), PropertyDefinition {
                            prop_type: "number".to_string(),
                            description: "Animation length in seconds (default: 10)".to_string(),
                            items: None,
                        });
                        props
                    },
                    required: vec!["data_json".to_string()],
                },
            },
            FunctionDeclaration {
                name: "blender_generate_lower_third".to_string(),
                description: "Generate an animated lower-third text overlay clip. Creates professional broadcast-style name plates and subtitle overlays, ideal for interview videos, documentaries, or tutorial content.".to_string(),
                parameters: Parameters {
                    param_type: "object".to_string(),
                    properties: {
                        let mut props = HashMap::new();
                        props.insert("name_text".to_string(), PropertyDefinition {
                            prop_type: "string".to_string(),
                            description: "Primary lower-third text (e.g. person name or topic heading)".to_string(),
                            items: None,
                        });
                        props.insert("subtitle_text".to_string(), PropertyDefinition {
                            prop_type: "string".to_string(),
                            description: "Secondary text below the main line (e.g. job title or context)".to_string(),
                            items: None,
                        });
                        props.insert("style".to_string(), PropertyDefinition {
                            prop_type: "string".to_string(),
                            description: "Animation and colour style (e.g. 'modern', 'news', 'minimal', 'neon')".to_string(),
                            items: None,
                        });
                        props.insert("duration".to_string(), PropertyDefinition {
                            prop_type: "number".to_string(),
                            description: "Display duration in seconds (default: 5)".to_string(),
                            items: None,
                        });
                        props
                    },
                    required: vec!["name_text".to_string()],
                },
            },
            FunctionDeclaration {
                name: "blender_generate_latex".to_string(),
                description: "Generate a LaTeX/Manim mathematical equation animation clip. Creates animated mathematical expressions, derivations, and equations — ideal for educational math/science videos.".to_string(),
                parameters: Parameters {
                    param_type: "object".to_string(),
                    properties: {
                        let mut props = HashMap::new();
                        props.insert("latex_expression".to_string(), PropertyDefinition {
                            prop_type: "string".to_string(),
                            description: r#"LaTeX math expression, e.g. r"\frac{d}{dt}\int_a^b f(x,t)dx" or r"E = mc^2""#.to_string(),
                            items: None,
                        });
                        props.insert("animation_type".to_string(), PropertyDefinition {
                            prop_type: "string".to_string(),
                            description: "Animation style: 'appear' (default), 'morph', or 'step_by_step'".to_string(),
                            items: None,
                        });
                        props.insert("duration".to_string(), PropertyDefinition {
                            prop_type: "number".to_string(),
                            description: "Clip length in seconds (default: 8)".to_string(),
                            items: None,
                        });
                        props.insert("background_style".to_string(), PropertyDefinition {
                            prop_type: "string".to_string(),
                            description: "Background: 'dark' (default), 'light', or 'transparent'".to_string(),
                            items: None,
                        });
                        props.insert("include_narration".to_string(), PropertyDefinition {
                            prop_type: "boolean".to_string(),
                            description: "Optional. If true, generate narration audio and a narrated video variant for this math render".to_string(),
                            items: None,
                        });
                        props.insert("narration_text".to_string(), PropertyDefinition {
                            prop_type: "string".to_string(),
                            description: "Optional narration script to speak during the animation".to_string(),
                            items: None,
                        });
                        props.insert("narration_speaker".to_string(), PropertyDefinition {
                            prop_type: "string".to_string(),
                            description: "Optional VibeVoice speaker preset name, e.g. 'Emma' or 'Carter'".to_string(),
                            items: None,
                        });
                        props
                    },
                    required: vec!["latex_expression".to_string()],
                },
            },

            FunctionDeclaration {
                name: "blender_generate_ui_mockup".to_string(),
                description: "Generate a 3D device UI mockup animation (iPhone, MacBook, browser, iPad) showing a screenshot on the device screen. Ideal for app demos, product showcases, and SaaS videos.".to_string(),
                parameters: Parameters {
                    param_type: "object".to_string(),
                    properties: {
                        let mut props = HashMap::new();
                        props.insert("device".to_string(), PropertyDefinition {
                            prop_type: "string".to_string(),
                            description: "Device frame type: 'iPhone' | 'MacBook' | 'browser' | 'iPad'".to_string(),
                            items: None,
                        });
                        props.insert("animation".to_string(), PropertyDefinition {
                            prop_type: "string".to_string(),
                            description: "Animation style: 'static' (PNG), 'reveal' (default), 'scroll', or 'tilt'".to_string(),
                            items: None,
                        });
                        props.insert("duration".to_string(), PropertyDefinition {
                            prop_type: "number".to_string(),
                            description: "Clip length in seconds (default: 5)".to_string(),
                            items: None,
                        });
                        props.insert("screenshot_url".to_string(), PropertyDefinition {
                            prop_type: "string".to_string(),
                            description: "URL of the screenshot/image to display on the device screen".to_string(),
                            items: None,
                        });
                        props
                    },
                    required: vec!["device".to_string()],
                },
            },

            FunctionDeclaration {
                name: "blender_generate_animation".to_string(),
                description: "Generate ANY Manim animation from a natural language description using LLM code generation. Use this for kinetic typography, abstract motion graphics, step-by-step explanations, or any creative animation that doesn't fit the specific latex/chart/scene categories.".to_string(),
                parameters: Parameters {
                    param_type: "object".to_string(),
                    properties: {
                        let mut props = HashMap::new();
                        props.insert("description".to_string(), PropertyDefinition {
                            prop_type: "string".to_string(),
                            description: "Natural language description of what to animate, e.g. 'Show the word SALE growing from small to large with a rainbow colour sweep, then explode into confetti particles'".to_string(),
                            items: None,
                        });
                        props.insert("duration".to_string(), PropertyDefinition {
                            prop_type: "number".to_string(),
                            description: "Clip length in seconds (default: 10)".to_string(),
                            items: None,
                        });
                        props.insert("background".to_string(), PropertyDefinition {
                            prop_type: "string".to_string(),
                            description: "Background style: 'dark' (default), 'light', or 'transparent'".to_string(),
                            items: None,
                        });
                        props.insert("quality".to_string(), PropertyDefinition {
                            prop_type: "string".to_string(),
                            description: "Render quality: 'l' (480p fast), 'm' (720p, default), 'h' (1080p slow)".to_string(),
                            items: None,
                        });
                        props.insert("include_narration".to_string(), PropertyDefinition {
                            prop_type: "boolean".to_string(),
                            description: "Optional. If true, generate narration audio and a narrated video variant for this animation".to_string(),
                            items: None,
                        });
                        props.insert("narration_text".to_string(), PropertyDefinition {
                            prop_type: "string".to_string(),
                            description: "Optional narration script to speak during the animation".to_string(),
                            items: None,
                        });
                        props.insert("narration_speaker".to_string(), PropertyDefinition {
                            prop_type: "string".to_string(),
                            description: "Optional VibeVoice speaker preset name, e.g. 'Emma' or 'Carter'".to_string(),
                            items: None,
                        });
                        props
                    },
                    required: vec!["description".to_string()],
                },
            },

            FunctionDeclaration {
                name: "blender_generate_chart".to_string(),
                description: "Generate an animated data visualisation clip (bar chart, line chart, pie chart, animated counter, or scatter plot) using Manim. Returns a video URL.".to_string(),
                parameters: Parameters {
                    param_type: "object".to_string(),
                    properties: {
                        let mut props = HashMap::new();
                        props.insert("chart_type".to_string(), PropertyDefinition {
                            prop_type: "string".to_string(),
                            description: "Chart type: 'bar_chart' | 'line_chart' | 'pie_chart' | 'counter' | 'scatter'".to_string(),
                            items: None,
                        });
                        props.insert("title".to_string(), PropertyDefinition {
                            prop_type: "string".to_string(),
                            description: "Chart title text".to_string(),
                            items: None,
                        });
                        props.insert("data".to_string(), PropertyDefinition {
                            prop_type: "string".to_string(),
                            description: "JSON array of data values, e.g. '[42, 78, 55, 90]' or for pie chart '[{\"label\":\"A\",\"value\":30}]'".to_string(),
                            items: None,
                        });
                        props.insert("labels".to_string(), PropertyDefinition {
                            prop_type: "string".to_string(),
                            description: "JSON array of labels, e.g. '[\"Q1\",\"Q2\",\"Q3\",\"Q4\"]'".to_string(),
                            items: None,
                        });
                        props.insert("duration".to_string(), PropertyDefinition {
                            prop_type: "number".to_string(),
                            description: "Clip length in seconds (default: 10)".to_string(),
                            items: None,
                        });
                        props.insert("colors".to_string(), PropertyDefinition {
                            prop_type: "string".to_string(),
                            description: "JSON array of Manim colour names, e.g. '[\"BLUE\",\"GREEN\",\"RED\"]'".to_string(),
                            items: None,
                        });
                        props
                    },
                    required: vec!["chart_type".to_string(), "title".to_string(), "data".to_string()],
                },
            },

            FunctionDeclaration {
                name: "blender_generate_flowchart".to_string(),
                description: "Generate an animated Manim flowchart with process boxes, decision diamonds, and arrows. Use for process diagrams, system architecture flows, and explainer videos.".to_string(),
                parameters: Parameters {
                    param_type: "object".to_string(),
                    properties: {
                        let mut props = HashMap::new();
                        props.insert("nodes".to_string(), PropertyDefinition { prop_type: "string".to_string(), description: "JSON array of nodes: [{\"id\":\"start\",\"label\":\"Start\",\"type\":\"start\"},{\"id\":\"step1\",\"label\":\"Process Data\",\"type\":\"process\"},{\"id\":\"decide\",\"label\":\"Valid?\",\"type\":\"decision\"},...] type: start|process|decision|end".to_string(), items: None });
                        props.insert("edges".to_string(), PropertyDefinition { prop_type: "string".to_string(), description: "JSON array of connections: [{\"from\":\"start\",\"to\":\"step1\"},{\"from\":\"decide\",\"to\":\"step2\",\"label\":\"Yes\"},...]".to_string(), items: None });
                        props.insert("title".to_string(), PropertyDefinition { prop_type: "string".to_string(), description: "Chart heading text".to_string(), items: None });
                        props.insert("duration".to_string(), PropertyDefinition { prop_type: "number".to_string(), description: "Clip length in seconds (default: 12)".to_string(), items: None });
                        props.insert("style".to_string(), PropertyDefinition { prop_type: "string".to_string(), description: "Visual style: 'dark' (default) | 'light' | 'blue'".to_string(), items: None });
                        props
                    },
                    required: vec!["nodes".to_string(), "edges".to_string()],
                },
            },

            FunctionDeclaration {
                name: "blender_generate_3d_math".to_string(),
                description: "Generate a 3D mathematics animation using Manim's ThreeDScene — ideal for academic content, math tutorials, and STEM explainer videos.".to_string(),
                parameters: Parameters {
                    param_type: "object".to_string(),
                    properties: {
                        let mut props = HashMap::new();
                        props.insert("scene_type".to_string(), PropertyDefinition { prop_type: "string".to_string(), description: "'surface' (3D function surface) | 'curve' (parametric helix) | 'vector_field' (2D arrow field) | 'torus' (spinning torus)".to_string(), items: None });
                        props.insert("title".to_string(), PropertyDefinition { prop_type: "string".to_string(), description: "Title text displayed on screen".to_string(), items: None });
                        props.insert("function".to_string(), PropertyDefinition { prop_type: "string".to_string(), description: "For scene_type=surface: 'wave' | 'sin' | 'cos' | 'saddle' | 'paraboloid' | 'ripple'".to_string(), items: None });
                        props.insert("duration".to_string(), PropertyDefinition { prop_type: "number".to_string(), description: "Clip length in seconds (default: 12)".to_string(), items: None });
                        props.insert("color".to_string(), PropertyDefinition { prop_type: "string".to_string(), description: "Surface color: 'BLUE' | 'RED' | 'GREEN' | 'GOLD' | 'PURPLE' | 'TEAL'".to_string(), items: None });
                        props
                    },
                    required: vec![],
                },
            },

            FunctionDeclaration {
                name: "blender_generate_code_animation".to_string(),
                description: "Generate an animated code syntax-highlighting clip — ideal for tech tutorials, YouTube programming content, and developer explainer videos.".to_string(),
                parameters: Parameters {
                    param_type: "object".to_string(),
                    properties: {
                        let mut props = HashMap::new();
                        props.insert("code".to_string(), PropertyDefinition { prop_type: "string".to_string(), description: "Source code string to display and animate".to_string(), items: None });
                        props.insert("language".to_string(), PropertyDefinition { prop_type: "string".to_string(), description: "Syntax language: 'python' | 'javascript' | 'rust' | 'cpp' | 'java' | 'bash' | 'sql' | 'typescript' | 'go'".to_string(), items: None });
                        props.insert("title".to_string(), PropertyDefinition { prop_type: "string".to_string(), description: "Heading shown above the code block".to_string(), items: None });
                        props.insert("highlight_lines".to_string(), PropertyDefinition { prop_type: "string".to_string(), description: "JSON array of 1-indexed line numbers to highlight, e.g. '[3,7,11]'".to_string(), items: None });
                        props.insert("reveal_mode".to_string(), PropertyDefinition { prop_type: "string".to_string(), description: "'line_by_line' (default) | 'all_at_once' | 'block'".to_string(), items: None });
                        props.insert("duration".to_string(), PropertyDefinition { prop_type: "number".to_string(), description: "Clip length in seconds (default: 12)".to_string(), items: None });
                        props.insert("style".to_string(), PropertyDefinition { prop_type: "string".to_string(), description: "Syntax theme: 'monokai' | 'dracula' | 'solarized-dark'".to_string(), items: None });
                        props
                    },
                    required: vec!["code".to_string()],
                },
            },

            FunctionDeclaration {
                name: "blender_generate_timeline".to_string(),
                description: "Generate an animated timeline, project roadmap, or Gantt-style clip — great for business explainers, project demos, and history videos.".to_string(),
                parameters: Parameters {
                    param_type: "object".to_string(),
                    properties: {
                        let mut props = HashMap::new();
                        props.insert("events".to_string(), PropertyDefinition { prop_type: "string".to_string(), description: "JSON array: [{\"date\":\"Jan\",\"label\":\"Project Kickoff\",\"color\":\"BLUE\"},{\"date\":\"Mar\",\"label\":\"MVP Launch\",\"color\":\"GREEN\"},...]".to_string(), items: None });
                        props.insert("title".to_string(), PropertyDefinition { prop_type: "string".to_string(), description: "Heading text".to_string(), items: None });
                        props.insert("duration".to_string(), PropertyDefinition { prop_type: "number".to_string(), description: "Clip length in seconds (default: 12)".to_string(), items: None });
                        props.insert("style".to_string(), PropertyDefinition { prop_type: "string".to_string(), description: "'dark' (default) | 'light' | 'gradient'".to_string(), items: None });
                        props.insert("orientation".to_string(), PropertyDefinition { prop_type: "string".to_string(), description: "'horizontal' (default) | 'vertical'".to_string(), items: None });
                        props
                    },
                    required: vec!["events".to_string()],
                },
            },

            FunctionDeclaration {
                name: "blender_generate_network_graph".to_string(),
                description: "Generate an animated network or knowledge graph — great for visualizing relationships, org charts, concept maps, and AI/ML topic maps.".to_string(),
                parameters: Parameters {
                    param_type: "object".to_string(),
                    properties: {
                        let mut props = HashMap::new();
                        props.insert("nodes".to_string(), PropertyDefinition { prop_type: "string".to_string(), description: "JSON array: [{\"id\":\"A\",\"label\":\"Machine Learning\",\"color\":\"BLUE\"},{\"id\":\"B\",\"label\":\"Deep Learning\",\"color\":\"GREEN\"},...]".to_string(), items: None });
                        props.insert("edges".to_string(), PropertyDefinition { prop_type: "string".to_string(), description: "JSON array: [{\"from\":\"A\",\"to\":\"B\"},{\"from\":\"A\",\"to\":\"C\",\"label\":\"includes\",\"directed\":true},...]".to_string(), items: None });
                        props.insert("title".to_string(), PropertyDefinition { prop_type: "string".to_string(), description: "Heading text".to_string(), items: None });
                        props.insert("layout".to_string(), PropertyDefinition { prop_type: "string".to_string(), description: "'radial' (hub-and-spoke, default) | 'circular' | 'spring'".to_string(), items: None });
                        props.insert("duration".to_string(), PropertyDefinition { prop_type: "number".to_string(), description: "Clip length in seconds (default: 12)".to_string(), items: None });
                        props.insert("style".to_string(), PropertyDefinition { prop_type: "string".to_string(), description: "'dark' | 'neon'".to_string(), items: None });
                        props
                    },
                    required: vec!["nodes".to_string(), "edges".to_string()],
                },
            },

            FunctionDeclaration {
                name: "blender_generate_logo_reveal".to_string(),
                description: "Generate a 3D extruded text / logo reveal animation in Blender — the most popular Fiverr motion graphics request.".to_string(),
                parameters: Parameters {
                    param_type: "object".to_string(),
                    properties: {
                        let mut props = HashMap::new();
                        props.insert("text".to_string(), PropertyDefinition { prop_type: "string".to_string(), description: "Brand name or main text to extrude and animate".to_string(), items: None });
                        props.insert("tagline".to_string(), PropertyDefinition { prop_type: "string".to_string(), description: "Optional secondary line (slogan / subtitle)".to_string(), items: None });
                        props.insert("style".to_string(), PropertyDefinition { prop_type: "string".to_string(), description: "'extrude_reveal' (default, Z-scale grow-in) | 'zoom_in' | 'split' | 'typewriter'".to_string(), items: None });
                        props.insert("color".to_string(), PropertyDefinition { prop_type: "string".to_string(), description: "JSON RGBA float array for text material, e.g. '[0.1, 0.5, 1.0, 1.0]'".to_string(), items: None });
                        props.insert("bg_color".to_string(), PropertyDefinition { prop_type: "string".to_string(), description: "JSON RGBA float array for background, e.g. '[0.02, 0.02, 0.05, 1.0]'".to_string(), items: None });
                        props.insert("duration".to_string(), PropertyDefinition { prop_type: "number".to_string(), description: "Clip length in seconds (default: 6)".to_string(), items: None });
                        props
                    },
                    required: vec!["text".to_string()],
                },
            },

            FunctionDeclaration {
                name: "blender_generate_abstract_bg".to_string(),
                description: "Generate an animated abstract background loop in Blender — useful as a video backdrop, intro overlay, or stock footage asset.".to_string(),
                parameters: Parameters {
                    param_type: "object".to_string(),
                    properties: {
                        let mut props = HashMap::new();
                        props.insert("style".to_string(), PropertyDefinition { prop_type: "string".to_string(), description: "'geometric' (orbiting shapes, default) | 'waves' | 'particles' | 'grid' (retro neon wireframe) | 'gradient'".to_string(), items: None });
                        props.insert("primary_color".to_string(), PropertyDefinition { prop_type: "string".to_string(), description: "JSON RGBA float array, e.g. '[0.05, 0.2, 0.8, 1.0]'".to_string(), items: None });
                        props.insert("secondary_color".to_string(), PropertyDefinition { prop_type: "string".to_string(), description: "JSON RGBA float array, e.g. '[0.8, 0.1, 0.5, 1.0]'".to_string(), items: None });
                        props.insert("duration".to_string(), PropertyDefinition { prop_type: "number".to_string(), description: "Clip length in seconds (default: 8)".to_string(), items: None });
                        props
                    },
                    required: vec![],
                },
            },

            FunctionDeclaration {
                name: "blender_generate_countdown".to_string(),
                description: "Generate a 3D animated countdown timer in Blender — useful for YouTube intros, live-stream countdowns, and event teasers.".to_string(),
                parameters: Parameters {
                    param_type: "object".to_string(),
                    properties: {
                        let mut props = HashMap::new();
                        props.insert("start_number".to_string(), PropertyDefinition { prop_type: "number".to_string(), description: "Count from this number (e.g. 10, 5, 3)".to_string(), items: None });
                        props.insert("end_number".to_string(), PropertyDefinition { prop_type: "number".to_string(), description: "Count to this number (e.g. 1 or 0)".to_string(), items: None });
                        props.insert("style".to_string(), PropertyDefinition { prop_type: "string".to_string(), description: "'bold' (default) | 'neon' | 'minimal' | 'cinematic'".to_string(), items: None });
                        props.insert("color".to_string(), PropertyDefinition { prop_type: "string".to_string(), description: "JSON RGBA float array for number material".to_string(), items: None });
                        props.insert("show_ring".to_string(), PropertyDefinition { prop_type: "string".to_string(), description: "'true' or 'false' — animated ring around number".to_string(), items: None });
                        props.insert("duration".to_string(), PropertyDefinition { prop_type: "number".to_string(), description: "Total clip duration in seconds (0 = 1s per count)".to_string(), items: None });
                        props
                    },
                    required: vec![],
                },
            },

            FunctionDeclaration {
                name: "blender_generate_particle_confetti".to_string(),
                description: "Generate an animated particle burst in Blender — confetti, snow, stars, rain, or bubbles. Great for celebration intros, event teasers, and festive video overlays.".to_string(),
                parameters: Parameters {
                    param_type: "object".to_string(),
                    properties: {
                        let mut props = HashMap::new();
                        props.insert("style".to_string(), PropertyDefinition { prop_type: "string".to_string(), description: "'confetti' | 'snow' | 'stars' | 'rain' | 'bubbles'".to_string(), items: None });
                        props.insert("count".to_string(), PropertyDefinition { prop_type: "number".to_string(), description: "Number of particles (default: 400)".to_string(), items: None });
                        props.insert("duration".to_string(), PropertyDefinition { prop_type: "number".to_string(), description: "Clip length in seconds (default: 6)".to_string(), items: None });
                        props.insert("primary_color".to_string(), PropertyDefinition { prop_type: "string".to_string(), description: "JSON RGBA float array e.g. '[1,0.3,0.1,1]'".to_string(), items: None });
                        props.insert("secondary_color".to_string(), PropertyDefinition { prop_type: "string".to_string(), description: "JSON RGBA float array for second color".to_string(), items: None });
                        props.insert("bg_color".to_string(), PropertyDefinition { prop_type: "string".to_string(), description: "JSON RGBA float array for background".to_string(), items: None });
                        props
                    },
                    required: vec![],
                },
            },

            FunctionDeclaration {
                name: "blender_generate_rigid_body_drop".to_string(),
                description: "Generate a physics rigid-body drop animation in Blender — 3D extruded letters or geometric objects fall and collide with realistic physics. Extremely popular for logo reveals and kinetic title cards.".to_string(),
                parameters: Parameters {
                    param_type: "object".to_string(),
                    properties: {
                        let mut props = HashMap::new();
                        props.insert("text".to_string(), PropertyDefinition { prop_type: "string".to_string(), description: "Text to extrude as falling 3D letters (when object_type='text')".to_string(), items: None });
                        props.insert("object_type".to_string(), PropertyDefinition { prop_type: "string".to_string(), description: "'text' | 'spheres' | 'cubes' | 'mixed'".to_string(), items: None });
                        props.insert("count".to_string(), PropertyDefinition { prop_type: "number".to_string(), description: "Number of objects if not text (default: 12)".to_string(), items: None });
                        props.insert("color".to_string(), PropertyDefinition { prop_type: "string".to_string(), description: "JSON RGBA float array".to_string(), items: None });
                        props.insert("bg_color".to_string(), PropertyDefinition { prop_type: "string".to_string(), description: "JSON RGBA float array for background".to_string(), items: None });
                        props.insert("style".to_string(), PropertyDefinition { prop_type: "string".to_string(), description: "'dark' | 'bright' | 'neon'".to_string(), items: None });
                        props.insert("duration".to_string(), PropertyDefinition { prop_type: "number".to_string(), description: "Clip length in seconds (default: 5)".to_string(), items: None });
                        props
                    },
                    required: vec![],
                },
            },

            FunctionDeclaration {
                name: "blender_generate_camera_path".to_string(),
                description: "Generate a smooth camera fly-through or orbit animation in Blender — orbit, helix, arc, dolly zoom, or linear flythrough. Perfect for product showcases, real estate walkthroughs, and cinematic scene reveals.".to_string(),
                parameters: Parameters {
                    param_type: "object".to_string(),
                    properties: {
                        let mut props = HashMap::new();
                        props.insert("path_type".to_string(), PropertyDefinition { prop_type: "string".to_string(), description: "'orbit' | 'helix' | 'arc' | 'dolly_zoom' | 'flythrough'".to_string(), items: None });
                        props.insert("subject".to_string(), PropertyDefinition { prop_type: "string".to_string(), description: "'spheres' | 'cubes' | 'text' | 'abstract' | 'landscape'".to_string(), items: None });
                        props.insert("title".to_string(), PropertyDefinition { prop_type: "string".to_string(), description: "Optional 3D text placed in scene".to_string(), items: None });
                        props.insert("color".to_string(), PropertyDefinition { prop_type: "string".to_string(), description: "JSON RGBA float array for objects".to_string(), items: None });
                        props.insert("bg_color".to_string(), PropertyDefinition { prop_type: "string".to_string(), description: "JSON RGBA float array for background".to_string(), items: None });
                        props.insert("style".to_string(), PropertyDefinition { prop_type: "string".to_string(), description: "'cinematic' | 'minimal' | 'neon'".to_string(), items: None });
                        props.insert("duration".to_string(), PropertyDefinition { prop_type: "number".to_string(), description: "Clip length in seconds (default: 8)".to_string(), items: None });
                        props
                    },
                    required: vec![],
                },
            },

            FunctionDeclaration {
                name: "blender_generate_toon_scene".to_string(),
                description: "Generate an NPR cartoon / toon-shaded Blender scene with bold outlines and flat colours — great for animated explainers, children's content, stylised brand videos, and cartoon-style intros.".to_string(),
                parameters: Parameters {
                    param_type: "object".to_string(),
                    properties: {
                        let mut props = HashMap::new();
                        props.insert("subject".to_string(), PropertyDefinition { prop_type: "string".to_string(), description: "'characters' | 'robots' | 'landscape' | 'abstract' | 'logo'".to_string(), items: None });
                        props.insert("title".to_string(), PropertyDefinition { prop_type: "string".to_string(), description: "Optional text label in scene".to_string(), items: None });
                        props.insert("outline_color".to_string(), PropertyDefinition { prop_type: "string".to_string(), description: "JSON RGBA float array for outlines".to_string(), items: None });
                        props.insert("primary_color".to_string(), PropertyDefinition { prop_type: "string".to_string(), description: "JSON RGBA float array for main objects".to_string(), items: None });
                        props.insert("bg_color".to_string(), PropertyDefinition { prop_type: "string".to_string(), description: "JSON RGBA float array for background".to_string(), items: None });
                        props.insert("outline_width".to_string(), PropertyDefinition { prop_type: "number".to_string(), description: "Outline thickness 0.5-5.0 (default: 1.5)".to_string(), items: None });
                        props.insert("flat_shading".to_string(), PropertyDefinition { prop_type: "boolean".to_string(), description: "true for pure cartoon flat look".to_string(), items: None });
                        props.insert("duration".to_string(), PropertyDefinition { prop_type: "number".to_string(), description: "Clip length in seconds (default: 6)".to_string(), items: None });
                        props
                    },
                    required: vec![],
                },
            },

            FunctionDeclaration {
                name: "blender_generate_grease_pencil_reveal".to_string(),
                description: "Generate a whiteboard / sketch draw-on text reveal using Blender Grease Pencil — letters appear stroke-by-stroke with BUILD modifier. Perfect for explainer videos, educational content, and whiteboard-style animations.".to_string(),
                parameters: Parameters {
                    param_type: "object".to_string(),
                    properties: {
                        let mut props = HashMap::new();
                        props.insert("text".to_string(), PropertyDefinition { prop_type: "string".to_string(), description: "Text to draw (max 12 characters)".to_string(), items: None });
                        props.insert("style".to_string(), PropertyDefinition { prop_type: "string".to_string(), description: "'whiteboard' | 'neon' | 'sketch' | 'chalkboard'".to_string(), items: None });
                        props.insert("color".to_string(), PropertyDefinition { prop_type: "string".to_string(), description: "JSON RGBA float array for strokes".to_string(), items: None });
                        props.insert("bg_color".to_string(), PropertyDefinition { prop_type: "string".to_string(), description: "JSON RGBA float array for background".to_string(), items: None });
                        props.insert("stroke_width".to_string(), PropertyDefinition { prop_type: "number".to_string(), description: "Line thickness 10-200 (default: 50)".to_string(), items: None });
                        props.insert("duration".to_string(), PropertyDefinition { prop_type: "number".to_string(), description: "Clip length in seconds (default: 6)".to_string(), items: None });
                        props
                    },
                    required: vec!["text".to_string()],
                },
            },

            FunctionDeclaration {
                name: "blender_generate_geometry_scatter".to_string(),
                description: "Generate a procedural instance-scatter animation in Blender — objects distributed across a plane, sphere, torus, or grid with animated wave displacement. Great for particle field backgrounds, product showcases, and abstract motion graphics.".to_string(),
                parameters: Parameters {
                    param_type: "object".to_string(),
                    properties: {
                        let mut props = HashMap::new();
                        props.insert("instance_type".to_string(), PropertyDefinition { prop_type: "string".to_string(), description: "'cubes' | 'spheres' | 'stars' | 'arrows' | 'crystals'".to_string(), items: None });
                        props.insert("surface".to_string(), PropertyDefinition { prop_type: "string".to_string(), description: "'plane' | 'sphere' | 'torus' | 'grid'".to_string(), items: None });
                        props.insert("count".to_string(), PropertyDefinition { prop_type: "number".to_string(), description: "Number of instances (default: 200)".to_string(), items: None });
                        props.insert("primary_color".to_string(), PropertyDefinition { prop_type: "string".to_string(), description: "JSON RGBA float array".to_string(), items: None });
                        props.insert("secondary_color".to_string(), PropertyDefinition { prop_type: "string".to_string(), description: "JSON RGBA float array for second color variant".to_string(), items: None });
                        props.insert("bg_color".to_string(), PropertyDefinition { prop_type: "string".to_string(), description: "JSON RGBA float array for background".to_string(), items: None });
                        props.insert("animated".to_string(), PropertyDefinition { prop_type: "boolean".to_string(), description: "true for wave displacement animation".to_string(), items: None });
                        props.insert("scale".to_string(), PropertyDefinition { prop_type: "number".to_string(), description: "Instance scale multiplier (default: 1.0)".to_string(), items: None });
                        props.insert("duration".to_string(), PropertyDefinition { prop_type: "number".to_string(), description: "Clip length in seconds (default: 8)".to_string(), items: None });
                        props
                    },
                    required: vec![],
                },
            },

            FunctionDeclaration {
                name: "blender_generate_text_animation".to_string(),
                description: "Generate kinetic typography / text animation using Manim. Great for YouTube intros, social media reels, brand reveals, and title sequences.".to_string(),
                parameters: Parameters {
                    param_type: "object".to_string(),
                    properties: {
                        let mut props = HashMap::new();
                        props.insert("text".to_string(), PropertyDefinition { prop_type: "string".to_string(), description: "Main text to animate".to_string(), items: None });
                        props.insert("subtitle".to_string(), PropertyDefinition { prop_type: "string".to_string(), description: "Optional smaller second line".to_string(), items: None });
                        props.insert("mode".to_string(), PropertyDefinition { prop_type: "string".to_string(), description: "'letter_by_letter' | 'word_by_word' | 'typewriter' | 'wave' | 'zoom_burst' | 'spin_in' | 'color_cycle' | 'highlight_words'".to_string(), items: None });
                        props.insert("color".to_string(), PropertyDefinition { prop_type: "string".to_string(), description: "Text colour name: 'WHITE' | 'BLUE' | 'RED' | 'GREEN' | 'GOLD' | 'YELLOW' | 'ORANGE'".to_string(), items: None });
                        props.insert("bg_color".to_string(), PropertyDefinition { prop_type: "string".to_string(), description: "'dark' | 'light'".to_string(), items: None });
                        props.insert("duration".to_string(), PropertyDefinition { prop_type: "number".to_string(), description: "Clip length in seconds (default: 8)".to_string(), items: None });
                        props.insert("font_size".to_string(), PropertyDefinition { prop_type: "number".to_string(), description: "Font size in points (default: 72)".to_string(), items: None });
                        props.insert("words_to_highlight".to_string(), PropertyDefinition { prop_type: "string".to_string(), description: "JSON array of words to highlight, e.g. '[\"SALE\",\"NOW\"]' (for highlight_words mode)".to_string(), items: None });
                        props
                    },
                    required: vec!["text".to_string()],
                },
            },

            FunctionDeclaration {
                name: "blender_generate_vector_field".to_string(),
                description: "Generate an animated vector field / flow visualization using Manim — ideal for physics, fluid dynamics, and math education content.".to_string(),
                parameters: Parameters {
                    param_type: "object".to_string(),
                    properties: {
                        let mut props = HashMap::new();
                        props.insert("field_type".to_string(), PropertyDefinition { prop_type: "string".to_string(), description: "'rotation' | 'radial' | 'sink' | 'saddle' | 'curl' | 'gravity'".to_string(), items: None });
                        props.insert("title".to_string(), PropertyDefinition { prop_type: "string".to_string(), description: "Heading text".to_string(), items: None });
                        props.insert("duration".to_string(), PropertyDefinition { prop_type: "number".to_string(), description: "Clip length in seconds (default: 12)".to_string(), items: None });
                        props.insert("show_streams".to_string(), PropertyDefinition { prop_type: "string".to_string(), description: "'true' (default) — render StreamLines on top of arrows".to_string(), items: None });
                        props.insert("color".to_string(), PropertyDefinition { prop_type: "string".to_string(), description: "'BLUE' | 'RED' | 'GREEN' | 'TEAL' | 'ORANGE'".to_string(), items: None });
                        props.insert("style".to_string(), PropertyDefinition { prop_type: "string".to_string(), description: "'dark' | 'grid'".to_string(), items: None });
                        props
                    },
                    required: vec![],
                },
            },

            FunctionDeclaration {
                name: "blender_generate_matrix_transform".to_string(),
                description: "Generate a linear algebra matrix transformation animation using Manim's LinearTransformationScene — ideal for math/STEM education content.".to_string(),
                parameters: Parameters {
                    param_type: "object".to_string(),
                    properties: {
                        let mut props = HashMap::new();
                        props.insert("matrix".to_string(), PropertyDefinition { prop_type: "string".to_string(), description: "JSON 2×2 matrix e.g. '[[0,-1],[1,0]]' (rotation), '[[2,0],[0,2]]' (scaling), '[[1,1],[0,1]]' (shear)".to_string(), items: None });
                        props.insert("title".to_string(), PropertyDefinition { prop_type: "string".to_string(), description: "Heading text".to_string(), items: None });
                        props.insert("duration".to_string(), PropertyDefinition { prop_type: "number".to_string(), description: "Clip length in seconds (default: 12)".to_string(), items: None });
                        props.insert("show_vectors".to_string(), PropertyDefinition { prop_type: "string".to_string(), description: "'true' (default) — show sample vectors being transformed".to_string(), items: None });
                        props.insert("show_det".to_string(), PropertyDefinition { prop_type: "string".to_string(), description: "'true' (default) — show determinant annotation".to_string(), items: None });
                        props
                    },
                    required: vec![],
                },
            },

            FunctionDeclaration {
                name: "blender_generate_polar_graph".to_string(),
                description: "Generate a polar coordinate / complex plane / function graph animation using Manim — great for advanced math and STEM content.".to_string(),
                parameters: Parameters {
                    param_type: "object".to_string(),
                    properties: {
                        let mut props = HashMap::new();
                        props.insert("plane_type".to_string(), PropertyDefinition { prop_type: "string".to_string(), description: "'polar' | 'complex' | 'number_plane'".to_string(), items: None });
                        props.insert("title".to_string(), PropertyDefinition { prop_type: "string".to_string(), description: "Heading text".to_string(), items: None });
                        props.insert("function".to_string(), PropertyDefinition { prop_type: "string".to_string(), description: "For polar: 'rose' | 'lemniscate' | 'spiral' | 'cardioid' | 'circle'. For number_plane: 'sin' | 'parabola'".to_string(), items: None });
                        props.insert("k_value".to_string(), PropertyDefinition { prop_type: "number".to_string(), description: "Number of petals for rose function (default: 4)".to_string(), items: None });
                        props.insert("duration".to_string(), PropertyDefinition { prop_type: "number".to_string(), description: "Clip length in seconds (default: 12)".to_string(), items: None });
                        props.insert("color".to_string(), PropertyDefinition { prop_type: "string".to_string(), description: "'BLUE' | 'RED' | 'GREEN' | 'PURPLE' | 'GOLD'".to_string(), items: None });
                        props
                    },
                    required: vec![],
                },
            },

            FunctionDeclaration {
                name: "blender_generate_geometry_proof".to_string(),
                description: "Generate an animated geometry proof using Manim — ideal for math tutors, STEM channels, and educational content.".to_string(),
                parameters: Parameters {
                    param_type: "object".to_string(),
                    properties: {
                        let mut props = HashMap::new();
                        props.insert("proof_type".to_string(), PropertyDefinition { prop_type: "string".to_string(), description: "'pythagorean' | 'circle_area' | 'triangle_sum' | 'boolean_ops'".to_string(), items: None });
                        props.insert("title".to_string(), PropertyDefinition { prop_type: "string".to_string(), description: "Heading text".to_string(), items: None });
                        props.insert("duration".to_string(), PropertyDefinition { prop_type: "number".to_string(), description: "Clip length in seconds (default: 14)".to_string(), items: None });
                        props.insert("color_a".to_string(), PropertyDefinition { prop_type: "string".to_string(), description: "Primary shape colour: 'BLUE' | 'RED' | 'GREEN' | 'GOLD'".to_string(), items: None });
                        props.insert("color_b".to_string(), PropertyDefinition { prop_type: "string".to_string(), description: "Secondary shape colour".to_string(), items: None });
                        props.insert("show_labels".to_string(), PropertyDefinition { prop_type: "string".to_string(), description: "'true' (default) — show formula/angle labels".to_string(), items: None });
                        props
                    },
                    required: vec![],
                },
            },

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
                name: "detect_scene_changes".to_string(),
                description: "Detects scene changes in a video and returns timestamps of each cut".to_string(),
                parameters: Parameters {
                    param_type: "object".to_string(),
                    properties: {
                        let mut props = HashMap::new();
                        props.insert("input_file".to_string(), PropertyDefinition { prop_type: "string".to_string(), description: "Path to the input video file".to_string(), items: None });
                        props.insert("threshold".to_string(), PropertyDefinition { prop_type: "number".to_string(), description: "Detection threshold 0–100 (default 40)".to_string(), items: None });
                        props
                    },
                    required: vec!["input_file".to_string()],
                },
            },
            FunctionDeclaration {
                name: "measure_loudness".to_string(),
                description: "Measures the audio loudness (mean and max volume) of a media file".to_string(),
                parameters: Parameters {
                    param_type: "object".to_string(),
                    properties: {
                        let mut props = HashMap::new();
                        props.insert("input_file".to_string(), PropertyDefinition { prop_type: "string".to_string(), description: "Path to the video or audio file to analyze".to_string(), items: None });
                        props
                    },
                    required: vec!["input_file".to_string()],
                },
            },
            FunctionDeclaration {
                name: "detect_silence".to_string(),
                description: "Detects silent segments in audio/video and returns timestamps and durations".to_string(),
                parameters: Parameters {
                    param_type: "object".to_string(),
                    properties: {
                        let mut props = HashMap::new();
                        props.insert("input_file".to_string(), PropertyDefinition { prop_type: "string".to_string(), description: "Path to the video or audio file".to_string(), items: None });
                        props.insert("noise_tolerance_db".to_string(), PropertyDefinition { prop_type: "number".to_string(), description: "Noise floor in dB (default -60)".to_string(), items: None });
                        props.insert("min_duration_sec".to_string(), PropertyDefinition { prop_type: "number".to_string(), description: "Minimum silence duration in seconds (default 0.1)".to_string(), items: None });
                        props
                    },
                    required: vec!["input_file".to_string()],
                },
            },
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
                name: "compare_ssim".to_string(),
                description: "Computes SSIM between a reference and distorted video.".to_string(),
                parameters: Parameters {
                    param_type: "object".to_string(),
                    properties: {
                        let mut props = HashMap::new();
                        props.insert("reference_file".to_string(), PropertyDefinition { prop_type: "string".to_string(), description: "Path to the original/reference video".to_string(), items: None });
                        props.insert("distorted_file".to_string(), PropertyDefinition { prop_type: "string".to_string(), description: "Path to the processed/distorted video".to_string(), items: None });
                        props
                    },
                    required: vec!["reference_file".to_string(), "distorted_file".to_string()],
                },
            },
            FunctionDeclaration {
                name: "compare_psnr".to_string(),
                description: "Computes PSNR between a reference and distorted video.".to_string(),
                parameters: Parameters {
                    param_type: "object".to_string(),
                    properties: {
                        let mut props = HashMap::new();
                        props.insert("reference_file".to_string(), PropertyDefinition { prop_type: "string".to_string(), description: "Path to the original/reference video".to_string(), items: None });
                        props.insert("distorted_file".to_string(), PropertyDefinition { prop_type: "string".to_string(), description: "Path to the processed/distorted video".to_string(), items: None });
                        props
                    },
                    required: vec!["reference_file".to_string(), "distorted_file".to_string()],
                },
            },
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
            FunctionDeclaration {
                name: "apply_shear".to_string(),
                description: "Applies horizontal and/or vertical shear transformation using the FFmpeg shear filter.".to_string(),
                parameters: Parameters {
                    param_type: "object".to_string(),
                    properties: {
                        let mut props = HashMap::new();
                        props.insert("input_file".to_string(), PropertyDefinition { prop_type: "string".to_string(), description: "Path to the input video file".to_string(), items: None });
                        props.insert("output_file".to_string(), PropertyDefinition { prop_type: "string".to_string(), description: "Path to save the output video".to_string(), items: None });
                        props.insert("shear_x".to_string(), PropertyDefinition { prop_type: "number".to_string(), description: "Horizontal shear factor (-2.0 to 2.0)".to_string(), items: None });
                        props.insert("shear_y".to_string(), PropertyDefinition { prop_type: "number".to_string(), description: "Vertical shear factor (-2.0 to 2.0)".to_string(), items: None });
                        props.insert("fill_color".to_string(), PropertyDefinition { prop_type: "string".to_string(), description: "Fill color for empty areas (default 'black')".to_string(), items: None });
                        props.insert("interpolation".to_string(), PropertyDefinition { prop_type: "string".to_string(), description: "Interpolation: nearest/bilinear/bicubic (default bilinear)".to_string(), items: None });
                        props
                    },
                    required: vec!["input_file".to_string(), "output_file".to_string()],
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
                name: "apply_stereo_widen".to_string(),
                description: "Widens stereo image using Haas effect via the FFmpeg stereowiden filter.".to_string(),
                parameters: Parameters {
                    param_type: "object".to_string(),
                    properties: {
                        let mut props = HashMap::new();
                        props.insert("input_file".to_string(), PropertyDefinition { prop_type: "string".to_string(), description: "Path to the input file".to_string(), items: None });
                        props.insert("output_file".to_string(), PropertyDefinition { prop_type: "string".to_string(), description: "Path to save the output file".to_string(), items: None });
                        props.insert("delay_ms".to_string(), PropertyDefinition { prop_type: "number".to_string(), description: "Delay in milliseconds (1–100, default 20)".to_string(), items: None });
                        props.insert("feedback".to_string(), PropertyDefinition { prop_type: "number".to_string(), description: "Feedback amount (0–0.9, default 0.3)".to_string(), items: None });
                        props.insert("crossfeed".to_string(), PropertyDefinition { prop_type: "number".to_string(), description: "Crossfeed amount (0–1, default 0.3)".to_string(), items: None });
                        props.insert("drymix".to_string(), PropertyDefinition { prop_type: "number".to_string(), description: "Dry mix amount (0–1, default 0.8)".to_string(), items: None });
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
                name: "measure_lufs".to_string(),
                description: "Measures integrated loudness (LUFS), loudness range (LRA), and true peak using EBU R128. Analysis only — no output file produced. YouTube target: -14 LUFS; broadcast: -23 LUFS.".to_string(),
                parameters: Parameters {
                    param_type: "object".to_string(),
                    properties: {
                        let mut props = HashMap::new();
                        props.insert("input_file".to_string(), PropertyDefinition { prop_type: "string".to_string(), description: "Path to the audio or video file to analyse".to_string(), items: None });
                        props.insert("target_lufs".to_string(), PropertyDefinition { prop_type: "number".to_string(), description: "Reference target loudness (default -23 LUFS).".to_string(), items: None });
                        props
                    },
                    required: vec!["input_file".to_string()],
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
                name: "measure_vmaf".to_string(),
                description: "Measures VMAF perceptual quality score between distorted and reference video. Analysis only.".to_string(),
                parameters: Parameters {
                    param_type: "object".to_string(),
                    properties: {
                        let mut props = HashMap::new();
                        props.insert("distorted_file".to_string(), PropertyDefinition { prop_type: "string".to_string(), description: "Path to the distorted video to evaluate".to_string(), items: None });
                        props.insert("reference_file".to_string(), PropertyDefinition { prop_type: "string".to_string(), description: "Path to the reference/original video".to_string(), items: None });
                        props.insert("model_path".to_string(), PropertyDefinition { prop_type: "string".to_string(), description: "Optional VMAF model .json path. Empty = default.".to_string(), items: None });
                        props
                    },
                    required: vec!["distorted_file".to_string(), "reference_file".to_string()],
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
                name: "apply_audio_pulsator".to_string(),
                description: "Adds a rhythmic amplitude pulsation to stereo audio (apulsator filter).".to_string(),
                parameters: Parameters {
                    param_type: "object".to_string(),
                    properties: {
                        let mut props = HashMap::new();
                        props.insert("input_file".to_string(), PropertyDefinition { prop_type: "string".to_string(), description: "Path to the input audio/video file".to_string(), items: None });
                        props.insert("output_file".to_string(), PropertyDefinition { prop_type: "string".to_string(), description: "Path to save the output".to_string(), items: None });
                        props.insert("hz".to_string(), PropertyDefinition { prop_type: "number".to_string(), description: "Pulse rate Hz. Default 2.".to_string(), items: None });
                        props.insert("amount".to_string(), PropertyDefinition { prop_type: "number".to_string(), description: "Modulation depth (0–1). Default 1.".to_string(), items: None });
                        props.insert("offset_l".to_string(), PropertyDefinition { prop_type: "number".to_string(), description: "Left phase offset (0–1). Default 0.".to_string(), items: None });
                        props.insert("offset_r".to_string(), PropertyDefinition { prop_type: "number".to_string(), description: "Right phase offset (0–1). Default 0.5.".to_string(), items: None });
                        props.insert("mode".to_string(), PropertyDefinition { prop_type: "string".to_string(), description: "LFO: sine/triangle/square/sawup/sawdown. Default sine.".to_string(), items: None });
                        props
                    },
                    required: vec!["input_file".to_string(), "output_file".to_string()],
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
                name: "apply_crossfeed".to_string(),
                description: "Crossfeed processing for comfortable headphone listening — reduces stereo width and adds inter-aural crosstalk.".to_string(),
                parameters: Parameters {
                    param_type: "object".to_string(),
                    properties: {
                        let mut props = HashMap::new();
                        props.insert("input_file".to_string(), PropertyDefinition { prop_type: "string".to_string(), description: "Path to the input audio/video file".to_string(), items: None });
                        props.insert("output_file".to_string(), PropertyDefinition { prop_type: "string".to_string(), description: "Path to save the output".to_string(), items: None });
                        props.insert("strength".to_string(), PropertyDefinition { prop_type: "number".to_string(), description: "Strength (0–1). Default 0.5.".to_string(), items: None });
                        props.insert("slope".to_string(), PropertyDefinition { prop_type: "number".to_string(), description: "Slope (0.01–1.0). Default 0.5.".to_string(), items: None });
                        props.insert("level_in".to_string(), PropertyDefinition { prop_type: "number".to_string(), description: "Input gain. Default 1.".to_string(), items: None });
                        props.insert("level_out".to_string(), PropertyDefinition { prop_type: "number".to_string(), description: "Output gain. Default 1.".to_string(), items: None });
                        props
                    },
                    required: vec!["input_file".to_string(), "output_file".to_string()],
                },
            },

            FunctionDeclaration {
                name: "apply_extrastereo".to_string(),
                description: "Increases stereo separation beyond the original recording (extrastereo filter).".to_string(),
                parameters: Parameters {
                    param_type: "object".to_string(),
                    properties: {
                        let mut props = HashMap::new();
                        props.insert("input_file".to_string(), PropertyDefinition { prop_type: "string".to_string(), description: "Path to the input audio/video file".to_string(), items: None });
                        props.insert("output_file".to_string(), PropertyDefinition { prop_type: "string".to_string(), description: "Path to save the output".to_string(), items: None });
                        props.insert("multiplier".to_string(), PropertyDefinition { prop_type: "number".to_string(), description: "Separation multiplier (-10–10). Default 2.5.".to_string(), items: None });
                        props.insert("clipping".to_string(), PropertyDefinition { prop_type: "boolean".to_string(), description: "Enable clipping prevention. Default false.".to_string(), items: None });
                        props
                    },
                    required: vec!["input_file".to_string(), "output_file".to_string()],
                },
            },

            FunctionDeclaration {
                name: "apply_firequalizer".to_string(),
                description: "Linear-phase FIR equalizer with arbitrary frequency/gain entries (firequalizer).".to_string(),
                parameters: Parameters {
                    param_type: "object".to_string(),
                    properties: {
                        let mut props = HashMap::new();
                        props.insert("input_file".to_string(), PropertyDefinition { prop_type: "string".to_string(), description: "Path to the input audio/video file".to_string(), items: None });
                        props.insert("output_file".to_string(), PropertyDefinition { prop_type: "string".to_string(), description: "Path to save the output".to_string(), items: None });
                        props.insert("gain_entry".to_string(), PropertyDefinition { prop_type: "string".to_string(), description: "Semicolon-separated entries e.g. 'entry(0,0);entry(1000,-6);entry(4000,0)'.".to_string(), items: None });
                        props
                    },
                    required: vec!["input_file".to_string(), "output_file".to_string(), "gain_entry".to_string()],
                },
            },

            FunctionDeclaration {
                name: "apply_biquad".to_string(),
                description: "Biquad IIR filter with user-supplied coefficients. For advanced DSP filter design.".to_string(),
                parameters: Parameters {
                    param_type: "object".to_string(),
                    properties: {
                        let mut props = HashMap::new();
                        props.insert("input_file".to_string(), PropertyDefinition { prop_type: "string".to_string(), description: "Path to the input audio/video file".to_string(), items: None });
                        props.insert("output_file".to_string(), PropertyDefinition { prop_type: "string".to_string(), description: "Path to save the output".to_string(), items: None });
                        props.insert("b0".to_string(), PropertyDefinition { prop_type: "number".to_string(), description: "b0. Default 1.".to_string(), items: None });
                        props.insert("b1".to_string(), PropertyDefinition { prop_type: "number".to_string(), description: "b1. Default 0.".to_string(), items: None });
                        props.insert("b2".to_string(), PropertyDefinition { prop_type: "number".to_string(), description: "b2. Default 0.".to_string(), items: None });
                        props.insert("a0".to_string(), PropertyDefinition { prop_type: "number".to_string(), description: "a0. Default 1.".to_string(), items: None });
                        props.insert("a1".to_string(), PropertyDefinition { prop_type: "number".to_string(), description: "a1. Default 0.".to_string(), items: None });
                        props.insert("a2".to_string(), PropertyDefinition { prop_type: "number".to_string(), description: "a2. Default 0.".to_string(), items: None });
                        props
                    },
                    required: vec!["input_file".to_string(), "output_file".to_string()],
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
                name: "apply_setsar".to_string(),
                description: "Set sample aspect ratio (SAR/PAR) of video without re-encoding. Fix anamorphic footage or set broadcast display ratio.".to_string(),
                parameters: {
                    let mut p = HashMap::new();
                    p.insert("input_file".to_string(), PropertyDefinition { prop_type: "string".to_string(), description: "Path to input video file".to_string(), items: None });
                    p.insert("output_file".to_string(), PropertyDefinition { prop_type: "string".to_string(), description: "Path to output video file".to_string(), items: None });
                    p.insert("sar".to_string(), PropertyDefinition { prop_type: "string".to_string(), description: "Sample aspect ratio as fraction (default: 1/1). E.g. '16/15' for NTSC".to_string(), items: None });
                    Parameters { param_type: "object".to_string(), properties: p, required: vec!["input_file".to_string(), "output_file".to_string()] }
                },
            },
            FunctionDeclaration {
                name: "apply_random_frames".to_string(),
                description: "Randomly reorder frames using FFmpeg random filter. Creates glitch/scrambled visual effect.".to_string(),
                parameters: {
                    let mut p = HashMap::new();
                    p.insert("input_file".to_string(), PropertyDefinition { prop_type: "string".to_string(), description: "Path to input video file".to_string(), items: None });
                    p.insert("output_file".to_string(), PropertyDefinition { prop_type: "string".to_string(), description: "Path to output video file".to_string(), items: None });
                    p.insert("frames".to_string(), PropertyDefinition { prop_type: "number".to_string(), description: "Size of random window in frames (default 30)".to_string(), items: None });
                    p.insert("seed".to_string(), PropertyDefinition { prop_type: "number".to_string(), description: "Random seed (-1 for random, default -1)".to_string(), items: None });
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
                name: "apply_audio_iir".to_string(),
                description: "Apply custom IIR filter to audio using aiir. Design arbitrary digital filters by specifying zeros, poles, and gains.".to_string(),
                parameters: {
                    let mut p = HashMap::new();
                    p.insert("input_file".to_string(), PropertyDefinition { prop_type: "string".to_string(), description: "Path to input audio or video file".to_string(), items: None });
                    p.insert("output_file".to_string(), PropertyDefinition { prop_type: "string".to_string(), description: "Path to output file".to_string(), items: None });
                    p.insert("zeros".to_string(), PropertyDefinition { prop_type: "string".to_string(), description: "IIR filter zeros coefficients (default 1)".to_string(), items: None });
                    p.insert("poles".to_string(), PropertyDefinition { prop_type: "string".to_string(), description: "IIR filter poles coefficients (default 1)".to_string(), items: None });
                    p.insert("gains".to_string(), PropertyDefinition { prop_type: "string".to_string(), description: "IIR filter gain coefficients (default 1)".to_string(), items: None });
                    Parameters { param_type: "object".to_string(), properties: p, required: vec!["input_file".to_string(), "output_file".to_string()] }
                },
            },
            FunctionDeclaration {
                name: "apply_audio_expression".to_string(),
                description: "Apply per-sample audio expression using aeval. Transform audio with math expressions for ring modulation, distortion, bit manipulation.".to_string(),
                parameters: {
                    let mut p = HashMap::new();
                    p.insert("input_file".to_string(), PropertyDefinition { prop_type: "string".to_string(), description: "Path to input audio or video file".to_string(), items: None });
                    p.insert("output_file".to_string(), PropertyDefinition { prop_type: "string".to_string(), description: "Path to output file".to_string(), items: None });
                    p.insert("exprs".to_string(), PropertyDefinition { prop_type: "string".to_string(), description: "Per-sample expression. E.g. 'val*0.5' for -6dB, 'val*sin(2*PI*440*t)' for ring mod".to_string(), items: None });
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
                name: "apply_cross_correlate".to_string(),
                description: "Cross-correlate two audio streams using axcorrelate. Measure similarity, find time alignment, or mix correlated audio.".to_string(),
                parameters: {
                    let mut p = HashMap::new();
                    p.insert("input_file".to_string(), PropertyDefinition { prop_type: "string".to_string(), description: "Path to first input audio file".to_string(), items: None });
                    p.insert("secondary_file".to_string(), PropertyDefinition { prop_type: "string".to_string(), description: "Path to second input audio file".to_string(), items: None });
                    p.insert("output_file".to_string(), PropertyDefinition { prop_type: "string".to_string(), description: "Path to output file".to_string(), items: None });
                    p.insert("size".to_string(), PropertyDefinition { prop_type: "number".to_string(), description: "Segment size in samples (default 256)".to_string(), items: None });
                    p.insert("algo".to_string(), PropertyDefinition { prop_type: "string".to_string(), description: "Algorithm: slow or fast (default fast)".to_string(), items: None });
                    Parameters { param_type: "object".to_string(), properties: p, required: vec!["input_file".to_string(), "secondary_file".to_string(), "output_file".to_string()] }
                },
            },
            FunctionDeclaration {
                name: "apply_audio_multiply".to_string(),
                description: "Ring modulation — multiply two audio streams sample by sample using amultiply. Creates metallic, robotic, bell-like timbres.".to_string(),
                parameters: {
                    let mut p = HashMap::new();
                    p.insert("input_file".to_string(), PropertyDefinition { prop_type: "string".to_string(), description: "Path to first input audio file (carrier)".to_string(), items: None });
                    p.insert("secondary_file".to_string(), PropertyDefinition { prop_type: "string".to_string(), description: "Path to second input audio file (modulator)".to_string(), items: None });
                    p.insert("output_file".to_string(), PropertyDefinition { prop_type: "string".to_string(), description: "Path to output file".to_string(), items: None });
                    Parameters { param_type: "object".to_string(), properties: p, required: vec!["input_file".to_string(), "secondary_file".to_string(), "output_file".to_string()] }
                },
            },
            FunctionDeclaration {
                name: "apply_audio_contrast".to_string(),
                description: "Enhance audio contrast using acontrast filter. Increases perceived loudness and punch without clipping.".to_string(),
                parameters: {
                    let mut p = HashMap::new();
                    p.insert("input_file".to_string(), PropertyDefinition { prop_type: "string".to_string(), description: "Path to input audio or video file".to_string(), items: None });
                    p.insert("output_file".to_string(), PropertyDefinition { prop_type: "string".to_string(), description: "Path to output file".to_string(), items: None });
                    p.insert("contrast".to_string(), PropertyDefinition { prop_type: "number".to_string(), description: "Contrast level 0-100 (default 33)".to_string(), items: None });
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
                name: "apply_fieldorder".to_string(),
                description: "Changes field order of interlaced video using FFmpeg fieldorder. Converts between TFF and BFF to fix combing artifacts.".to_string(),
                parameters: {
                    let mut p = HashMap::new();
                    p.insert("input_file".to_string(), PropertyDefinition { prop_type: "string".to_string(), description: "Path to input interlaced video".to_string(), items: None });
                    p.insert("output_file".to_string(), PropertyDefinition { prop_type: "string".to_string(), description: "Path for output video".to_string(), items: None });
                    p.insert("order".to_string(), PropertyDefinition { prop_type: "string".to_string(), description: "Target field order: tff (default) or bff".to_string(), items: None });
                    Parameters { param_type: "object".to_string(), properties: p, required: vec!["input_file".to_string(), "output_file".to_string()] }
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
                name: "apply_freezeframes".to_string(),
                description: "Replaces a range of video frames with a freeze frame using FFmpeg freezeframes. Use to freeze a moment, create a pause effect, or replace damaged frames.".to_string(),
                parameters: {
                    let mut p = HashMap::new();
                    p.insert("input_file".to_string(), PropertyDefinition { prop_type: "string".to_string(), description: "Path to input video file".to_string(), items: None });
                    p.insert("output_file".to_string(), PropertyDefinition { prop_type: "string".to_string(), description: "Path for output video file".to_string(), items: None });
                    p.insert("first".to_string(), PropertyDefinition { prop_type: "number".to_string(), description: "First frame number to replace (0-indexed)".to_string(), items: None });
                    p.insert("last".to_string(), PropertyDefinition { prop_type: "number".to_string(), description: "Last frame number to replace (inclusive)".to_string(), items: None });
                    p.insert("replace".to_string(), PropertyDefinition { prop_type: "number".to_string(), description: "Frame number to use as freeze source (default: 0)".to_string(), items: None });
                    Parameters { param_type: "object".to_string(), properties: p, required: vec!["input_file".to_string(), "output_file".to_string(), "first".to_string(), "last".to_string()] }
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

            FunctionDeclaration {
                name: "measure_video_entropy".to_string(),
                description: "Measures frame entropy using FFmpeg entropy filter. Higher = more detail/complexity. Analysis only, no output file.".to_string(),
                parameters: {
                    let mut p = HashMap::new();
                    p.insert("input_file".to_string(), PropertyDefinition { prop_type: "string".to_string(), description: "Path to input video file".to_string(), items: None });
                    Parameters { param_type: "object".to_string(), properties: p, required: vec!["input_file".to_string()] }
                },
            },

            FunctionDeclaration {
                name: "apply_compensation_delay".to_string(),
                description: "Precise time-alignment delay using FFmpeg compensationdelay. Specified in physical distance (mm/cm/m) at a given temperature. Essential for microphone time alignment.".to_string(),
                parameters: {
                    let mut p = HashMap::new();
                    p.insert("input_file".to_string(), PropertyDefinition { prop_type: "string".to_string(), description: "Path to input audio/video file".to_string(), items: None });
                    p.insert("output_file".to_string(), PropertyDefinition { prop_type: "string".to_string(), description: "Path for output file".to_string(), items: None });
                    p.insert("mm".to_string(), PropertyDefinition { prop_type: "number".to_string(), description: "Delay distance millimetres (default: 0)".to_string(), items: None });
                    p.insert("cm".to_string(), PropertyDefinition { prop_type: "number".to_string(), description: "Delay distance centimetres (default: 0)".to_string(), items: None });
                    p.insert("m".to_string(), PropertyDefinition { prop_type: "number".to_string(), description: "Delay distance metres (default: 0)".to_string(), items: None });
                    p.insert("dry".to_string(), PropertyDefinition { prop_type: "number".to_string(), description: "Dry signal mix 0–1 (default: 0)".to_string(), items: None });
                    p.insert("wet".to_string(), PropertyDefinition { prop_type: "number".to_string(), description: "Wet signal mix 0–1 (default: 1)".to_string(), items: None });
                    p.insert("temp".to_string(), PropertyDefinition { prop_type: "number".to_string(), description: "Temperature Celsius for speed of sound (default: 20)".to_string(), items: None });
                    Parameters { param_type: "object".to_string(), properties: p, required: vec!["input_file".to_string(), "output_file".to_string()] }
                },
            },

            FunctionDeclaration {
                name: "apply_earwax".to_string(),
                description: "Applies earwax 3D audio enhancement using FFmpeg earwax for more immersive headphone listening.".to_string(),
                parameters: {
                    let mut p = HashMap::new();
                    p.insert("input_file".to_string(), PropertyDefinition { prop_type: "string".to_string(), description: "Path to input audio/video file".to_string(), items: None });
                    p.insert("output_file".to_string(), PropertyDefinition { prop_type: "string".to_string(), description: "Path for output file".to_string(), items: None });
                    Parameters { param_type: "object".to_string(), properties: p, required: vec!["input_file".to_string(), "output_file".to_string()] }
                },
            },

            FunctionDeclaration {
                name: "apply_allpass_filter".to_string(),
                description: "Two-pole all-pass filter using FFmpeg allpass. Passes all frequencies unchanged in amplitude but alters phase. Use for phase correction between microphones.".to_string(),
                parameters: {
                    let mut p = HashMap::new();
                    p.insert("input_file".to_string(), PropertyDefinition { prop_type: "string".to_string(), description: "Path to input audio/video file".to_string(), items: None });
                    p.insert("output_file".to_string(), PropertyDefinition { prop_type: "string".to_string(), description: "Path for output file".to_string(), items: None });
                    p.insert("frequency".to_string(), PropertyDefinition { prop_type: "number".to_string(), description: "Centre frequency Hz for maximum phase shift (default: 3000)".to_string(), items: None });
                    p.insert("width".to_string(), PropertyDefinition { prop_type: "number".to_string(), description: "Filter width (default: 0.707 for Q mode)".to_string(), items: None });
                    p.insert("width_type".to_string(), PropertyDefinition { prop_type: "string".to_string(), description: "Width type: q=Q (default), h=Hz, o=octaves, s=slope".to_string(), items: None });
                    p.insert("mix".to_string(), PropertyDefinition { prop_type: "number".to_string(), description: "Wet/dry mix 0–1 (default: 1.0)".to_string(), items: None });
                    Parameters { param_type: "object".to_string(), properties: p, required: vec!["input_file".to_string(), "output_file".to_string(), "frequency".to_string()] }
                },
            },

            FunctionDeclaration {
                name: "apply_highshelf".to_string(),
                description: "High-shelf EQ using FFmpeg highshelf. Boosts/cuts all frequencies above the shelf. Use for adding air (boost >8kHz) or rolling off harsh highs.".to_string(),
                parameters: {
                    let mut p = HashMap::new();
                    p.insert("input_file".to_string(), PropertyDefinition { prop_type: "string".to_string(), description: "Path to input audio/video file".to_string(), items: None });
                    p.insert("output_file".to_string(), PropertyDefinition { prop_type: "string".to_string(), description: "Path for output file".to_string(), items: None });
                    p.insert("frequency".to_string(), PropertyDefinition { prop_type: "number".to_string(), description: "Shelf frequency Hz (e.g. 8000, 12000)".to_string(), items: None });
                    p.insert("gain".to_string(), PropertyDefinition { prop_type: "number".to_string(), description: "Gain in dB (positive=boost, negative=cut)".to_string(), items: None });
                    p.insert("width".to_string(), PropertyDefinition { prop_type: "number".to_string(), description: "Shelf slope/width (default: 0.5)".to_string(), items: None });
                    p.insert("width_type".to_string(), PropertyDefinition { prop_type: "string".to_string(), description: "Width type: s=slope (default), h=Hz, q=Q, o=octaves".to_string(), items: None });
                    p.insert("poles".to_string(), PropertyDefinition { prop_type: "number".to_string(), description: "Filter poles 1 or 2 (default: 2)".to_string(), items: None });
                    Parameters { param_type: "object".to_string(), properties: p, required: vec!["input_file".to_string(), "output_file".to_string(), "frequency".to_string(), "gain".to_string()] }
                },
            },

            FunctionDeclaration {
                name: "apply_lowshelf".to_string(),
                description: "Low-shelf EQ using FFmpeg lowshelf. Boosts/cuts all frequencies below the shelf. Use for adding warmth (boost <200Hz) or removing rumble.".to_string(),
                parameters: {
                    let mut p = HashMap::new();
                    p.insert("input_file".to_string(), PropertyDefinition { prop_type: "string".to_string(), description: "Path to input audio/video file".to_string(), items: None });
                    p.insert("output_file".to_string(), PropertyDefinition { prop_type: "string".to_string(), description: "Path for output file".to_string(), items: None });
                    p.insert("frequency".to_string(), PropertyDefinition { prop_type: "number".to_string(), description: "Shelf frequency Hz (e.g. 80, 120, 200)".to_string(), items: None });
                    p.insert("gain".to_string(), PropertyDefinition { prop_type: "number".to_string(), description: "Gain in dB (positive=boost bass, negative=cut)".to_string(), items: None });
                    p.insert("width".to_string(), PropertyDefinition { prop_type: "number".to_string(), description: "Shelf slope/width (default: 0.5)".to_string(), items: None });
                    p.insert("width_type".to_string(), PropertyDefinition { prop_type: "string".to_string(), description: "Width type: s=slope (default), h=Hz, q=Q, o=octaves".to_string(), items: None });
                    p.insert("poles".to_string(), PropertyDefinition { prop_type: "number".to_string(), description: "Filter poles 1 or 2 (default: 2)".to_string(), items: None });
                    Parameters { param_type: "object".to_string(), properties: p, required: vec!["input_file".to_string(), "output_file".to_string(), "frequency".to_string(), "gain".to_string()] }
                },
            },

            FunctionDeclaration {
                name: "apply_surround_upmix".to_string(),
                description: "Upmixes stereo audio to 5.1 or 7.1 surround using FFmpeg surround filter.".to_string(),
                parameters: {
                    let mut p = HashMap::new();
                    p.insert("input_file".to_string(), PropertyDefinition { prop_type: "string".to_string(), description: "Path to input stereo audio/video".to_string(), items: None });
                    p.insert("output_file".to_string(), PropertyDefinition { prop_type: "string".to_string(), description: "Path for output surround file".to_string(), items: None });
                    p.insert("chl_out".to_string(), PropertyDefinition { prop_type: "string".to_string(), description: "Output channel layout (default: 5.1). Options: 5.1, 7.1, quadrature".to_string(), items: None });
                    p.insert("chl_in".to_string(), PropertyDefinition { prop_type: "string".to_string(), description: "Input channel layout (default: stereo)".to_string(), items: None });
                    p.insert("level_in".to_string(), PropertyDefinition { prop_type: "number".to_string(), description: "Input gain (default: 1.0)".to_string(), items: None });
                    p.insert("level_out".to_string(), PropertyDefinition { prop_type: "number".to_string(), description: "Output gain (default: 1.0)".to_string(), items: None });
                    Parameters { param_type: "object".to_string(), properties: p, required: vec!["input_file".to_string(), "output_file".to_string()] }
                },
            },

            FunctionDeclaration {
                name: "detect_volume_levels".to_string(),
                description: "Measures max and mean volume levels using FFmpeg volumedetect. Reports peak and RMS values. Analysis only — no output file.".to_string(),
                parameters: {
                    let mut p = HashMap::new();
                    p.insert("input_file".to_string(), PropertyDefinition { prop_type: "string".to_string(), description: "Path to input audio/video file".to_string(), items: None });
                    Parameters { param_type: "object".to_string(), properties: p, required: vec!["input_file".to_string()] }
                },
            },

            // ================================================================
            // PHASE I BATCH 8 — extract_alpha, merge_alpha, framestep, swaprect,
            //                   fillborders, chromanr, weave, interlace,
            //                   denoise_audio_fft, loop_audio, dc_shift, dynamic_range,
            //                   single_eq_band, stereotools, asetrate
            // ================================================================

            FunctionDeclaration {
                name: "extract_alpha_channel".to_string(),
                description: "Extracts the alpha (transparency) channel from a video as greyscale using FFmpeg alphaextract.".to_string(),
                parameters: {
                    let mut p = HashMap::new();
                    p.insert("input_file".to_string(), PropertyDefinition { prop_type: "string".to_string(), description: "Path to input video with alpha channel".to_string(), items: None });
                    p.insert("output_file".to_string(), PropertyDefinition { prop_type: "string".to_string(), description: "Path for output greyscale video/image".to_string(), items: None });
                    Parameters { param_type: "object".to_string(), properties: p, required: vec!["input_file".to_string(), "output_file".to_string()] }
                },
            },

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
                name: "apply_framestep".to_string(),
                description: "Outputs every Nth frame from video using FFmpeg framestep. step=2 = half frame rate, step=3 = third frame rate. Use for time-lapse from normal footage.".to_string(),
                parameters: {
                    let mut p = HashMap::new();
                    p.insert("input_file".to_string(), PropertyDefinition { prop_type: "string".to_string(), description: "Path to input video file".to_string(), items: None });
                    p.insert("output_file".to_string(), PropertyDefinition { prop_type: "string".to_string(), description: "Path for output video file".to_string(), items: None });
                    p.insert("step".to_string(), PropertyDefinition { prop_type: "number".to_string(), description: "Keep every Nth frame (default: 1)".to_string(), items: None });
                    Parameters { param_type: "object".to_string(), properties: p, required: vec!["input_file".to_string(), "output_file".to_string()] }
                },
            },

            FunctionDeclaration {
                name: "apply_swaprect".to_string(),
                description: "Swaps two rectangular regions within a video frame using FFmpeg swaprect.".to_string(),
                parameters: {
                    let mut p = HashMap::new();
                    p.insert("input_file".to_string(), PropertyDefinition { prop_type: "string".to_string(), description: "Path to input video file".to_string(), items: None });
                    p.insert("output_file".to_string(), PropertyDefinition { prop_type: "string".to_string(), description: "Path for output video file".to_string(), items: None });
                    p.insert("x1".to_string(), PropertyDefinition { prop_type: "number".to_string(), description: "X of first rectangle".to_string(), items: None });
                    p.insert("y1".to_string(), PropertyDefinition { prop_type: "number".to_string(), description: "Y of first rectangle".to_string(), items: None });
                    p.insert("x2".to_string(), PropertyDefinition { prop_type: "number".to_string(), description: "X of second rectangle".to_string(), items: None });
                    p.insert("y2".to_string(), PropertyDefinition { prop_type: "number".to_string(), description: "Y of second rectangle".to_string(), items: None });
                    p.insert("w".to_string(), PropertyDefinition { prop_type: "number".to_string(), description: "Width of both rectangles".to_string(), items: None });
                    p.insert("h".to_string(), PropertyDefinition { prop_type: "number".to_string(), description: "Height of both rectangles".to_string(), items: None });
                    Parameters { param_type: "object".to_string(), properties: p, required: vec!["input_file".to_string(), "output_file".to_string(), "w".to_string(), "h".to_string()] }
                },
            },

            FunctionDeclaration {
                name: "apply_fillborders".to_string(),
                description: "Fills video border pixels using FFmpeg fillborders. Modes: smear (extend edge), mirror, wrap, fixed (solid color). Use to remove thin black borders or extend content to fill edges.".to_string(),
                parameters: {
                    let mut p = HashMap::new();
                    p.insert("input_file".to_string(), PropertyDefinition { prop_type: "string".to_string(), description: "Path to input video file".to_string(), items: None });
                    p.insert("output_file".to_string(), PropertyDefinition { prop_type: "string".to_string(), description: "Path for output video file".to_string(), items: None });
                    p.insert("left".to_string(), PropertyDefinition { prop_type: "number".to_string(), description: "Pixels to fill on left (default: 0)".to_string(), items: None });
                    p.insert("right".to_string(), PropertyDefinition { prop_type: "number".to_string(), description: "Pixels to fill on right (default: 0)".to_string(), items: None });
                    p.insert("top".to_string(), PropertyDefinition { prop_type: "number".to_string(), description: "Pixels to fill on top (default: 0)".to_string(), items: None });
                    p.insert("bottom".to_string(), PropertyDefinition { prop_type: "number".to_string(), description: "Pixels to fill on bottom (default: 0)".to_string(), items: None });
                    p.insert("mode".to_string(), PropertyDefinition { prop_type: "string".to_string(), description: "Fill mode: smear (default), mirror, wrap, fixed".to_string(), items: None });
                    p.insert("color".to_string(), PropertyDefinition { prop_type: "string".to_string(), description: "Color for fixed mode (default: black)".to_string(), items: None });
                    Parameters { param_type: "object".to_string(), properties: p, required: vec!["input_file".to_string(), "output_file".to_string()] }
                },
            },

            FunctionDeclaration {
                name: "apply_chromanr".to_string(),
                description: "Reduces chroma noise using FFmpeg chromanr. Averages chroma in spatial windows where values are similar. Preserves luma sharpness.".to_string(),
                parameters: {
                    let mut p = HashMap::new();
                    p.insert("input_file".to_string(), PropertyDefinition { prop_type: "string".to_string(), description: "Path to input video file".to_string(), items: None });
                    p.insert("output_file".to_string(), PropertyDefinition { prop_type: "string".to_string(), description: "Path for output video file".to_string(), items: None });
                    p.insert("thres".to_string(), PropertyDefinition { prop_type: "number".to_string(), description: "Chroma threshold 1.0–200.0 (default: 30.0)".to_string(), items: None });
                    p.insert("sizew".to_string(), PropertyDefinition { prop_type: "number".to_string(), description: "Window width pixels (default: 5)".to_string(), items: None });
                    p.insert("sizeh".to_string(), PropertyDefinition { prop_type: "number".to_string(), description: "Window height pixels (default: 5)".to_string(), items: None });
                    p.insert("stepw".to_string(), PropertyDefinition { prop_type: "number".to_string(), description: "Horizontal step (default: 1)".to_string(), items: None });
                    p.insert("steph".to_string(), PropertyDefinition { prop_type: "number".to_string(), description: "Vertical step (default: 1)".to_string(), items: None });
                    Parameters { param_type: "object".to_string(), properties: p, required: vec!["input_file".to_string(), "output_file".to_string()] }
                },
            },

            FunctionDeclaration {
                name: "apply_weave".to_string(),
                description: "Weaves separate fields into interlaced frames using FFmpeg weave. Inverse of deinterlacing.".to_string(),
                parameters: {
                    let mut p = HashMap::new();
                    p.insert("input_file".to_string(), PropertyDefinition { prop_type: "string".to_string(), description: "Path to input video (stream of fields)".to_string(), items: None });
                    p.insert("output_file".to_string(), PropertyDefinition { prop_type: "string".to_string(), description: "Path for output interlaced video".to_string(), items: None });
                    p.insert("first_field".to_string(), PropertyDefinition { prop_type: "string".to_string(), description: "First field: top (default) or bottom".to_string(), items: None });
                    Parameters { param_type: "object".to_string(), properties: p, required: vec!["input_file".to_string(), "output_file".to_string()] }
                },
            },

            FunctionDeclaration {
                name: "apply_interlace".to_string(),
                description: "Creates interlaced video from progressive input using FFmpeg interlace. For broadcast delivery (PAL 50i, NTSC 29.97i).".to_string(),
                parameters: {
                    let mut p = HashMap::new();
                    p.insert("input_file".to_string(), PropertyDefinition { prop_type: "string".to_string(), description: "Path to input progressive video".to_string(), items: None });
                    p.insert("output_file".to_string(), PropertyDefinition { prop_type: "string".to_string(), description: "Path for output interlaced video".to_string(), items: None });
                    p.insert("scan".to_string(), PropertyDefinition { prop_type: "string".to_string(), description: "Field order: tff (top first, default) or bff".to_string(), items: None });
                    p.insert("lowpass".to_string(), PropertyDefinition { prop_type: "number".to_string(), description: "Vertical low-pass 0–2 (default: 1). 0=off, 1=linear, 2=complex".to_string(), items: None });
                    Parameters { param_type: "object".to_string(), properties: p, required: vec!["input_file".to_string(), "output_file".to_string()] }
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

            FunctionDeclaration {
                name: "apply_dc_shift".to_string(),
                description: "Applies DC offset correction to audio using FFmpeg dcshift. Fixes recordings with DC offset from cheap microphones or interfaces.".to_string(),
                parameters: {
                    let mut p = HashMap::new();
                    p.insert("input_file".to_string(), PropertyDefinition { prop_type: "string".to_string(), description: "Path to input audio/video file".to_string(), items: None });
                    p.insert("output_file".to_string(), PropertyDefinition { prop_type: "string".to_string(), description: "Path for output file".to_string(), items: None });
                    p.insert("shift".to_string(), PropertyDefinition { prop_type: "number".to_string(), description: "DC shift to apply -1.0–1.0. Negative to cancel positive DC offset".to_string(), items: None });
                    p.insert("limitergain".to_string(), PropertyDefinition { prop_type: "number".to_string(), description: "Limiter gain 0.0–1.0 to prevent clipping (0=disabled, default: 0)".to_string(), items: None });
                    Parameters { param_type: "object".to_string(), properties: p, required: vec!["input_file".to_string(), "output_file".to_string(), "shift".to_string()] }
                },
            },

            FunctionDeclaration {
                name: "measure_dynamic_range".to_string(),
                description: "Measures audio dynamic range using FFmpeg drmeter. Reports DR (crest factor), peak, and RMS values. Analysis only — no output file.".to_string(),
                parameters: {
                    let mut p = HashMap::new();
                    p.insert("input_file".to_string(), PropertyDefinition { prop_type: "string".to_string(), description: "Path to input audio/video file".to_string(), items: None });
                    Parameters { param_type: "object".to_string(), properties: p, required: vec!["input_file".to_string()] }
                },
            },

            FunctionDeclaration {
                name: "apply_single_eq_band".to_string(),
                description: "Single-band parametric EQ using FFmpeg equalizer. One targeted correction at a chosen frequency, width, and gain.".to_string(),
                parameters: {
                    let mut p = HashMap::new();
                    p.insert("input_file".to_string(), PropertyDefinition { prop_type: "string".to_string(), description: "Path to input audio/video file".to_string(), items: None });
                    p.insert("output_file".to_string(), PropertyDefinition { prop_type: "string".to_string(), description: "Path for output file".to_string(), items: None });
                    p.insert("frequency".to_string(), PropertyDefinition { prop_type: "number".to_string(), description: "Centre frequency in Hz (e.g. 1000)".to_string(), items: None });
                    p.insert("width".to_string(), PropertyDefinition { prop_type: "number".to_string(), description: "Band width (default: 1.0 octave)".to_string(), items: None });
                    p.insert("gain".to_string(), PropertyDefinition { prop_type: "number".to_string(), description: "Gain in dB (positive=boost, negative=cut)".to_string(), items: None });
                    p.insert("width_type".to_string(), PropertyDefinition { prop_type: "string".to_string(), description: "Width type: h=Hz, q=Q, o=octaves (default), s=slope, k=kHz".to_string(), items: None });
                    Parameters { param_type: "object".to_string(), properties: p, required: vec!["input_file".to_string(), "output_file".to_string(), "frequency".to_string(), "gain".to_string()] }
                },
            },

            FunctionDeclaration {
                name: "apply_stereotools".to_string(),
                description: "Professional stereo field manipulation using FFmpeg stereotools. Independent levels, balance, phase inversion, muting, and mode switching (LR/MS/mono/swap).".to_string(),
                parameters: {
                    let mut p = HashMap::new();
                    p.insert("input_file".to_string(), PropertyDefinition { prop_type: "string".to_string(), description: "Path to input audio/video file".to_string(), items: None });
                    p.insert("output_file".to_string(), PropertyDefinition { prop_type: "string".to_string(), description: "Path for output file".to_string(), items: None });
                    p.insert("level_in".to_string(), PropertyDefinition { prop_type: "number".to_string(), description: "Input gain 0.015–64.0 (default: 1.0)".to_string(), items: None });
                    p.insert("level_out".to_string(), PropertyDefinition { prop_type: "number".to_string(), description: "Output gain 0.015–64.0 (default: 1.0)".to_string(), items: None });
                    p.insert("balance_in".to_string(), PropertyDefinition { prop_type: "number".to_string(), description: "Input balance -1.0–1.0 (default: 0.0)".to_string(), items: None });
                    p.insert("balance_out".to_string(), PropertyDefinition { prop_type: "number".to_string(), description: "Output balance -1.0–1.0 (default: 0.0)".to_string(), items: None });
                    p.insert("softclip".to_string(), PropertyDefinition { prop_type: "boolean".to_string(), description: "Soft clip output (default: false)".to_string(), items: None });
                    p.insert("mutel".to_string(), PropertyDefinition { prop_type: "boolean".to_string(), description: "Mute left channel (default: false)".to_string(), items: None });
                    p.insert("muter".to_string(), PropertyDefinition { prop_type: "boolean".to_string(), description: "Mute right channel (default: false)".to_string(), items: None });
                    p.insert("phasel".to_string(), PropertyDefinition { prop_type: "boolean".to_string(), description: "Invert left phase (default: false)".to_string(), items: None });
                    p.insert("phaser".to_string(), PropertyDefinition { prop_type: "boolean".to_string(), description: "Invert right phase (default: false)".to_string(), items: None });
                    p.insert("mode".to_string(), PropertyDefinition { prop_type: "string".to_string(), description: "Mode: lr>lr (default), lr>ms, ms>lr, lr>ll, lr>rr, lr>l+r, lr>rl, ms>ll, ms>rr, ms>l+r".to_string(), items: None });
                    Parameters { param_type: "object".to_string(), properties: p, required: vec!["input_file".to_string(), "output_file".to_string()] }
                },
            },

            FunctionDeclaration {
                name: "apply_asetrate".to_string(),
                description: "Changes audio sample rate metadata without resampling (FFmpeg asetrate). Shifts pitch and speed together like tape speed. Use for lo-fi/chipmunk/slowed effects or fixing mistagged rates.".to_string(),
                parameters: {
                    let mut p = HashMap::new();
                    p.insert("input_file".to_string(), PropertyDefinition { prop_type: "string".to_string(), description: "Path to input audio/video file".to_string(), items: None });
                    p.insert("output_file".to_string(), PropertyDefinition { prop_type: "string".to_string(), description: "Path for output file".to_string(), items: None });
                    p.insert("sample_rate".to_string(), PropertyDefinition { prop_type: "number".to_string(), description: "New sample rate Hz. Higher=slower/lower pitch, lower=faster/higher. Default: 44100".to_string(), items: None });
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
                name: "apply_color_key".to_string(),
                description: "Removes a specific colour from video making it transparent using FFmpeg colorkey. Use to key out flat colour backgrounds.".to_string(),
                parameters: {
                    let mut p = HashMap::new();
                    p.insert("input_file".to_string(), PropertyDefinition { prop_type: "string".to_string(), description: "Path to input video file".to_string(), items: None });
                    p.insert("output_file".to_string(), PropertyDefinition { prop_type: "string".to_string(), description: "Path for output video file".to_string(), items: None });
                    p.insert("color".to_string(), PropertyDefinition { prop_type: "string".to_string(), description: "Colour to key out as hex (default: 0x00FF00 green)".to_string(), items: None });
                    p.insert("similarity".to_string(), PropertyDefinition { prop_type: "number".to_string(), description: "Similarity radius 0.0–1.0 (default: 0.1)".to_string(), items: None });
                    p.insert("blend".to_string(), PropertyDefinition { prop_type: "number".to_string(), description: "Blend factor 0.0–1.0 (default: 0.0)".to_string(), items: None });
                    Parameters { param_type: "object".to_string(), properties: p, required: vec!["input_file".to_string(), "output_file".to_string()] }
                },
            },

            FunctionDeclaration {
                name: "apply_monochrome".to_string(),
                description: "Converts video to stylised B&W using FFmpeg monochrome with chroma bias controls for cinematic looks.".to_string(),
                parameters: {
                    let mut p = HashMap::new();
                    p.insert("input_file".to_string(), PropertyDefinition { prop_type: "string".to_string(), description: "Path to input video file".to_string(), items: None });
                    p.insert("output_file".to_string(), PropertyDefinition { prop_type: "string".to_string(), description: "Path for output video file".to_string(), items: None });
                    p.insert("cb".to_string(), PropertyDefinition { prop_type: "number".to_string(), description: "Blue-yellow chroma bias -1.0–1.0 (default: 0.0)".to_string(), items: None });
                    p.insert("cr".to_string(), PropertyDefinition { prop_type: "number".to_string(), description: "Red-green chroma bias -1.0–1.0 (default: 0.0)".to_string(), items: None });
                    p.insert("size".to_string(), PropertyDefinition { prop_type: "number".to_string(), description: "Colour band size 0.0–1.0 (default: 1.0)".to_string(), items: None });
                    Parameters { param_type: "object".to_string(), properties: p, required: vec!["input_file".to_string(), "output_file".to_string()] }
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
                name: "apply_greyedge".to_string(),
                description: "Auto white-balances video using FFmpeg greyedge (grey edge assumption). Analyses edges to estimate scene illuminant and corrects colour cast.".to_string(),
                parameters: {
                    let mut p = HashMap::new();
                    p.insert("input_file".to_string(), PropertyDefinition { prop_type: "string".to_string(), description: "Path to input video file".to_string(), items: None });
                    p.insert("output_file".to_string(), PropertyDefinition { prop_type: "string".to_string(), description: "Path for output video file".to_string(), items: None });
                    p.insert("difford".to_string(), PropertyDefinition { prop_type: "number".to_string(), description: "Differentiation order 0–2 (default: 1). 0=grey world, 1=grey edge 1st, 2=grey edge 2nd".to_string(), items: None });
                    p.insert("minknorm".to_string(), PropertyDefinition { prop_type: "number".to_string(), description: "Minkowski norm 0–20 (default: 1). 0=max norm, 1=L1, 2=L2".to_string(), items: None });
                    p.insert("sigma".to_string(), PropertyDefinition { prop_type: "number".to_string(), description: "Gaussian blur sigma 0.0–200.0 (default: 1.0)".to_string(), items: None });
                    Parameters { param_type: "object".to_string(), properties: p, required: vec!["input_file".to_string(), "output_file".to_string()] }
                },
            },

            FunctionDeclaration {
                name: "apply_fade_video".to_string(),
                description: "Applies video fade-in or fade-out using FFmpeg fade. Fades from/to any colour (default black). Use for intros, outros, scene transitions.".to_string(),
                parameters: {
                    let mut p = HashMap::new();
                    p.insert("input_file".to_string(), PropertyDefinition { prop_type: "string".to_string(), description: "Path to input video file".to_string(), items: None });
                    p.insert("output_file".to_string(), PropertyDefinition { prop_type: "string".to_string(), description: "Path for output video file".to_string(), items: None });
                    p.insert("fade_type".to_string(), PropertyDefinition { prop_type: "string".to_string(), description: "Fade direction: in or out (default: in)".to_string(), items: None });
                    p.insert("start_time".to_string(), PropertyDefinition { prop_type: "number".to_string(), description: "Timestamp in seconds where fade starts (default: 0.0)".to_string(), items: None });
                    p.insert("duration".to_string(), PropertyDefinition { prop_type: "number".to_string(), description: "Duration of the fade in seconds (default: 1.0)".to_string(), items: None });
                    p.insert("color".to_string(), PropertyDefinition { prop_type: "string".to_string(), description: "Colour to fade from/to (default: black). e.g. black, white".to_string(), items: None });
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
                name: "apply_crystalizer".to_string(),
                description: "Enhances audio transients and detail using FFmpeg crystalizer. Creates a hyper-detailed glassy texture by boosting frequency contrasts.".to_string(),
                parameters: {
                    let mut p = HashMap::new();
                    p.insert("input_file".to_string(), PropertyDefinition { prop_type: "string".to_string(), description: "Path to input audio/video file".to_string(), items: None });
                    p.insert("output_file".to_string(), PropertyDefinition { prop_type: "string".to_string(), description: "Path for output file".to_string(), items: None });
                    p.insert("i".to_string(), PropertyDefinition { prop_type: "number".to_string(), description: "Intensity 0.0–10.0 (default: 2.0)".to_string(), items: None });
                    p.insert("clip".to_string(), PropertyDefinition { prop_type: "boolean".to_string(), description: "Clip output to prevent distortion (default: false)".to_string(), items: None });
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

            FunctionDeclaration {
                name: "apply_super_equalizer".to_string(),
                description: "18-band graphic equalizer using FFmpeg superequalizer. Bands from 65Hz to 24kHz. Gain 0.0–20.0, unity = 10.0.".to_string(),
                parameters: {
                    let mut p = HashMap::new();
                    p.insert("input_file".to_string(), PropertyDefinition { prop_type: "string".to_string(), description: "Path to input audio/video file".to_string(), items: None });
                    p.insert("output_file".to_string(), PropertyDefinition { prop_type: "string".to_string(), description: "Path for output file".to_string(), items: None });
                    p.insert("bands".to_string(), PropertyDefinition { prop_type: "string".to_string(), description: "18-band gain string. Format: '1b=V:2b=V:...:18b=V' where 10.0=unity. Example bass boost: '1b=14:2b=13:3b=12:4b=11:5b=10:...:18b=10'".to_string(), items: None });
                    Parameters { param_type: "object".to_string(), properties: p, required: vec!["input_file".to_string(), "output_file".to_string()] }
                },
            },

            // ================================================================
            // PHASE I BATCH 6 — colormatrix, chromashift, cas, nlmeans_video, spp, pp,
            //                   mestimate, midequalizer, median_spatial,
            //                   acrusher, atempo, asetnsamples, apad, asubcut, asupercut
            // ================================================================

            FunctionDeclaration {
                name: "apply_colormatrix".to_string(),
                description: "Converts video between colour matrix standards (bt601, bt709, bt2020, etc.) to fix colour space mismatches.".to_string(),
                parameters: Parameters { param_type: "object".to_string(), properties: { let mut p = HashMap::new(); p.insert("input_file".to_string(), PropertyDefinition { prop_type: "string".to_string(), description: "Input video".to_string(), items: None }); p.insert("output_file".to_string(), PropertyDefinition { prop_type: "string".to_string(), description: "Output path".to_string(), items: None }); p.insert("src".to_string(), PropertyDefinition { prop_type: "string".to_string(), description: "Source matrix: bt601,bt709,smpte240m,fcc,bt2020 (default bt601)".to_string(), items: None }); p.insert("dst".to_string(), PropertyDefinition { prop_type: "string".to_string(), description: "Target matrix: bt601,bt709,smpte240m,fcc,bt2020 (default bt709)".to_string(), items: None }); p }, required: vec!["input_file".to_string(), "output_file".to_string()] },
            },

            FunctionDeclaration {
                name: "apply_chromashift".to_string(),
                description: "Shifts chroma channels horizontally and vertically for chromatic aberration or colour-fringing effects.".to_string(),
                parameters: Parameters { param_type: "object".to_string(), properties: { let mut p = HashMap::new(); p.insert("input_file".to_string(), PropertyDefinition { prop_type: "string".to_string(), description: "Input video".to_string(), items: None }); p.insert("output_file".to_string(), PropertyDefinition { prop_type: "string".to_string(), description: "Output path".to_string(), items: None }); p.insert("cbh".to_string(), PropertyDefinition { prop_type: "number".to_string(), description: "Cb horizontal shift px (default 0)".to_string(), items: None }); p.insert("cbv".to_string(), PropertyDefinition { prop_type: "number".to_string(), description: "Cb vertical shift px (default 0)".to_string(), items: None }); p.insert("crh".to_string(), PropertyDefinition { prop_type: "number".to_string(), description: "Cr horizontal shift px (default 0)".to_string(), items: None }); p.insert("crv".to_string(), PropertyDefinition { prop_type: "number".to_string(), description: "Cr vertical shift px (default 0)".to_string(), items: None }); p }, required: vec!["input_file".to_string(), "output_file".to_string()] },
            },

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

            FunctionDeclaration {
                name: "apply_acrusher".to_string(),
                description: "Applies bit-crusher/lo-fi distortion using acrusher filter for vintage, lo-fi, or glitch audio effects.".to_string(),
                parameters: Parameters {
                    param_type: "object".to_string(),
                    properties: {
                        let mut p = HashMap::new();
                        p.insert("input_file".to_string(), PropertyDefinition { prop_type: "string".to_string(), description: "Input audio/video".to_string(), items: None });
                        p.insert("output_file".to_string(), PropertyDefinition { prop_type: "string".to_string(), description: "Output path".to_string(), items: None });
                        p.insert("level_in".to_string(), PropertyDefinition { prop_type: "number".to_string(), description: "Input gain (default 1.0)".to_string(), items: None });
                        p.insert("level_out".to_string(), PropertyDefinition { prop_type: "number".to_string(), description: "Output gain (default 1.0)".to_string(), items: None });
                        p.insert("bits".to_string(), PropertyDefinition { prop_type: "number".to_string(), description: "Target bit depth 1-64 (default 8)".to_string(), items: None });
                        p.insert("mix".to_string(), PropertyDefinition { prop_type: "number".to_string(), description: "Dry/wet 0.0-1.0 (default 0.5)".to_string(), items: None });
                        p.insert("mode".to_string(), PropertyDefinition { prop_type: "string".to_string(), description: "lin or log (default log)".to_string(), items: None });
                        p.insert("dc".to_string(), PropertyDefinition { prop_type: "number".to_string(), description: "DC bias (default 1.0)".to_string(), items: None });
                        p.insert("aa".to_string(), PropertyDefinition { prop_type: "number".to_string(), description: "Antialiasing 0.0-1.0 (default 0.5)".to_string(), items: None });
                        p.insert("samples".to_string(), PropertyDefinition { prop_type: "number".to_string(), description: "Sample reduction factor (default 1.0)".to_string(), items: None });
                        p.insert("lfo".to_string(), PropertyDefinition { prop_type: "boolean".to_string(), description: "LFO modulation on bits (default false)".to_string(), items: None });
                        p.insert("lforange".to_string(), PropertyDefinition { prop_type: "number".to_string(), description: "LFO range in bits (default 20.0)".to_string(), items: None });
                        p.insert("lforate".to_string(), PropertyDefinition { prop_type: "number".to_string(), description: "LFO rate Hz (default 0.3)".to_string(), items: None });
                        p
                    },
                    required: vec!["input_file".to_string(), "output_file".to_string()],
                },
            },

            FunctionDeclaration {
                name: "apply_atempo".to_string(),
                description: "Changes audio tempo without pitch shift using atempo. Supports 0.5x-100x; chains filters automatically for extreme values.".to_string(),
                parameters: Parameters { param_type: "object".to_string(), properties: { let mut p = HashMap::new(); p.insert("input_file".to_string(), PropertyDefinition { prop_type: "string".to_string(), description: "Input audio/video".to_string(), items: None }); p.insert("output_file".to_string(), PropertyDefinition { prop_type: "string".to_string(), description: "Output path".to_string(), items: None }); p.insert("tempo".to_string(), PropertyDefinition { prop_type: "number".to_string(), description: "Speed multiplier: 0.5=half,2.0=double (default 1.0)".to_string(), items: None }); p }, required: vec!["input_file".to_string(), "output_file".to_string(), "tempo".to_string()] },
            },

            FunctionDeclaration {
                name: "apply_asetnsamples".to_string(),
                description: "Sets a fixed number of audio samples per output frame using asetnsamples for consistent downstream processing.".to_string(),
                parameters: Parameters { param_type: "object".to_string(), properties: { let mut p = HashMap::new(); p.insert("input_file".to_string(), PropertyDefinition { prop_type: "string".to_string(), description: "Input audio/video".to_string(), items: None }); p.insert("output_file".to_string(), PropertyDefinition { prop_type: "string".to_string(), description: "Output path".to_string(), items: None }); p.insert("nb_samples".to_string(), PropertyDefinition { prop_type: "number".to_string(), description: "Samples per frame (default 1024)".to_string(), items: None }); p.insert("pad".to_string(), PropertyDefinition { prop_type: "boolean".to_string(), description: "Pad last frame with silence (default true)".to_string(), items: None }); p }, required: vec!["input_file".to_string(), "output_file".to_string()] },
            },

            FunctionDeclaration {
                name: "apply_apad".to_string(),
                description: "Pads audio with silence at the end using apad to ensure minimum duration or add a silence tail.".to_string(),
                parameters: Parameters { param_type: "object".to_string(), properties: { let mut p = HashMap::new(); p.insert("input_file".to_string(), PropertyDefinition { prop_type: "string".to_string(), description: "Input audio/video".to_string(), items: None }); p.insert("output_file".to_string(), PropertyDefinition { prop_type: "string".to_string(), description: "Output path".to_string(), items: None }); p.insert("pad_dur".to_string(), PropertyDefinition { prop_type: "number".to_string(), description: "Seconds of silence to add (default 0)".to_string(), items: None }); p.insert("whole_dur".to_string(), PropertyDefinition { prop_type: "number".to_string(), description: "Pad until total duration in seconds (default 0)".to_string(), items: None }); p.insert("pad_len".to_string(), PropertyDefinition { prop_type: "number".to_string(), description: "Sample count to add (default 0)".to_string(), items: None }); p.insert("whole_len".to_string(), PropertyDefinition { prop_type: "number".to_string(), description: "Pad until total sample count (default 0)".to_string(), items: None }); p }, required: vec!["input_file".to_string(), "output_file".to_string()] },
            },

            FunctionDeclaration {
                name: "apply_asubcut".to_string(),
                description: "Cuts sub-bass frequencies below a cutoff using asubcut high-pass filter — removes rumble and low-end handling noise.".to_string(),
                parameters: Parameters { param_type: "object".to_string(), properties: { let mut p = HashMap::new(); p.insert("input_file".to_string(), PropertyDefinition { prop_type: "string".to_string(), description: "Input audio/video".to_string(), items: None }); p.insert("output_file".to_string(), PropertyDefinition { prop_type: "string".to_string(), description: "Output path".to_string(), items: None }); p.insert("cutoff".to_string(), PropertyDefinition { prop_type: "number".to_string(), description: "Cutoff Hz (default 20.0)".to_string(), items: None }); p.insert("order".to_string(), PropertyDefinition { prop_type: "number".to_string(), description: "Filter order 3-20 (default 10)".to_string(), items: None }); p.insert("level".to_string(), PropertyDefinition { prop_type: "number".to_string(), description: "Output level (default 1.0)".to_string(), items: None }); p }, required: vec!["input_file".to_string(), "output_file".to_string()] },
            },

            FunctionDeclaration {
                name: "apply_asupercut".to_string(),
                description: "Cuts super-treble frequencies above a cutoff using asupercut low-pass filter — removes ultrasonic artefacts.".to_string(),
                parameters: Parameters { param_type: "object".to_string(), properties: { let mut p = HashMap::new(); p.insert("input_file".to_string(), PropertyDefinition { prop_type: "string".to_string(), description: "Input audio/video".to_string(), items: None }); p.insert("output_file".to_string(), PropertyDefinition { prop_type: "string".to_string(), description: "Output path".to_string(), items: None }); p.insert("cutoff".to_string(), PropertyDefinition { prop_type: "number".to_string(), description: "Cutoff Hz (default 20000.0)".to_string(), items: None }); p.insert("order".to_string(), PropertyDefinition { prop_type: "number".to_string(), description: "Filter order 3-20 (default 10)".to_string(), items: None }); p.insert("level".to_string(), PropertyDefinition { prop_type: "number".to_string(), description: "Output level (default 1.0)".to_string(), items: None }); p }, required: vec!["input_file".to_string(), "output_file".to_string()] },
            },

            // ================================================================
            // PHASE I BATCH 5 — threshold, maskedclamp, roberts, sobel, prewitt, kirsch,
            //                   video_limiter, bilateral, unsharp_mask, lagfun, tinterlace,
            //                   datascope, fspp, haas, aemphasis
            // ================================================================

            FunctionDeclaration {
                name: "apply_threshold".to_string(),
                description: "Applies pixel-value thresholding to a video — pixels below a floor clip to black, above a ceiling clip to max.".to_string(),
                parameters: Parameters { param_type: "object".to_string(), properties: { let mut p = HashMap::new(); p.insert("input_file".to_string(), PropertyDefinition { prop_type: "string".to_string(), description: "Input video file".to_string(), items: None }); p.insert("output_file".to_string(), PropertyDefinition { prop_type: "string".to_string(), description: "Output file path".to_string(), items: None }); p.insert("planes".to_string(), PropertyDefinition { prop_type: "number".to_string(), description: "Bitmask of planes (default 15)".to_string(), items: None }); p }, required: vec!["input_file".to_string(), "output_file".to_string()] },
            },

            FunctionDeclaration {
                name: "apply_maskedclamp".to_string(),
                description: "Clamps each pixel between a dark and bright reference stream using maskedclamp filter.".to_string(),
                parameters: Parameters { param_type: "object".to_string(), properties: { let mut p = HashMap::new(); p.insert("input_file".to_string(), PropertyDefinition { prop_type: "string".to_string(), description: "Input video file".to_string(), items: None }); p.insert("output_file".to_string(), PropertyDefinition { prop_type: "string".to_string(), description: "Output file path".to_string(), items: None }); p.insert("undershoot".to_string(), PropertyDefinition { prop_type: "number".to_string(), description: "Allowed undershoot below min (default 0)".to_string(), items: None }); p.insert("overshoot".to_string(), PropertyDefinition { prop_type: "number".to_string(), description: "Allowed overshoot above max (default 0)".to_string(), items: None }); p.insert("planes".to_string(), PropertyDefinition { prop_type: "number".to_string(), description: "Bitmask of planes (default 15)".to_string(), items: None }); p }, required: vec!["input_file".to_string(), "output_file".to_string()] },
            },

            FunctionDeclaration {
                name: "apply_roberts".to_string(),
                description: "Applies Roberts cross edge detection operator to a video.".to_string(),
                parameters: Parameters { param_type: "object".to_string(), properties: { let mut p = HashMap::new(); p.insert("input_file".to_string(), PropertyDefinition { prop_type: "string".to_string(), description: "Input video file".to_string(), items: None }); p.insert("output_file".to_string(), PropertyDefinition { prop_type: "string".to_string(), description: "Output file path".to_string(), items: None }); p.insert("planes".to_string(), PropertyDefinition { prop_type: "number".to_string(), description: "Bitmask of planes (default 15)".to_string(), items: None }); p.insert("scale".to_string(), PropertyDefinition { prop_type: "number".to_string(), description: "Amplification scale (default 1.0)".to_string(), items: None }); p.insert("delta".to_string(), PropertyDefinition { prop_type: "number".to_string(), description: "Offset (default 0.0)".to_string(), items: None }); p }, required: vec!["input_file".to_string(), "output_file".to_string()] },
            },

            FunctionDeclaration {
                name: "apply_sobel".to_string(),
                description: "Applies Sobel edge detection to a video.".to_string(),
                parameters: Parameters { param_type: "object".to_string(), properties: { let mut p = HashMap::new(); p.insert("input_file".to_string(), PropertyDefinition { prop_type: "string".to_string(), description: "Input video file".to_string(), items: None }); p.insert("output_file".to_string(), PropertyDefinition { prop_type: "string".to_string(), description: "Output file path".to_string(), items: None }); p.insert("planes".to_string(), PropertyDefinition { prop_type: "number".to_string(), description: "Bitmask of planes (default 15)".to_string(), items: None }); p.insert("scale".to_string(), PropertyDefinition { prop_type: "number".to_string(), description: "Amplification scale (default 1.0)".to_string(), items: None }); p.insert("delta".to_string(), PropertyDefinition { prop_type: "number".to_string(), description: "Offset (default 0.0)".to_string(), items: None }); p }, required: vec!["input_file".to_string(), "output_file".to_string()] },
            },

            FunctionDeclaration {
                name: "apply_prewitt".to_string(),
                description: "Applies Prewitt edge detection operator to a video.".to_string(),
                parameters: Parameters { param_type: "object".to_string(), properties: { let mut p = HashMap::new(); p.insert("input_file".to_string(), PropertyDefinition { prop_type: "string".to_string(), description: "Input video file".to_string(), items: None }); p.insert("output_file".to_string(), PropertyDefinition { prop_type: "string".to_string(), description: "Output file path".to_string(), items: None }); p.insert("planes".to_string(), PropertyDefinition { prop_type: "number".to_string(), description: "Bitmask of planes (default 15)".to_string(), items: None }); p.insert("scale".to_string(), PropertyDefinition { prop_type: "number".to_string(), description: "Amplification scale (default 1.0)".to_string(), items: None }); p.insert("delta".to_string(), PropertyDefinition { prop_type: "number".to_string(), description: "Offset (default 0.0)".to_string(), items: None }); p }, required: vec!["input_file".to_string(), "output_file".to_string()] },
            },

            FunctionDeclaration {
                name: "apply_kirsch".to_string(),
                description: "Applies Kirsch edge detection compass operator to a video.".to_string(),
                parameters: Parameters { param_type: "object".to_string(), properties: { let mut p = HashMap::new(); p.insert("input_file".to_string(), PropertyDefinition { prop_type: "string".to_string(), description: "Input video file".to_string(), items: None }); p.insert("output_file".to_string(), PropertyDefinition { prop_type: "string".to_string(), description: "Output file path".to_string(), items: None }); p.insert("planes".to_string(), PropertyDefinition { prop_type: "number".to_string(), description: "Bitmask of planes (default 15)".to_string(), items: None }); p.insert("scale".to_string(), PropertyDefinition { prop_type: "number".to_string(), description: "Amplification scale (default 1.0)".to_string(), items: None }); p.insert("delta".to_string(), PropertyDefinition { prop_type: "number".to_string(), description: "Offset (default 0.0)".to_string(), items: None }); p }, required: vec!["input_file".to_string(), "output_file".to_string()] },
            },

            FunctionDeclaration {
                name: "apply_video_limiter".to_string(),
                description: "Clamps video pixel values to [min, max] range for broadcast-legal signal enforcement.".to_string(),
                parameters: Parameters { param_type: "object".to_string(), properties: { let mut p = HashMap::new(); p.insert("input_file".to_string(), PropertyDefinition { prop_type: "string".to_string(), description: "Input video file".to_string(), items: None }); p.insert("output_file".to_string(), PropertyDefinition { prop_type: "string".to_string(), description: "Output file path".to_string(), items: None }); p.insert("min".to_string(), PropertyDefinition { prop_type: "number".to_string(), description: "Minimum pixel value (default 0)".to_string(), items: None }); p.insert("max".to_string(), PropertyDefinition { prop_type: "number".to_string(), description: "Maximum pixel value (default 65535)".to_string(), items: None }); p.insert("planes".to_string(), PropertyDefinition { prop_type: "number".to_string(), description: "Bitmask of planes (default 15)".to_string(), items: None }); p }, required: vec!["input_file".to_string(), "output_file".to_string()] },
            },

            FunctionDeclaration {
                name: "apply_bilateral".to_string(),
                description: "Applies bilateral filter for edge-preserving noise reduction — smooths uniform areas while keeping sharp edges.".to_string(),
                parameters: Parameters { param_type: "object".to_string(), properties: { let mut p = HashMap::new(); p.insert("input_file".to_string(), PropertyDefinition { prop_type: "string".to_string(), description: "Input video file".to_string(), items: None }); p.insert("output_file".to_string(), PropertyDefinition { prop_type: "string".to_string(), description: "Output file path".to_string(), items: None }); p.insert("sigmaS".to_string(), PropertyDefinition { prop_type: "number".to_string(), description: "Spatial sigma (0.1-512, default 0.1)".to_string(), items: None }); p.insert("sigmaR".to_string(), PropertyDefinition { prop_type: "number".to_string(), description: "Range sigma (0.1-1.0, default 0.1)".to_string(), items: None }); p.insert("planes".to_string(), PropertyDefinition { prop_type: "number".to_string(), description: "Bitmask of planes (default 1=luma)".to_string(), items: None }); p }, required: vec!["input_file".to_string(), "output_file".to_string()] },
            },

            FunctionDeclaration {
                name: "apply_unsharp_mask".to_string(),
                description: "Applies unsharp mask for precise sharpening or blurring of luma and chroma planes independently.".to_string(),
                parameters: Parameters {
                    param_type: "object".to_string(),
                    properties: {
                        let mut p = HashMap::new();
                        p.insert("input_file".to_string(), PropertyDefinition { prop_type: "string".to_string(), description: "Input video file".to_string(), items: None });
                        p.insert("output_file".to_string(), PropertyDefinition { prop_type: "string".to_string(), description: "Output file path".to_string(), items: None });
                        p.insert("luma_x".to_string(), PropertyDefinition { prop_type: "number".to_string(), description: "Luma kernel width (odd 3-23, default 5)".to_string(), items: None });
                        p.insert("luma_y".to_string(), PropertyDefinition { prop_type: "number".to_string(), description: "Luma kernel height (odd 3-23, default 5)".to_string(), items: None });
                        p.insert("luma_amount".to_string(), PropertyDefinition { prop_type: "number".to_string(), description: "Luma amount; positive=sharpen, negative=blur (default 1.0)".to_string(), items: None });
                        p.insert("chroma_x".to_string(), PropertyDefinition { prop_type: "number".to_string(), description: "Chroma kernel width (default 5)".to_string(), items: None });
                        p.insert("chroma_y".to_string(), PropertyDefinition { prop_type: "number".to_string(), description: "Chroma kernel height (default 5)".to_string(), items: None });
                        p.insert("chroma_amount".to_string(), PropertyDefinition { prop_type: "number".to_string(), description: "Chroma amount (default 0.0)".to_string(), items: None });
                        p
                    },
                    required: vec!["input_file".to_string(), "output_file".to_string()],
                },
            },

            FunctionDeclaration {
                name: "apply_lagfun".to_string(),
                description: "Applies lagfun EMA for slow ghost trails and motion blur effect by blending frames over time with a decay factor.".to_string(),
                parameters: Parameters { param_type: "object".to_string(), properties: { let mut p = HashMap::new(); p.insert("input_file".to_string(), PropertyDefinition { prop_type: "string".to_string(), description: "Input video file".to_string(), items: None }); p.insert("output_file".to_string(), PropertyDefinition { prop_type: "string".to_string(), description: "Output file path".to_string(), items: None }); p.insert("decay".to_string(), PropertyDefinition { prop_type: "number".to_string(), description: "EMA decay 0.0-1.0; higher=longer trail (default 0.95)".to_string(), items: None }); p.insert("planes".to_string(), PropertyDefinition { prop_type: "number".to_string(), description: "Bitmask of planes (default 15)".to_string(), items: None }); p }, required: vec!["input_file".to_string(), "output_file".to_string()] },
            },

            FunctionDeclaration {
                name: "apply_tinterlace".to_string(),
                description: "Applies temporal field interlacing for broadcast output using tinterlace filter.".to_string(),
                parameters: Parameters { param_type: "object".to_string(), properties: { let mut p = HashMap::new(); p.insert("input_file".to_string(), PropertyDefinition { prop_type: "string".to_string(), description: "Input video file".to_string(), items: None }); p.insert("output_file".to_string(), PropertyDefinition { prop_type: "string".to_string(), description: "Output file path".to_string(), items: None }); p.insert("mode".to_string(), PropertyDefinition { prop_type: "number".to_string(), description: "Interlace mode 0-7 (default 0=merge)".to_string(), items: None }); p.insert("flags".to_string(), PropertyDefinition { prop_type: "string".to_string(), description: "Flags: vlpf or cvlpf (default vlpf)".to_string(), items: None }); p }, required: vec!["input_file".to_string(), "output_file".to_string()] },
            },

            FunctionDeclaration {
                name: "apply_datascope".to_string(),
                description: "Renders a datascope showing raw pixel values at a region — useful for colour accuracy analysis and broadcast QC.".to_string(),
                parameters: Parameters {
                    param_type: "object".to_string(),
                    properties: {
                        let mut p = HashMap::new();
                        p.insert("input_file".to_string(), PropertyDefinition { prop_type: "string".to_string(), description: "Input video file".to_string(), items: None });
                        p.insert("output_file".to_string(), PropertyDefinition { prop_type: "string".to_string(), description: "Output file path".to_string(), items: None });
                        p.insert("size".to_string(), PropertyDefinition { prop_type: "string".to_string(), description: "Output size (default hd720)".to_string(), items: None });
                        p.insert("x".to_string(), PropertyDefinition { prop_type: "number".to_string(), description: "X position of pixel region (default 0)".to_string(), items: None });
                        p.insert("y".to_string(), PropertyDefinition { prop_type: "number".to_string(), description: "Y position of pixel region (default 0)".to_string(), items: None });
                        p.insert("mode".to_string(), PropertyDefinition { prop_type: "number".to_string(), description: "Display mode: 0=mono,1=color,2=color2 (default 0)".to_string(), items: None });
                        p.insert("opacity".to_string(), PropertyDefinition { prop_type: "number".to_string(), description: "Overlay opacity 0.0-1.0 (default 0.75)".to_string(), items: None });
                        p.insert("axis".to_string(), PropertyDefinition { prop_type: "boolean".to_string(), description: "Show axis labels (default false)".to_string(), items: None });
                        p
                    },
                    required: vec!["input_file".to_string(), "output_file".to_string()],
                },
            },

            FunctionDeclaration {
                name: "apply_fspp".to_string(),
                description: "Applies Fast Super Pixel (fspp) frequency-domain denoising for smooth noise removal while preserving detail.".to_string(),
                parameters: Parameters { param_type: "object".to_string(), properties: { let mut p = HashMap::new(); p.insert("input_file".to_string(), PropertyDefinition { prop_type: "string".to_string(), description: "Input video file".to_string(), items: None }); p.insert("output_file".to_string(), PropertyDefinition { prop_type: "string".to_string(), description: "Output file path".to_string(), items: None }); p.insert("quality".to_string(), PropertyDefinition { prop_type: "number".to_string(), description: "Quality iterations (4-5, default 4)".to_string(), items: None }); p.insert("strength".to_string(), PropertyDefinition { prop_type: "number".to_string(), description: "Denoising strength (-15 to 32, default 0)".to_string(), items: None }); p.insert("use_bframe_qp".to_string(), PropertyDefinition { prop_type: "boolean".to_string(), description: "Use B-frame QP (default false)".to_string(), items: None }); p }, required: vec!["input_file".to_string(), "output_file".to_string()] },
            },

            FunctionDeclaration {
                name: "apply_haas".to_string(),
                description: "Applies Haas effect for stereo widening by delaying one channel to create perceived spatial width.".to_string(),
                parameters: Parameters {
                    param_type: "object".to_string(),
                    properties: {
                        let mut p = HashMap::new();
                        p.insert("input_file".to_string(), PropertyDefinition { prop_type: "string".to_string(), description: "Input audio/video file".to_string(), items: None });
                        p.insert("output_file".to_string(), PropertyDefinition { prop_type: "string".to_string(), description: "Output file path".to_string(), items: None });
                        p.insert("level_in".to_string(), PropertyDefinition { prop_type: "number".to_string(), description: "Input gain (default 1.0)".to_string(), items: None });
                        p.insert("level_out".to_string(), PropertyDefinition { prop_type: "number".to_string(), description: "Output gain (default 1.0)".to_string(), items: None });
                        p.insert("side_gain".to_string(), PropertyDefinition { prop_type: "number".to_string(), description: "Side channel gain (default 1.0)".to_string(), items: None });
                        p.insert("middle_source".to_string(), PropertyDefinition { prop_type: "string".to_string(), description: "Middle source: mid/left/right/side (default mid)".to_string(), items: None });
                        p.insert("middle_phase".to_string(), PropertyDefinition { prop_type: "boolean".to_string(), description: "Invert middle phase (default false)".to_string(), items: None });
                        p.insert("left_delay".to_string(), PropertyDefinition { prop_type: "number".to_string(), description: "Left delay ms 0-40 (default 2.5)".to_string(), items: None });
                        p.insert("left_balance".to_string(), PropertyDefinition { prop_type: "number".to_string(), description: "Left balance -1.0-1.0 (default -1.0)".to_string(), items: None });
                        p.insert("right_delay".to_string(), PropertyDefinition { prop_type: "number".to_string(), description: "Right delay ms 0-40 (default 2.5)".to_string(), items: None });
                        p.insert("right_balance".to_string(), PropertyDefinition { prop_type: "number".to_string(), description: "Right balance -1.0-1.0 (default 1.0)".to_string(), items: None });
                        p
                    },
                    required: vec!["input_file".to_string(), "output_file".to_string()],
                },
            },

            FunctionDeclaration {
                name: "apply_aemphasis".to_string(),
                description: "Applies audio emphasis/de-emphasis curves (RIAA, CD, FM) for vinyl/tape/FM pre-emphasis or de-emphasis.".to_string(),
                parameters: Parameters { param_type: "object".to_string(), properties: { let mut p = HashMap::new(); p.insert("input_file".to_string(), PropertyDefinition { prop_type: "string".to_string(), description: "Input audio/video file".to_string(), items: None }); p.insert("output_file".to_string(), PropertyDefinition { prop_type: "string".to_string(), description: "Output file path".to_string(), items: None }); p.insert("level_in".to_string(), PropertyDefinition { prop_type: "number".to_string(), description: "Input gain (default 1.0)".to_string(), items: None }); p.insert("level_out".to_string(), PropertyDefinition { prop_type: "number".to_string(), description: "Output gain (default 1.0)".to_string(), items: None }); p.insert("mode".to_string(), PropertyDefinition { prop_type: "string".to_string(), description: "reproduction or production (default reproduction)".to_string(), items: None }); p.insert("emph_type".to_string(), PropertyDefinition { prop_type: "string".to_string(), description: "Curve: riaa, cd, 50fm, 75fm, 50kf, 75kf (default cd)".to_string(), items: None }); p }, required: vec!["input_file".to_string(), "output_file".to_string()] },
            },

            // ================================================================
            // PHASE I BATCH 4 — negate, pixelize, colorlevels, pseudocolor, colorhold, shuffleplanes,
            //                   blackdetect, idet, vstack, hstack, setdar, stereo3d, telecine, pullup, thumbnail
            // ================================================================

            FunctionDeclaration {
                name: "apply_negate".to_string(),
                description: "Inverts video colours (negative) via FFmpeg negate. Can target individual R/G/B channels.".to_string(),
                parameters: Parameters { param_type: "object".to_string(), properties: { let mut p = HashMap::new(); p.insert("input_file".to_string(), PropertyDefinition { prop_type: "string".to_string(), description: "Input video file".to_string(), items: None }); p.insert("output_file".to_string(), PropertyDefinition { prop_type: "string".to_string(), description: "Output file path".to_string(), items: None }); p.insert("components".to_string(), PropertyDefinition { prop_type: "number".to_string(), description: "Bitmask 1=R,2=G,4=B. Default 7 (RGB).".to_string(), items: None }); p }, required: vec!["input_file".to_string(), "output_file".to_string()] },
            },

            FunctionDeclaration {
                name: "apply_pixelize".to_string(),
                description: "Pixelates/mosaics video via FFmpeg pixelize. Block-based mosaic for censorship or artistic style.".to_string(),
                parameters: Parameters { param_type: "object".to_string(), properties: { let mut p = HashMap::new(); p.insert("input_file".to_string(), PropertyDefinition { prop_type: "string".to_string(), description: "Input video file".to_string(), items: None }); p.insert("output_file".to_string(), PropertyDefinition { prop_type: "string".to_string(), description: "Output file path".to_string(), items: None }); p.insert("width".to_string(), PropertyDefinition { prop_type: "number".to_string(), description: "Block width px. Default 16.".to_string(), items: None }); p.insert("height".to_string(), PropertyDefinition { prop_type: "number".to_string(), description: "Block height px. Default same as width.".to_string(), items: None }); p.insert("mode".to_string(), PropertyDefinition { prop_type: "number".to_string(), description: "0=avg (default), 1=blocks.".to_string(), items: None }); p }, required: vec!["input_file".to_string(), "output_file".to_string()] },
            },

            FunctionDeclaration {
                name: "apply_colorlevels".to_string(),
                description: "Clips and remaps colour levels per channel via FFmpeg colorlevels. Set input/output black and white points.".to_string(),
                parameters: Parameters {
                    param_type: "object".to_string(),
                    properties: {
                        let mut props = HashMap::new();
                        props.insert("input_file".to_string(), PropertyDefinition { prop_type: "string".to_string(), description: "Input video file".to_string(), items: None });
                        props.insert("output_file".to_string(), PropertyDefinition { prop_type: "string".to_string(), description: "Output file path".to_string(), items: None });
                        props.insert("rimin".to_string(), PropertyDefinition { prop_type: "number".to_string(), description: "Red input min 0–1. Default 0.".to_string(), items: None });
                        props.insert("rimax".to_string(), PropertyDefinition { prop_type: "number".to_string(), description: "Red input max 0–1. Default 1.".to_string(), items: None });
                        props.insert("gimin".to_string(), PropertyDefinition { prop_type: "number".to_string(), description: "Green input min. Default 0.".to_string(), items: None });
                        props.insert("gimax".to_string(), PropertyDefinition { prop_type: "number".to_string(), description: "Green input max. Default 1.".to_string(), items: None });
                        props.insert("bimin".to_string(), PropertyDefinition { prop_type: "number".to_string(), description: "Blue input min. Default 0.".to_string(), items: None });
                        props.insert("bimax".to_string(), PropertyDefinition { prop_type: "number".to_string(), description: "Blue input max. Default 1.".to_string(), items: None });
                        props.insert("romin".to_string(), PropertyDefinition { prop_type: "number".to_string(), description: "Output min all channels. Default 0.".to_string(), items: None });
                        props.insert("romax".to_string(), PropertyDefinition { prop_type: "number".to_string(), description: "Output max all channels. Default 1.".to_string(), items: None });
                        props
                    },
                    required: vec!["input_file".to_string(), "output_file".to_string()],
                },
            },

            FunctionDeclaration {
                name: "apply_pseudocolor".to_string(),
                description: "False-colour visualisation via FFmpeg pseudocolor. Maps luminance to scientific palettes (magma, viridis, turbo, etc).".to_string(),
                parameters: Parameters { param_type: "object".to_string(), properties: { let mut p = HashMap::new(); p.insert("input_file".to_string(), PropertyDefinition { prop_type: "string".to_string(), description: "Input video file".to_string(), items: None }); p.insert("output_file".to_string(), PropertyDefinition { prop_type: "string".to_string(), description: "Output file path".to_string(), items: None }); p.insert("preset".to_string(), PropertyDefinition { prop_type: "number".to_string(), description: "Palette 0=magma,1=inferno,2=plasma,3=viridis,4=turbo. Default 0.".to_string(), items: None }); p.insert("opacity".to_string(), PropertyDefinition { prop_type: "number".to_string(), description: "Effect opacity 0–1. Default 1.".to_string(), items: None }); p }, required: vec!["input_file".to_string(), "output_file".to_string()] },
            },

            FunctionDeclaration {
                name: "apply_colorhold".to_string(),
                description: "Selective colour via FFmpeg colorhold: keeps one colour, desaturates rest. Classic movie-poster effect.".to_string(),
                parameters: Parameters { param_type: "object".to_string(), properties: { let mut p = HashMap::new(); p.insert("input_file".to_string(), PropertyDefinition { prop_type: "string".to_string(), description: "Input video file".to_string(), items: None }); p.insert("output_file".to_string(), PropertyDefinition { prop_type: "string".to_string(), description: "Output file path".to_string(), items: None }); p.insert("color".to_string(), PropertyDefinition { prop_type: "string".to_string(), description: "Colour to preserve. Default 'red'.".to_string(), items: None }); p.insert("similarity".to_string(), PropertyDefinition { prop_type: "number".to_string(), description: "Match range 0–1. Default 0.1.".to_string(), items: None }); p.insert("blend".to_string(), PropertyDefinition { prop_type: "number".to_string(), description: "Boundary blend 0–1. Default 0.".to_string(), items: None }); p }, required: vec!["input_file".to_string(), "output_file".to_string()] },
            },

            FunctionDeclaration {
                name: "apply_shuffleplanes".to_string(),
                description: "Reorders video colour planes via FFmpeg shuffleplanes. Enables R↔B swap, channel isolation, and false-colour effects.".to_string(),
                parameters: Parameters { param_type: "object".to_string(), properties: { let mut p = HashMap::new(); p.insert("input_file".to_string(), PropertyDefinition { prop_type: "string".to_string(), description: "Input video file".to_string(), items: None }); p.insert("output_file".to_string(), PropertyDefinition { prop_type: "string".to_string(), description: "Output file path".to_string(), items: None }); p.insert("map0".to_string(), PropertyDefinition { prop_type: "number".to_string(), description: "Source for output plane 0. Default 0.".to_string(), items: None }); p.insert("map1".to_string(), PropertyDefinition { prop_type: "number".to_string(), description: "Source for output plane 1. Default 1.".to_string(), items: None }); p.insert("map2".to_string(), PropertyDefinition { prop_type: "number".to_string(), description: "Source for output plane 2. Default 2.".to_string(), items: None }); p.insert("map3".to_string(), PropertyDefinition { prop_type: "number".to_string(), description: "Source for output plane 3. Default 3.".to_string(), items: None }); p }, required: vec!["input_file".to_string(), "output_file".to_string()] },
            },

            FunctionDeclaration {
                name: "detect_black_frames".to_string(),
                description: "Detects black/near-black frame segments via FFmpeg blackdetect. Returns timestamps. Analysis only.".to_string(),
                parameters: Parameters { param_type: "object".to_string(), properties: { let mut p = HashMap::new(); p.insert("input_file".to_string(), PropertyDefinition { prop_type: "string".to_string(), description: "Input video file".to_string(), items: None }); p.insert("black_min_duration".to_string(), PropertyDefinition { prop_type: "number".to_string(), description: "Min black segment duration s. Default 2.0.".to_string(), items: None }); p.insert("picture_black_ratio_th".to_string(), PropertyDefinition { prop_type: "number".to_string(), description: "Fraction of frame that must be black. Default 0.98.".to_string(), items: None }); p.insert("pixel_black_th".to_string(), PropertyDefinition { prop_type: "number".to_string(), description: "Pixel luminance black threshold 0–1. Default 0.10.".to_string(), items: None }); p }, required: vec!["input_file".to_string()] },
            },

            FunctionDeclaration {
                name: "detect_interlace_type".to_string(),
                description: "Detects progressive/TFF/BFF interlacing via FFmpeg idet. Analysis only.".to_string(),
                parameters: Parameters { param_type: "object".to_string(), properties: { let mut p = HashMap::new(); p.insert("input_file".to_string(), PropertyDefinition { prop_type: "string".to_string(), description: "Input video file".to_string(), items: None }); p }, required: vec!["input_file".to_string()] },
            },

            FunctionDeclaration {
                name: "apply_vstack".to_string(),
                description: "Stacks two videos vertically via FFmpeg vstack. Both must have same width.".to_string(),
                parameters: Parameters { param_type: "object".to_string(), properties: { let mut p = HashMap::new(); p.insert("input_file".to_string(), PropertyDefinition { prop_type: "string".to_string(), description: "Top video file".to_string(), items: None }); p.insert("secondary_file".to_string(), PropertyDefinition { prop_type: "string".to_string(), description: "Bottom video file".to_string(), items: None }); p.insert("output_file".to_string(), PropertyDefinition { prop_type: "string".to_string(), description: "Output file path".to_string(), items: None }); p.insert("shortest".to_string(), PropertyDefinition { prop_type: "boolean".to_string(), description: "End on shorter clip. Default false.".to_string(), items: None }); p }, required: vec!["input_file".to_string(), "secondary_file".to_string(), "output_file".to_string()] },
            },

            FunctionDeclaration {
                name: "apply_hstack".to_string(),
                description: "Stacks two videos horizontally (side by side) via FFmpeg hstack. Both must have same height.".to_string(),
                parameters: Parameters { param_type: "object".to_string(), properties: { let mut p = HashMap::new(); p.insert("input_file".to_string(), PropertyDefinition { prop_type: "string".to_string(), description: "Left video file".to_string(), items: None }); p.insert("secondary_file".to_string(), PropertyDefinition { prop_type: "string".to_string(), description: "Right video file".to_string(), items: None }); p.insert("output_file".to_string(), PropertyDefinition { prop_type: "string".to_string(), description: "Output file path".to_string(), items: None }); p.insert("shortest".to_string(), PropertyDefinition { prop_type: "boolean".to_string(), description: "End on shorter clip. Default false.".to_string(), items: None }); p }, required: vec!["input_file".to_string(), "secondary_file".to_string(), "output_file".to_string()] },
            },

            FunctionDeclaration {
                name: "apply_setdar".to_string(),
                description: "Sets display aspect ratio via FFmpeg setdar without re-encoding. Corrects anamorphic/mis-tagged footage.".to_string(),
                parameters: Parameters { param_type: "object".to_string(), properties: { let mut p = HashMap::new(); p.insert("input_file".to_string(), PropertyDefinition { prop_type: "string".to_string(), description: "Input video file".to_string(), items: None }); p.insert("output_file".to_string(), PropertyDefinition { prop_type: "string".to_string(), description: "Output file path".to_string(), items: None }); p.insert("dar".to_string(), PropertyDefinition { prop_type: "string".to_string(), description: "Aspect ratio e.g. '16/9', '4/3'. Default '16/9'.".to_string(), items: None }); p }, required: vec!["input_file".to_string(), "output_file".to_string()] },
            },

            FunctionDeclaration {
                name: "apply_stereo3d".to_string(),
                description: "Converts between stereoscopic 3D formats via FFmpeg stereo3d (SBS, over-under, anaglyph, mono).".to_string(),
                parameters: Parameters { param_type: "object".to_string(), properties: { let mut p = HashMap::new(); p.insert("input_file".to_string(), PropertyDefinition { prop_type: "string".to_string(), description: "Input stereo 3D video file".to_string(), items: None }); p.insert("output_file".to_string(), PropertyDefinition { prop_type: "string".to_string(), description: "Output file path".to_string(), items: None }); p.insert("input_format".to_string(), PropertyDefinition { prop_type: "string".to_string(), description: "Input format: sbsl, sbsr, abl, abr. Default 'sbsl'.".to_string(), items: None }); p.insert("output_format".to_string(), PropertyDefinition { prop_type: "string".to_string(), description: "Output format: arcd=red-cyan anaglyph, ml=mono left, mr=mono right. Default 'arcd'.".to_string(), items: None }); p }, required: vec!["input_file".to_string(), "output_file".to_string()] },
            },

            FunctionDeclaration {
                name: "apply_telecine".to_string(),
                description: "Applies 3:2 pulldown telecine (24fps→29.97fps) via FFmpeg telecine.".to_string(),
                parameters: Parameters { param_type: "object".to_string(), properties: { let mut p = HashMap::new(); p.insert("input_file".to_string(), PropertyDefinition { prop_type: "string".to_string(), description: "Input 24fps video file".to_string(), items: None }); p.insert("output_file".to_string(), PropertyDefinition { prop_type: "string".to_string(), description: "Output 29.97fps file path".to_string(), items: None }); p.insert("pattern".to_string(), PropertyDefinition { prop_type: "string".to_string(), description: "Pulldown pattern. Default '23'.".to_string(), items: None }); p.insert("first_field".to_string(), PropertyDefinition { prop_type: "number".to_string(), description: "0=top, 1=bottom. Default 0.".to_string(), items: None }); p }, required: vec!["input_file".to_string(), "output_file".to_string()] },
            },

            FunctionDeclaration {
                name: "apply_pullup".to_string(),
                description: "Removes 3:2 pulldown (inverse telecine) via FFmpeg pullup. Recovers 24fps from 29.97fps telecined content.".to_string(),
                parameters: Parameters { param_type: "object".to_string(), properties: { let mut p = HashMap::new(); p.insert("input_file".to_string(), PropertyDefinition { prop_type: "string".to_string(), description: "Input telecined 29.97fps video file".to_string(), items: None }); p.insert("output_file".to_string(), PropertyDefinition { prop_type: "string".to_string(), description: "Output 24fps file path".to_string(), items: None }); p }, required: vec!["input_file".to_string(), "output_file".to_string()] },
            },

            FunctionDeclaration {
                name: "select_thumbnail_frame".to_string(),
                description: "Selects the best representative thumbnail frame via FFmpeg thumbnail filter. Analyses N-frame batches for the most representative frame.".to_string(),
                parameters: Parameters { param_type: "object".to_string(), properties: { let mut p = HashMap::new(); p.insert("input_file".to_string(), PropertyDefinition { prop_type: "string".to_string(), description: "Input video file".to_string(), items: None }); p.insert("output_file".to_string(), PropertyDefinition { prop_type: "string".to_string(), description: "Output image file (e.g. thumb.jpg)".to_string(), items: None }); p.insert("n".to_string(), PropertyDefinition { prop_type: "number".to_string(), description: "Frames per batch. Default 100.".to_string(), items: None }); p }, required: vec!["input_file".to_string(), "output_file".to_string()] },
            },

            // ================================================================
            // PHASE I BATCH 3 — Blur variants, grain, rotation, geq, CCM, denoisers, LUT3D, SITI, amplify
            // ================================================================

            FunctionDeclaration {
                name: "apply_gaussian_blur".to_string(),
                description: "Gaussian blur via FFmpeg gblur filter. Smooth natural blur with configurable sigma.".to_string(),
                parameters: Parameters {
                    param_type: "object".to_string(),
                    properties: {
                        let mut props = HashMap::new();
                        props.insert("input_file".to_string(), PropertyDefinition { prop_type: "string".to_string(), description: "Input video file".to_string(), items: None });
                        props.insert("output_file".to_string(), PropertyDefinition { prop_type: "string".to_string(), description: "Output file path".to_string(), items: None });
                        props.insert("sigma".to_string(), PropertyDefinition { prop_type: "number".to_string(), description: "Blur sigma. Default 3.".to_string(), items: None });
                        props.insert("steps".to_string(), PropertyDefinition { prop_type: "number".to_string(), description: "Blur passes 1–6. Default 1.".to_string(), items: None });
                        props.insert("planes".to_string(), PropertyDefinition { prop_type: "number".to_string(), description: "Planes bitmask. Default 15.".to_string(), items: None });
                        props
                    },
                    required: vec!["input_file".to_string(), "output_file".to_string()],
                },
            },

            FunctionDeclaration {
                name: "apply_box_blur".to_string(),
                description: "Box (average) blur via FFmpeg avgblur. Fast rectangular blur.".to_string(),
                parameters: Parameters {
                    param_type: "object".to_string(),
                    properties: {
                        let mut props = HashMap::new();
                        props.insert("input_file".to_string(), PropertyDefinition { prop_type: "string".to_string(), description: "Input video file".to_string(), items: None });
                        props.insert("output_file".to_string(), PropertyDefinition { prop_type: "string".to_string(), description: "Output file path".to_string(), items: None });
                        props.insert("size_x".to_string(), PropertyDefinition { prop_type: "number".to_string(), description: "Horizontal kernel size px. Default 3.".to_string(), items: None });
                        props.insert("size_y".to_string(), PropertyDefinition { prop_type: "number".to_string(), description: "Vertical kernel size px. Default same as size_x.".to_string(), items: None });
                        props.insert("planes".to_string(), PropertyDefinition { prop_type: "number".to_string(), description: "Planes bitmask. Default 15.".to_string(), items: None });
                        props
                    },
                    required: vec!["input_file".to_string(), "output_file".to_string()],
                },
            },

            FunctionDeclaration {
                name: "apply_smart_blur".to_string(),
                description: "Smart blur via FFmpeg smartblur. Blurs flat areas while preserving edges.".to_string(),
                parameters: Parameters {
                    param_type: "object".to_string(),
                    properties: {
                        let mut props = HashMap::new();
                        props.insert("input_file".to_string(), PropertyDefinition { prop_type: "string".to_string(), description: "Input video file".to_string(), items: None });
                        props.insert("output_file".to_string(), PropertyDefinition { prop_type: "string".to_string(), description: "Output file path".to_string(), items: None });
                        props.insert("luma_radius".to_string(), PropertyDefinition { prop_type: "number".to_string(), description: "Luma blur radius 0.1–5.0. Default 1.0.".to_string(), items: None });
                        props.insert("luma_strength".to_string(), PropertyDefinition { prop_type: "number".to_string(), description: "Strength -1 to 1. Negative = blur, positive = sharpen. Default -0.3.".to_string(), items: None });
                        props.insert("luma_threshold".to_string(), PropertyDefinition { prop_type: "number".to_string(), description: "Edge threshold -30 to 30. Default 0.".to_string(), items: None });
                        props
                    },
                    required: vec!["input_file".to_string(), "output_file".to_string()],
                },
            },

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
                name: "apply_rotate_angle".to_string(),
                description: "Rotates video by arbitrary radians via FFmpeg rotate filter. Unlike rotate_video, supports any angle.".to_string(),
                parameters: Parameters {
                    param_type: "object".to_string(),
                    properties: {
                        let mut props = HashMap::new();
                        props.insert("input_file".to_string(), PropertyDefinition { prop_type: "string".to_string(), description: "Input video file".to_string(), items: None });
                        props.insert("output_file".to_string(), PropertyDefinition { prop_type: "string".to_string(), description: "Output file path".to_string(), items: None });
                        props.insert("angle_rad".to_string(), PropertyDefinition { prop_type: "number".to_string(), description: "Angle in radians. PI/6=30°, PI/4=45°, PI/2=90°.".to_string(), items: None });
                        props.insert("fillcolor".to_string(), PropertyDefinition { prop_type: "string".to_string(), description: "Fill colour for exposed corners. Default 'black'.".to_string(), items: None });
                        props.insert("expand".to_string(), PropertyDefinition { prop_type: "boolean".to_string(), description: "Expand canvas to fit content. Default false.".to_string(), items: None });
                        props
                    },
                    required: vec!["input_file".to_string(), "output_file".to_string(), "angle_rad".to_string()],
                },
            },

            FunctionDeclaration {
                name: "apply_geq".to_string(),
                description: "Per-pixel formula manipulation via FFmpeg geq. Write math expressions for luma/chroma using X,Y,W,H and functions like lum(),r(),g(),b().".to_string(),
                parameters: Parameters {
                    param_type: "object".to_string(),
                    properties: {
                        let mut props = HashMap::new();
                        props.insert("input_file".to_string(), PropertyDefinition { prop_type: "string".to_string(), description: "Input video file".to_string(), items: None });
                        props.insert("output_file".to_string(), PropertyDefinition { prop_type: "string".to_string(), description: "Output file path".to_string(), items: None });
                        props.insert("lum_expr".to_string(), PropertyDefinition { prop_type: "string".to_string(), description: "Luma expression. Default 'lum(X,Y)'. Example: 'lum(W-X,Y)' mirrors horizontally.".to_string(), items: None });
                        props.insert("cb_expr".to_string(), PropertyDefinition { prop_type: "string".to_string(), description: "Cb chroma expression. Default 'cb(X,Y)'.".to_string(), items: None });
                        props.insert("cr_expr".to_string(), PropertyDefinition { prop_type: "string".to_string(), description: "Cr chroma expression. Default 'cr(X,Y)'.".to_string(), items: None });
                        props
                    },
                    required: vec!["input_file".to_string(), "output_file".to_string()],
                },
            },

            FunctionDeclaration {
                name: "apply_colorchannelmixer".to_string(),
                description: "Colour channel matrix mixer via FFmpeg colorchannelmixer. Control R/G/B cross-channel contributions for grading, channel swap, or precise greyscale.".to_string(),
                parameters: Parameters {
                    param_type: "object".to_string(),
                    properties: {
                        let mut props = HashMap::new();
                        props.insert("input_file".to_string(), PropertyDefinition { prop_type: "string".to_string(), description: "Input video file".to_string(), items: None });
                        props.insert("output_file".to_string(), PropertyDefinition { prop_type: "string".to_string(), description: "Output file path".to_string(), items: None });
                        props.insert("rr".to_string(), PropertyDefinition { prop_type: "number".to_string(), description: "R→R contribution. Default 1.0.".to_string(), items: None });
                        props.insert("rg".to_string(), PropertyDefinition { prop_type: "number".to_string(), description: "G→R contribution. Default 0.0.".to_string(), items: None });
                        props.insert("rb".to_string(), PropertyDefinition { prop_type: "number".to_string(), description: "B→R contribution. Default 0.0.".to_string(), items: None });
                        props.insert("gr".to_string(), PropertyDefinition { prop_type: "number".to_string(), description: "R→G contribution. Default 0.0.".to_string(), items: None });
                        props.insert("gg".to_string(), PropertyDefinition { prop_type: "number".to_string(), description: "G→G contribution. Default 1.0.".to_string(), items: None });
                        props.insert("gb".to_string(), PropertyDefinition { prop_type: "number".to_string(), description: "B→G contribution. Default 0.0.".to_string(), items: None });
                        props.insert("br".to_string(), PropertyDefinition { prop_type: "number".to_string(), description: "R→B contribution. Default 0.0.".to_string(), items: None });
                        props.insert("bg".to_string(), PropertyDefinition { prop_type: "number".to_string(), description: "G→B contribution. Default 0.0.".to_string(), items: None });
                        props.insert("bb".to_string(), PropertyDefinition { prop_type: "number".to_string(), description: "B→B contribution. Default 1.0.".to_string(), items: None });
                        props
                    },
                    required: vec!["input_file".to_string(), "output_file".to_string()],
                },
            },

            FunctionDeclaration {
                name: "apply_atadenoise".to_string(),
                description: "Adaptive temporal averaging denoiser via FFmpeg atadenoise. Great for consistent temporal noise without blurring motion.".to_string(),
                parameters: Parameters {
                    param_type: "object".to_string(),
                    properties: {
                        let mut props = HashMap::new();
                        props.insert("input_file".to_string(), PropertyDefinition { prop_type: "string".to_string(), description: "Input video file".to_string(), items: None });
                        props.insert("output_file".to_string(), PropertyDefinition { prop_type: "string".to_string(), description: "Output file path".to_string(), items: None });
                        props.insert("window_size".to_string(), PropertyDefinition { prop_type: "number".to_string(), description: "Temporal window (odd, 5–129). Default 9.".to_string(), items: None });
                        props.insert("threshold_a".to_string(), PropertyDefinition { prop_type: "number".to_string(), description: "Low threshold 0–1. Default 0.02.".to_string(), items: None });
                        props.insert("threshold_b".to_string(), PropertyDefinition { prop_type: "number".to_string(), description: "High threshold 0–1. Default 0.04.".to_string(), items: None });
                        props.insert("planes".to_string(), PropertyDefinition { prop_type: "number".to_string(), description: "Planes bitmask. Default 7 (YUV).".to_string(), items: None });
                        props
                    },
                    required: vec!["input_file".to_string(), "output_file".to_string()],
                },
            },

            FunctionDeclaration {
                name: "apply_vaguedenoiser".to_string(),
                description: "Wavelet-based video denoiser via FFmpeg vaguedenoiser. Preserves fine detail. Good for broadcast and archival.".to_string(),
                parameters: Parameters {
                    param_type: "object".to_string(),
                    properties: {
                        let mut props = HashMap::new();
                        props.insert("input_file".to_string(), PropertyDefinition { prop_type: "string".to_string(), description: "Input video file".to_string(), items: None });
                        props.insert("output_file".to_string(), PropertyDefinition { prop_type: "string".to_string(), description: "Output file path".to_string(), items: None });
                        props.insert("threshold".to_string(), PropertyDefinition { prop_type: "number".to_string(), description: "Denoising threshold. Default 2.0.".to_string(), items: None });
                        props.insert("method".to_string(), PropertyDefinition { prop_type: "number".to_string(), description: "Method: 0=soft, 1=hard, 2=garrote. Default 0.".to_string(), items: None });
                        props.insert("nsteps".to_string(), PropertyDefinition { prop_type: "number".to_string(), description: "Wavelet steps. Default 6.".to_string(), items: None });
                        props.insert("percent".to_string(), PropertyDefinition { prop_type: "number".to_string(), description: "Denoising percent 0–100. Default 85.".to_string(), items: None });
                        props.insert("planes".to_string(), PropertyDefinition { prop_type: "number".to_string(), description: "Planes bitmask. Default 7.".to_string(), items: None });
                        props
                    },
                    required: vec!["input_file".to_string(), "output_file".to_string()],
                },
            },

            FunctionDeclaration {
                name: "apply_fftdnoiz".to_string(),
                description: "FFT-based video denoiser via FFmpeg fftdnoiz. Excellent for uniform additive noise. Works in frequency domain.".to_string(),
                parameters: Parameters {
                    param_type: "object".to_string(),
                    properties: {
                        let mut props = HashMap::new();
                        props.insert("input_file".to_string(), PropertyDefinition { prop_type: "string".to_string(), description: "Input video file".to_string(), items: None });
                        props.insert("output_file".to_string(), PropertyDefinition { prop_type: "string".to_string(), description: "Output file path".to_string(), items: None });
                        props.insert("sigma".to_string(), PropertyDefinition { prop_type: "number".to_string(), description: "Noise sigma 0.1–30. Default 1.0.".to_string(), items: None });
                        props.insert("amount".to_string(), PropertyDefinition { prop_type: "number".to_string(), description: "Denoising amount 0–1. Default 0.96.".to_string(), items: None });
                        props.insert("block_size".to_string(), PropertyDefinition { prop_type: "number".to_string(), description: "FFT block size (power of 2). Default 32.".to_string(), items: None });
                        props.insert("overlap".to_string(), PropertyDefinition { prop_type: "number".to_string(), description: "Block overlap 0.2–0.8. Default 0.5.".to_string(), items: None });
                        props.insert("planes".to_string(), PropertyDefinition { prop_type: "number".to_string(), description: "Planes bitmask. Default 7.".to_string(), items: None });
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
                name: "measure_siti".to_string(),
                description: "Measures Spatial Information (SI) and Temporal Information (TI) via FFmpeg siti. Analysis only — no output file.".to_string(),
                parameters: Parameters {
                    param_type: "object".to_string(),
                    properties: {
                        let mut props = HashMap::new();
                        props.insert("input_file".to_string(), PropertyDefinition { prop_type: "string".to_string(), description: "Input video file".to_string(), items: None });
                        props
                    },
                    required: vec!["input_file".to_string()],
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

            FunctionDeclaration {
                name: "apply_amplify".to_string(),
                description: "Amplifies pixel changes between frames via FFmpeg amplify. Makes subtle temporal motion visible.".to_string(),
                parameters: Parameters {
                    param_type: "object".to_string(),
                    properties: {
                        let mut props = HashMap::new();
                        props.insert("input_file".to_string(), PropertyDefinition { prop_type: "string".to_string(), description: "Input video file".to_string(), items: None });
                        props.insert("output_file".to_string(), PropertyDefinition { prop_type: "string".to_string(), description: "Output file path".to_string(), items: None });
                        props.insert("radius".to_string(), PropertyDefinition { prop_type: "number".to_string(), description: "Temporal radius frames. Default 2.".to_string(), items: None });
                        props.insert("factor".to_string(), PropertyDefinition { prop_type: "number".to_string(), description: "Amplification factor. Default 2.0.".to_string(), items: None });
                        props.insert("threshold".to_string(), PropertyDefinition { prop_type: "number".to_string(), description: "Min change threshold. Default 10.".to_string(), items: None });
                        props.insert("planes".to_string(), PropertyDefinition { prop_type: "number".to_string(), description: "Planes bitmask. Default 7 (YUV).".to_string(), items: None });
                        props
                    },
                    required: vec!["input_file".to_string(), "output_file".to_string()],
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
                name: "apply_dilation".to_string(),
                description: "Morphological dilation: expands bright regions via FFmpeg dilation filter.".to_string(),
                parameters: Parameters {
                    param_type: "object".to_string(),
                    properties: {
                        let mut props = HashMap::new();
                        props.insert("input_file".to_string(), PropertyDefinition { prop_type: "string".to_string(), description: "Path to the input video file".to_string(), items: None });
                        props.insert("output_file".to_string(), PropertyDefinition { prop_type: "string".to_string(), description: "Path to save the dilation output".to_string(), items: None });
                        props.insert("threshold0".to_string(), PropertyDefinition { prop_type: "number".to_string(), description: "Max change for plane 0. Default 65535.".to_string(), items: None });
                        props.insert("threshold1".to_string(), PropertyDefinition { prop_type: "number".to_string(), description: "Max change for plane 1. Default 65535.".to_string(), items: None });
                        props.insert("threshold2".to_string(), PropertyDefinition { prop_type: "number".to_string(), description: "Max change for plane 2. Default 65535.".to_string(), items: None });
                        props.insert("threshold3".to_string(), PropertyDefinition { prop_type: "number".to_string(), description: "Max change for plane 3. Default 65535.".to_string(), items: None });
                        props.insert("coordinates".to_string(), PropertyDefinition { prop_type: "number".to_string(), description: "8-bit neighbour bitmask. Default 255.".to_string(), items: None });
                        props
                    },
                    required: vec!["input_file".to_string(), "output_file".to_string()],
                },
            },

            FunctionDeclaration {
                name: "apply_erosion".to_string(),
                description: "Morphological erosion: shrinks bright regions via FFmpeg erosion filter.".to_string(),
                parameters: Parameters {
                    param_type: "object".to_string(),
                    properties: {
                        let mut props = HashMap::new();
                        props.insert("input_file".to_string(), PropertyDefinition { prop_type: "string".to_string(), description: "Path to the input video file".to_string(), items: None });
                        props.insert("output_file".to_string(), PropertyDefinition { prop_type: "string".to_string(), description: "Path to save the erosion output".to_string(), items: None });
                        props.insert("threshold0".to_string(), PropertyDefinition { prop_type: "number".to_string(), description: "Max change for plane 0. Default 65535.".to_string(), items: None });
                        props.insert("threshold1".to_string(), PropertyDefinition { prop_type: "number".to_string(), description: "Max change for plane 1. Default 65535.".to_string(), items: None });
                        props.insert("threshold2".to_string(), PropertyDefinition { prop_type: "number".to_string(), description: "Max change for plane 2. Default 65535.".to_string(), items: None });
                        props.insert("threshold3".to_string(), PropertyDefinition { prop_type: "number".to_string(), description: "Max change for plane 3. Default 65535.".to_string(), items: None });
                        props.insert("coordinates".to_string(), PropertyDefinition { prop_type: "number".to_string(), description: "8-bit neighbour bitmask. Default 255.".to_string(), items: None });
                        props
                    },
                    required: vec!["input_file".to_string(), "output_file".to_string()],
                },
            },

            FunctionDeclaration {
                name: "apply_median_filter".to_string(),
                description: "Median filter for salt-and-pepper noise removal via FFmpeg median. Edge-preserving non-linear filter.".to_string(),
                parameters: Parameters {
                    param_type: "object".to_string(),
                    properties: {
                        let mut props = HashMap::new();
                        props.insert("input_file".to_string(), PropertyDefinition { prop_type: "string".to_string(), description: "Path to the input video file".to_string(), items: None });
                        props.insert("output_file".to_string(), PropertyDefinition { prop_type: "string".to_string(), description: "Path to save the filtered output".to_string(), items: None });
                        props.insert("radius".to_string(), PropertyDefinition { prop_type: "number".to_string(), description: "Kernel radius 1–127. Default 1.".to_string(), items: None });
                        props.insert("planes".to_string(), PropertyDefinition { prop_type: "number".to_string(), description: "Planes bitmask. Default 15 (all).".to_string(), items: None });
                        props
                    },
                    required: vec!["input_file".to_string(), "output_file".to_string()],
                },
            },

            FunctionDeclaration {
                name: "apply_histogram_eq".to_string(),
                description: "Global histogram equalisation via FFmpeg histeq. Improves contrast by stretching the luminance range.".to_string(),
                parameters: Parameters {
                    param_type: "object".to_string(),
                    properties: {
                        let mut props = HashMap::new();
                        props.insert("input_file".to_string(), PropertyDefinition { prop_type: "string".to_string(), description: "Path to the input video file".to_string(), items: None });
                        props.insert("output_file".to_string(), PropertyDefinition { prop_type: "string".to_string(), description: "Path to save the equalised output".to_string(), items: None });
                        props.insert("strength".to_string(), PropertyDefinition { prop_type: "number".to_string(), description: "Strength 0–1. Default 0.2.".to_string(), items: None });
                        props.insert("intensity".to_string(), PropertyDefinition { prop_type: "number".to_string(), description: "Intensity 0–1. Default 0.21.".to_string(), items: None });
                        props.insert("antibanding".to_string(), PropertyDefinition { prop_type: "string".to_string(), description: "Anti-banding: none (default), weak, strong.".to_string(), items: None });
                        props
                    },
                    required: vec!["input_file".to_string(), "output_file".to_string()],
                },
            },

            FunctionDeclaration {
                name: "apply_clahe".to_string(),
                description: "CLAHE local contrast enhancement via FFmpeg clahe. Avoids overexposing highlights compared to global histeq.".to_string(),
                parameters: Parameters {
                    param_type: "object".to_string(),
                    properties: {
                        let mut props = HashMap::new();
                        props.insert("input_file".to_string(), PropertyDefinition { prop_type: "string".to_string(), description: "Path to the input video file".to_string(), items: None });
                        props.insert("output_file".to_string(), PropertyDefinition { prop_type: "string".to_string(), description: "Path to save the CLAHE output".to_string(), items: None });
                        props.insert("clip_limit".to_string(), PropertyDefinition { prop_type: "number".to_string(), description: "Clip limit 1–100. Default 25.".to_string(), items: None });
                        props.insert("nb_tiles_x".to_string(), PropertyDefinition { prop_type: "number".to_string(), description: "Horizontal tile count. Default 8.".to_string(), items: None });
                        props.insert("nb_tiles_y".to_string(), PropertyDefinition { prop_type: "number".to_string(), description: "Vertical tile count. Default 8.".to_string(), items: None });
                        props
                    },
                    required: vec!["input_file".to_string(), "output_file".to_string()],
                },
            },

            FunctionDeclaration {
                name: "apply_deblock".to_string(),
                description: "Removes DCT block artefacts from compressed video via FFmpeg deblock. Improves quality of old H.264/MPEG footage.".to_string(),
                parameters: Parameters {
                    param_type: "object".to_string(),
                    properties: {
                        let mut props = HashMap::new();
                        props.insert("input_file".to_string(), PropertyDefinition { prop_type: "string".to_string(), description: "Path to the input video file".to_string(), items: None });
                        props.insert("output_file".to_string(), PropertyDefinition { prop_type: "string".to_string(), description: "Path to save the deblocked output".to_string(), items: None });
                        props.insert("filter_type".to_string(), PropertyDefinition { prop_type: "number".to_string(), description: "Strength level 1–4. Default 4.".to_string(), items: None });
                        props.insert("block_size".to_string(), PropertyDefinition { prop_type: "number".to_string(), description: "Block size px. Default 8.".to_string(), items: None });
                        props.insert("strength".to_string(), PropertyDefinition { prop_type: "number".to_string(), description: "Alpha/beta/gamma/delta 0–1. Default 0.5.".to_string(), items: None });
                        props.insert("planes".to_string(), PropertyDefinition { prop_type: "number".to_string(), description: "Planes bitmask. Default 15 (all).".to_string(), items: None });
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
                name: "apply_convolution".to_string(),
                description: "Custom convolution kernel via FFmpeg convolution filter. Sharpen, blur, emboss, or edge-detect with any NxN matrix.".to_string(),
                parameters: Parameters {
                    param_type: "object".to_string(),
                    properties: {
                        let mut props = HashMap::new();
                        props.insert("input_file".to_string(), PropertyDefinition { prop_type: "string".to_string(), description: "Path to the input video file".to_string(), items: None });
                        props.insert("output_file".to_string(), PropertyDefinition { prop_type: "string".to_string(), description: "Path to save the filtered output".to_string(), items: None });
                        props.insert("matrix".to_string(), PropertyDefinition { prop_type: "string".to_string(), description: "Space-separated kernel values (9 or 25). Default: '0 -1 0 -1 5 -1 0 -1 0' (sharpen).".to_string(), items: None });
                        props.insert("rdiv".to_string(), PropertyDefinition { prop_type: "number".to_string(), description: "Normalisation divisor. Default 1.0.".to_string(), items: None });
                        props.insert("bias".to_string(), PropertyDefinition { prop_type: "number".to_string(), description: "Bias added after convolution. Default 0.".to_string(), items: None });
                        props.insert("planes".to_string(), PropertyDefinition { prop_type: "number".to_string(), description: "Planes bitmask. Default 15 (all).".to_string(), items: None });
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

            FunctionDeclaration {
                name: "measure_silence".to_string(),
                description: "Detects silent segments via FFmpeg silencedetect. Returns silence_start/end/duration timestamps. Analysis only.".to_string(),
                parameters: Parameters {
                    param_type: "object".to_string(),
                    properties: {
                        let mut props = HashMap::new();
                        props.insert("input_file".to_string(), PropertyDefinition { prop_type: "string".to_string(), description: "Path to the input audio/video file".to_string(), items: None });
                        props.insert("noise_db".to_string(), PropertyDefinition { prop_type: "number".to_string(), description: "Noise floor dBFS (negative). Default -30.".to_string(), items: None });
                        props.insert("duration_s".to_string(), PropertyDefinition { prop_type: "number".to_string(), description: "Min silence duration seconds. Default 0.5.".to_string(), items: None });
                        props
                    },
                    required: vec!["input_file".to_string()],
                },
            },

            FunctionDeclaration {
                name: "measure_audio_spectrum".to_string(),
                description: "Renders audio frequency spectrum as a video via FFmpeg showspectrum. Visualise frequency content for EQ decisions.".to_string(),
                parameters: Parameters {
                    param_type: "object".to_string(),
                    properties: {
                        let mut props = HashMap::new();
                        props.insert("input_file".to_string(), PropertyDefinition { prop_type: "string".to_string(), description: "Path to the input audio/video file".to_string(), items: None });
                        props.insert("output_file".to_string(), PropertyDefinition { prop_type: "string".to_string(), description: "Path to save the spectrum video".to_string(), items: None });
                        props.insert("width".to_string(), PropertyDefinition { prop_type: "number".to_string(), description: "Output width px. Default 1024.".to_string(), items: None });
                        props.insert("height".to_string(), PropertyDefinition { prop_type: "number".to_string(), description: "Output height px. Default 512.".to_string(), items: None });
                        props.insert("mode".to_string(), PropertyDefinition { prop_type: "string".to_string(), description: "Display mode: combined (default), separate.".to_string(), items: None });
                        props.insert("color".to_string(), PropertyDefinition { prop_type: "string".to_string(), description: "Colour scheme: intensity (default), fire, moreland, rainbow.".to_string(), items: None });
                        props
                    },
                    required: vec!["input_file".to_string(), "output_file".to_string()],
                },
            },

            // ================================================================
            // PHASE H — Codec / Format Depth
            // ================================================================

            FunctionDeclaration {
                name: "encode_vp9".to_string(),
                description: "Encodes video to VP9 via libvpx-vp9 with Opus audio. Best open codec for web delivery.".to_string(),
                parameters: Parameters {
                    param_type: "object".to_string(),
                    properties: {
                        let mut props = HashMap::new();
                        props.insert("input_file".to_string(), PropertyDefinition { prop_type: "string".to_string(), description: "Path to the input video file".to_string(), items: None });
                        props.insert("output_file".to_string(), PropertyDefinition { prop_type: "string".to_string(), description: "Path to save the VP9 output (.webm or .mkv)".to_string(), items: None });
                        props.insert("crf".to_string(), PropertyDefinition { prop_type: "number".to_string(), description: "Constant quality (0–63). Default 31.".to_string(), items: None });
                        props.insert("bitrate".to_string(), PropertyDefinition { prop_type: "string".to_string(), description: "Target bitrate e.g. '2M'. Leave empty for CRF-only.".to_string(), items: None });
                        props.insert("speed".to_string(), PropertyDefinition { prop_type: "number".to_string(), description: "CPU speed preset (0–5). Default 2.".to_string(), items: None });
                        props.insert("threads".to_string(), PropertyDefinition { prop_type: "number".to_string(), description: "Thread count. Default 4.".to_string(), items: None });
                        props
                    },
                    required: vec!["input_file".to_string(), "output_file".to_string()],
                },
            },

            FunctionDeclaration {
                name: "encode_av1".to_string(),
                description: "Encodes video to AV1 via libaom-av1 or libsvtav1. ~30% better compression than VP9/H.265.".to_string(),
                parameters: Parameters {
                    param_type: "object".to_string(),
                    properties: {
                        let mut props = HashMap::new();
                        props.insert("input_file".to_string(), PropertyDefinition { prop_type: "string".to_string(), description: "Path to the input video file".to_string(), items: None });
                        props.insert("output_file".to_string(), PropertyDefinition { prop_type: "string".to_string(), description: "Path to save the AV1 output (.webm or .mkv)".to_string(), items: None });
                        props.insert("crf".to_string(), PropertyDefinition { prop_type: "number".to_string(), description: "Constant quality (0–63). Default 30.".to_string(), items: None });
                        props.insert("speed".to_string(), PropertyDefinition { prop_type: "number".to_string(), description: "CPU speed preset (0–8 libaom, 0–12 svtav1). Default 4.".to_string(), items: None });
                        props.insert("threads".to_string(), PropertyDefinition { prop_type: "number".to_string(), description: "Thread count. Default 4.".to_string(), items: None });
                        props.insert("encoder".to_string(), PropertyDefinition { prop_type: "string".to_string(), description: "AV1 encoder: libaom-av1 or libsvtav1. Default libaom-av1.".to_string(), items: None });
                        props
                    },
                    required: vec!["input_file".to_string(), "output_file".to_string()],
                },
            },

            FunctionDeclaration {
                name: "encode_hevc".to_string(),
                description: "Encodes video to H.265/HEVC via libx265. ~50% better compression than H.264 at equivalent quality.".to_string(),
                parameters: Parameters {
                    param_type: "object".to_string(),
                    properties: {
                        let mut props = HashMap::new();
                        props.insert("input_file".to_string(), PropertyDefinition { prop_type: "string".to_string(), description: "Path to the input video file".to_string(), items: None });
                        props.insert("output_file".to_string(), PropertyDefinition { prop_type: "string".to_string(), description: "Path to save the HEVC output (.mp4 or .mkv)".to_string(), items: None });
                        props.insert("crf".to_string(), PropertyDefinition { prop_type: "number".to_string(), description: "Constant Rate Factor (0–51). Default 28.".to_string(), items: None });
                        props.insert("preset".to_string(), PropertyDefinition { prop_type: "string".to_string(), description: "Speed preset: ultrafast…veryslow. Default medium.".to_string(), items: None });
                        props.insert("tune".to_string(), PropertyDefinition { prop_type: "string".to_string(), description: "Tuning: grain, zerolatency, fastdecode, animation. Leave empty for default.".to_string(), items: None });
                        props
                    },
                    required: vec!["input_file".to_string(), "output_file".to_string()],
                },
            },

            FunctionDeclaration {
                name: "encode_opus".to_string(),
                description: "Encodes audio to Opus via libopus. Best-in-class lossy audio codec for streaming and music.".to_string(),
                parameters: Parameters {
                    param_type: "object".to_string(),
                    properties: {
                        let mut props = HashMap::new();
                        props.insert("input_file".to_string(), PropertyDefinition { prop_type: "string".to_string(), description: "Path to the input audio or video file".to_string(), items: None });
                        props.insert("output_file".to_string(), PropertyDefinition { prop_type: "string".to_string(), description: "Path to save the Opus audio output (.opus or .ogg)".to_string(), items: None });
                        props.insert("bitrate_kbps".to_string(), PropertyDefinition { prop_type: "number".to_string(), description: "Target bitrate kbps (6–510). Default 128.".to_string(), items: None });
                        props.insert("vbr".to_string(), PropertyDefinition { prop_type: "string".to_string(), description: "VBR mode: true or false. Default true.".to_string(), items: None });
                        props.insert("compression".to_string(), PropertyDefinition { prop_type: "number".to_string(), description: "Compression level (0–10). Default 10.".to_string(), items: None });
                        props
                    },
                    required: vec!["input_file".to_string(), "output_file".to_string()],
                },
            },

            FunctionDeclaration {
                name: "encode_hdr10".to_string(),
                description: "Encodes video to HDR10 via libx265 with mastering display and MaxCLL/MaxFALL metadata.".to_string(),
                parameters: Parameters {
                    param_type: "object".to_string(),
                    properties: {
                        let mut props = HashMap::new();
                        props.insert("input_file".to_string(), PropertyDefinition { prop_type: "string".to_string(), description: "Path to the input HDR video file".to_string(), items: None });
                        props.insert("output_file".to_string(), PropertyDefinition { prop_type: "string".to_string(), description: "Path to save the HDR10 output".to_string(), items: None });
                        props.insert("crf".to_string(), PropertyDefinition { prop_type: "number".to_string(), description: "CRF (0–51). Default 22.".to_string(), items: None });
                        props.insert("preset".to_string(), PropertyDefinition { prop_type: "string".to_string(), description: "x265 preset. Default slow.".to_string(), items: None });
                        props.insert("master_display".to_string(), PropertyDefinition { prop_type: "string".to_string(), description: "Mastering display primaries string. Leave empty for Rec.2020 defaults.".to_string(), items: None });
                        props.insert("max_cll".to_string(), PropertyDefinition { prop_type: "string".to_string(), description: "MaxCLL,MaxFALL e.g. '1000,400'. Leave empty for default.".to_string(), items: None });
                        props
                    },
                    required: vec!["input_file".to_string(), "output_file".to_string()],
                },
            },

            FunctionDeclaration {
                name: "encode_nvenc".to_string(),
                description: "Hardware-accelerated encoding via NVIDIA NVENC (CUDA GPU). Extremely fast with near-software quality.".to_string(),
                parameters: Parameters {
                    param_type: "object".to_string(),
                    properties: {
                        let mut props = HashMap::new();
                        props.insert("input_file".to_string(), PropertyDefinition { prop_type: "string".to_string(), description: "Path to the input video file".to_string(), items: None });
                        props.insert("output_file".to_string(), PropertyDefinition { prop_type: "string".to_string(), description: "Path to save the NVENC output".to_string(), items: None });
                        props.insert("codec".to_string(), PropertyDefinition { prop_type: "string".to_string(), description: "Codec: h264 (default), hevc, av1.".to_string(), items: None });
                        props.insert("preset".to_string(), PropertyDefinition { prop_type: "string".to_string(), description: "NVENC preset p1 (fastest)–p7 (best). Default p4.".to_string(), items: None });
                        props.insert("bitrate".to_string(), PropertyDefinition { prop_type: "string".to_string(), description: "Target bitrate e.g. '8M'. Leave empty to use CQ only.".to_string(), items: None });
                        props.insert("cq".to_string(), PropertyDefinition { prop_type: "number".to_string(), description: "Constant quality (0–51). Default 23.".to_string(), items: None });
                        props
                    },
                    required: vec!["input_file".to_string(), "output_file".to_string()],
                },
            },

            FunctionDeclaration {
                name: "encode_vaapi".to_string(),
                description: "Hardware-accelerated encoding via Intel/AMD VAAPI GPU on Linux.".to_string(),
                parameters: Parameters {
                    param_type: "object".to_string(),
                    properties: {
                        let mut props = HashMap::new();
                        props.insert("input_file".to_string(), PropertyDefinition { prop_type: "string".to_string(), description: "Path to the input video file".to_string(), items: None });
                        props.insert("output_file".to_string(), PropertyDefinition { prop_type: "string".to_string(), description: "Path to save the VAAPI output".to_string(), items: None });
                        props.insert("codec".to_string(), PropertyDefinition { prop_type: "string".to_string(), description: "Codec: h264 (default), hevc, vp9, av1.".to_string(), items: None });
                        props.insert("quality".to_string(), PropertyDefinition { prop_type: "number".to_string(), description: "QP value (1–51). Lower = better. Default 23.".to_string(), items: None });
                        props.insert("profile".to_string(), PropertyDefinition { prop_type: "string".to_string(), description: "Encoding profile: high (default), main, baseline.".to_string(), items: None });
                        props
                    },
                    required: vec!["input_file".to_string(), "output_file".to_string()],
                },
            },

            FunctionDeclaration {
                name: "encode_qsv".to_string(),
                description: "Hardware-accelerated encoding via Intel Quick Sync Video (QSV). Very fast on Intel integrated and Arc GPUs.".to_string(),
                parameters: Parameters {
                    param_type: "object".to_string(),
                    properties: {
                        let mut props = HashMap::new();
                        props.insert("input_file".to_string(), PropertyDefinition { prop_type: "string".to_string(), description: "Path to the input video file".to_string(), items: None });
                        props.insert("output_file".to_string(), PropertyDefinition { prop_type: "string".to_string(), description: "Path to save the QSV output".to_string(), items: None });
                        props.insert("codec".to_string(), PropertyDefinition { prop_type: "string".to_string(), description: "Codec: h264 (default), hevc, av1, vp9.".to_string(), items: None });
                        props.insert("preset".to_string(), PropertyDefinition { prop_type: "string".to_string(), description: "Speed preset: veryfast…veryslow. Default medium.".to_string(), items: None });
                        props.insert("bitrate".to_string(), PropertyDefinition { prop_type: "string".to_string(), description: "Target bitrate e.g. '6M'. Leave empty for auto.".to_string(), items: None });
                        props
                    },
                    required: vec!["input_file".to_string(), "output_file".to_string()],
                },
            },

            FunctionDeclaration {
                name: "encode_prores".to_string(),
                description: "Encodes video to Apple ProRes via prores_ks. Professional intermediate codec for editing workflows.".to_string(),
                parameters: Parameters {
                    param_type: "object".to_string(),
                    properties: {
                        let mut props = HashMap::new();
                        props.insert("input_file".to_string(), PropertyDefinition { prop_type: "string".to_string(), description: "Path to the input video file".to_string(), items: None });
                        props.insert("output_file".to_string(), PropertyDefinition { prop_type: "string".to_string(), description: "Path to save the ProRes output (.mov)".to_string(), items: None });
                        props.insert("profile".to_string(), PropertyDefinition { prop_type: "number".to_string(), description: "ProRes profile: 0=Proxy, 1=LT, 2=Standard, 3=HQ (default), 4=4444, 5=4444XQ.".to_string(), items: None });
                        props.insert("vendor".to_string(), PropertyDefinition { prop_type: "string".to_string(), description: "4-char vendor tag. Default 'apl0'.".to_string(), items: None });
                        props
                    },
                    required: vec!["input_file".to_string(), "output_file".to_string()],
                },
            },

            FunctionDeclaration {
                name: "encode_dnxhd".to_string(),
                description: "Encodes video to Avid DNxHD/DNxHR. Professional intermediate codec for Avid Media Composer and post-production.".to_string(),
                parameters: Parameters {
                    param_type: "object".to_string(),
                    properties: {
                        let mut props = HashMap::new();
                        props.insert("input_file".to_string(), PropertyDefinition { prop_type: "string".to_string(), description: "Path to the input video file".to_string(), items: None });
                        props.insert("output_file".to_string(), PropertyDefinition { prop_type: "string".to_string(), description: "Path to save the DNxHD/DNxHR output (.mxf or .mov)".to_string(), items: None });
                        props.insert("profile".to_string(), PropertyDefinition { prop_type: "string".to_string(), description: "DNxHR profile: dnxhr_lb, dnxhr_sq (default), dnxhr_hq, dnxhr_hqx, dnxhr_444.".to_string(), items: None });
                        props
                    },
                    required: vec!["input_file".to_string(), "output_file".to_string()],
                },
            },

            FunctionDeclaration {
                name: "encode_gif".to_string(),
                description: "Creates high-quality animated GIF using FFmpeg 2-pass palette optimisation. Much better quality than naive GIF conversion.".to_string(),
                parameters: Parameters {
                    param_type: "object".to_string(),
                    properties: {
                        let mut props = HashMap::new();
                        props.insert("input_file".to_string(), PropertyDefinition { prop_type: "string".to_string(), description: "Path to the input video file".to_string(), items: None });
                        props.insert("output_file".to_string(), PropertyDefinition { prop_type: "string".to_string(), description: "Path to save the animated GIF".to_string(), items: None });
                        props.insert("fps".to_string(), PropertyDefinition { prop_type: "number".to_string(), description: "Frames per second. Default 15.".to_string(), items: None });
                        props.insert("scale".to_string(), PropertyDefinition { prop_type: "number".to_string(), description: "Output width in pixels. Default 480.".to_string(), items: None });
                        props.insert("loop_count".to_string(), PropertyDefinition { prop_type: "number".to_string(), description: "Loop count: 0=infinite (default), -1=no loop, N=loop N times.".to_string(), items: None });
                        props
                    },
                    required: vec!["input_file".to_string(), "output_file".to_string()],
                },
            },

            FunctionDeclaration {
                name: "encode_webm".to_string(),
                description: "Encodes video to WebM container (VP8/VP9 + Vorbis/Opus). Open web format supported by all modern browsers.".to_string(),
                parameters: Parameters {
                    param_type: "object".to_string(),
                    properties: {
                        let mut props = HashMap::new();
                        props.insert("input_file".to_string(), PropertyDefinition { prop_type: "string".to_string(), description: "Path to the input video file".to_string(), items: None });
                        props.insert("output_file".to_string(), PropertyDefinition { prop_type: "string".to_string(), description: "Path to save the WebM output".to_string(), items: None });
                        props.insert("video_codec".to_string(), PropertyDefinition { prop_type: "string".to_string(), description: "Video codec: vp8 (default, fast) or vp9 (better quality).".to_string(), items: None });
                        props.insert("audio_codec".to_string(), PropertyDefinition { prop_type: "string".to_string(), description: "Audio codec: vorbis (default) or opus.".to_string(), items: None });
                        props.insert("crf".to_string(), PropertyDefinition { prop_type: "number".to_string(), description: "CRF quality. VP8: 4–63 (default 10), VP9: 0–63 (default 31).".to_string(), items: None });
                        props.insert("bitrate".to_string(), PropertyDefinition { prop_type: "string".to_string(), description: "Target bitrate e.g. '1M'. Leave empty for CRF-only.".to_string(), items: None });
                        props
                    },
                    required: vec!["input_file".to_string(), "output_file".to_string()],
                },
            },

            // ================================================================
            // PHASE G — AI/ML Filters
            // ================================================================

            FunctionDeclaration {
                name: "detect_objects_dnn".to_string(),
                description: "Runs DNN-based object detection on a video using FFmpeg dnn_detect. Draws bounding boxes around detected objects.".to_string(),
                parameters: Parameters {
                    param_type: "object".to_string(),
                    properties: {
                        let mut props = HashMap::new();
                        props.insert("input_file".to_string(), PropertyDefinition { prop_type: "string".to_string(), description: "Path to the input video file".to_string(), items: None });
                        props.insert("output_file".to_string(), PropertyDefinition { prop_type: "string".to_string(), description: "Path to save the annotated output video".to_string(), items: None });
                        props.insert("model".to_string(), PropertyDefinition { prop_type: "string".to_string(), description: "Path to DNN model file (yolov3.weights or model.onnx)".to_string(), items: None });
                        props.insert("backend".to_string(), PropertyDefinition { prop_type: "string".to_string(), description: "DNN backend: native, openvino, tensorflow, pytorch, onnx. Default 'native'.".to_string(), items: None });
                        props.insert("confidence".to_string(), PropertyDefinition { prop_type: "number".to_string(), description: "Minimum confidence threshold (0–1). Default 0.5.".to_string(), items: None });
                        props.insert("labels".to_string(), PropertyDefinition { prop_type: "string".to_string(), description: "Optional path to labels file".to_string(), items: None });
                        props
                    },
                    required: vec!["input_file".to_string(), "output_file".to_string(), "model".to_string()],
                },
            },

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

            FunctionDeclaration {
                name: "detect_frozen_frames".to_string(),
                description: "Detects frozen/stuck frames in a video using FFmpeg freezedetect. Returns timestamps and durations of freeze events. Analysis-only.".to_string(),
                parameters: Parameters {
                    param_type: "object".to_string(),
                    properties: {
                        let mut props = HashMap::new();
                        props.insert("input_file".to_string(), PropertyDefinition { prop_type: "string".to_string(), description: "Path to the input video file to analyze".to_string(), items: None });
                        props.insert("noise_db".to_string(), PropertyDefinition { prop_type: "number".to_string(), description: "Noise threshold in dB (negative). Default -60.".to_string(), items: None });
                        props.insert("duration".to_string(), PropertyDefinition { prop_type: "number".to_string(), description: "Minimum freeze duration in seconds. Default 2.0.".to_string(), items: None });
                        props
                    },
                    required: vec!["input_file".to_string()],
                },
            },

            FunctionDeclaration {
                name: "apply_edgedetect".to_string(),
                description: "Detects and visualises edges in a video using FFmpeg edgedetect filter. Useful for stylised looks, motion analysis, and visual effects.".to_string(),
                parameters: Parameters {
                    param_type: "object".to_string(),
                    properties: {
                        let mut props = HashMap::new();
                        props.insert("input_file".to_string(), PropertyDefinition { prop_type: "string".to_string(), description: "Path to the input video file".to_string(), items: None });
                        props.insert("output_file".to_string(), PropertyDefinition { prop_type: "string".to_string(), description: "Path to save the edge-detected output video".to_string(), items: None });
                        props.insert("low".to_string(), PropertyDefinition { prop_type: "number".to_string(), description: "Low hysteresis threshold (0–1). Default 0.0625.".to_string(), items: None });
                        props.insert("high".to_string(), PropertyDefinition { prop_type: "number".to_string(), description: "High hysteresis threshold (0–1). Default 0.1875.".to_string(), items: None });
                        props.insert("mode".to_string(), PropertyDefinition { prop_type: "string".to_string(), description: "Edge mode: wires, colormix, canny. Default 'wires'.".to_string(), items: None });
                        props
                    },
                    required: vec!["input_file".to_string(), "output_file".to_string()],
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
        ]
    }

    /// Filter tools by name (for dynamic tool selection)
    /// Returns only the tools whose names are in the provided list
    pub fn filter_tools_by_name(tool_names: &[String]) -> Vec<FunctionDeclaration> {
        let all_tools = Self::create_video_editing_tools();
        all_tools
            .into_iter()
            .filter(|tool| tool_names.contains(&tool.name))
            .collect()
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
                    Part::Text { text: analysis_prompt.to_string() },
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
                                    if let Some(data) = inline_data.get("data").and_then(|d| d.as_str()) {
                                        // Decode base64 image data
                                        use base64::{Engine as _, engine::general_purpose};
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
            let error_text = response.text().await.unwrap_or_else(|_| "Unknown error".to_string());
            Err(format!("Gemini Image API error ({}): {}", status, error_text).into())
        }
    }

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
            let error_text = response.text().await.unwrap_or_else(|_| "Unknown error".to_string());
            tracing::error!("Gemini API error ({}): {}", status, error_text);
        }

        // Fallback SVG if API call fails
        tracing::info!("Using fallback default SVG");
        Ok(self.create_default_svg())
    }

    fn create_svg_from_description(&self, description: &str) -> String {
        let mut rng = rand::thread_rng();
        
        // Extract colors from description or use defaults
        let colors = if description.contains("#") {
            vec!["#667eea", "#764ba2", "#3498db", "#2980b9"]
        } else {
            vec!["#667eea", "#764ba2", "#3498db", "#2980b9", "#8e44ad", "#2c3e50"]
        };

        let primary_color = colors[rng.gen_range(0..colors.len())];
        let secondary_color = colors[rng.gen_range(0..colors.len())];
        
        // Generate random shapes and positions
        let circles = (0..5).map(|_| {
            format!(
                r#"<circle cx="{}" cy="{}" r="{}" fill="{}" opacity="0.{}"/>"#,
                rng.gen_range(0..1920),
                rng.gen_range(0..1080),
                rng.gen_range(50..200),
                colors[rng.gen_range(0..colors.len())],
                rng.gen_range(1..4)
            )
        }).collect::<Vec<_>>().join("\n        ");

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
        let circles = (0..rng.gen_range(3..7)).map(|_| {
            format!(
                r#"<circle cx="{}" cy="{}" r="{}" fill="{}" opacity="0.{}"/>"#,
                rng.gen_range(100..1820),
                rng.gen_range(100..980),
                rng.gen_range(40..150),
                palette[rng.gen_range(0..palette.len())],
                rng.gen_range(1..4)
            )
        }).collect::<Vec<_>>().join("\n        ");

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
            rng.gen_range(0..30), rng.gen_range(0..30), // gradient start
            rng.gen_range(70..100), rng.gen_range(70..100), // gradient end
            primary_color,
            rng.gen_range(40..60), // middle stop
            palette[rng.gen_range(0..palette.len())],
            secondary_color,
            rng.gen_range(2..5), // blur amount
            circles, rectangles,
            // Timeline elements
            rng.gen_range(80..200), rng.gen_range(80..200),
            rng.gen_range(80..200), rng.gen_range(110..230),
            rng.gen_range(80..200), rng.gen_range(140..260),
            // Play button
            rng.gen_range(1600..1800), rng.gen_range(150..300),
            // Play triangle
            rng.gen_range(1590..1790), rng.gen_range(140..290),
            rng.gen_range(1610..1810), rng.gen_range(150..300),
            rng.gen_range(1590..1790), rng.gen_range(160..310),
            // Waveform
            rng.gen_range(1400..1500), rng.gen_range(800..900), rng.gen_range(20..60),
            rng.gen_range(1410..1510), rng.gen_range(820..920), rng.gen_range(15..45),
            rng.gen_range(1420..1520), rng.gen_range(810..910), rng.gen_range(25..65),
        )
    }

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
</svg>"#.to_string()
    }

    pub fn create_background_image_prompt(theme: &str) -> String {
        let prompts = vec![
            format!("Create a modern, abstract background image for a video editing application with {} theme. Include subtle geometric shapes, gradients in purple and blue tones, and video-related iconography like film strips, play buttons, or waveforms. Make it professional and clean with a tech aesthetic.", theme),
            format!("Design a creative background with {} style showing video editing concepts. Include abstract representations of timelines, video frames, color grading elements, and modern UI elements. Use a color palette of deep blues, purples, and subtle accents. Keep it minimalist and sophisticated.", theme),
            format!("Generate a {} themed background for a video editing platform. Show artistic representations of creativity tools like cameras, editing interfaces, sound waves, and light effects. Use gradients and modern design elements with a professional color scheme of blues and purples.", theme),
            format!("Create a {} style background featuring video production elements. Include abstract film reels, digital effects, color gradients, and modern tech aesthetics. Make it suitable for a professional video editing application with clean, contemporary design.", theme),
        ];

        let themes = vec![
            "cinematic", "creative", "professional", "artistic", "modern", 
            "tech-focused", "minimalist", "dynamic", "elegant", "innovative"
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
            Be specific and detailed in your analysis as if you're truly watching the video.".to_string()
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
                        tracing::info!("Successfully analyzed video content using Gemini 2.5 Flash");
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
        style_prompt: Option<&str>
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
            request_body["generationConfig"]["speechConfig"]["languageCode"] = serde_json::Value::String(language_code.to_string());
        }

        // Add style prompt if provided
        if let Some(prompt) = style_prompt {
            request_body["systemInstruction"] = serde_json::json!({
                "parts": [{
                    "text": format!("Generate speech with the following style and tone: {}", prompt)
                }]
            });
        }

        tracing::info!("🎵 Generating speech audio for text: '{}' with voice: {}", 
                      &text[..text.len().min(100)], voice);

        let response = self.client
            .post(&format!("{}/v1beta/models/gemini-2.5-flash:generateContent", self.base_url))
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
        tracing::debug!("Gemini TTS response: {}", serde_json::to_string_pretty(&response_json)?);

        // Extract audio data from response
        if let Some(candidates) = response_json["candidates"].as_array() {
            if let Some(candidate) = candidates.get(0) {
                if let Some(content) = candidate.get("content") {
                    if let Some(parts) = content["parts"].as_array() {
                        for part in parts {
                            if let Some(inline_data) = part.get("inlineData") {
                                if let Some(data) = inline_data["data"].as_str() {
                                    // Decode base64 audio data
                                    let audio_data = base64::prelude::BASE64_STANDARD
                                        .decode(data)
                                        .map_err(|e| format!("Failed to decode audio data: {}", e))?;
                                    
                                    tracing::info!("✅ Generated {} bytes of audio data", audio_data.len());
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
        style: Option<&str>
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
            duration_seconds, company_name, product_description,
            audience, ad_style, duration_seconds, duration_seconds * 3, // ~3 words per second
            company_name, product_description, duration_seconds, audience, ad_style
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

        tracing::info!("🎬 Generating advertisement script for {} ({}s duration)", company_name, duration_seconds);

        let response = self.generate_content(request).await?;
        
        // Extract the script text from response
        if let Some(candidate) = response.candidates.first() {
            if let Some(ref content) = candidate.content {
                if let Some(part) = content.parts.first() {
                    if let Part::Text { text } = part {
                        tracing::info!("✅ Generated {}-word script for {} second ad", text.split_whitespace().count(), duration_seconds);
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
        style: Option<&str>
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
                    duration_seconds, subject, description,
                    audience, video_style, duration_seconds, duration_seconds * 2,
                    subject, description, duration_seconds, audience, video_style
                )
            },
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
                    duration_seconds, subject, description,
                    audience, video_style, duration_seconds, duration_seconds * 3,
                    subject, description, duration_seconds, audience, video_style
                )
            },
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
                    duration_seconds, subject, description,
                    audience, video_style, duration_seconds, duration_seconds * 3,
                    subject, description, duration_seconds, audience, video_style
                )
            },
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
                    duration_seconds, subject, description,
                    audience, video_style, duration_seconds, duration_seconds * 3,
                    subject, description, duration_seconds, audience, video_style
                )
            },
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
                    duration_seconds, video_type, subject, description,
                    audience, video_style, duration_seconds, duration_seconds * 3,
                    subject, description, video_type, duration_seconds, audience, video_style
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

        tracing::info!("🎬 Generating {} video script for '{}' ({}s duration)", video_type, subject, duration_seconds);

        let response = self.generate_content(request).await?;
        
        // Extract the script text from response
        if let Some(candidate) = response.candidates.first() {
            if let Some(ref content) = candidate.content {
                if let Some(part) = content.parts.first() {
                    if let Part::Text { text } = part {
                        tracing::info!("✅ Generated {}-word {} script for {} second video",
                                      text.split_whitespace().count(), video_type, duration_seconds);
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
    ) -> Result<crate::clipping::gemini_video_analyzer::VideoAnalysis, Box<dyn std::error::Error + Send + Sync>> {
        // Acquire concurrency permit for this expensive video analysis call.
        let _permit = self.semaphore.acquire().await
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
                tracing::debug!("Gemini video analysis response (first 1000 chars): {}", &response_text[..response_text.len().min(1000)]);

                // Parse Gemini response wrapper
                let response_json: serde_json::Value = serde_json::from_str(&response_text)
                    .map_err(|e| format!("Failed to parse Gemini response: {} — body: {}", e, &response_text[..response_text.len().min(500)]))?;

                // Extract text from candidates[0].content.parts[0].text
                let text = response_json["candidates"][0]["content"]["parts"][0]["text"]
                    .as_str()
                    .ok_or("Gemini response missing text content")?;

                // Parse the JSON text content as VideoAnalysis
                let analysis: crate::clipping::gemini_video_analyzer::VideoAnalysis =
                    serde_json::from_str(text)
                        .map_err(|e| format!("Failed to parse VideoAnalysis JSON: {} — text: {}", e, &text[..text.len().min(1000)]))?;

                tracing::info!(
                    "✅ Video analysis complete: {} viral moments identified (overall quality: {:.2})",
                    analysis.viral_moments.len(),
                    analysis.overall_quality
                );

                return Ok(analysis);
            }

            let error_text = response.text().await.unwrap_or_else(|_| "Unknown error".to_string());

            // On 429 rate limit, back off and retry
            if status.as_u16() == 429 && attempt < max_attempts - 1 {
                let retry_secs = parse_gemini_retry_delay(&error_text, 30.0);
                let wait_secs = (retry_secs + 5.0) as u64;
                tracing::warn!("⏳ Gemini rate limited (429, attempt {}/{}). Waiting {}s…", attempt + 1, max_attempts, wait_secs);
                tokio::time::sleep(tokio::time::Duration::from_secs(wait_secs)).await;
                last_error = format!("Gemini rate limited: {}", error_text).into();
                continue;
            }

            last_error = format!("Gemini API error (HTTP {}): {}", status, error_text).into();

            if attempt < max_attempts - 1 {
                tracing::warn!("Gemini attempt {}/{} failed, retrying: {}", attempt + 1, max_attempts, last_error);
                tokio::time::sleep(tokio::time::Duration::from_secs(5)).await;
            }
        }

        Err(last_error)
    }

    /// Analyze a locally downloaded video file by extracting JPEG frames and sending them
    /// to Gemini as a multi-image request.
    ///
    /// Used for Twitch VODs (and any non-YouTube source) where Gemini's fileData URI path
    /// only supports YouTube URLs. Produces the same `VideoAnalysis` schema as
    /// `analyze_video_from_url`.
    ///
    /// Frame count: 1 frame per 2 minutes of footage, clamped 8–20.
    /// Each frame is labeled with its timestamp so Gemini can report accurate `start_sec`/`end_sec`.
    pub async fn analyze_video_from_local_file(
        &self,
        video_path: &str,
        clips_per_video: usize,
        min_duration_secs: f64,
        max_duration_secs: f64,
        high_performing_factors: &[String],
    ) -> Result<crate::clipping::gemini_video_analyzer::VideoAnalysis, Box<dyn std::error::Error + Send + Sync>> {
        let _permit = self
            .semaphore
            .acquire()
            .await
            .map_err(|e| format!("Gemini semaphore error: {}", e))?;

        tracing::info!(
            "🎬 Analyzing local video via extracted frames: {}",
            video_path
        );

        // Get total duration via ffprobe
        let total_dur = crate::core::get_video_duration(video_path)
            .map_err(|e| format!("Failed to get video duration for '{}': {}", video_path, e))?;

        // 1 frame per 2 minutes, clamped 8–20
        let num_frames = ((total_dur / 120.0).round() as usize).clamp(8, 20);

        // Extract frames at evenly spaced timestamps
        let mut frame_paths: Vec<String> = Vec::new();
        let mut frame_timestamps: Vec<f64> = Vec::new();
        for i in 0..num_frames {
            let ts = if num_frames == 1 {
                total_dur * 0.5
            } else {
                total_dur * (i as f64 / (num_frames - 1) as f64)
            };
            let ts = ts.clamp(0.1, total_dur - 0.1);
            let path = crate::utils::ffmpeg_utils::create_temp_file(
                &format!("local_analysis_frame_{}", i),
                "jpg",
            );
            match crate::utils::ffmpeg_utils::extract_frame_at_timestamp(video_path, ts, &path) {
                Ok(p) => {
                    frame_paths.push(p);
                    frame_timestamps.push(ts);
                }
                Err(e) => {
                    tracing::warn!(
                        "Frame {}/{} extraction at {:.1}s failed (skipping): {}",
                        i + 1,
                        num_frames,
                        ts,
                        e
                    );
                }
            }
        }

        if frame_paths.is_empty() {
            return Err(format!(
                "Failed to extract any frames from '{}' ({:.0}s)",
                video_path, total_dur
            )
            .into());
        }

        // Read frame bytes then immediately clean up temp files
        let mut frame_data: Vec<(f64, Vec<u8>)> = Vec::new();
        for (path, ts) in frame_paths.iter().zip(frame_timestamps.iter()) {
            match tokio::fs::read(path).await {
                Ok(bytes) => frame_data.push((*ts, bytes)),
                Err(e) => tracing::warn!("Failed to read frame {}: {}", path, e),
            }
        }
        crate::utils::ffmpeg_utils::cleanup_temp_files(&frame_paths);

        if frame_data.is_empty() {
            return Err("Failed to read any frame data from extracted frames".into());
        }

        tracing::info!(
            "📸 Extracted {}/{} frames for Gemini analysis (video: {:.0}s)",
            frame_data.len(),
            num_frames,
            total_dur
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
            r#"You are analyzing a video via {n} sampled frames. Total duration: {total_dur:.0}s ({total_min:.1} minutes).

Each frame is labeled with its exact timestamp [t=Xs]. Use these timestamps to estimate accurate start/end times for viral clips.

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
            n = frame_data.len(),
            total_dur = total_dur,
            total_min = total_dur / 60.0,
            clips_per_video = clips_per_video,
            min_dur = min_duration_secs,
            max_dur = max_duration_secs,
            learned_hint = learned_factors_hint,
        );

        // Build parts array: timestamp context, then interleaved [label, image] pairs, then prompt
        let mut parts: Vec<serde_json::Value> = Vec::new();

        let frame_label_list: Vec<String> = frame_data
            .iter()
            .map(|(ts, _)| format!("[t={:.1}s]", ts))
            .collect();
        parts.push(serde_json::json!({
            "text": format!("Frame timestamps in order: {}", frame_label_list.join(", "))
        }));

        for (ts, bytes) in &frame_data {
            parts.push(serde_json::json!({ "text": format!("[t={:.1}s]", ts) }));
            parts.push(serde_json::json!({
                "inlineData": {
                    "mimeType": "image/jpeg",
                    "data": BASE64_STANDARD.encode(bytes)
                }
            }));
        }

        parts.push(serde_json::json!({ "text": prompt }));

        let request_body = serde_json::json!({
            "contents": [{"role": "user", "parts": parts}],
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
                            "Failed to parse VideoAnalysis JSON from local-file analysis: {} — text: {}",
                            e,
                            &text[..text.len().min(500)]
                        )
                    })?;

                tracing::info!(
                    "✅ Local-file analysis complete: {} viral moments (quality: {:.2})",
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
                    "⏳ Gemini rate limited (local-file analysis, attempt {}/{}). Waiting {}s…",
                    attempt + 1, max_attempts, wait_secs
                );
                tokio::time::sleep(tokio::time::Duration::from_secs(wait_secs)).await;
                last_error = format!("Rate limited: {}", error_text).into();
                continue;
            }

            last_error = format!("Gemini API error (HTTP {}): {}", status, error_text).into();

            if attempt < max_attempts - 1 {
                tracing::warn!(
                    "Local-file analysis attempt {}/{} failed, retrying: {}",
                    attempt + 1, max_attempts, last_error
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
        let _permit = self.semaphore.acquire().await
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
                    attempt + 1, max_attempts, wait
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
