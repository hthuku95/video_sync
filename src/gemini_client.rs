use reqwest::Client;
use serde::{Deserialize, Serialize};
use serde_json::Value;
use std::collections::HashMap;
use rand::Rng;
use base64::prelude::*;

#[derive(Debug, Clone)]
pub struct GeminiClient {
    client: Client,
    api_key: String,
    base_url: String,
}

#[derive(Debug, Serialize, Deserialize)]
pub struct GenerateContentRequest {
    pub contents: Vec<Content>,
    pub tools: Option<Vec<Tool>>,
    #[serde(rename = "generationConfig")]
    pub generation_config: Option<GenerationConfig>,
    #[serde(rename = "toolConfig", skip_serializing_if = "Option::is_none")]
    pub tool_config: Option<ToolConfig>,
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

#[derive(Debug, Serialize, Deserialize)]
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

#[derive(Debug, Serialize, Deserialize)]
pub struct GenerationConfig {
    pub temperature: f32,
    #[serde(rename = "topK")]
    pub top_k: u32,
    #[serde(rename = "topP")]
    pub top_p: f32,
    #[serde(rename = "maxOutputTokens")]
    pub max_output_tokens: u32,
}

#[derive(Debug, Serialize, Deserialize)]
pub struct ToolConfig {
    #[serde(rename = "functionCallingConfig")]
    pub function_calling_config: FunctionCallingConfig,
}

#[derive(Debug, Serialize, Deserialize)]
pub struct FunctionCallingConfig {
    pub mode: FunctionCallingMode,
}

#[derive(Debug, Serialize, Deserialize)]
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
pub struct ImageGenerationRequest {
    pub prompt: String,
    pub model: String,
    pub output_mime_type: String,
}

#[derive(Debug, Serialize, Deserialize)]
pub struct ImageGenerationResponse {
    pub candidates: Vec<ImageCandidate>,
}

#[derive(Debug, Serialize, Deserialize)]
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
        }
    }

    pub async fn generate_content(
        &self,
        request: GenerateContentRequest,
    ) -> Result<GenerateContentResponse, Box<dyn std::error::Error + Send + Sync>> {
        let url = format!(
            "{}/models/gemini-2.5-flash:generateContent?key={}",
            self.base_url, self.api_key
        );

        // Debug: Log the request to see if thought signatures are present
        if let Ok(request_json) = serde_json::to_string_pretty(&request) {
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

        let response = self
            .client
            .post(&url)
            .header("Content-Type", "application/json")
            .json(&request)
            .send()
            .await?;

        if response.status().is_success() {
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
                    Ok(result)
                },
                Err(parse_error) => {
                    tracing::error!("Failed to parse Gemini response: {}", parse_error);
                    tracing::error!("Response body: {}", response_text);
                    Err(format!("error decoding response body: {}", parse_error).into())
                }
            }
        } else {
            let error_text = response.text().await?;
            Err(format!("Gemini API error: {}", error_text).into())
        }
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

    /// Generate an image using Nano Banana Pro (Gemini 3 Pro Image Preview)
    pub async fn generate_image(
        &self,
        prompt: &str,
        aspect_ratio: Option<&str>,
        image_size: Option<&str>,
    ) -> Result<Vec<u8>, Box<dyn std::error::Error + Send + Sync>> {
        // Build the request with image configuration
        let mut config_map = serde_json::Map::new();

        // Add image_config
        let mut image_config = serde_json::Map::new();
        image_config.insert(
            "aspect_ratio".to_string(),
            serde_json::Value::String(aspect_ratio.unwrap_or("1:1").to_string())
        );
        image_config.insert(
            "image_size".to_string(),
            serde_json::Value::String(image_size.unwrap_or("2K").to_string())
        );
        config_map.insert("image_config".to_string(), serde_json::Value::Object(image_config));

        // Response modalities
        config_map.insert(
            "response_modalities".to_string(),
            serde_json::Value::Array(vec![
                serde_json::Value::String("IMAGE".to_string())
            ])
        );

        let request = serde_json::json!({
            "contents": [{
                "parts": [{
                    "text": prompt
                }],
                "role": "user"
            }],
            "generationConfig": config_map
        });

        let url = format!(
            "{}/models/gemini-2.5-flash:generateContent?key={}",
            self.base_url, self.api_key
        );

        tracing::debug!("Nano Banana Pro Image Request: {}", serde_json::to_string_pretty(&request)?);

        let response = self
            .client
            .post(&url)
            .header("Content-Type", "application/json")
            .json(&request)
            .send()
            .await?;

        if response.status().is_success() {
            let response_text = response.text().await?;
            tracing::debug!("Nano Banana Pro response: {}", response_text);

            let response_json: serde_json::Value = serde_json::from_str(&response_text)?;

            // Extract base64 image from response
            if let Some(candidates) = response_json["candidates"].as_array() {
                if let Some(candidate) = candidates.first() {
                    if let Some(content) = candidate.get("content") {
                        if let Some(parts) = content["parts"].as_array() {
                            for part in parts {
                                if let Some(inline_data) = part.get("inlineData") {
                                    if let Some(data) = inline_data["data"].as_str() {
                                        // Decode base64 image
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
            Err(format!("Nano Banana Pro API error: {}", error_text).into())
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
                description: "Generates an image using Google's Nano Banana Pro (Gemini 3 Pro Image) model based on a text prompt. Supports high-resolution (2K, 4K) image generation with advanced text rendering. Use this to create custom images, overlays, backgrounds, or any visual elements needed for video editing.".to_string(),
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
                        props
                    },
                    required: vec!["prompt".to_string(), "output_file".to_string()],
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
                            description: "Whether to add background music (default: false)".to_string(),
                            items: None,
                        });
                        props.insert("num_clips".to_string(), PropertyDefinition {
                            prop_type: "number".to_string(),
                            description: "Number of video clips to use from Pexels (default: 3-5 based on duration)".to_string(),
                            items: None,
                        });
                        props
                    },
                    required: vec!["topic".to_string(), "output_file".to_string()],
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
        ]
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

        let encoded_data = base64::encode(&video_data);
        
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
        frame_interval_seconds: Option<f64>,
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
}