use backoff::{future::retry, ExponentialBackoff};
use reqwest::Client;
use serde::{Deserialize, Serialize};
use serde_json::{json, Value};
use std::collections::HashMap;
use std::time::Duration;

#[derive(Debug, Clone)]
pub struct ClaudeClient {
    client: Client,
    api_key: String,
    base_url: String,
    model: String,
}

#[derive(Debug, Serialize, Deserialize)]
pub struct ClaudeRequest {
    pub model: String,
    pub max_tokens: u32,
    pub messages: Vec<ClaudeMessage>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub system: Option<String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub tools: Option<Vec<ClaudeTool>>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub temperature: Option<f32>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub tool_choice: Option<ToolChoice>,
}

#[derive(Debug, Serialize, Deserialize, Clone)]
#[serde(tag = "type")]
pub enum ToolChoice {
    #[serde(rename = "auto")]
    Auto,
    #[serde(rename = "any")]
    Any,
    #[serde(rename = "tool")]
    Tool { name: String },
}

#[derive(Debug, Serialize, Deserialize, Clone)]
pub struct ClaudeMessage {
    pub role: String,
    pub content: ClaudeContent,
}

#[derive(Debug, Serialize, Deserialize, Clone)]
#[serde(untagged)]
pub enum ClaudeContent {
    Text(String),
    Blocks(Vec<ContentBlock>),
}

#[derive(Debug, Serialize, Deserialize, Clone)]
#[serde(tag = "type")]
pub enum ContentBlock {
    #[serde(rename = "text")]
    Text { text: String },
    #[serde(rename = "image")]
    Image { source: ImageSource },
    #[serde(rename = "tool_use")]
    ToolUse {
        id: String,
        name: String,
        input: Value,
    },
    #[serde(rename = "tool_result")]
    ToolResult {
        tool_use_id: String,
        content: String,
        #[serde(skip_serializing_if = "Option::is_none")]
        is_error: Option<bool>,
    },
}

#[derive(Debug, Serialize, Deserialize, Clone)]
pub struct ImageSource {
    #[serde(rename = "type")]
    pub source_type: String,
    pub media_type: String,
    pub data: String,
}

#[derive(Debug, Serialize, Deserialize, Clone)]
pub struct ClaudeTool {
    pub name: String,
    pub description: String,
    pub input_schema: InputSchema,
}

#[derive(Debug, Serialize, Deserialize, Clone)]
pub struct InputSchema {
    #[serde(rename = "type")]
    pub schema_type: String,
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
pub struct ClaudeResponse {
    pub id: String,
    pub model: String,
    pub role: String,
    pub content: Vec<ResponseContent>,
    pub stop_reason: Option<String>,
    pub usage: Usage,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(tag = "type")]
pub enum ResponseContent {
    #[serde(rename = "text")]
    Text { text: String },
    #[serde(rename = "tool_use")]
    ToolUse {
        id: String,
        name: String,
        input: Value,
    },
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct Usage {
    pub input_tokens: u32,
    pub output_tokens: u32,
}

impl ClaudeClient {
    pub fn new(api_key: String) -> Self {
        Self {
            client: Client::new(),
            api_key,
            base_url: "https://api.anthropic.com/v1".to_string(),
            model: "claude-sonnet-4-5".to_string(),
        }
    }

    pub async fn generate_content(
        &self,
        messages: Vec<ClaudeMessage>,
        tools: Option<Vec<ClaudeTool>>,
        system: Option<String>,
    ) -> Result<ClaudeResponse, String> {
        // Let Claude decide when to use tools (Auto mode)
        // This allows natural conversation for greetings/questions
        // Claude will call tools when needed for video editing tasks
        let tool_choice = if tools.is_some() {
            Some(ToolChoice::Auto) // Auto allows Claude to respond normally or call tools as needed
        } else {
            None
        };

        let request = ClaudeRequest {
            model: self.model.clone(),
            max_tokens: 8192,
            messages,
            system,
            tools,
            temperature: Some(0.7),
            tool_choice,
        };

        tracing::debug!(
            "Claude API Request: {} tools provided",
            request.tools.as_ref().map(|t| t.len()).unwrap_or(0)
        );
        tracing::debug!(
            "Claude API Request messages count: {}",
            request.messages.len()
        );

        // Configure exponential backoff for retries
        let backoff_config = ExponentialBackoff {
            initial_interval: Duration::from_secs(1),
            max_interval: Duration::from_secs(30),
            multiplier: 2.0,
            max_elapsed_time: Some(Duration::from_secs(300)), // 5 minutes total retry time
            ..Default::default()
        };

        // Retry logic for transient errors (503, 502, connection errors)
        let operation = || async {
            let response = self
                .client
                .post(format!("{}/messages", self.base_url))
                .header("x-api-key", &self.api_key)
                .header("anthropic-version", "2023-06-01")
                .header("content-type", "application/json")
                .timeout(Duration::from_secs(120)) // 2-minute timeout per request
                .json(&request)
                .send()
                .await
                .map_err(|e| {
                    // Check if it's a connection/timeout error that should be retried
                    if e.is_connect() || e.is_timeout() {
                        tracing::warn!("Claude API connection error (retrying): {}", e);
                        backoff::Error::transient(format!("Connection error: {}", e))
                    } else {
                        tracing::error!("Claude API permanent error: {}", e);
                        backoff::Error::permanent(format!("Request error: {}", e))
                    }
                })?;

            let status = response.status();
            let response_text = response.text().await.map_err(|e| {
                backoff::Error::permanent(format!("Failed to read response: {}", e))
            })?;

            tracing::debug!("Claude API Response (status {}): {}", status, response_text);

            // Retry on 503, 502, 429 (rate limit), 500 errors
            if status.as_u16() == 503
                || status.as_u16() == 502
                || status.as_u16() == 429
                || status.as_u16() == 500
            {
                tracing::warn!(
                    "Claude API returned {} (retrying): {}",
                    status,
                    response_text
                );
                return Err(backoff::Error::transient(format!(
                    "API error ({}): {}",
                    status, response_text
                )));
            }

            if !status.is_success() {
                tracing::error!("Claude API permanent error ({}): {}", status, response_text);
                return Err(backoff::Error::permanent(format!(
                    "API error ({}): {}",
                    status, response_text
                )));
            }

            serde_json::from_str(&response_text).map_err(|e| {
                backoff::Error::permanent(format!(
                    "Failed to parse response: {}. Response: {}",
                    e, response_text
                ))
            })
        };

        // Execute with retry
        match retry(backoff_config, operation).await {
            Ok(response) => Ok(response),
            Err(e) => Err(e),
        }
    }

    pub async fn generate_text(&self, prompt: &str) -> Result<String, String> {
        let messages = vec![ClaudeMessage {
            role: "user".to_string(),
            content: ClaudeContent::Text(prompt.to_string()),
        }];

        let response = self.generate_content(messages, None, None).await?;

        // Extract text from response
        for content in response.content {
            if let ResponseContent::Text { text } = content {
                return Ok(text);
            }
        }

        Err("No text content in Claude response".to_string())
    }

    pub async fn generate_embeddings(&self, texts: Vec<String>) -> Result<Vec<Vec<f32>>, String> {
        // Claude doesn't have native embeddings API
        // We'll use Voyage AI embeddings (compatible with Claude)
        // For now, return a placeholder implementation
        // You can integrate voyage-ai-rust or similar

        tracing::warn!("Claude embeddings not implemented yet, using placeholder");

        // Return dummy embeddings for now (768 dimensions to match Gemini)
        Ok(texts.iter().map(|_| vec![0.0; 768]).collect())
    }

    pub fn create_video_editing_tools() -> Vec<ClaudeTool> {
        vec![
            ClaudeTool {
                name: "trim_video".to_string(),
                description: "Trims a video to specified start and end times".to_string(),
                input_schema: InputSchema {
                    schema_type: "object".to_string(),
                    properties: HashMap::from([
                        ("input_file".to_string(), PropertyDefinition {
                            prop_type: "string".to_string(),
                            description: "Path to the input video file".to_string(),
                            items: None,
                        }),
                        ("output_file".to_string(), PropertyDefinition {
                            prop_type: "string".to_string(),
                            description: "Path to save the trimmed video".to_string(),
                            items: None,
                        }),
                        ("start_seconds".to_string(), PropertyDefinition {
                            prop_type: "number".to_string(),
                            description: "Start time in seconds".to_string(),
                            items: None,
                        }),
                        ("end_seconds".to_string(), PropertyDefinition {
                            prop_type: "number".to_string(),
                            description: "End time in seconds".to_string(),
                            items: None,
                        }),
                    ]),
                    required: vec!["input_file".to_string(), "output_file".to_string(), "start_seconds".to_string(), "end_seconds".to_string()],
                },
            },
            ClaudeTool {
                name: "merge_videos".to_string(),
                description: "Merges multiple video files into a single video".to_string(),
                input_schema: InputSchema {
                    schema_type: "object".to_string(),
                    properties: HashMap::from([
                        ("input_files".to_string(), PropertyDefinition {
                            prop_type: "array".to_string(),
                            description: "Array of input video file paths".to_string(),
                            items: Some(Box::new(PropertyDefinition {
                                prop_type: "string".to_string(),
                                description: "Video file path".to_string(),
                                items: None,
                            })),
                        }),
                        ("output_file".to_string(), PropertyDefinition {
                            prop_type: "string".to_string(),
                            description: "Path to save the merged video".to_string(),
                            items: None,
                        }),
                    ]),
                    required: vec!["input_files".to_string(), "output_file".to_string()],
                },
            },
            ClaudeTool {
                name: "analyze_video".to_string(),
                description: "Analyzes a video file and returns metadata".to_string(),
                input_schema: InputSchema {
                    schema_type: "object".to_string(),
                    properties: HashMap::from([
                        ("input_file".to_string(), PropertyDefinition {
                            prop_type: "string".to_string(),
                            description: "Path to the video file to analyze".to_string(),
                            items: None,
                        }),
                    ]),
                    required: vec!["input_file".to_string()],
                },
            },
            ClaudeTool {
                name: "add_text_overlay".to_string(),
                description: "Adds text overlay to a video at specified position".to_string(),
                input_schema: InputSchema {
                    schema_type: "object".to_string(),
                    properties: HashMap::from([
                        ("input_file".to_string(), PropertyDefinition {
                            prop_type: "string".to_string(),
                            description: "Path to the input video file".to_string(),
                            items: None,
                        }),
                        ("output_file".to_string(), PropertyDefinition {
                            prop_type: "string".to_string(),
                            description: "Path to save the video with text overlay".to_string(),
                            items: None,
                        }),
                        ("text".to_string(), PropertyDefinition {
                            prop_type: "string".to_string(),
                            description: "The text to overlay on the video".to_string(),
                            items: None,
                        }),
                        ("x".to_string(), PropertyDefinition {
                            prop_type: "number".to_string(),
                            description: "X position of the text".to_string(),
                            items: None,
                        }),
                        ("y".to_string(), PropertyDefinition {
                            prop_type: "number".to_string(),
                            description: "Y position of the text".to_string(),
                            items: None,
                        }),
                        ("font_size".to_string(), PropertyDefinition {
                            prop_type: "number".to_string(),
                            description: "Font size (default: 24)".to_string(),
                            items: None,
                        }),
                        ("color".to_string(), PropertyDefinition {
                            prop_type: "string".to_string(),
                            description: "Text color (default: white)".to_string(),
                            items: None,
                        }),
                    ]),
                    required: vec!["input_file".to_string(), "output_file".to_string(), "text".to_string(), "x".to_string(), "y".to_string()],
                },
            },
            ClaudeTool {
                name: "resize_video".to_string(),
                description: "Resizes a video to specified dimensions".to_string(),
                input_schema: InputSchema {
                    schema_type: "object".to_string(),
                    properties: HashMap::from([
                        ("input_file".to_string(), PropertyDefinition {
                            prop_type: "string".to_string(),
                            description: "Path to the input video file".to_string(),
                            items: None,
                        }),
                        ("output_file".to_string(), PropertyDefinition {
                            prop_type: "string".to_string(),
                            description: "Path to save the resized video".to_string(),
                            items: None,
                        }),
                        ("width".to_string(), PropertyDefinition {
                            prop_type: "number".to_string(),
                            description: "Target width in pixels".to_string(),
                            items: None,
                        }),
                        ("height".to_string(), PropertyDefinition {
                            prop_type: "number".to_string(),
                            description: "Target height in pixels".to_string(),
                            items: None,
                        }),
                    ]),
                    required: vec!["input_file".to_string(), "output_file".to_string(), "width".to_string(), "height".to_string()],
                },
            },
            ClaudeTool {
                name: "convert_format".to_string(),
                description: "Converts a video from one format to another".to_string(),
                input_schema: InputSchema {
                    schema_type: "object".to_string(),
                    properties: HashMap::from([
                        ("input_file".to_string(), PropertyDefinition {
                            prop_type: "string".to_string(),
                            description: "Path to the input video file".to_string(),
                            items: None,
                        }),
                        ("output_file".to_string(), PropertyDefinition {
                            prop_type: "string".to_string(),
                            description: "Path to save the converted video".to_string(),
                            items: None,
                        }),
                        ("format".to_string(), PropertyDefinition {
                            prop_type: "string".to_string(),
                            description: "Target format (e.g., mp4, avi, mov, webm)".to_string(),
                            items: None,
                        }),
                    ]),
                    required: vec!["input_file".to_string(), "output_file".to_string(), "format".to_string()],
                },
            },
            ClaudeTool {
                name: "adjust_volume".to_string(),
                description: "Adjusts the audio volume of a video".to_string(),
                input_schema: InputSchema {
                    schema_type: "object".to_string(),
                    properties: HashMap::from([
                        ("input_file".to_string(), PropertyDefinition {
                            prop_type: "string".to_string(),
                            description: "Path to the input video file".to_string(),
                            items: None,
                        }),
                        ("output_file".to_string(), PropertyDefinition {
                            prop_type: "string".to_string(),
                            description: "Path to save the video with adjusted volume".to_string(),
                            items: None,
                        }),
                        ("volume_factor".to_string(), PropertyDefinition {
                            prop_type: "number".to_string(),
                            description: "Volume multiplier (1.0 = original, 0.5 = half, 2.0 = double)".to_string(),
                            items: None,
                        }),
                    ]),
                    required: vec!["input_file".to_string(), "output_file".to_string(), "volume_factor".to_string()],
                },
            },
            ClaudeTool {
                name: "apply_filter".to_string(),
                description: "Applies visual filters to a video including grayscale (black and white), sepia, blur, sharpen, vintage, brightness, contrast, and saturation filters".to_string(),
                input_schema: InputSchema {
                    schema_type: "object".to_string(),
                    properties: HashMap::from([
                        ("input_file".to_string(), PropertyDefinition {
                            prop_type: "string".to_string(),
                            description: "Path to the input video file".to_string(),
                            items: None,
                        }),
                        ("output_file".to_string(), PropertyDefinition {
                            prop_type: "string".to_string(),
                            description: "Path to save the filtered video".to_string(),
                            items: None,
                        }),
                        ("filter_type".to_string(), PropertyDefinition {
                            prop_type: "string".to_string(),
                            description: "Type of filter to apply: 'grayscale' (black and white), 'sepia', 'blur', 'sharpen', 'vintage', 'brightness', 'contrast', 'saturation'".to_string(),
                            items: None,
                        }),
                        ("intensity".to_string(), PropertyDefinition {
                            prop_type: "number".to_string(),
                            description: "Filter intensity from 0.0 to 1.0 (default: 1.0)".to_string(),
                            items: None,
                        }),
                    ]),
                    required: vec!["input_file".to_string(), "output_file".to_string(), "filter_type".to_string()],
                },
            },
            ClaudeTool {
                name: "split_video".to_string(),
                description: "Splits a video into multiple segments of specified duration".to_string(),
                input_schema: InputSchema {
                    schema_type: "object".to_string(),
                    properties: HashMap::from([
                        ("input_file".to_string(), PropertyDefinition {
                            prop_type: "string".to_string(),
                            description: "Path to the input video file".to_string(),
                            items: None,
                        }),
                        ("output_prefix".to_string(), PropertyDefinition {
                            prop_type: "string".to_string(),
                            description: "Prefix for output segment files".to_string(),
                            items: None,
                        }),
                        ("segment_duration".to_string(), PropertyDefinition {
                            prop_type: "number".to_string(),
                            description: "Duration of each segment in seconds".to_string(),
                            items: None,
                        }),
                    ]),
                    required: vec!["input_file".to_string(), "output_prefix".to_string(), "segment_duration".to_string()],
                },
            },
            ClaudeTool {
                name: "crop_video".to_string(),
                description: "Crops a video to specified dimensions and position".to_string(),
                input_schema: InputSchema {
                    schema_type: "object".to_string(),
                    properties: HashMap::from([
                        ("input_file".to_string(), PropertyDefinition {
                            prop_type: "string".to_string(),
                            description: "Path to the input video file".to_string(),
                            items: None,
                        }),
                        ("output_file".to_string(), PropertyDefinition {
                            prop_type: "string".to_string(),
                            description: "Path to save the cropped video".to_string(),
                            items: None,
                        }),
                        ("x".to_string(), PropertyDefinition {
                            prop_type: "number".to_string(),
                            description: "X coordinate of crop area".to_string(),
                            items: None,
                        }),
                        ("y".to_string(), PropertyDefinition {
                            prop_type: "number".to_string(),
                            description: "Y coordinate of crop area".to_string(),
                            items: None,
                        }),
                        ("width".to_string(), PropertyDefinition {
                            prop_type: "number".to_string(),
                            description: "Width of crop area".to_string(),
                            items: None,
                        }),
                        ("height".to_string(), PropertyDefinition {
                            prop_type: "number".to_string(),
                            description: "Height of crop area".to_string(),
                            items: None,
                        }),
                    ]),
                    required: vec!["input_file".to_string(), "output_file".to_string(), "x".to_string(), "y".to_string(), "width".to_string(), "height".to_string()],
                },
            },
            ClaudeTool {
                name: "rotate_video".to_string(),
                description: "Rotates a video by specified degrees".to_string(),
                input_schema: InputSchema {
                    schema_type: "object".to_string(),
                    properties: HashMap::from([
                        ("input_file".to_string(), PropertyDefinition {
                            prop_type: "string".to_string(),
                            description: "Path to the input video file".to_string(),
                            items: None,
                        }),
                        ("output_file".to_string(), PropertyDefinition {
                            prop_type: "string".to_string(),
                            description: "Path to save the rotated video".to_string(),
                            items: None,
                        }),
                        ("degrees".to_string(), PropertyDefinition {
                            prop_type: "number".to_string(),
                            description: "Rotation angle in degrees (90, 180, 270)".to_string(),
                            items: None,
                        }),
                    ]),
                    required: vec!["input_file".to_string(), "output_file".to_string(), "degrees".to_string()],
                },
            },
            ClaudeTool {
                name: "adjust_speed".to_string(),
                description: "Adjusts the playback speed of a video".to_string(),
                input_schema: InputSchema {
                    schema_type: "object".to_string(),
                    properties: HashMap::from([
                        ("input_file".to_string(), PropertyDefinition {
                            prop_type: "string".to_string(),
                            description: "Path to the input video file".to_string(),
                            items: None,
                        }),
                        ("output_file".to_string(), PropertyDefinition {
                            prop_type: "string".to_string(),
                            description: "Path to save the speed-adjusted video".to_string(),
                            items: None,
                        }),
                        ("speed_factor".to_string(), PropertyDefinition {
                            prop_type: "number".to_string(),
                            description: "Speed multiplier (0.5 = half speed, 2.0 = double speed)".to_string(),
                            items: None,
                        }),
                    ]),
                    required: vec!["input_file".to_string(), "output_file".to_string(), "speed_factor".to_string()],
                },
            },
            ClaudeTool {
                name: "flip_video".to_string(),
                description: "Flips a video horizontally or vertically".to_string(),
                input_schema: InputSchema {
                    schema_type: "object".to_string(),
                    properties: HashMap::from([
                        ("input_file".to_string(), PropertyDefinition {
                            prop_type: "string".to_string(),
                            description: "Path to the input video file".to_string(),
                            items: None,
                        }),
                        ("output_file".to_string(), PropertyDefinition {
                            prop_type: "string".to_string(),
                            description: "Path to save the flipped video".to_string(),
                            items: None,
                        }),
                        ("direction".to_string(), PropertyDefinition {
                            prop_type: "string".to_string(),
                            description: "Flip direction: 'horizontal' or 'vertical'".to_string(),
                            items: None,
                        }),
                    ]),
                    required: vec!["input_file".to_string(), "output_file".to_string(), "direction".to_string()],
                },
            },
            ClaudeTool {
                name: "add_overlay".to_string(),
                description: "Adds an image or video overlay on top of the main video".to_string(),
                input_schema: InputSchema {
                    schema_type: "object".to_string(),
                    properties: HashMap::from([
                        ("input_file".to_string(), PropertyDefinition {
                            prop_type: "string".to_string(),
                            description: "Path to the input video file".to_string(),
                            items: None,
                        }),
                        ("output_file".to_string(), PropertyDefinition {
                            prop_type: "string".to_string(),
                            description: "Path to save the video with overlay".to_string(),
                            items: None,
                        }),
                        ("overlay_file".to_string(), PropertyDefinition {
                            prop_type: "string".to_string(),
                            description: "Path to the overlay image or video file".to_string(),
                            items: None,
                        }),
                        ("x".to_string(), PropertyDefinition {
                            prop_type: "number".to_string(),
                            description: "X position of the overlay".to_string(),
                            items: None,
                        }),
                        ("y".to_string(), PropertyDefinition {
                            prop_type: "number".to_string(),
                            description: "Y position of the overlay".to_string(),
                            items: None,
                        }),
                    ]),
                    required: vec!["input_file".to_string(), "output_file".to_string(), "overlay_file".to_string(), "x".to_string(), "y".to_string()],
                },
            },
            ClaudeTool {
                name: "extract_audio".to_string(),
                description: "Extracts audio track from a video file".to_string(),
                input_schema: InputSchema {
                    schema_type: "object".to_string(),
                    properties: HashMap::from([
                        ("input_file".to_string(), PropertyDefinition {
                            prop_type: "string".to_string(),
                            description: "Path to the input video file".to_string(),
                            items: None,
                        }),
                        ("output_file".to_string(), PropertyDefinition {
                            prop_type: "string".to_string(),
                            description: "Path to save the extracted audio".to_string(),
                            items: None,
                        }),
                        ("format".to_string(), PropertyDefinition {
                            prop_type: "string".to_string(),
                            description: "Audio format (mp3, wav, aac, etc.)".to_string(),
                            items: None,
                        }),
                    ]),
                    required: vec!["input_file".to_string(), "output_file".to_string(), "format".to_string()],
                },
            },
            ClaudeTool {
                name: "add_audio".to_string(),
                description: "Adds an audio track to a video or replaces existing audio".to_string(),
                input_schema: InputSchema {
                    schema_type: "object".to_string(),
                    properties: HashMap::from([
                        ("input_file".to_string(), PropertyDefinition {
                            prop_type: "string".to_string(),
                            description: "Path to the input video file".to_string(),
                            items: None,
                        }),
                        ("output_file".to_string(), PropertyDefinition {
                            prop_type: "string".to_string(),
                            description: "Path to save the video with new audio".to_string(),
                            items: None,
                        }),
                        ("audio_file".to_string(), PropertyDefinition {
                            prop_type: "string".to_string(),
                            description: "Path to the audio file to add".to_string(),
                            items: None,
                        }),
                        ("replace".to_string(), PropertyDefinition {
                            prop_type: "boolean".to_string(),
                            description: "Whether to replace existing audio (true) or mix (false)".to_string(),
                            items: None,
                        }),
                    ]),
                    required: vec!["input_file".to_string(), "output_file".to_string(), "audio_file".to_string()],
                },
            },
            ClaudeTool {
                name: "fade_audio".to_string(),
                description: "Applies fade in/out effects to video audio".to_string(),
                input_schema: InputSchema {
                    schema_type: "object".to_string(),
                    properties: HashMap::from([
                        ("input_file".to_string(), PropertyDefinition {
                            prop_type: "string".to_string(),
                            description: "Path to the input video file".to_string(),
                            items: None,
                        }),
                        ("output_file".to_string(), PropertyDefinition {
                            prop_type: "string".to_string(),
                            description: "Path to save the video with fade effect".to_string(),
                            items: None,
                        }),
                        ("fade_in_duration".to_string(), PropertyDefinition {
                            prop_type: "number".to_string(),
                            description: "Fade in duration in seconds (0 for no fade in)".to_string(),
                            items: None,
                        }),
                        ("fade_out_duration".to_string(), PropertyDefinition {
                            prop_type: "number".to_string(),
                            description: "Fade out duration in seconds (0 for no fade out)".to_string(),
                            items: None,
                        }),
                    ]),
                    required: vec!["input_file".to_string(), "output_file".to_string(), "fade_in_duration".to_string(), "fade_out_duration".to_string()],
                },
            },
            ClaudeTool {
                name: "compress_video".to_string(),
                description: "Compresses a video to reduce file size while maintaining quality".to_string(),
                input_schema: InputSchema {
                    schema_type: "object".to_string(),
                    properties: HashMap::from([
                        ("input_file".to_string(), PropertyDefinition {
                            prop_type: "string".to_string(),
                            description: "Path to the input video file".to_string(),
                            items: None,
                        }),
                        ("output_file".to_string(), PropertyDefinition {
                            prop_type: "string".to_string(),
                            description: "Path to save the compressed video".to_string(),
                            items: None,
                        }),
                        ("quality".to_string(), PropertyDefinition {
                            prop_type: "string".to_string(),
                            description: "Compression quality: 'high', 'medium', 'low'".to_string(),
                            items: None,
                        }),
                    ]),
                    required: vec!["input_file".to_string(), "output_file".to_string(), "quality".to_string()],
                },
            },
            ClaudeTool {
                name: "export_for_platform".to_string(),
                description: "Exports video optimized for specific platforms (YouTube, Instagram, TikTok, etc.)".to_string(),
                input_schema: InputSchema {
                    schema_type: "object".to_string(),
                    properties: HashMap::from([
                        ("input_file".to_string(), PropertyDefinition {
                            prop_type: "string".to_string(),
                            description: "Path to the input video file".to_string(),
                            items: None,
                        }),
                        ("output_file".to_string(), PropertyDefinition {
                            prop_type: "string".to_string(),
                            description: "Path to save the platform-optimized video".to_string(),
                            items: None,
                        }),
                        ("platform".to_string(), PropertyDefinition {
                            prop_type: "string".to_string(),
                            description: "Target platform: 'youtube', 'instagram', 'tiktok', 'twitter', 'facebook'".to_string(),
                            items: None,
                        }),
                    ]),
                    required: vec!["input_file".to_string(), "output_file".to_string(), "platform".to_string()],
                },
            },
            ClaudeTool {
                name: "picture_in_picture".to_string(),
                description: "Creates a picture-in-picture effect with two video sources".to_string(),
                input_schema: InputSchema {
                    schema_type: "object".to_string(),
                    properties: HashMap::from([
                        ("main_video".to_string(), PropertyDefinition {
                            prop_type: "string".to_string(),
                            description: "Path to the main background video".to_string(),
                            items: None,
                        }),
                        ("pip_video".to_string(), PropertyDefinition {
                            prop_type: "string".to_string(),
                            description: "Path to the picture-in-picture video".to_string(),
                            items: None,
                        }),
                        ("output_file".to_string(), PropertyDefinition {
                            prop_type: "string".to_string(),
                            description: "Path to save the PiP video".to_string(),
                            items: None,
                        }),
                        ("x".to_string(), PropertyDefinition {
                            prop_type: "number".to_string(),
                            description: "X position of the PiP window".to_string(),
                            items: None,
                        }),
                        ("y".to_string(), PropertyDefinition {
                            prop_type: "number".to_string(),
                            description: "Y position of the PiP window".to_string(),
                            items: None,
                        }),
                        ("scale".to_string(), PropertyDefinition {
                            prop_type: "number".to_string(),
                            description: "Scale factor for PiP window (0.1 to 1.0)".to_string(),
                            items: None,
                        }),
                    ]),
                    required: vec!["main_video".to_string(), "pip_video".to_string(), "output_file".to_string(), "x".to_string(), "y".to_string(), "scale".to_string()],
                },
            },
            ClaudeTool {
                name: "chroma_key".to_string(),
                description: "Applies chroma key (green screen) effect to replace background".to_string(),
                input_schema: InputSchema {
                    schema_type: "object".to_string(),
                    properties: HashMap::from([
                        ("input_file".to_string(), PropertyDefinition {
                            prop_type: "string".to_string(),
                            description: "Path to the input video with green screen".to_string(),
                            items: None,
                        }),
                        ("background_file".to_string(), PropertyDefinition {
                            prop_type: "string".to_string(),
                            description: "Path to the background video or image".to_string(),
                            items: None,
                        }),
                        ("output_file".to_string(), PropertyDefinition {
                            prop_type: "string".to_string(),
                            description: "Path to save the chroma key video".to_string(),
                            items: None,
                        }),
                        ("key_color".to_string(), PropertyDefinition {
                            prop_type: "string".to_string(),
                            description: "Color to key out (default: green)".to_string(),
                            items: None,
                        }),
                        ("similarity".to_string(), PropertyDefinition {
                            prop_type: "number".to_string(),
                            description: "Color similarity threshold (0.0 to 1.0)".to_string(),
                            items: None,
                        }),
                    ]),
                    required: vec!["input_file".to_string(), "background_file".to_string(), "output_file".to_string()],
                },
            },
            ClaudeTool {
                name: "split_screen".to_string(),
                description: "Creates a split screen effect with multiple video sources".to_string(),
                input_schema: InputSchema {
                    schema_type: "object".to_string(),
                    properties: HashMap::from([
                        ("video1".to_string(), PropertyDefinition {
                            prop_type: "string".to_string(),
                            description: "Path to the first video".to_string(),
                            items: None,
                        }),
                        ("video2".to_string(), PropertyDefinition {
                            prop_type: "string".to_string(),
                            description: "Path to the second video".to_string(),
                            items: None,
                        }),
                        ("output_file".to_string(), PropertyDefinition {
                            prop_type: "string".to_string(),
                            description: "Path to save the split screen video".to_string(),
                            items: None,
                        }),
                        ("orientation".to_string(), PropertyDefinition {
                            prop_type: "string".to_string(),
                            description: "Split orientation: 'horizontal' or 'vertical'".to_string(),
                            items: None,
                        }),
                    ]),
                    required: vec!["video1".to_string(), "video2".to_string(), "output_file".to_string(), "orientation".to_string()],
                },
            },
            ClaudeTool {
                name: "scale_video".to_string(),
                description: "Scales a video by a specific factor while maintaining aspect ratio".to_string(),
                input_schema: InputSchema {
                    schema_type: "object".to_string(),
                    properties: HashMap::from([
                        ("input_file".to_string(), PropertyDefinition {
                            prop_type: "string".to_string(),
                            description: "Path to the input video file".to_string(),
                            items: None,
                        }),
                        ("output_file".to_string(), PropertyDefinition {
                            prop_type: "string".to_string(),
                            description: "Path to save the scaled video".to_string(),
                            items: None,
                        }),
                        ("scale_factor".to_string(), PropertyDefinition {
                            prop_type: "number".to_string(),
                            description: "Scale factor (0.5 = half size, 2.0 = double size)".to_string(),
                            items: None,
                        }),
                    ]),
                    required: vec!["input_file".to_string(), "output_file".to_string(), "scale_factor".to_string()],
                },
            },
            ClaudeTool {
                name: "stabilize_video".to_string(),
                description: "Applies video stabilization to reduce camera shake".to_string(),
                input_schema: InputSchema {
                    schema_type: "object".to_string(),
                    properties: HashMap::from([
                        ("input_file".to_string(), PropertyDefinition {
                            prop_type: "string".to_string(),
                            description: "Path to the input video file".to_string(),
                            items: None,
                        }),
                        ("output_file".to_string(), PropertyDefinition {
                            prop_type: "string".to_string(),
                            description: "Path to save the stabilized video".to_string(),
                            items: None,
                        }),
                        ("strength".to_string(), PropertyDefinition {
                            prop_type: "number".to_string(),
                            description: "Stabilization strength (1-10, higher = more stabilization)".to_string(),
                            items: None,
                        }),
                    ]),
                    required: vec!["input_file".to_string(), "output_file".to_string(), "strength".to_string()],
                },
            },
            ClaudeTool {
                name: "create_thumbnail".to_string(),
                description: "Creates a thumbnail image from a video at specified time".to_string(),
                input_schema: InputSchema {
                    schema_type: "object".to_string(),
                    properties: HashMap::from([
                        ("input_file".to_string(), PropertyDefinition {
                            prop_type: "string".to_string(),
                            description: "Path to the input video file".to_string(),
                            items: None,
                        }),
                        ("output_file".to_string(), PropertyDefinition {
                            prop_type: "string".to_string(),
                            description: "Path to save the thumbnail image".to_string(),
                            items: None,
                        }),
                        ("timestamp".to_string(), PropertyDefinition {
                            prop_type: "number".to_string(),
                            description: "Time in seconds to capture thumbnail".to_string(),
                            items: None,
                        }),
                        ("width".to_string(), PropertyDefinition {
                            prop_type: "number".to_string(),
                            description: "Thumbnail width in pixels".to_string(),
                            items: None,
                        }),
                        ("height".to_string(), PropertyDefinition {
                            prop_type: "number".to_string(),
                            description: "Thumbnail height in pixels".to_string(),
                            items: None,
                        }),
                    ]),
                    required: vec!["input_file".to_string(), "output_file".to_string(), "timestamp".to_string()],
                },
            },
            ClaudeTool {
                name: "adjust_color".to_string(),
                description: "Adjusts color properties like brightness, contrast, saturation, and hue".to_string(),
                input_schema: InputSchema {
                    schema_type: "object".to_string(),
                    properties: HashMap::from([
                        ("input_file".to_string(), PropertyDefinition {
                            prop_type: "string".to_string(),
                            description: "Path to the input video file".to_string(),
                            items: None,
                        }),
                        ("output_file".to_string(), PropertyDefinition {
                            prop_type: "string".to_string(),
                            description: "Path to save the color-adjusted video".to_string(),
                            items: None,
                        }),
                        ("brightness".to_string(), PropertyDefinition {
                            prop_type: "number".to_string(),
                            description: "Brightness adjustment (-1.0 to 1.0, 0 = no change)".to_string(),
                            items: None,
                        }),
                        ("contrast".to_string(), PropertyDefinition {
                            prop_type: "number".to_string(),
                            description: "Contrast adjustment (-1.0 to 1.0, 0 = no change)".to_string(),
                            items: None,
                        }),
                        ("saturation".to_string(), PropertyDefinition {
                            prop_type: "number".to_string(),
                            description: "Saturation adjustment (-1.0 to 1.0, 0 = no change)".to_string(),
                            items: None,
                        }),
                        ("hue".to_string(), PropertyDefinition {
                            prop_type: "number".to_string(),
                            description: "Hue adjustment in degrees (-180 to 180, 0 = no change)".to_string(),
                            items: None,
                        }),
                    ]),
                    required: vec!["input_file".to_string(), "output_file".to_string()],
                },
            },
            ClaudeTool {
                name: "add_subtitles".to_string(),
                description: "Adds subtitles to a video from a text file or inline text".to_string(),
                input_schema: InputSchema {
                    schema_type: "object".to_string(),
                    properties: HashMap::from([
                        ("input_file".to_string(), PropertyDefinition {
                            prop_type: "string".to_string(),
                            description: "Path to the input video file".to_string(),
                            items: None,
                        }),
                        ("output_file".to_string(), PropertyDefinition {
                            prop_type: "string".to_string(),
                            description: "Path to save the video with subtitles".to_string(),
                            items: None,
                        }),
                        ("subtitle_text".to_string(), PropertyDefinition {
                            prop_type: "string".to_string(),
                            description: "Subtitle text or path to subtitle file (.srt, .vtt)".to_string(),
                            items: None,
                        }),
                        ("font_size".to_string(), PropertyDefinition {
                            prop_type: "number".to_string(),
                            description: "Font size for subtitles (default: 20)".to_string(),
                            items: None,
                        }),
                        ("color".to_string(), PropertyDefinition {
                            prop_type: "string".to_string(),
                            description: "Subtitle color (default: white)".to_string(),
                            items: None,
                        }),
                    ]),
                    required: vec!["input_file".to_string(), "output_file".to_string(), "subtitle_text".to_string()],
                },
            },
            ClaudeTool {
                name: "extract_frames".to_string(),
                description: "Extracts individual frames from a video as image files".to_string(),
                input_schema: InputSchema {
                    schema_type: "object".to_string(),
                    properties: HashMap::from([
                        ("input_file".to_string(), PropertyDefinition {
                            prop_type: "string".to_string(),
                            description: "Path to the input video file".to_string(),
                            items: None,
                        }),
                        ("output_dir".to_string(), PropertyDefinition {
                            prop_type: "string".to_string(),
                            description: "Directory to save extracted frames".to_string(),
                            items: None,
                        }),
                        ("frame_rate".to_string(), PropertyDefinition {
                            prop_type: "number".to_string(),
                            description: "Extract one frame every N seconds (default: 1)".to_string(),
                            items: None,
                        }),
                        ("format".to_string(), PropertyDefinition {
                            prop_type: "string".to_string(),
                            description: "Image format for frames (png, jpg, etc.)".to_string(),
                            items: None,
                        }),
                    ]),
                    required: vec!["input_file".to_string(), "output_dir".to_string()],
                },
            },
            ClaudeTool {
                name: "pexels_search".to_string(),
                description: "Searches Pexels for stock videos and images based on query".to_string(),
                input_schema: InputSchema {
                    schema_type: "object".to_string(),
                    properties: HashMap::from([
                        ("query".to_string(), PropertyDefinition {
                            prop_type: "string".to_string(),
                            description: "Search query for stock content".to_string(),
                            items: None,
                        }),
                        ("media_type".to_string(), PropertyDefinition {
                            prop_type: "string".to_string(),
                            description: "Media type to search: 'videos' or 'photos'".to_string(),
                            items: None,
                        }),
                        ("per_page".to_string(), PropertyDefinition {
                            prop_type: "number".to_string(),
                            description: "Number of results to return (1-80, default: 15)".to_string(),
                            items: None,
                        }),
                    ]),
                    required: vec!["query".to_string(), "media_type".to_string()],
                },
            },
            ClaudeTool {
                name: "sketchfab_search".to_string(),
                description: "Searches Sketchfab for 3D models by keyword. Use to find glTF/GLB models for Blender scenes, backgrounds, and props. Returns model UID, name, author, likes, and viewer URL.".to_string(),
                input_schema: InputSchema {
                    schema_type: "object".to_string(),
                    properties: HashMap::from([
                        ("query".to_string(), PropertyDefinition {
                            prop_type: "string".to_string(),
                            description: "Search keyword for 3D models (e.g. 'robot', 'ocean', 'medieval castle')".to_string(),
                            items: None,
                        }),
                        ("categories".to_string(), PropertyDefinition {
                            prop_type: "string".to_string(),
                            description: "Optional. Filter by category slug (comma-separated). E.g. 'animals-pets,architecture,characters,science-technology,transport'.".to_string(),
                            items: None,
                        }),
                        ("animated".to_string(), PropertyDefinition {
                            prop_type: "boolean".to_string(),
                            description: "Optional. Filter for animated models only (true/false)".to_string(),
                            items: None,
                        }),
                        ("sort_by".to_string(), PropertyDefinition {
                            prop_type: "string".to_string(),
                            description: "Optional. Sort order: '-likeCount' (most liked), '-viewCount' (most viewed), '-createdAt' (newest), '-publishedAt' (recently published)".to_string(),
                            items: None,
                        }),
                    ]),
                    required: vec!["query".to_string()],
                },
            },
            ClaudeTool {
                name: "sketchfab_get_model".to_string(),
                description: "Gets detailed information about a specific Sketchfab 3D model by UID. Returns full metadata including vertex/face count, tags, license, and animation info.".to_string(),
                input_schema: InputSchema {
                    schema_type: "object".to_string(),
                    properties: HashMap::from([
                        ("uid".to_string(), PropertyDefinition {
                            prop_type: "string".to_string(),
                            description: "The model UID from a previous sketchfab_search result".to_string(),
                            items: None,
                        }),
                    ]),
                    required: vec!["uid".to_string()],
                },
            },
            ClaudeTool {
                name: "sketchfab_download".to_string(),
                description: "Downloads a 3D model from Sketchfab by UID and uploads it to R2 cloud storage. Returns a permanent cloud URL for use in Blender scenes.".to_string(),
                input_schema: InputSchema {
                    schema_type: "object".to_string(),
                    properties: HashMap::from([
                        ("uid".to_string(), PropertyDefinition {
                            prop_type: "string".to_string(),
                            description: "The model UID from a previous sketchfab_search result".to_string(),
                            items: None,
                        }),
                        ("format".to_string(), PropertyDefinition {
                            prop_type: "string".to_string(),
                            description: "Optional. Download format: 'gltf' (ZIP with .gltf + textures, default), 'usdz', or 'source'".to_string(),
                            items: None,
                        }),
                    ]),
                    required: vec!["uid".to_string()],
                },
            },
            ClaudeTool {
                name: "analyze_image".to_string(),
                description: "Analyzes an image and provides detailed description using AI".to_string(),
                input_schema: InputSchema {
                    schema_type: "object".to_string(),
                    properties: HashMap::from([
                        ("image_path".to_string(), PropertyDefinition {
                            prop_type: "string".to_string(),
                            description: "Path to the image file to analyze".to_string(),
                            items: None,
                        }),
                        ("analysis_type".to_string(), PropertyDefinition {
                            prop_type: "string".to_string(),
                            description: "Type of analysis: 'general', 'detailed', 'objects', 'colors'".to_string(),
                            items: None,
                        }),
                    ]),
                    required: vec!["image_path".to_string()],
                },
            },
            ClaudeTool {
                name: "generate_text_to_speech".to_string(),
                description: "Generates speech audio from text using Eleven Labs TTS (with Gemini fallback). Supports 17+ premium voices with ultra-low latency (75ms). Perfect for narration, voiceovers, and character voices.".to_string(),
                input_schema: InputSchema {
                    schema_type: "object".to_string(),
                    properties: HashMap::from([
                        ("text".to_string(), PropertyDefinition {
                            prop_type: "string".to_string(),
                            description: "Text to convert to speech".to_string(),
                            items: None,
                        }),
                        ("output_file".to_string(), PropertyDefinition {
                            prop_type: "string".to_string(),
                            description: "Path to save the generated audio file (e.g., 'outputs/narration.mp3')".to_string(),
                            items: None,
                        }),
                        ("voice".to_string(), PropertyDefinition {
                            prop_type: "string".to_string(),
                            description: "Voice name: Rachel (default, young female), Drew (male, news), Clyde (male, veteran), Bella (female, soft), Emily (female, calm), Adam (male, deep), Paul (male, reporter), Domi (female, strong), Elli (female, emotional), Grace (female, young), Matilda (female, warm), Arnold (male, crisp), Callum (male, hoarse), Daniel (male, deep), Ethan (male, young), Liam (male, articulate), Thomas (male, calm)".to_string(),
                            items: None,
                        }),
                        ("model".to_string(), PropertyDefinition {
                            prop_type: "string".to_string(),
                            description: "Model: 'eleven_flash_v2_5' (75ms latency, default), 'eleven_multilingual_v2' (highest quality), 'eleven_turbo_v2_5' (fast)".to_string(),
                            items: None,
                        }),
                    ]),
                    required: vec!["text".to_string(), "output_file".to_string()],
                },
            },
            ClaudeTool {
                name: "generate_sound_effect".to_string(),
                description: "Generates custom sound effects from text descriptions using Eleven Labs. Create cinematic sound design, Foley, ambient sounds, impacts, transitions, etc. Duration: 0.5-30 seconds.".to_string(),
                input_schema: InputSchema {
                    schema_type: "object".to_string(),
                    properties: HashMap::from([
                        ("description".to_string(), PropertyDefinition {
                            prop_type: "string".to_string(),
                            description: "Detailed description of the sound effect (e.g., 'cinematic explosion with rumble', 'door creaking slowly')".to_string(),
                            items: None,
                        }),
                        ("output_file".to_string(), PropertyDefinition {
                            prop_type: "string".to_string(),
                            description: "Path to save the sound effect (e.g., 'outputs/explosion.mp3')".to_string(),
                            items: None,
                        }),
                        ("duration_seconds".to_string(), PropertyDefinition {
                            prop_type: "number".to_string(),
                            description: "Duration in seconds (0.5-30, default: 5)".to_string(),
                            items: None,
                        }),
                        ("prompt_influence".to_string(), PropertyDefinition {
                            prop_type: "number".to_string(),
                            description: "How closely to follow prompt (0-1, default: 0.5). Higher = more precise".to_string(),
                            items: None,
                        }),
                    ]),
                    required: vec!["description".to_string(), "output_file".to_string()],
                },
            },
            ClaudeTool {
                name: "generate_music".to_string(),
                description: "Generates studio-grade background music from text prompts using Eleven Music. Create music in any genre, mood, style. Supports custom structure, lyrics, tempo. Commercial use cleared. Duration: 10-300 seconds.".to_string(),
                input_schema: InputSchema {
                    schema_type: "object".to_string(),
                    properties: HashMap::from([
                        ("prompt".to_string(), PropertyDefinition {
                            prop_type: "string".to_string(),
                            description: "Music description (e.g., 'upbeat electronic dance music 120 BPM', 'peaceful piano meditation', 'epic cinematic orchestral with drums'). Can include genre, mood, instruments, tempo, structure, lyrics.".to_string(),
                            items: None,
                        }),
                        ("output_file".to_string(), PropertyDefinition {
                            prop_type: "string".to_string(),
                            description: "Path to save the music file (e.g., 'outputs/background_music.mp3')".to_string(),
                            items: None,
                        }),
                        ("duration_seconds".to_string(), PropertyDefinition {
                            prop_type: "number".to_string(),
                            description: "Music duration in seconds (10-300, default: 30)".to_string(),
                            items: None,
                        }),
                    ]),
                    required: vec!["prompt".to_string(), "output_file".to_string()],
                },
            },
            ClaudeTool {
                name: "add_voiceover_to_video".to_string(),
                description: "Convenience tool that generates voiceover speech and adds it to a video in one step. Combines text-to-speech generation with audio mixing automatically.".to_string(),
                input_schema: InputSchema {
                    schema_type: "object".to_string(),
                    properties: HashMap::from([
                        ("input_video".to_string(), PropertyDefinition {
                            prop_type: "string".to_string(),
                            description: "Path to the input video file".to_string(),
                            items: None,
                        }),
                        ("voiceover_text".to_string(), PropertyDefinition {
                            prop_type: "string".to_string(),
                            description: "Text for the voiceover narration".to_string(),
                            items: None,
                        }),
                        ("output_video".to_string(), PropertyDefinition {
                            prop_type: "string".to_string(),
                            description: "Path to save the video with voiceover (e.g., 'outputs/narrated_video.mp4')".to_string(),
                            items: None,
                        }),
                        ("voice".to_string(), PropertyDefinition {
                            prop_type: "string".to_string(),
                            description: "Voice name (same as generate_text_to_speech, default: Rachel)".to_string(),
                            items: None,
                        }),
                    ]),
                    required: vec!["input_video".to_string(), "voiceover_text".to_string(), "output_video".to_string()],
                },
            },
            ClaudeTool {
                name: "transcribe_audio_url".to_string(),
                description: "Transcribes speech from a public audio URL using the shared VibeVoice transcription service. Useful for voice notes, podcast clips, narration drafts, interviews, and subtitle prep.".to_string(),
                input_schema: InputSchema {
                    schema_type: "object".to_string(),
                    properties: HashMap::from([
                        ("audio_url".to_string(), PropertyDefinition {
                            prop_type: "string".to_string(),
                            description: "Publicly accessible audio URL to transcribe".to_string(),
                            items: None,
                        }),
                        ("language".to_string(), PropertyDefinition {
                            prop_type: "string".to_string(),
                            description: "Optional language hint such as 'en', 'sw', or 'fr'".to_string(),
                            items: None,
                        }),
                        ("hotwords".to_string(), PropertyDefinition {
                            prop_type: "array".to_string(),
                            description: "Optional list of terms, names, or product words to bias the transcription toward".to_string(),
                            items: Some(Box::new(PropertyDefinition {
                                prop_type: "string".to_string(),
                                description: "Hotword".to_string(),
                                items: None,
                            })),
                        }),
                    ]),
                    required: vec!["audio_url".to_string()],
                },
            },
            ClaudeTool {
                name: "generate_video_script".to_string(),
                description: "Generates a video script based on topic and requirements using AI".to_string(),
                input_schema: InputSchema {
                    schema_type: "object".to_string(),
                    properties: HashMap::from([
                        ("topic".to_string(), PropertyDefinition {
                            prop_type: "string".to_string(),
                            description: "Topic or theme for the video script".to_string(),
                            items: None,
                        }),
                        ("duration".to_string(), PropertyDefinition {
                            prop_type: "number".to_string(),
                            description: "Target video duration in seconds".to_string(),
                            items: None,
                        }),
                        ("style".to_string(), PropertyDefinition {
                            prop_type: "string".to_string(),
                            description: "Script style: 'educational', 'entertainment', 'commercial', 'documentary'".to_string(),
                            items: None,
                        }),
                        ("tone".to_string(), PropertyDefinition {
                            prop_type: "string".to_string(),
                            description: "Script tone: 'casual', 'professional', 'humorous', 'serious'".to_string(),
                            items: None,
                        }),
                    ]),
                    required: vec!["topic".to_string(), "duration".to_string()],
                },
            },
            ClaudeTool {
                name: "create_blank_video".to_string(),
                description: "Creates a blank video with specified color, duration, and dimensions".to_string(),
                input_schema: InputSchema {
                    schema_type: "object".to_string(),
                    properties: HashMap::from([
                        ("output_file".to_string(), PropertyDefinition {
                            prop_type: "string".to_string(),
                            description: "Path to save the blank video".to_string(),
                            items: None,
                        }),
                        ("duration".to_string(), PropertyDefinition {
                            prop_type: "number".to_string(),
                            description: "Duration of the blank video in seconds".to_string(),
                            items: None,
                        }),
                        ("width".to_string(), PropertyDefinition {
                            prop_type: "number".to_string(),
                            description: "Video width in pixels".to_string(),
                            items: None,
                        }),
                        ("height".to_string(), PropertyDefinition {
                            prop_type: "number".to_string(),
                            description: "Video height in pixels".to_string(),
                            items: None,
                        }),
                        ("color".to_string(), PropertyDefinition {
                            prop_type: "string".to_string(),
                            description: "Background color (hex code or color name, default: black)".to_string(),
                            items: None,
                        }),
                    ]),
                    required: vec!["output_file".to_string(), "duration".to_string(), "width".to_string(), "height".to_string()],
                },
            },
            ClaudeTool {
                name: "pexels_download_video".to_string(),
                description: "Downloads a video from Pexels given the video file URL".to_string(),
                input_schema: InputSchema {
                    schema_type: "object".to_string(),
                    properties: HashMap::from([
                        ("video_url".to_string(), PropertyDefinition {
                            prop_type: "string".to_string(),
                            description: "Pexels video file URL (from pexels_search results)".to_string(),
                            items: None,
                        }),
                        ("output_file".to_string(), PropertyDefinition {
                            prop_type: "string".to_string(),
                            description: "Local path to save the downloaded video".to_string(),
                            items: None,
                        }),
                        ("quality".to_string(), PropertyDefinition {
                            prop_type: "string".to_string(),
                            description: "Video quality: 'hd', 'sd', 'low' (optional)".to_string(),
                            items: None,
                        }),
                    ]),
                    required: vec!["video_url".to_string(), "output_file".to_string()],
                },
            },
            ClaudeTool {
                name: "pexels_download_photo".to_string(),
                description: "Downloads a photo from Pexels given the photo URL".to_string(),
                input_schema: InputSchema {
                    schema_type: "object".to_string(),
                    properties: HashMap::from([
                        ("photo_url".to_string(), PropertyDefinition {
                            prop_type: "string".to_string(),
                            description: "Pexels photo URL (from pexels_search results)".to_string(),
                            items: None,
                        }),
                        ("output_file".to_string(), PropertyDefinition {
                            prop_type: "string".to_string(),
                            description: "Local path to save the downloaded photo".to_string(),
                            items: None,
                        }),
                        ("size".to_string(), PropertyDefinition {
                            prop_type: "string".to_string(),
                            description: "Photo size: 'original', 'large', 'medium', 'small' (optional)".to_string(),
                            items: None,
                        }),
                    ]),
                    required: vec!["photo_url".to_string(), "output_file".to_string()],
                },
            },
            ClaudeTool {
                name: "pexels_get_trending".to_string(),
                description: "Gets trending/popular videos from Pexels without needing a search query".to_string(),
                input_schema: InputSchema {
                    schema_type: "object".to_string(),
                    properties: HashMap::from([
                        ("per_page".to_string(), PropertyDefinition {
                            prop_type: "number".to_string(),
                            description: "Number of results to return (1-80, default: 15)".to_string(),
                            items: None,
                        }),
                    ]),
                    required: vec![],
                },
            },
            ClaudeTool {
                name: "pexels_get_curated".to_string(),
                description: "Gets curated/hand-picked photos from Pexels without needing a search query".to_string(),
                input_schema: InputSchema {
                    schema_type: "object".to_string(),
                    properties: HashMap::from([
                        ("per_page".to_string(), PropertyDefinition {
                            prop_type: "number".to_string(),
                            description: "Number of results to return (1-80, default: 15)".to_string(),
                            items: None,
                        }),
                    ]),
                    required: vec![],
                },
            },
            ClaudeTool {
                name: "view_video".to_string(),
                description: "Views/analyzes a video by retrieving its vectorized embeddings from the database. This allows you to 'see' what's in a video without re-processing it. Use this to understand video content, verify edits, or check what a previously generated video contains. Returns detailed frame-by-frame analysis and overall summary. Accepts either a local path or a cloud URL (prefixed with https://).".to_string(),
                input_schema: InputSchema {
                    schema_type: "object".to_string(),
                    properties: HashMap::from([
                        ("video_path".to_string(), PropertyDefinition {
                            prop_type: "string".to_string(),
                            description: "Path or cloud URL to the video file to view/analyze (e.g., 'outputs/edited_video.mp4' or an https:// cloud URL)".to_string(),
                            items: None,
                        }),
                    ]),
                    required: vec!["video_path".to_string()],
                },
            },
            ClaudeTool {
                name: "review_video".to_string(),
                description: "Reviews an output video to verify it meets the user's original requirements. Use this in the final stage of video editing/generation to confirm quality before presenting to the user. Compares the video's vectorized analysis against the user's request to check if edits were applied correctly. Accepts either a local path or a cloud URL.".to_string(),
                input_schema: InputSchema {
                    schema_type: "object".to_string(),
                    properties: HashMap::from([
                        ("video_path".to_string(), PropertyDefinition {
                            prop_type: "string".to_string(),
                            description: "Path or cloud URL to the output video to review (e.g., local path or https:// cloud URL)".to_string(),
                            items: None,
                        }),
                        ("original_request".to_string(), PropertyDefinition {
                            prop_type: "string".to_string(),
                            description: "The original user request/requirements to verify against".to_string(),
                            items: None,
                        }),
                        ("expected_features".to_string(), PropertyDefinition {
                            prop_type: "array".to_string(),
                            description: "List of expected features that should be present (e.g., ['grayscale filter', 'text overlay', 'trimmed to 10s'])".to_string(),
                            items: Some(Box::new(PropertyDefinition {
                                prop_type: "string".to_string(),
                                description: "Expected feature".to_string(),
                                items: None,
                            })),
                        }),
                    ]),
                    required: vec!["video_path".to_string(), "original_request".to_string()],
                },
            },
            ClaudeTool {
                name: "view_image".to_string(),
                description: "Views/analyzes an image file using AI vision. Use this to verify generated images, inspect stock photos from Pexels, or check overlay images before using them in videos. Returns detailed analysis of content, colors, composition, style, and suitability for video use. Accepts either a local path or a cloud URL (prefixed with https://).".to_string(),
                input_schema: InputSchema {
                    schema_type: "object".to_string(),
                    properties: HashMap::from([
                        ("image_path".to_string(), PropertyDefinition {
                            prop_type: "string".to_string(),
                            description: "Path or cloud URL to the image file to view/analyze (e.g., 'outputs/generated_logo.png' or an https:// cloud URL)".to_string(),
                            items: None,
                        }),
                    ]),
                    required: vec!["image_path".to_string()],
                },
            },
            ClaudeTool {
                name: "generate_image".to_string(),
                description: "Generates an image from scratch using Google's Gemini image model based on a text prompt. Use when you need to create a custom image that doesn't exist yet — e.g. branded backgrounds, custom overlay graphics, title cards, logos. For editing an existing image file, use edit_image instead.".to_string(),
                input_schema: InputSchema {
                    schema_type: "object".to_string(),
                    properties: HashMap::from([
                        ("prompt".to_string(), PropertyDefinition {
                            prop_type: "string".to_string(),
                            description: "Detailed text description of the image to generate. Be specific about style, lighting, composition, and details.".to_string(),
                            items: None,
                        }),
                        ("output_file".to_string(), PropertyDefinition {
                            prop_type: "string".to_string(),
                            description: "Path where the generated image should be saved (e.g., 'outputs/generated_overlay.png')".to_string(),
                            items: None,
                        }),
                        ("aspect_ratio".to_string(), PropertyDefinition {
                            prop_type: "string".to_string(),
                            description: "Aspect ratio: '1:1', '16:9', '9:16', '4:3', '3:4' (default: '1:1')".to_string(),
                            items: None,
                        }),
                        ("image_size".to_string(), PropertyDefinition {
                            prop_type: "string".to_string(),
                            description: "Resolution: '1K' (1024px), '2K' (2048px), '4K' (4096px) (default: '2K')".to_string(),
                            items: None,
                        }),
                        ("model".to_string(), PropertyDefinition {
                            prop_type: "string".to_string(),
                            description: "Model to use: 'fast'/'nano' for quick generation, 'quality'/'pro' for best results (default: 'quality'). Or pass an explicit Gemini model ID.".to_string(),
                            items: None,
                        }),
                    ]),
                    required: vec!["prompt".to_string(), "output_file".to_string()],
                },
            },
            ClaudeTool {
                name: "read_website_content".to_string(),
                description: "Fetch and read a website URL, returning its title, description, and main text content. Use this to understand what a website is about before generating a video script, voiceover, or Blender animation about it.".to_string(),
                input_schema: InputSchema {
                    schema_type: "object".to_string(),
                    properties: HashMap::from([
                        ("url".to_string(), PropertyDefinition {
                            prop_type: "string".to_string(),
                            description: "The website URL to read (e.g. 'https://stripe.com')".to_string(),
                            items: None,
                        }),
                    ]),
                    required: vec!["url".to_string()],
                },
            },
            ClaudeTool {
                name: "browserbase_fetch_url".to_string(),
                description: "Fetch a website URL using BrowserBase — a cloud browser that renders JavaScript, solves CAPTCHAs, and returns clean markdown content. Use this instead of read_website_content for JavaScript-heavy SPAs, sites that block plain HTTP fetches, or when you need the full rendered page as markdown. Returns the page content as clean markdown (up to 8000 chars). Falls back to read_website_content if BrowserBase is not configured.".to_string(),
                input_schema: InputSchema {
                    schema_type: "object".to_string(),
                    properties: HashMap::from([
                        ("url".to_string(), PropertyDefinition {
                            prop_type: "string".to_string(),
                            description: "The website URL to fetch (e.g. 'https://stripe.com')".to_string(),
                            items: None,
                        }),
                    ]),
                    required: vec!["url".to_string()],
                },
            },
            ClaudeTool {
                name: "fetch_website_image".to_string(),
                description: "Fetch the hero/og:image from a website URL. Use this when a user provides a website URL (e.g. netflix.com, stripe.com) and you need to extract its visual for use in a Blender landing page animation or product mockup. Returns the image URL that you can pass to blender_generate_scene_type's reference_image_url parameter.".to_string(),
                input_schema: InputSchema {
                    schema_type: "object".to_string(),
                    properties: HashMap::from([
                        ("url".to_string(), PropertyDefinition {
                            prop_type: "string".to_string(),
                            description: "The website URL to extract the hero image from (e.g. 'https://netflix.com')".to_string(),
                            items: None,
                        }),
                    ]),
                    required: vec!["url".to_string()],
                },
            },
            ClaudeTool {
                name: "edit_image".to_string(),
                description: "Edit or transform an existing image using AI. Use when you need to: modify a downloaded Pexels photo, add text/graphics to a video frame, change the style of an image, remove or replace elements, or create a variant of an existing image. Requires a path to the source image on disk. Example workflow: extract a frame with extract_frames, then call edit_image to add a title overlay before compositing it back.".to_string(),
                input_schema: InputSchema {
                    schema_type: "object".to_string(),
                    properties: HashMap::from([
                        ("input_image".to_string(), PropertyDefinition {
                            prop_type: "string".to_string(),
                            description: "Path to the source image file to edit (e.g., 'outputs/frame.jpg' or 'outputs/pexels_photo.jpg')".to_string(),
                            items: None,
                        }),
                        ("prompt".to_string(), PropertyDefinition {
                            prop_type: "string".to_string(),
                            description: "Instructions describing what edits to make (e.g., 'add bold white title text at the top saying VIDEOSYNC', 'make it look cinematic with warm tones', 'remove the background')".to_string(),
                            items: None,
                        }),
                        ("output_file".to_string(), PropertyDefinition {
                            prop_type: "string".to_string(),
                            description: "Path where the edited image should be saved (e.g., 'outputs/edited_overlay.jpg')".to_string(),
                            items: None,
                        }),
                        ("aspect_ratio".to_string(), PropertyDefinition {
                            prop_type: "string".to_string(),
                            description: "Output aspect ratio: '1:1', '16:9', '9:16', '4:3', '3:4' (default: '16:9')".to_string(),
                            items: None,
                        }),
                        ("model".to_string(), PropertyDefinition {
                            prop_type: "string".to_string(),
                            description: "Model to use: 'fast'/'nano' for quick edits, 'quality'/'pro' for best results (default: 'quality')".to_string(),
                            items: None,
                        }),
                    ]),
                    required: vec!["input_image".to_string(), "prompt".to_string(), "output_file".to_string()],
                },
            },
            // ── Option A: Agentic video generation pipeline tools ──────────────
            ClaudeTool {
                name: "generate_video_queries".to_string(),
                description: "Generate diverse Pexels search queries from a high-level video topic. Use this as the FIRST step when building a video from scratch instead of auto_generate_video — it gives you the search queries you then use with pexels_search, analyze_pexels_thumbnail, pexels_download_video, verify_clip_quality_tool, and merge_videos for full agentic control over every clip selection decision.".to_string(),
                input_schema: InputSchema {
                    schema_type: "object".to_string(),
                    properties: HashMap::from([
                        ("topic".to_string(), PropertyDefinition {
                            prop_type: "string".to_string(),
                            description: "The video topic or concept (e.g. 'sunrise over mountain peaks', 'startup office culture')".to_string(),
                            items: None,
                        }),
                        ("num_queries".to_string(), PropertyDefinition {
                            prop_type: "number".to_string(),
                            description: "How many search queries to generate (default: 5, matches the number of clips you want)".to_string(),
                            items: None,
                        }),
                    ]),
                    required: vec!["topic".to_string()],
                },
            },
            ClaudeTool {
                name: "analyze_pexels_thumbnail".to_string(),
                description: "Download a Pexels video thumbnail URL and analyze it with Gemini vision to score its relevance to your video topic (1-10). Use this AFTER pexels_search, on each result's video_pictures[0].picture URL, BEFORE calling pexels_download_video — so you only download full clips that are actually relevant. Score >= 5 means proceed; score < 5 means skip and try the next result.".to_string(),
                input_schema: InputSchema {
                    schema_type: "object".to_string(),
                    properties: HashMap::from([
                        ("thumbnail_url".to_string(), PropertyDefinition {
                            prop_type: "string".to_string(),
                            description: "The Pexels thumbnail image URL from video_pictures[0].picture in the search results".to_string(),
                            items: None,
                        }),
                        ("topic".to_string(), PropertyDefinition {
                            prop_type: "string".to_string(),
                            description: "Your video topic — used to judge relevance (e.g. 'sunset beach')".to_string(),
                            items: None,
                        }),
                    ]),
                    required: vec!["thumbnail_url".to_string(), "topic".to_string()],
                },
            },
            ClaudeTool {
                name: "verify_clip_quality_tool".to_string(),
                description: "Run FFmpeg quality checks on a downloaded video clip: duration > 1s, no frozen frames >= 1.5s, no black frames >= 1s. Use this AFTER pexels_download_video to confirm the clip is usable before adding it to your merge list. Returns '✅ QA passed' or '❌ QA failed: reason'.".to_string(),
                input_schema: InputSchema {
                    schema_type: "object".to_string(),
                    properties: HashMap::from([
                        ("file_path".to_string(), PropertyDefinition {
                            prop_type: "string".to_string(),
                            description: "Path to the downloaded video clip to verify (e.g. 'outputs/clip_0_abc123.mp4')".to_string(),
                            items: None,
                        }),
                    ]),
                    required: vec!["file_path".to_string()],
                },
            },
            ClaudeTool {
                name: "run_video_qa".to_string(),
                description: "Run a full automated QA suite on a finished video file using FFmpeg signal analysis. Returns a structured report covering: duration, resolution, FPS, format, file size, audio presence, frozen frames, black frames, and scene change count. Use this AFTER merge_videos and BEFORE presenting the output to the user. Complements view_video (AI content review) and review_video (pass/fail verdict).".to_string(),
                input_schema: InputSchema {
                    schema_type: "object".to_string(),
                    properties: HashMap::from([
                        ("file_path".to_string(), PropertyDefinition {
                            prop_type: "string".to_string(),
                            description: "Path to the video file to QA (e.g. 'outputs/final_video.mp4')".to_string(),
                            items: None,
                        }),
                    ]),
                    required: vec!["file_path".to_string()],
                },
            },
            ClaudeTool {
                name: "auto_generate_video".to_string(),
                description: "Orchestrates automatic video generation from a topic/prompt. This high-level tool searches Pexels for stock footage, generates images, downloads clips, merges them, adds text overlays, music, and exports a complete video. Perfect for creating videos from scratch.".to_string(),
                input_schema: InputSchema {
                    schema_type: "object".to_string(),
                    properties: HashMap::from([
                        ("topic".to_string(), PropertyDefinition {
                            prop_type: "string".to_string(),
                            description: "Topic or description of the video to create (e.g., 'A motivational video about success')".to_string(),
                            items: None,
                        }),
                        ("output_file".to_string(), PropertyDefinition {
                            prop_type: "string".to_string(),
                            description: "Path where the final video should be saved".to_string(),
                            items: None,
                        }),
                        ("duration".to_string(), PropertyDefinition {
                            prop_type: "number".to_string(),
                            description: "Target video duration in seconds (default: 30)".to_string(),
                            items: None,
                        }),
                        ("style".to_string(), PropertyDefinition {
                            prop_type: "string".to_string(),
                            description: "Video style: 'cinematic', 'minimal', 'energetic', 'calm', 'corporate' (default: 'cinematic')".to_string(),
                            items: None,
                        }),
                        ("include_text_overlays".to_string(), PropertyDefinition {
                            prop_type: "boolean".to_string(),
                            description: "Whether to add text overlays with key messages (default: true)".to_string(),
                            items: None,
                        }),
                        ("include_music".to_string(), PropertyDefinition {
                            prop_type: "boolean".to_string(),
                            description: "Whether to add background music (default: false)".to_string(),
                            items: None,
                        }),
                        ("num_clips".to_string(), PropertyDefinition {
                            prop_type: "number".to_string(),
                            description: "Number of video clips to use from Pexels (default: 3-5 based on duration)".to_string(),
                            items: None,
                        }),
                    ]),
                    required: vec!["topic".to_string(), "output_file".to_string()],
                },
            },
            // Chat title management tool
            ClaudeTool {
                name: "set_chat_title".to_string(),
                description: "Sets a descriptive title for the current chat session. Use this to give the conversation a meaningful title based on the user's request or the work being done.".to_string(),
                input_schema: InputSchema {
                    schema_type: "object".to_string(),
                    properties: HashMap::from([
                        ("title".to_string(), PropertyDefinition {
                            prop_type: "string".to_string(),
                            description: "A concise, descriptive title for this chat session (max 100 characters)".to_string(),
                            items: None,
                        }),
                    ]),
                    required: vec!["title".to_string()],
                },
            },

            // =====================================================================
            // YOUTUBE INTEGRATION TOOLS (READ-ONLY RESEARCH & OPTIMIZATION)
            // =====================================================================

            ClaudeTool {
                name: "optimize_youtube_metadata".to_string(),
                description: "Analyzes a video file and generates SEO-optimized YouTube metadata (title, description, tags) to maximize discoverability and engagement. Uses AI to understand video content and suggest compelling, keyword-rich metadata. Returns suggestions only - does not upload or modify anything. Parameters: video_path (required) - path to video file, target_audience (optional) - intended audience like 'gaming', 'education', 'vlog', style (optional) - 'clickbait', 'professional', or 'casual'.".to_string(),
                input_schema: InputSchema {
                    schema_type: "object".to_string(),
                    properties: HashMap::from([
                        ("video_path".to_string(), PropertyDefinition {
                            prop_type: "string".to_string(),
                            description: "Path to the video file to analyze for metadata optimization".to_string(),
                            items: None,
                        }),
                        ("target_audience".to_string(), PropertyDefinition {
                            prop_type: "string".to_string(),
                            description: "Target audience type: 'gaming', 'education', 'vlog', 'entertainment', 'tech', 'music', etc.".to_string(),
                            items: None,
                        }),
                        ("style".to_string(), PropertyDefinition {
                            prop_type: "string".to_string(),
                            description: "Metadata style: 'clickbait' (attention-grabbing), 'professional' (formal), 'casual' (conversational)".to_string(),
                            items: None,
                        }),
                    ]),
                    required: vec!["video_path".to_string()],
                },
            },
            ClaudeTool {
                name: "analyze_youtube_performance".to_string(),
                description: "Fetches analytics data for a YouTube video and provides AI-powered insights on performance, audience engagement, and optimization opportunities. Analyzes views, watch time, likes, comments, shares, and subscriber gain/loss. Identifies strengths and areas for improvement. READ-ONLY tool - does not modify anything. Parameters: video_id (required) - YouTube video ID, date_range (optional) - number of days to analyze (default 30).".to_string(),
                input_schema: InputSchema {
                    schema_type: "object".to_string(),
                    properties: HashMap::from([
                        ("video_id".to_string(), PropertyDefinition {
                            prop_type: "string".to_string(),
                            description: "YouTube video ID (the alphanumeric code from youtube.com/watch?v=VIDEO_ID)".to_string(),
                            items: None,
                        }),
                        ("date_range_days".to_string(), PropertyDefinition {
                            prop_type: "number".to_string(),
                            description: "Number of days to analyze (default: 30, max: 365)".to_string(),
                            items: None,
                        }),
                    ]),
                    required: vec!["video_id".to_string()],
                },
            },
            ClaudeTool {
                name: "suggest_content_ideas".to_string(),
                description: "Analyzes the user's YouTube channel performance and current trending topics to suggest data-driven content ideas that are likely to perform well. Provides 5-10 specific video ideas with rationale based on what's working for the channel and what's trending in the niche. READ-ONLY research tool. Parameters: channel_id (optional) - if not provided, uses user's primary channel, category (optional) - focus area like 'gaming', 'tutorial', 'vlog', num_ideas (optional) - number of ideas to generate (default 5).".to_string(),
                input_schema: InputSchema {
                    schema_type: "object".to_string(),
                    properties: HashMap::from([
                        ("channel_id".to_string(), PropertyDefinition {
                            prop_type: "number".to_string(),
                            description: "Internal channel ID from database (optional - if not provided, uses user's first active channel)".to_string(),
                            items: None,
                        }),
                        ("category".to_string(), PropertyDefinition {
                            prop_type: "string".to_string(),
                            description: "Content category to focus on: 'gaming', 'tech', 'education', 'entertainment', 'music', etc.".to_string(),
                            items: None,
                        }),
                        ("num_ideas".to_string(), PropertyDefinition {
                            prop_type: "number".to_string(),
                            description: "Number of content ideas to generate (default: 5, max: 10)".to_string(),
                            items: None,
                        }),
                    ]),
                    required: vec![],
                },
            },
            ClaudeTool {
                name: "search_youtube_trends".to_string(),
                description: "Searches for trending YouTube videos in a specific category or by keyword to understand what content is performing well. Useful for competitive research and identifying content gaps. Returns video titles, view counts, engagement metrics, and channel information. READ-ONLY research tool. Parameters: query (optional) - search keywords, region_code (optional) - two-letter country code like 'US', 'GB', category (optional) - content category, max_results (optional) - max 50.".to_string(),
                input_schema: InputSchema {
                    schema_type: "object".to_string(),
                    properties: HashMap::from([
                        ("query".to_string(), PropertyDefinition {
                            prop_type: "string".to_string(),
                            description: "Search query/keywords (optional - if not provided, returns general trending)".to_string(),
                            items: None,
                        }),
                        ("region_code".to_string(), PropertyDefinition {
                            prop_type: "string".to_string(),
                            description: "Two-letter country code (ISO 3166-1 alpha-2): 'US', 'GB', 'CA', 'AU', etc. (default: 'US')".to_string(),
                            items: None,
                        }),
                        ("category".to_string(), PropertyDefinition {
                            prop_type: "string".to_string(),
                            description: "Content category: 'gaming', 'music', 'education', 'entertainment', 'sports', 'tech'".to_string(),
                            items: None,
                        }),
                        ("max_results".to_string(), PropertyDefinition {
                            prop_type: "number".to_string(),
                            description: "Maximum number of results to return (default: 10, max: 50)".to_string(),
                            items: None,
                        }),
                    ]),
                    required: vec![],
                },
            },
            ClaudeTool {
                name: "search_youtube_channels".to_string(),
                description: "Searches for YouTube channels by name or keywords. Useful for finding specific creators, competitors, or channels in a particular niche. Returns channel names, descriptions, subscriber counts, and channel IDs. READ-ONLY research tool. Parameters: query (required) - channel name or keywords to search for, max_results (optional) - max 50, order (optional) - 'relevance', 'viewCount', 'videoCount'.".to_string(),
                input_schema: InputSchema {
                    schema_type: "object".to_string(),
                    properties: HashMap::from([
                        ("query".to_string(), PropertyDefinition {
                            prop_type: "string".to_string(),
                            description: "Channel name or keywords to search for (e.g., 'MrBeast', 'chess tutorials', 'cooking channels')".to_string(),
                            items: None,
                        }),
                        ("max_results".to_string(), PropertyDefinition {
                            prop_type: "number".to_string(),
                            description: "Maximum number of channels to return (default: 10, max: 50)".to_string(),
                            items: None,
                        }),
                        ("order".to_string(), PropertyDefinition {
                            prop_type: "string".to_string(),
                            description: "Sort order: 'relevance' (default), 'viewCount', 'videoCount'".to_string(),
                            items: None,
                        }),
                    ]),
                    required: vec!["query".to_string()],
                },
            },

            // CRITICAL: Agent control tool for proper task completion
            ClaudeTool {
                name: "submit_final_answer".to_string(),
                description: "**CRITICAL COMPLETION TOOL**: Call this tool ONLY when you have successfully completed ALL parts of the user's request. This signals that all operations are done and no more work is needed. Parameters: summary (required) - brief description of what was accomplished, output_files (optional) - array of file paths created.".to_string(),
                input_schema: InputSchema {
                    schema_type: "object".to_string(),
                    properties: HashMap::from([
                        ("summary".to_string(), PropertyDefinition {
                            prop_type: "string".to_string(),
                            description: "A natural, conversational summary of what was accomplished".to_string(),
                            items: None,
                        }),
                        ("output_files".to_string(), PropertyDefinition {
                            prop_type: "array".to_string(),
                            description: "Array of output file paths that were created during this request".to_string(),
                            items: Some(Box::new(PropertyDefinition {
                                prop_type: "string".to_string(),
                                description: "File path".to_string(),
                                items: None,
                            })),
                        }),
                    ]),
                    required: vec!["summary".to_string()],
                },
            },
            ClaudeTool {
                name: "add_transition".to_string(),
                description: "Adds a transition effect between two video clips using FFmpeg xfade filter".to_string(),
                input_schema: InputSchema {
                    schema_type: "object".to_string(),
                    properties: HashMap::from([
                        ("input_file1".to_string(), PropertyDefinition {
                            prop_type: "string".to_string(),
                            description: "Path to the first video file".to_string(),
                            items: None,
                        }),
                        ("input_file2".to_string(), PropertyDefinition {
                            prop_type: "string".to_string(),
                            description: "Path to the second video file".to_string(),
                            items: None,
                        }),
                        ("output_file".to_string(), PropertyDefinition {
                            prop_type: "string".to_string(),
                            description: "Path to save the output video with transition".to_string(),
                            items: None,
                        }),
                        ("transition_type".to_string(), PropertyDefinition {
                            prop_type: "string".to_string(),
                            description: "Transition type: fade, dissolve, wipeleft, wiperight, circleopen, circleclose, radial, pixelize, diagtl, diagtr".to_string(),
                            items: None,
                        }),
                        ("duration_seconds".to_string(), PropertyDefinition {
                            prop_type: "number".to_string(),
                            description: "Transition duration in seconds (0.5–3.0 recommended)".to_string(),
                            items: None,
                        }),
                        ("offset_seconds".to_string(), PropertyDefinition {
                            prop_type: "number".to_string(),
                            description: "Time offset in seconds where the transition starts".to_string(),
                            items: None,
                        }),
                    ]),
                    required: vec![
                        "input_file1".to_string(), "input_file2".to_string(),
                        "output_file".to_string(), "transition_type".to_string(),
                        "duration_seconds".to_string(), "offset_seconds".to_string(),
                    ],
                },
            },
            ClaudeTool {
                name: "add_animated_text".to_string(),
                description: "Adds animated text to a video (fade_in, slide_in, or typewriter effects)".to_string(),
                input_schema: InputSchema {
                    schema_type: "object".to_string(),
                    properties: HashMap::from([
                        ("input_file".to_string(), PropertyDefinition {
                            prop_type: "string".to_string(),
                            description: "Path to the input video file".to_string(),
                            items: None,
                        }),
                        ("output_file".to_string(), PropertyDefinition {
                            prop_type: "string".to_string(),
                            description: "Path to save the video with animated text".to_string(),
                            items: None,
                        }),
                        ("text".to_string(), PropertyDefinition {
                            prop_type: "string".to_string(),
                            description: "Text to display on the video".to_string(),
                            items: None,
                        }),
                        ("animation_type".to_string(), PropertyDefinition {
                            prop_type: "string".to_string(),
                            description: "Animation type: 'fade_in', 'slide_in', or 'typewriter'".to_string(),
                            items: None,
                        }),
                        ("start_time".to_string(), PropertyDefinition {
                            prop_type: "number".to_string(),
                            description: "Time in seconds when the text animation starts".to_string(),
                            items: None,
                        }),
                        ("duration".to_string(), PropertyDefinition {
                            prop_type: "number".to_string(),
                            description: "Duration in seconds for the text animation".to_string(),
                            items: None,
                        }),
                    ]),
                    required: vec![
                        "input_file".to_string(), "output_file".to_string(),
                        "text".to_string(), "animation_type".to_string(),
                        "start_time".to_string(), "duration".to_string(),
                    ],
                },
            },
            ClaudeTool {
                name: "apply_filter_chain".to_string(),
                description: "Applies a chain of video filters (brightness, contrast, saturation, blur) in sequence".to_string(),
                input_schema: InputSchema {
                    schema_type: "object".to_string(),
                    properties: HashMap::from([
                        ("input_file".to_string(), PropertyDefinition {
                            prop_type: "string".to_string(),
                            description: "Path to the input video file".to_string(),
                            items: None,
                        }),
                        ("output_file".to_string(), PropertyDefinition {
                            prop_type: "string".to_string(),
                            description: "Path to save the filtered video".to_string(),
                            items: None,
                        }),
                        ("filters".to_string(), PropertyDefinition {
                            prop_type: "array".to_string(),
                            description: "Array of filter objects with 'name' (brightness|contrast|saturation|blur) and 'value' (number) fields".to_string(),
                            items: Some(Box::new(PropertyDefinition {
                                prop_type: "object".to_string(),
                                description: "Filter object with name and value".to_string(),
                                items: None,
                            })),
                        }),
                    ]),
                    required: vec!["input_file".to_string(), "output_file".to_string(), "filters".to_string()],
                },
            },
            ClaudeTool {
                name: "apply_audio_effect".to_string(),
                description: "Applies an audio effect (echo, reverb, chorus) to the video's audio track".to_string(),
                input_schema: InputSchema {
                    schema_type: "object".to_string(),
                    properties: HashMap::from([
                        ("input_file".to_string(), PropertyDefinition {
                            prop_type: "string".to_string(),
                            description: "Path to the input video file".to_string(),
                            items: None,
                        }),
                        ("output_file".to_string(), PropertyDefinition {
                            prop_type: "string".to_string(),
                            description: "Path to save the video with audio effect applied".to_string(),
                            items: None,
                        }),
                        ("effect".to_string(), PropertyDefinition {
                            prop_type: "string".to_string(),
                            description: "Audio effect type: 'echo', 'reverb', or 'chorus'".to_string(),
                            items: None,
                        }),
                        ("intensity".to_string(), PropertyDefinition {
                            prop_type: "number".to_string(),
                            description: "Effect intensity from 0.0 (subtle) to 1.0 (strong)".to_string(),
                            items: None,
                        }),
                    ]),
                    required: vec!["input_file".to_string(), "output_file".to_string(), "effect".to_string(), "intensity".to_string()],
                },
            },
            ClaudeTool {
                name: "deinterlace_video".to_string(),
                description: "Deinterlaces an interlaced video using the yadif filter for smoother playback".to_string(),
                input_schema: InputSchema {
                    schema_type: "object".to_string(),
                    properties: HashMap::from([
                        ("input_file".to_string(), PropertyDefinition {
                            prop_type: "string".to_string(),
                            description: "Path to the interlaced input video file".to_string(),
                            items: None,
                        }),
                        ("output_file".to_string(), PropertyDefinition {
                            prop_type: "string".to_string(),
                            description: "Path to save the deinterlaced video".to_string(),
                            items: None,
                        }),
                        ("mode".to_string(), PropertyDefinition {
                            prop_type: "string".to_string(),
                            description: "Deinterlace mode: '0' (send_frame), '1' (send_field), '2' (send_frame_nospatial)".to_string(),
                            items: None,
                        }),
                    ]),
                    required: vec!["input_file".to_string(), "output_file".to_string(), "mode".to_string()],
                },
            },
            // ================================================================
            // BATCH 1 — Wire existing Rust functions
            // ================================================================
            ClaudeTool {
                name: "create_thumbnail_hd".to_string(),
                description: "Creates an HD thumbnail at a custom resolution from a video frame".to_string(),
                input_schema: InputSchema {
                    schema_type: "object".to_string(),
                    properties: HashMap::from([
                        ("input_file".to_string(), PropertyDefinition {
                            prop_type: "string".to_string(),
                            description: "Path to the input video file".to_string(),
                            items: None,
                        }),
                        ("output_file".to_string(), PropertyDefinition {
                            prop_type: "string".to_string(),
                            description: "Path to save the thumbnail image".to_string(),
                            items: None,
                        }),
                        ("timestamp".to_string(), PropertyDefinition {
                            prop_type: "number".to_string(),
                            description: "Time in seconds to extract the frame".to_string(),
                            items: None,
                        }),
                        ("width".to_string(), PropertyDefinition {
                            prop_type: "number".to_string(),
                            description: "Target width in pixels (e.g. 1280)".to_string(),
                            items: None,
                        }),
                        ("height".to_string(), PropertyDefinition {
                            prop_type: "number".to_string(),
                            description: "Target height in pixels (e.g. 720)".to_string(),
                            items: None,
                        }),
                    ]),
                    required: vec!["input_file".to_string(), "output_file".to_string(), "timestamp".to_string(), "width".to_string(), "height".to_string()],
                },
            },
            ClaudeTool {
                name: "get_video_duration".to_string(),
                description: "Returns the duration of a video file in seconds".to_string(),
                input_schema: InputSchema {
                    schema_type: "object".to_string(),
                    properties: HashMap::from([
                        ("input_file".to_string(), PropertyDefinition {
                            prop_type: "string".to_string(),
                            description: "Path to the video file".to_string(),
                            items: None,
                        }),
                    ]),
                    required: vec!["input_file".to_string()],
                },
            },
            // ================================================================
            // BATCH 2 — Color Grading
            // ================================================================
            ClaudeTool {
                name: "adjust_hue".to_string(),
                description: "Adjusts the hue and saturation of a video using the FFmpeg hue filter".to_string(),
                input_schema: InputSchema {
                    schema_type: "object".to_string(),
                    properties: HashMap::from([
                        ("input_file".to_string(), PropertyDefinition {
                            prop_type: "string".to_string(),
                            description: "Path to the input video file".to_string(),
                            items: None,
                        }),
                        ("output_file".to_string(), PropertyDefinition {
                            prop_type: "string".to_string(),
                            description: "Path to save the output video".to_string(),
                            items: None,
                        }),
                        ("hue_degrees".to_string(), PropertyDefinition {
                            prop_type: "number".to_string(),
                            description: "Hue rotation in degrees (-180 to 180)".to_string(),
                            items: None,
                        }),
                        ("saturation_factor".to_string(), PropertyDefinition {
                            prop_type: "number".to_string(),
                            description: "Saturation multiplier (0 = grayscale, 1 = original, 3 = very saturated)".to_string(),
                            items: None,
                        }),
                    ]),
                    required: vec!["input_file".to_string(), "output_file".to_string(), "hue_degrees".to_string(), "saturation_factor".to_string()],
                },
            },
            ClaudeTool {
                name: "color_balance".to_string(),
                description: "Adjusts color balance in shadows, midtones, and highlights using separate R/G/B controls".to_string(),
                input_schema: InputSchema {
                    schema_type: "object".to_string(),
                    properties: HashMap::from([
                        ("input_file".to_string(), PropertyDefinition {
                            prop_type: "string".to_string(),
                            description: "Path to the input video file".to_string(),
                            items: None,
                        }),
                        ("output_file".to_string(), PropertyDefinition {
                            prop_type: "string".to_string(),
                            description: "Path to save the output video".to_string(),
                            items: None,
                        }),
                        ("shadows_r".to_string(), PropertyDefinition {
                            prop_type: "number".to_string(),
                            description: "Shadow red adjustment (-1.0 to 1.0)".to_string(),
                            items: None,
                        }),
                        ("shadows_g".to_string(), PropertyDefinition {
                            prop_type: "number".to_string(),
                            description: "Shadow green adjustment (-1.0 to 1.0)".to_string(),
                            items: None,
                        }),
                        ("shadows_b".to_string(), PropertyDefinition {
                            prop_type: "number".to_string(),
                            description: "Shadow blue adjustment (-1.0 to 1.0)".to_string(),
                            items: None,
                        }),
                        ("midtones_r".to_string(), PropertyDefinition {
                            prop_type: "number".to_string(),
                            description: "Midtone red adjustment (-1.0 to 1.0)".to_string(),
                            items: None,
                        }),
                        ("midtones_g".to_string(), PropertyDefinition {
                            prop_type: "number".to_string(),
                            description: "Midtone green adjustment (-1.0 to 1.0)".to_string(),
                            items: None,
                        }),
                        ("midtones_b".to_string(), PropertyDefinition {
                            prop_type: "number".to_string(),
                            description: "Midtone blue adjustment (-1.0 to 1.0)".to_string(),
                            items: None,
                        }),
                        ("highlights_r".to_string(), PropertyDefinition {
                            prop_type: "number".to_string(),
                            description: "Highlight red adjustment (-1.0 to 1.0)".to_string(),
                            items: None,
                        }),
                        ("highlights_g".to_string(), PropertyDefinition {
                            prop_type: "number".to_string(),
                            description: "Highlight green adjustment (-1.0 to 1.0)".to_string(),
                            items: None,
                        }),
                        ("highlights_b".to_string(), PropertyDefinition {
                            prop_type: "number".to_string(),
                            description: "Highlight blue adjustment (-1.0 to 1.0)".to_string(),
                            items: None,
                        }),
                    ]),
                    required: vec!["input_file".to_string(), "output_file".to_string()],
                },
            },
            ClaudeTool {
                name: "normalize_video".to_string(),
                description: "Normalizes video brightness/luminance across frames using FFmpeg normalize filter".to_string(),
                input_schema: InputSchema {
                    schema_type: "object".to_string(),
                    properties: HashMap::from([
                        ("input_file".to_string(), PropertyDefinition {
                            prop_type: "string".to_string(),
                            description: "Path to the input video file".to_string(),
                            items: None,
                        }),
                        ("output_file".to_string(), PropertyDefinition {
                            prop_type: "string".to_string(),
                            description: "Path to save the output video".to_string(),
                            items: None,
                        }),
                        ("smoothing".to_string(), PropertyDefinition {
                            prop_type: "number".to_string(),
                            description: "Temporal smoothing window size (0 = per-frame, larger = smoother)".to_string(),
                            items: None,
                        }),
                    ]),
                    required: vec!["input_file".to_string(), "output_file".to_string()],
                },
            },
            ClaudeTool {
                name: "apply_lut".to_string(),
                description: "Applies a 3D LUT (Look-Up Table) file to a video for color grading".to_string(),
                input_schema: InputSchema {
                    schema_type: "object".to_string(),
                    properties: HashMap::from([
                        ("input_file".to_string(), PropertyDefinition {
                            prop_type: "string".to_string(),
                            description: "Path to the input video file".to_string(),
                            items: None,
                        }),
                        ("output_file".to_string(), PropertyDefinition {
                            prop_type: "string".to_string(),
                            description: "Path to save the output video".to_string(),
                            items: None,
                        }),
                        ("lut_file".to_string(), PropertyDefinition {
                            prop_type: "string".to_string(),
                            description: "Path to the .cube or .3dl LUT file".to_string(),
                            items: None,
                        }),
                        ("interp".to_string(), PropertyDefinition {
                            prop_type: "string".to_string(),
                            description: "Interpolation method: nearest, trilinear, or tetrahedral".to_string(),
                            items: None,
                        }),
                    ]),
                    required: vec!["input_file".to_string(), "output_file".to_string(), "lut_file".to_string()],
                },
            },
            // ================================================================
            // BATCH 3 — Denoising & Sharpening
            // ================================================================
            ClaudeTool {
                name: "denoise_video".to_string(),
                description: "Reduces video noise using the hqdn3d high-quality denoiser filter".to_string(),
                input_schema: InputSchema {
                    schema_type: "object".to_string(),
                    properties: HashMap::from([
                        ("input_file".to_string(), PropertyDefinition {
                            prop_type: "string".to_string(),
                            description: "Path to the input video file".to_string(),
                            items: None,
                        }),
                        ("output_file".to_string(), PropertyDefinition {
                            prop_type: "string".to_string(),
                            description: "Path to save the denoised video".to_string(),
                            items: None,
                        }),
                        ("luma_spatial".to_string(), PropertyDefinition {
                            prop_type: "number".to_string(),
                            description: "Spatial luma denoising strength (0–10)".to_string(),
                            items: None,
                        }),
                        ("luma_temporal".to_string(), PropertyDefinition {
                            prop_type: "number".to_string(),
                            description: "Temporal luma denoising strength (0–10)".to_string(),
                            items: None,
                        }),
                        ("chroma_spatial".to_string(), PropertyDefinition {
                            prop_type: "number".to_string(),
                            description: "Spatial chroma denoising strength (0–10)".to_string(),
                            items: None,
                        }),
                        ("chroma_temporal".to_string(), PropertyDefinition {
                            prop_type: "number".to_string(),
                            description: "Temporal chroma denoising strength (0–10)".to_string(),
                            items: None,
                        }),
                    ]),
                    required: vec!["input_file".to_string(), "output_file".to_string()],
                },
            },
            ClaudeTool {
                name: "unsharp_mask".to_string(),
                description: "Applies an unsharp mask to sharpen or blur a video using FFmpeg unsharp filter".to_string(),
                input_schema: InputSchema {
                    schema_type: "object".to_string(),
                    properties: HashMap::from([
                        ("input_file".to_string(), PropertyDefinition {
                            prop_type: "string".to_string(),
                            description: "Path to the input video file".to_string(),
                            items: None,
                        }),
                        ("output_file".to_string(), PropertyDefinition {
                            prop_type: "string".to_string(),
                            description: "Path to save the output video".to_string(),
                            items: None,
                        }),
                        ("luma_msize_x".to_string(), PropertyDefinition {
                            prop_type: "number".to_string(),
                            description: "Luma matrix horizontal size (3–23, must be odd)".to_string(),
                            items: None,
                        }),
                        ("luma_msize_y".to_string(), PropertyDefinition {
                            prop_type: "number".to_string(),
                            description: "Luma matrix vertical size (3–23, must be odd)".to_string(),
                            items: None,
                        }),
                        ("luma_amount".to_string(), PropertyDefinition {
                            prop_type: "number".to_string(),
                            description: "Sharpening amount (-1.5 to 1.5; positive sharpens, negative blurs)".to_string(),
                            items: None,
                        }),
                    ]),
                    required: vec!["input_file".to_string(), "output_file".to_string(), "luma_msize_x".to_string(), "luma_msize_y".to_string(), "luma_amount".to_string()],
                },
            },
            ClaudeTool {
                name: "reduce_noise".to_string(),
                description: "Reduces noise using the non-local means (nlmeans) denoiser — high quality but slow".to_string(),
                input_schema: InputSchema {
                    schema_type: "object".to_string(),
                    properties: HashMap::from([
                        ("input_file".to_string(), PropertyDefinition {
                            prop_type: "string".to_string(),
                            description: "Path to the input video file".to_string(),
                            items: None,
                        }),
                        ("output_file".to_string(), PropertyDefinition {
                            prop_type: "string".to_string(),
                            description: "Path to save the denoised video".to_string(),
                            items: None,
                        }),
                        ("strength".to_string(), PropertyDefinition {
                            prop_type: "number".to_string(),
                            description: "Denoising strength (1–30; lower = less denoising)".to_string(),
                            items: None,
                        }),
                        ("research_size".to_string(), PropertyDefinition {
                            prop_type: "number".to_string(),
                            description: "Research window size (9–45; larger = better quality but slower)".to_string(),
                            items: None,
                        }),
                        ("patch_size".to_string(), PropertyDefinition {
                            prop_type: "number".to_string(),
                            description: "Patch size (3–21; must be odd)".to_string(),
                            items: None,
                        }),
                    ]),
                    required: vec!["input_file".to_string(), "output_file".to_string(), "strength".to_string()],
                },
            },
            // ================================================================
            // BATCH 4 — Audio Processing
            // ================================================================
            ClaudeTool {
                name: "compress_audio".to_string(),
                description: "Applies dynamic range compression to audio using FFmpeg acompressor".to_string(),
                input_schema: InputSchema {
                    schema_type: "object".to_string(),
                    properties: HashMap::from([
                        ("input_file".to_string(), PropertyDefinition {
                            prop_type: "string".to_string(),
                            description: "Path to the input file".to_string(),
                            items: None,
                        }),
                        ("output_file".to_string(), PropertyDefinition {
                            prop_type: "string".to_string(),
                            description: "Path to save the output file".to_string(),
                            items: None,
                        }),
                        ("threshold_db".to_string(), PropertyDefinition {
                            prop_type: "number".to_string(),
                            description: "Threshold in dB (-50 to 0) above which compression is applied".to_string(),
                            items: None,
                        }),
                        ("ratio".to_string(), PropertyDefinition {
                            prop_type: "number".to_string(),
                            description: "Compression ratio (1–20; 4:1 is typical)".to_string(),
                            items: None,
                        }),
                        ("attack_ms".to_string(), PropertyDefinition {
                            prop_type: "number".to_string(),
                            description: "Attack time in milliseconds".to_string(),
                            items: None,
                        }),
                        ("release_ms".to_string(), PropertyDefinition {
                            prop_type: "number".to_string(),
                            description: "Release time in milliseconds".to_string(),
                            items: None,
                        }),
                        ("makeup_gain_db".to_string(), PropertyDefinition {
                            prop_type: "number".to_string(),
                            description: "Makeup gain in dB to apply after compression".to_string(),
                            items: None,
                        }),
                    ]),
                    required: vec!["input_file".to_string(), "output_file".to_string(), "threshold_db".to_string(), "ratio".to_string()],
                },
            },
            ClaudeTool {
                name: "normalize_audio".to_string(),
                description: "Normalizes audio loudness to a target LUFS level using FFmpeg loudnorm".to_string(),
                input_schema: InputSchema {
                    schema_type: "object".to_string(),
                    properties: HashMap::from([
                        ("input_file".to_string(), PropertyDefinition {
                            prop_type: "string".to_string(),
                            description: "Path to the input file".to_string(),
                            items: None,
                        }),
                        ("output_file".to_string(), PropertyDefinition {
                            prop_type: "string".to_string(),
                            description: "Path to save the output file".to_string(),
                            items: None,
                        }),
                        ("target_lufs".to_string(), PropertyDefinition {
                            prop_type: "number".to_string(),
                            description: "Target loudness in LUFS (e.g. -16 for YouTube, -14 for streaming)".to_string(),
                            items: None,
                        }),
                        ("loudness_range_target".to_string(), PropertyDefinition {
                            prop_type: "number".to_string(),
                            description: "Loudness range target in LU (1–20)".to_string(),
                            items: None,
                        }),
                        ("true_peak_dbtp".to_string(), PropertyDefinition {
                            prop_type: "number".to_string(),
                            description: "Maximum true peak in dBTP (e.g. -1.0)".to_string(),
                            items: None,
                        }),
                    ]),
                    required: vec!["input_file".to_string(), "output_file".to_string(), "target_lufs".to_string()],
                },
            },
            ClaudeTool {
                name: "equalize_audio".to_string(),
                description: "Applies parametric equalization to a specific frequency band in the audio".to_string(),
                input_schema: InputSchema {
                    schema_type: "object".to_string(),
                    properties: HashMap::from([
                        ("input_file".to_string(), PropertyDefinition {
                            prop_type: "string".to_string(),
                            description: "Path to the input file".to_string(),
                            items: None,
                        }),
                        ("output_file".to_string(), PropertyDefinition {
                            prop_type: "string".to_string(),
                            description: "Path to save the output file".to_string(),
                            items: None,
                        }),
                        ("frequency_hz".to_string(), PropertyDefinition {
                            prop_type: "number".to_string(),
                            description: "Center frequency in Hz (e.g. 1000 for 1kHz)".to_string(),
                            items: None,
                        }),
                        ("gain_db".to_string(), PropertyDefinition {
                            prop_type: "number".to_string(),
                            description: "Gain in dB (-20 to 20; positive boosts, negative cuts)".to_string(),
                            items: None,
                        }),
                        ("bandwidth_hz".to_string(), PropertyDefinition {
                            prop_type: "number".to_string(),
                            description: "Bandwidth in Hz".to_string(),
                            items: None,
                        }),
                        ("eq_type".to_string(), PropertyDefinition {
                            prop_type: "string".to_string(),
                            description: "EQ type: peak, lowshelf, or highshelf".to_string(),
                            items: None,
                        }),
                    ]),
                    required: vec!["input_file".to_string(), "output_file".to_string(), "frequency_hz".to_string(), "gain_db".to_string()],
                },
            },
            ClaudeTool {
                name: "gate_audio".to_string(),
                description: "Applies a noise gate to audio, silencing signals below a threshold".to_string(),
                input_schema: InputSchema {
                    schema_type: "object".to_string(),
                    properties: HashMap::from([
                        ("input_file".to_string(), PropertyDefinition {
                            prop_type: "string".to_string(),
                            description: "Path to the input file".to_string(),
                            items: None,
                        }),
                        ("output_file".to_string(), PropertyDefinition {
                            prop_type: "string".to_string(),
                            description: "Path to save the output file".to_string(),
                            items: None,
                        }),
                        ("threshold_db".to_string(), PropertyDefinition {
                            prop_type: "number".to_string(),
                            description: "Gate threshold in dB (signals below this are attenuated)".to_string(),
                            items: None,
                        }),
                        ("ratio".to_string(), PropertyDefinition {
                            prop_type: "number".to_string(),
                            description: "Gate ratio (how much to attenuate below threshold)".to_string(),
                            items: None,
                        }),
                        ("attack_ms".to_string(), PropertyDefinition {
                            prop_type: "number".to_string(),
                            description: "Attack time in milliseconds".to_string(),
                            items: None,
                        }),
                        ("release_ms".to_string(), PropertyDefinition {
                            prop_type: "number".to_string(),
                            description: "Release time in milliseconds".to_string(),
                            items: None,
                        }),
                    ]),
                    required: vec!["input_file".to_string(), "output_file".to_string(), "threshold_db".to_string()],
                },
            },
            ClaudeTool {
                name: "denoise_audio".to_string(),
                description: "Reduces background noise from audio using FFmpeg afftdn spectral denoiser".to_string(),
                input_schema: InputSchema {
                    schema_type: "object".to_string(),
                    properties: HashMap::from([
                        ("input_file".to_string(), PropertyDefinition {
                            prop_type: "string".to_string(),
                            description: "Path to the input file".to_string(),
                            items: None,
                        }),
                        ("output_file".to_string(), PropertyDefinition {
                            prop_type: "string".to_string(),
                            description: "Path to save the output file".to_string(),
                            items: None,
                        }),
                        ("noise_floor_db".to_string(), PropertyDefinition {
                            prop_type: "number".to_string(),
                            description: "Estimated noise floor in dB (e.g. -40)".to_string(),
                            items: None,
                        }),
                        ("noise_reduction_db".to_string(), PropertyDefinition {
                            prop_type: "number".to_string(),
                            description: "Amount of noise reduction in dB".to_string(),
                            items: None,
                        }),
                        ("track_noise".to_string(), PropertyDefinition {
                            prop_type: "boolean".to_string(),
                            description: "Whether to track/adapt the noise profile over time".to_string(),
                            items: None,
                        }),
                    ]),
                    required: vec!["input_file".to_string(), "output_file".to_string()],
                },
            },
            // ================================================================
            // BATCH 5 — Video Composition & Layout
            // ================================================================
            ClaudeTool {
                name: "pad_video".to_string(),
                description: "Adds padding (borders/letterbox/pillarbox) around the video to reach a target resolution".to_string(),
                input_schema: InputSchema {
                    schema_type: "object".to_string(),
                    properties: HashMap::from([
                        ("input_file".to_string(), PropertyDefinition {
                            prop_type: "string".to_string(),
                            description: "Path to the input video file".to_string(),
                            items: None,
                        }),
                        ("output_file".to_string(), PropertyDefinition {
                            prop_type: "string".to_string(),
                            description: "Path to save the padded video".to_string(),
                            items: None,
                        }),
                        ("width".to_string(), PropertyDefinition {
                            prop_type: "number".to_string(),
                            description: "Total output width in pixels".to_string(),
                            items: None,
                        }),
                        ("height".to_string(), PropertyDefinition {
                            prop_type: "number".to_string(),
                            description: "Total output height in pixels".to_string(),
                            items: None,
                        }),
                        ("x_offset".to_string(), PropertyDefinition {
                            prop_type: "number".to_string(),
                            description: "Horizontal offset of the video within the padded frame".to_string(),
                            items: None,
                        }),
                        ("y_offset".to_string(), PropertyDefinition {
                            prop_type: "number".to_string(),
                            description: "Vertical offset of the video within the padded frame".to_string(),
                            items: None,
                        }),
                        ("color".to_string(), PropertyDefinition {
                            prop_type: "string".to_string(),
                            description: "Padding color (e.g. 'black', 'white', 'blue', or hex like '0x1a1a1a')".to_string(),
                            items: None,
                        }),
                    ]),
                    required: vec!["input_file".to_string(), "output_file".to_string(), "width".to_string(), "height".to_string()],
                },
            },
            ClaudeTool {
                name: "blend_videos".to_string(),
                description: "Blends two video layers together using a compositing blend mode".to_string(),
                input_schema: InputSchema {
                    schema_type: "object".to_string(),
                    properties: HashMap::from([
                        ("input_file1".to_string(), PropertyDefinition {
                            prop_type: "string".to_string(),
                            description: "Path to the base (bottom) video file".to_string(),
                            items: None,
                        }),
                        ("input_file2".to_string(), PropertyDefinition {
                            prop_type: "string".to_string(),
                            description: "Path to the overlay (top) video file".to_string(),
                            items: None,
                        }),
                        ("output_file".to_string(), PropertyDefinition {
                            prop_type: "string".to_string(),
                            description: "Path to save the blended video".to_string(),
                            items: None,
                        }),
                        ("blend_mode".to_string(), PropertyDefinition {
                            prop_type: "string".to_string(),
                            description: "Blend mode: addition, multiply, screen, overlay, hardlight, softlight, difference, exclusion".to_string(),
                            items: None,
                        }),
                        ("opacity".to_string(), PropertyDefinition {
                            prop_type: "number".to_string(),
                            description: "Opacity of the blend (0.0–1.0)".to_string(),
                            items: None,
                        }),
                    ]),
                    required: vec!["input_file1".to_string(), "input_file2".to_string(), "output_file".to_string(), "blend_mode".to_string()],
                },
            },
            ClaudeTool {
                name: "stack_videos".to_string(),
                description: "Stacks two videos side by side (horizontal) or top to bottom (vertical)".to_string(),
                input_schema: InputSchema {
                    schema_type: "object".to_string(),
                    properties: HashMap::from([
                        ("input_file1".to_string(), PropertyDefinition {
                            prop_type: "string".to_string(),
                            description: "Path to the first video file".to_string(),
                            items: None,
                        }),
                        ("input_file2".to_string(), PropertyDefinition {
                            prop_type: "string".to_string(),
                            description: "Path to the second video file".to_string(),
                            items: None,
                        }),
                        ("output_file".to_string(), PropertyDefinition {
                            prop_type: "string".to_string(),
                            description: "Path to save the stacked video".to_string(),
                            items: None,
                        }),
                        ("direction".to_string(), PropertyDefinition {
                            prop_type: "string".to_string(),
                            description: "Stack direction: 'horizontal' (side by side) or 'vertical' (top/bottom)".to_string(),
                            items: None,
                        }),
                    ]),
                    required: vec!["input_file1".to_string(), "input_file2".to_string(), "output_file".to_string()],
                },
            },
            ClaudeTool {
                name: "add_vignette".to_string(),
                description: "Adds a vignette effect (darkened edges) to the video".to_string(),
                input_schema: InputSchema {
                    schema_type: "object".to_string(),
                    properties: HashMap::from([
                        ("input_file".to_string(), PropertyDefinition {
                            prop_type: "string".to_string(),
                            description: "Path to the input video file".to_string(),
                            items: None,
                        }),
                        ("output_file".to_string(), PropertyDefinition {
                            prop_type: "string".to_string(),
                            description: "Path to save the output video".to_string(),
                            items: None,
                        }),
                        ("angle".to_string(), PropertyDefinition {
                            prop_type: "number".to_string(),
                            description: "Vignette angle in radians (0 to 1.5708; PI/4 ≈ 0.785 is typical)".to_string(),
                            items: None,
                        }),
                        ("mode".to_string(), PropertyDefinition {
                            prop_type: "string".to_string(),
                            description: "Vignette direction: 'forward' (darken edges) or 'backward' (brighten edges)".to_string(),
                            items: None,
                        }),
                    ]),
                    required: vec!["input_file".to_string(), "output_file".to_string()],
                },
            },
            ClaudeTool {
                name: "draw_box".to_string(),
                description: "Draws a rectangle/box on the video, useful for highlighting areas or creating borders".to_string(),
                input_schema: InputSchema {
                    schema_type: "object".to_string(),
                    properties: HashMap::from([
                        ("input_file".to_string(), PropertyDefinition {
                            prop_type: "string".to_string(),
                            description: "Path to the input video file".to_string(),
                            items: None,
                        }),
                        ("output_file".to_string(), PropertyDefinition {
                            prop_type: "string".to_string(),
                            description: "Path to save the output video".to_string(),
                            items: None,
                        }),
                        ("x".to_string(), PropertyDefinition {
                            prop_type: "number".to_string(),
                            description: "X coordinate of the box top-left corner".to_string(),
                            items: None,
                        }),
                        ("y".to_string(), PropertyDefinition {
                            prop_type: "number".to_string(),
                            description: "Y coordinate of the box top-left corner".to_string(),
                            items: None,
                        }),
                        ("width".to_string(), PropertyDefinition {
                            prop_type: "number".to_string(),
                            description: "Width of the box in pixels".to_string(),
                            items: None,
                        }),
                        ("height".to_string(), PropertyDefinition {
                            prop_type: "number".to_string(),
                            description: "Height of the box in pixels".to_string(),
                            items: None,
                        }),
                        ("color".to_string(), PropertyDefinition {
                            prop_type: "string".to_string(),
                            description: "Box color (e.g. 'red', 'white', 'yellow@0.5' for transparency)".to_string(),
                            items: None,
                        }),
                        ("thickness".to_string(), PropertyDefinition {
                            prop_type: "number".to_string(),
                            description: "Border thickness in pixels (0 = filled box)".to_string(),
                            items: None,
                        }),
                    ]),
                    required: vec!["input_file".to_string(), "output_file".to_string(), "x".to_string(), "y".to_string(), "width".to_string(), "height".to_string(), "color".to_string()],
                },
            },
            // ================================================================
            // BATCH 6 — Motion, Time & Frame Effects
            // ================================================================
            ClaudeTool {
                name: "reverse_video".to_string(),
                description: "Reverses both video and audio of a clip to play it backwards".to_string(),
                input_schema: InputSchema {
                    schema_type: "object".to_string(),
                    properties: HashMap::from([
                        ("input_file".to_string(), PropertyDefinition {
                            prop_type: "string".to_string(),
                            description: "Path to the input video file".to_string(),
                            items: None,
                        }),
                        ("output_file".to_string(), PropertyDefinition {
                            prop_type: "string".to_string(),
                            description: "Path to save the reversed video".to_string(),
                            items: None,
                        }),
                    ]),
                    required: vec!["input_file".to_string(), "output_file".to_string()],
                },
            },
            ClaudeTool {
                name: "loop_video".to_string(),
                description: "Loops a video a specified number of times, capped to a maximum duration".to_string(),
                input_schema: InputSchema {
                    schema_type: "object".to_string(),
                    properties: HashMap::from([
                        ("input_file".to_string(), PropertyDefinition {
                            prop_type: "string".to_string(),
                            description: "Path to the input video file".to_string(),
                            items: None,
                        }),
                        ("output_file".to_string(), PropertyDefinition {
                            prop_type: "string".to_string(),
                            description: "Path to save the looped video".to_string(),
                            items: None,
                        }),
                        ("loop_count".to_string(), PropertyDefinition {
                            prop_type: "number".to_string(),
                            description: "Number of loops (-1 for infinite, limited by loop_duration_sec)".to_string(),
                            items: None,
                        }),
                        ("loop_duration_sec".to_string(), PropertyDefinition {
                            prop_type: "number".to_string(),
                            description: "Maximum duration of the output in seconds".to_string(),
                            items: None,
                        }),
                    ]),
                    required: vec!["input_file".to_string(), "output_file".to_string(), "loop_count".to_string(), "loop_duration_sec".to_string()],
                },
            },
            ClaudeTool {
                name: "zoompan".to_string(),
                description: "Applies a Ken Burns-style zoom and pan effect to a video or image".to_string(),
                input_schema: InputSchema {
                    schema_type: "object".to_string(),
                    properties: HashMap::from([
                        ("input_file".to_string(), PropertyDefinition {
                            prop_type: "string".to_string(),
                            description: "Path to the input video or image file".to_string(),
                            items: None,
                        }),
                        ("output_file".to_string(), PropertyDefinition {
                            prop_type: "string".to_string(),
                            description: "Path to save the output video".to_string(),
                            items: None,
                        }),
                        ("zoom_factor".to_string(), PropertyDefinition {
                            prop_type: "number".to_string(),
                            description: "Zoom level (1.0 = no zoom, 2.0 = 2x zoom)".to_string(),
                            items: None,
                        }),
                        ("x_expr".to_string(), PropertyDefinition {
                            prop_type: "string".to_string(),
                            description: "X pan expression (default: 'iw/2-(iw/zoom/2)' for center)".to_string(),
                            items: None,
                        }),
                        ("y_expr".to_string(), PropertyDefinition {
                            prop_type: "string".to_string(),
                            description: "Y pan expression (default: 'ih/2-(ih/zoom/2)' for center)".to_string(),
                            items: None,
                        }),
                        ("duration_frames".to_string(), PropertyDefinition {
                            prop_type: "number".to_string(),
                            description: "Duration of the effect in frames".to_string(),
                            items: None,
                        }),
                        ("fps".to_string(), PropertyDefinition {
                            prop_type: "number".to_string(),
                            description: "Output frames per second (default: 25)".to_string(),
                            items: None,
                        }),
                    ]),
                    required: vec!["input_file".to_string(), "output_file".to_string(), "zoom_factor".to_string(), "duration_frames".to_string()],
                },
            },
            ClaudeTool {
                name: "minterpolate".to_string(),
                description: "Increases video frame rate using motion interpolation for smooth slow-motion or high-fps output".to_string(),
                input_schema: InputSchema {
                    schema_type: "object".to_string(),
                    properties: HashMap::from([
                        ("input_file".to_string(), PropertyDefinition {
                            prop_type: "string".to_string(),
                            description: "Path to the input video file".to_string(),
                            items: None,
                        }),
                        ("output_file".to_string(), PropertyDefinition {
                            prop_type: "string".to_string(),
                            description: "Path to save the interpolated video".to_string(),
                            items: None,
                        }),
                        ("fps_target".to_string(), PropertyDefinition {
                            prop_type: "number".to_string(),
                            description: "Target frame rate (e.g. 60 for 60fps output)".to_string(),
                            items: None,
                        }),
                        ("mi_mode".to_string(), PropertyDefinition {
                            prop_type: "string".to_string(),
                            description: "Interpolation mode: dup (duplicate frames), blend (blend frames), mci (motion-compensated, best quality)".to_string(),
                            items: None,
                        }),
                    ]),
                    required: vec!["input_file".to_string(), "output_file".to_string(), "fps_target".to_string()],
                },
            },
            // ================================================================
            // BATCH 7 — Media Analysis Tools
            // ================================================================
            ClaudeTool {
                name: "detect_scene_changes".to_string(),
                description: "Detects scene changes in a video and returns timestamps of each cut".to_string(),
                input_schema: InputSchema {
                    schema_type: "object".to_string(),
                    properties: HashMap::from([
                        ("input_file".to_string(), PropertyDefinition {
                            prop_type: "string".to_string(),
                            description: "Path to the input video file".to_string(),
                            items: None,
                        }),
                        ("threshold".to_string(), PropertyDefinition {
                            prop_type: "number".to_string(),
                            description: "Detection threshold 0–100 (lower = more sensitive; default 40)".to_string(),
                            items: None,
                        }),
                    ]),
                    required: vec!["input_file".to_string()],
                },
            },
            ClaudeTool {
                name: "measure_loudness".to_string(),
                description: "Measures the audio loudness (mean and max volume) of a video or audio file".to_string(),
                input_schema: InputSchema {
                    schema_type: "object".to_string(),
                    properties: HashMap::from([
                        ("input_file".to_string(), PropertyDefinition {
                            prop_type: "string".to_string(),
                            description: "Path to the video or audio file to analyze".to_string(),
                            items: None,
                        }),
                    ]),
                    required: vec!["input_file".to_string()],
                },
            },
            ClaudeTool {
                name: "detect_silence".to_string(),
                description: "Detects silent segments in audio/video and returns their timestamps and durations".to_string(),
                input_schema: InputSchema {
                    schema_type: "object".to_string(),
                    properties: HashMap::from([
                        ("input_file".to_string(), PropertyDefinition {
                            prop_type: "string".to_string(),
                            description: "Path to the video or audio file".to_string(),
                            items: None,
                        }),
                        ("noise_tolerance_db".to_string(), PropertyDefinition {
                            prop_type: "number".to_string(),
                            description: "Noise floor in dB; signals below this are considered silence (default -60)".to_string(),
                            items: None,
                        }),
                        ("min_duration_sec".to_string(), PropertyDefinition {
                            prop_type: "number".to_string(),
                            description: "Minimum silence duration in seconds to detect (default 0.1)".to_string(),
                            items: None,
                        }),
                    ]),
                    required: vec!["input_file".to_string()],
                },
            },
            ClaudeTool {
                name: "export_custom_quality".to_string(),
                description: "Exports a video with custom quality, resolution, and bitrate settings".to_string(),
                input_schema: InputSchema {
                    schema_type: "object".to_string(),
                    properties: HashMap::from([
                        ("input_file".to_string(), PropertyDefinition {
                            prop_type: "string".to_string(),
                            description: "Path to the input video file".to_string(),
                            items: None,
                        }),
                        ("output_file".to_string(), PropertyDefinition {
                            prop_type: "string".to_string(),
                            description: "Path to save the exported video".to_string(),
                            items: None,
                        }),
                        ("quality".to_string(), PropertyDefinition {
                            prop_type: "string".to_string(),
                            description: "Quality preset: 'low', 'medium', 'high', or 'ultra'".to_string(),
                            items: None,
                        }),
                        ("width".to_string(), PropertyDefinition {
                            prop_type: "number".to_string(),
                            description: "Optional output width in pixels (e.g. 1920)".to_string(),
                            items: None,
                        }),
                        ("height".to_string(), PropertyDefinition {
                            prop_type: "number".to_string(),
                            description: "Optional output height in pixels (e.g. 1080)".to_string(),
                            items: None,
                        }),
                        ("bitrate_kbps".to_string(), PropertyDefinition {
                            prop_type: "number".to_string(),
                            description: "Optional video bitrate in kbps (e.g. 8000 for 8Mbps). Overrides quality CRF.".to_string(),
                            items: None,
                        }),
                    ]),
                    required: vec!["input_file".to_string(), "output_file".to_string(), "quality".to_string()],
                },
            },
            // ================================================================
            // BATCH 8 — Advanced Color Grading
            // ================================================================
            ClaudeTool {
                name: "adjust_curves".to_string(),
                description: "Adjusts color curves using the FFmpeg curves filter. Supports presets or custom per-channel control points.".to_string(),
                input_schema: InputSchema {
                    schema_type: "object".to_string(),
                    properties: HashMap::from([
                        ("input_file".to_string(), PropertyDefinition { prop_type: "string".to_string(), description: "Path to the input video file".to_string(), items: None }),
                        ("output_file".to_string(), PropertyDefinition { prop_type: "string".to_string(), description: "Path to save the output video".to_string(), items: None }),
                        ("preset".to_string(), PropertyDefinition { prop_type: "string".to_string(), description: "Preset name: none/color_negative/cross_process/darker/increase_contrast/lighter/linear_contrast/medium_contrast/negative/strong_contrast/vintage".to_string(), items: None }),
                        ("master".to_string(), PropertyDefinition { prop_type: "string".to_string(), description: "Master curve control points e.g. '0/0 0.5/0.6 1/1'".to_string(), items: None }),
                        ("red_channel".to_string(), PropertyDefinition { prop_type: "string".to_string(), description: "Red channel curve control points".to_string(), items: None }),
                        ("green_channel".to_string(), PropertyDefinition { prop_type: "string".to_string(), description: "Green channel curve control points".to_string(), items: None }),
                        ("blue_channel".to_string(), PropertyDefinition { prop_type: "string".to_string(), description: "Blue channel curve control points".to_string(), items: None }),
                    ]),
                    required: vec!["input_file".to_string(), "output_file".to_string()],
                },
            },
            ClaudeTool {
                name: "adjust_levels".to_string(),
                description: "Adjusts input/output levels per channel using the FFmpeg colorlevels filter.".to_string(),
                input_schema: InputSchema {
                    schema_type: "object".to_string(),
                    properties: HashMap::from([
                        ("input_file".to_string(), PropertyDefinition { prop_type: "string".to_string(), description: "Path to the input video file".to_string(), items: None }),
                        ("output_file".to_string(), PropertyDefinition { prop_type: "string".to_string(), description: "Path to save the output video".to_string(), items: None }),
                        ("rimin".to_string(), PropertyDefinition { prop_type: "number".to_string(), description: "Red input black point (0.0–1.0, default 0.0)".to_string(), items: None }),
                        ("rimax".to_string(), PropertyDefinition { prop_type: "number".to_string(), description: "Red input white point (0.0–1.0, default 1.0)".to_string(), items: None }),
                        ("gimin".to_string(), PropertyDefinition { prop_type: "number".to_string(), description: "Green input black point (0.0–1.0, default 0.0)".to_string(), items: None }),
                        ("gimax".to_string(), PropertyDefinition { prop_type: "number".to_string(), description: "Green input white point (0.0–1.0, default 1.0)".to_string(), items: None }),
                        ("bimin".to_string(), PropertyDefinition { prop_type: "number".to_string(), description: "Blue input black point (0.0–1.0, default 0.0)".to_string(), items: None }),
                        ("bimax".to_string(), PropertyDefinition { prop_type: "number".to_string(), description: "Blue input white point (0.0–1.0, default 1.0)".to_string(), items: None }),
                        ("romin".to_string(), PropertyDefinition { prop_type: "number".to_string(), description: "Red output black point (0.0–1.0, default 0.0)".to_string(), items: None }),
                        ("romax".to_string(), PropertyDefinition { prop_type: "number".to_string(), description: "Red output white point (0.0–1.0, default 1.0)".to_string(), items: None }),
                        ("gomin".to_string(), PropertyDefinition { prop_type: "number".to_string(), description: "Green output black point (0.0–1.0, default 0.0)".to_string(), items: None }),
                        ("gomax".to_string(), PropertyDefinition { prop_type: "number".to_string(), description: "Green output white point (0.0–1.0, default 1.0)".to_string(), items: None }),
                        ("bomin".to_string(), PropertyDefinition { prop_type: "number".to_string(), description: "Blue output black point (0.0–1.0, default 0.0)".to_string(), items: None }),
                        ("bomax".to_string(), PropertyDefinition { prop_type: "number".to_string(), description: "Blue output white point (0.0–1.0, default 1.0)".to_string(), items: None }),
                    ]),
                    required: vec!["input_file".to_string(), "output_file".to_string()],
                },
            },
            ClaudeTool {
                name: "split_tone".to_string(),
                description: "Applies split toning to shadows and highlights using the FFmpeg colorbalance filter.".to_string(),
                input_schema: InputSchema {
                    schema_type: "object".to_string(),
                    properties: HashMap::from([
                        ("input_file".to_string(), PropertyDefinition { prop_type: "string".to_string(), description: "Path to the input video file".to_string(), items: None }),
                        ("output_file".to_string(), PropertyDefinition { prop_type: "string".to_string(), description: "Path to save the output video".to_string(), items: None }),
                        ("shadow_hue".to_string(), PropertyDefinition { prop_type: "number".to_string(), description: "Hue angle for shadows (0–360)".to_string(), items: None }),
                        ("shadow_saturation".to_string(), PropertyDefinition { prop_type: "number".to_string(), description: "Saturation for shadows (0.0–1.0)".to_string(), items: None }),
                        ("highlight_hue".to_string(), PropertyDefinition { prop_type: "number".to_string(), description: "Hue angle for highlights (0–360)".to_string(), items: None }),
                        ("highlight_saturation".to_string(), PropertyDefinition { prop_type: "number".to_string(), description: "Saturation for highlights (0.0–1.0)".to_string(), items: None }),
                        ("balance".to_string(), PropertyDefinition { prop_type: "number".to_string(), description: "Midtone bias toward shadows/highlights (-1.0 to 1.0)".to_string(), items: None }),
                    ]),
                    required: vec!["input_file".to_string(), "output_file".to_string()],
                },
            },
            ClaudeTool {
                name: "convert_colorspace".to_string(),
                description: "Converts video colorspace using the FFmpeg colorspace filter.".to_string(),
                input_schema: InputSchema {
                    schema_type: "object".to_string(),
                    properties: HashMap::from([
                        ("input_file".to_string(), PropertyDefinition { prop_type: "string".to_string(), description: "Path to the input video file".to_string(), items: None }),
                        ("output_file".to_string(), PropertyDefinition { prop_type: "string".to_string(), description: "Path to save the output video".to_string(), items: None }),
                        ("colorspace".to_string(), PropertyDefinition { prop_type: "string".to_string(), description: "Target colorspace: bt709/bt2020/smpte170m/smpte240m".to_string(), items: None }),
                        ("transfer_characteristics".to_string(), PropertyDefinition { prop_type: "string".to_string(), description: "Transfer characteristics: bt709/bt2020-10/smpte2084/arib-std-b67".to_string(), items: None }),
                        ("color_primaries".to_string(), PropertyDefinition { prop_type: "string".to_string(), description: "Color primaries: bt709/bt2020/smpte170m".to_string(), items: None }),
                    ]),
                    required: vec!["input_file".to_string(), "output_file".to_string(), "colorspace".to_string()],
                },
            },
            ClaudeTool {
                name: "apply_tonemap".to_string(),
                description: "Applies HDR to SDR tonemapping using the FFmpeg tonemap filter.".to_string(),
                input_schema: InputSchema {
                    schema_type: "object".to_string(),
                    properties: HashMap::from([
                        ("input_file".to_string(), PropertyDefinition { prop_type: "string".to_string(), description: "Path to the input video file".to_string(), items: None }),
                        ("output_file".to_string(), PropertyDefinition { prop_type: "string".to_string(), description: "Path to save the output video".to_string(), items: None }),
                        ("algorithm".to_string(), PropertyDefinition { prop_type: "string".to_string(), description: "Tonemapping algorithm: none/linear/gamma/clip/reinhard/hable/mobius".to_string(), items: None }),
                        ("peak".to_string(), PropertyDefinition { prop_type: "number".to_string(), description: "Reference peak luminance (0 = auto-detect)".to_string(), items: None }),
                        ("desat".to_string(), PropertyDefinition { prop_type: "number".to_string(), description: "Desaturation strength for bright colors (0.0–1.0, default 0.5)".to_string(), items: None }),
                    ]),
                    required: vec!["input_file".to_string(), "output_file".to_string(), "algorithm".to_string()],
                },
            },
            // ================================================================
            // BATCH 9 — Audio Tone Shaping
            // ================================================================
            ClaudeTool {
                name: "filter_highpass".to_string(),
                description: "Applies a high-pass filter to audio, removing frequencies below the cutoff using FFmpeg highpass filter.".to_string(),
                input_schema: InputSchema {
                    schema_type: "object".to_string(),
                    properties: HashMap::from([
                        ("input_file".to_string(), PropertyDefinition { prop_type: "string".to_string(), description: "Path to the input file".to_string(), items: None }),
                        ("output_file".to_string(), PropertyDefinition { prop_type: "string".to_string(), description: "Path to save the output file".to_string(), items: None }),
                        ("frequency_hz".to_string(), PropertyDefinition { prop_type: "number".to_string(), description: "Cutoff frequency in Hz".to_string(), items: None }),
                        ("poles".to_string(), PropertyDefinition { prop_type: "number".to_string(), description: "Number of poles (1 or 2, default 2)".to_string(), items: None }),
                        ("width_hz".to_string(), PropertyDefinition { prop_type: "number".to_string(), description: "Filter width in Hz (default 0.707)".to_string(), items: None }),
                    ]),
                    required: vec!["input_file".to_string(), "output_file".to_string(), "frequency_hz".to_string()],
                },
            },
            ClaudeTool {
                name: "filter_lowpass".to_string(),
                description: "Applies a low-pass filter to audio, removing frequencies above the cutoff using FFmpeg lowpass filter.".to_string(),
                input_schema: InputSchema {
                    schema_type: "object".to_string(),
                    properties: HashMap::from([
                        ("input_file".to_string(), PropertyDefinition { prop_type: "string".to_string(), description: "Path to the input file".to_string(), items: None }),
                        ("output_file".to_string(), PropertyDefinition { prop_type: "string".to_string(), description: "Path to save the output file".to_string(), items: None }),
                        ("frequency_hz".to_string(), PropertyDefinition { prop_type: "number".to_string(), description: "Cutoff frequency in Hz".to_string(), items: None }),
                        ("poles".to_string(), PropertyDefinition { prop_type: "number".to_string(), description: "Number of poles (1 or 2, default 2)".to_string(), items: None }),
                        ("width_hz".to_string(), PropertyDefinition { prop_type: "number".to_string(), description: "Filter width in Hz (default 0.707)".to_string(), items: None }),
                    ]),
                    required: vec!["input_file".to_string(), "output_file".to_string(), "frequency_hz".to_string()],
                },
            },
            ClaudeTool {
                name: "adjust_bass".to_string(),
                description: "Boosts or cuts bass frequencies using the FFmpeg bass/lowshelf filter.".to_string(),
                input_schema: InputSchema {
                    schema_type: "object".to_string(),
                    properties: HashMap::from([
                        ("input_file".to_string(), PropertyDefinition { prop_type: "string".to_string(), description: "Path to the input file".to_string(), items: None }),
                        ("output_file".to_string(), PropertyDefinition { prop_type: "string".to_string(), description: "Path to save the output file".to_string(), items: None }),
                        ("gain_db".to_string(), PropertyDefinition { prop_type: "number".to_string(), description: "Gain in dB (-20 to 20)".to_string(), items: None }),
                        ("frequency_hz".to_string(), PropertyDefinition { prop_type: "number".to_string(), description: "Center frequency in Hz (default 100)".to_string(), items: None }),
                        ("width_hz".to_string(), PropertyDefinition { prop_type: "number".to_string(), description: "Shelf width in Hz (default 0.5)".to_string(), items: None }),
                    ]),
                    required: vec!["input_file".to_string(), "output_file".to_string(), "gain_db".to_string()],
                },
            },
            ClaudeTool {
                name: "adjust_treble".to_string(),
                description: "Boosts or cuts treble frequencies using the FFmpeg treble/highshelf filter.".to_string(),
                input_schema: InputSchema {
                    schema_type: "object".to_string(),
                    properties: HashMap::from([
                        ("input_file".to_string(), PropertyDefinition { prop_type: "string".to_string(), description: "Path to the input file".to_string(), items: None }),
                        ("output_file".to_string(), PropertyDefinition { prop_type: "string".to_string(), description: "Path to save the output file".to_string(), items: None }),
                        ("gain_db".to_string(), PropertyDefinition { prop_type: "number".to_string(), description: "Gain in dB (-20 to 20)".to_string(), items: None }),
                        ("frequency_hz".to_string(), PropertyDefinition { prop_type: "number".to_string(), description: "Center frequency in Hz (default 3000)".to_string(), items: None }),
                        ("width_hz".to_string(), PropertyDefinition { prop_type: "number".to_string(), description: "Shelf width in Hz (default 0.5)".to_string(), items: None }),
                    ]),
                    required: vec!["input_file".to_string(), "output_file".to_string(), "gain_db".to_string()],
                },
            },
            ClaudeTool {
                name: "audio_compand".to_string(),
                description: "Applies dynamic range compression/expansion using the FFmpeg compand filter.".to_string(),
                input_schema: InputSchema {
                    schema_type: "object".to_string(),
                    properties: HashMap::from([
                        ("input_file".to_string(), PropertyDefinition { prop_type: "string".to_string(), description: "Path to the input file".to_string(), items: None }),
                        ("output_file".to_string(), PropertyDefinition { prop_type: "string".to_string(), description: "Path to save the output file".to_string(), items: None }),
                        ("attacks".to_string(), PropertyDefinition { prop_type: "string".to_string(), description: "Attack time(s) per channel comma-separated (default '0.3')".to_string(), items: None }),
                        ("decays".to_string(), PropertyDefinition { prop_type: "string".to_string(), description: "Decay time(s) per channel (default '0.8')".to_string(), items: None }),
                        ("points".to_string(), PropertyDefinition { prop_type: "string".to_string(), description: "Input/output level pairs (default '-70/-70 -60/-20 1/0')".to_string(), items: None }),
                        ("soft_knee_db".to_string(), PropertyDefinition { prop_type: "number".to_string(), description: "Soft knee width in dB (default 0.01)".to_string(), items: None }),
                        ("gain_db".to_string(), PropertyDefinition { prop_type: "number".to_string(), description: "Output gain in dB (default 0)".to_string(), items: None }),
                    ]),
                    required: vec!["input_file".to_string(), "output_file".to_string()],
                },
            },
            ClaudeTool {
                name: "add_audio_delay".to_string(),
                description: "Adds delay to audio channels using the FFmpeg adelay filter.".to_string(),
                input_schema: InputSchema {
                    schema_type: "object".to_string(),
                    properties: HashMap::from([
                        ("input_file".to_string(), PropertyDefinition { prop_type: "string".to_string(), description: "Path to the input file".to_string(), items: None }),
                        ("output_file".to_string(), PropertyDefinition { prop_type: "string".to_string(), description: "Path to save the output file".to_string(), items: None }),
                        ("delays_ms".to_string(), PropertyDefinition { prop_type: "string".to_string(), description: "Delay in ms per channel e.g. '500|500' or '1000'".to_string(), items: None }),
                        ("all_channels".to_string(), PropertyDefinition { prop_type: "boolean".to_string(), description: "Apply same delay to all channels (default true)".to_string(), items: None }),
                    ]),
                    required: vec!["input_file".to_string(), "output_file".to_string(), "delays_ms".to_string()],
                },
            },
            ClaudeTool {
                name: "add_phaser".to_string(),
                description: "Adds a phaser effect to audio using the FFmpeg aphaser filter.".to_string(),
                input_schema: InputSchema {
                    schema_type: "object".to_string(),
                    properties: HashMap::from([
                        ("input_file".to_string(), PropertyDefinition { prop_type: "string".to_string(), description: "Path to the input file".to_string(), items: None }),
                        ("output_file".to_string(), PropertyDefinition { prop_type: "string".to_string(), description: "Path to save the output file".to_string(), items: None }),
                        ("in_gain".to_string(), PropertyDefinition { prop_type: "number".to_string(), description: "Input gain (default 0.4)".to_string(), items: None }),
                        ("out_gain".to_string(), PropertyDefinition { prop_type: "number".to_string(), description: "Output gain (default 0.74)".to_string(), items: None }),
                        ("delay_ms".to_string(), PropertyDefinition { prop_type: "number".to_string(), description: "Delay in milliseconds (default 3.0)".to_string(), items: None }),
                        ("decay".to_string(), PropertyDefinition { prop_type: "number".to_string(), description: "Decay (0–1, default 0.4)".to_string(), items: None }),
                        ("speed_hz".to_string(), PropertyDefinition { prop_type: "number".to_string(), description: "Modulation speed in Hz (default 0.5)".to_string(), items: None }),
                        ("type".to_string(), PropertyDefinition { prop_type: "string".to_string(), description: "Waveform type: triangular/sinusoidal (default triangular)".to_string(), items: None }),
                    ]),
                    required: vec!["input_file".to_string(), "output_file".to_string()],
                },
            },
            // ================================================================
            // BATCH 10 — Audio Restoration
            // ================================================================
            ClaudeTool {
                name: "remove_clicks".to_string(),
                description: "Removes clicks and pops from audio using the FFmpeg adeclick filter.".to_string(),
                input_schema: InputSchema {
                    schema_type: "object".to_string(),
                    properties: HashMap::from([
                        ("input_file".to_string(), PropertyDefinition { prop_type: "string".to_string(), description: "Path to the input file".to_string(), items: None }),
                        ("output_file".to_string(), PropertyDefinition { prop_type: "string".to_string(), description: "Path to save the output file".to_string(), items: None }),
                        ("window_ms".to_string(), PropertyDefinition { prop_type: "number".to_string(), description: "Analysis window size in ms (55–100, default 55)".to_string(), items: None }),
                        ("overlap_pct".to_string(), PropertyDefinition { prop_type: "number".to_string(), description: "Window overlap percentage (50–95, default 75)".to_string(), items: None }),
                        ("ar_order".to_string(), PropertyDefinition { prop_type: "number".to_string(), description: "AR model order (default 2)".to_string(), items: None }),
                        ("threshold".to_string(), PropertyDefinition { prop_type: "number".to_string(), description: "Detection threshold (1–100, default 2)".to_string(), items: None }),
                    ]),
                    required: vec!["input_file".to_string(), "output_file".to_string()],
                },
            },
            ClaudeTool {
                name: "restore_clipping".to_string(),
                description: "Restores clipped audio samples using the FFmpeg adeclip filter.".to_string(),
                input_schema: InputSchema {
                    schema_type: "object".to_string(),
                    properties: HashMap::from([
                        ("input_file".to_string(), PropertyDefinition { prop_type: "string".to_string(), description: "Path to the input file".to_string(), items: None }),
                        ("output_file".to_string(), PropertyDefinition { prop_type: "string".to_string(), description: "Path to save the output file".to_string(), items: None }),
                        ("window_ms".to_string(), PropertyDefinition { prop_type: "number".to_string(), description: "Analysis window size in ms (default 55)".to_string(), items: None }),
                        ("overlap_pct".to_string(), PropertyDefinition { prop_type: "number".to_string(), description: "Window overlap percentage (default 75)".to_string(), items: None }),
                        ("ar_order".to_string(), PropertyDefinition { prop_type: "number".to_string(), description: "AR model order (default 8)".to_string(), items: None }),
                        ("threshold".to_string(), PropertyDefinition { prop_type: "number".to_string(), description: "Detection threshold (default 10)".to_string(), items: None }),
                    ]),
                    required: vec!["input_file".to_string(), "output_file".to_string()],
                },
            },
            ClaudeTool {
                name: "remove_silence".to_string(),
                description: "Removes silence from audio using the FFmpeg silenceremove filter.".to_string(),
                input_schema: InputSchema {
                    schema_type: "object".to_string(),
                    properties: HashMap::from([
                        ("input_file".to_string(), PropertyDefinition { prop_type: "string".to_string(), description: "Path to the input file".to_string(), items: None }),
                        ("output_file".to_string(), PropertyDefinition { prop_type: "string".to_string(), description: "Path to save the output file".to_string(), items: None }),
                        ("start_periods".to_string(), PropertyDefinition { prop_type: "number".to_string(), description: "Silence periods to remove at start (default 1)".to_string(), items: None }),
                        ("start_threshold_db".to_string(), PropertyDefinition { prop_type: "number".to_string(), description: "Start silence threshold in dB (default -50)".to_string(), items: None }),
                        ("stop_periods".to_string(), PropertyDefinition { prop_type: "number".to_string(), description: "Silence periods to remove throughout (-1 = all, default -1)".to_string(), items: None }),
                        ("stop_threshold_db".to_string(), PropertyDefinition { prop_type: "number".to_string(), description: "Stop silence threshold in dB (default -50)".to_string(), items: None }),
                        ("stop_duration_sec".to_string(), PropertyDefinition { prop_type: "number".to_string(), description: "Minimum silence duration to remove in seconds (default 0.1)".to_string(), items: None }),
                    ]),
                    required: vec!["input_file".to_string(), "output_file".to_string()],
                },
            },
            // ================================================================
            // BATCH 11 — Quality Metrics
            // ================================================================
            ClaudeTool {
                name: "compare_ssim".to_string(),
                description: "Computes SSIM (Structural Similarity Index) between a reference and distorted video.".to_string(),
                input_schema: InputSchema {
                    schema_type: "object".to_string(),
                    properties: HashMap::from([
                        ("reference_file".to_string(), PropertyDefinition { prop_type: "string".to_string(), description: "Path to the original/reference video".to_string(), items: None }),
                        ("distorted_file".to_string(), PropertyDefinition { prop_type: "string".to_string(), description: "Path to the processed/distorted video".to_string(), items: None }),
                    ]),
                    required: vec!["reference_file".to_string(), "distorted_file".to_string()],
                },
            },
            ClaudeTool {
                name: "compare_psnr".to_string(),
                description: "Computes PSNR (Peak Signal-to-Noise Ratio) between a reference and distorted video.".to_string(),
                input_schema: InputSchema {
                    schema_type: "object".to_string(),
                    properties: HashMap::from([
                        ("reference_file".to_string(), PropertyDefinition { prop_type: "string".to_string(), description: "Path to the original/reference video".to_string(), items: None }),
                        ("distorted_file".to_string(), PropertyDefinition { prop_type: "string".to_string(), description: "Path to the processed/distorted video".to_string(), items: None }),
                    ]),
                    required: vec!["reference_file".to_string(), "distorted_file".to_string()],
                },
            },
            ClaudeTool {
                name: "analyze_audio_stats".to_string(),
                description: "Analyzes audio statistics (RMS, peak, crest factor) using the FFmpeg astats filter.".to_string(),
                input_schema: InputSchema {
                    schema_type: "object".to_string(),
                    properties: HashMap::from([
                        ("input_file".to_string(), PropertyDefinition { prop_type: "string".to_string(), description: "Path to the input file".to_string(), items: None }),
                        ("reset_interval".to_string(), PropertyDefinition { prop_type: "number".to_string(), description: "Reset stats every N seconds (0 = whole file, default 0)".to_string(), items: None }),
                    ]),
                    required: vec!["input_file".to_string()],
                },
            },
            ClaudeTool {
                name: "analyze_video_signal".to_string(),
                description: "Analyzes video signal statistics (luma/chroma min/max, saturation) using the FFmpeg signalstats filter.".to_string(),
                input_schema: InputSchema {
                    schema_type: "object".to_string(),
                    properties: HashMap::from([
                        ("input_file".to_string(), PropertyDefinition { prop_type: "string".to_string(), description: "Path to the input video file".to_string(), items: None }),
                    ]),
                    required: vec!["input_file".to_string()],
                },
            },
            // ================================================================
            // BATCH 12 — Geometric Transforms
            // ================================================================
            ClaudeTool {
                name: "correct_perspective".to_string(),
                description: "Corrects perspective distortion by mapping four corner points using the FFmpeg perspective filter.".to_string(),
                input_schema: InputSchema {
                    schema_type: "object".to_string(),
                    properties: HashMap::from([
                        ("input_file".to_string(), PropertyDefinition { prop_type: "string".to_string(), description: "Path to the input video file".to_string(), items: None }),
                        ("output_file".to_string(), PropertyDefinition { prop_type: "string".to_string(), description: "Path to save the output video".to_string(), items: None }),
                        ("x0".to_string(), PropertyDefinition { prop_type: "number".to_string(), description: "Top-left corner X coordinate".to_string(), items: None }),
                        ("y0".to_string(), PropertyDefinition { prop_type: "number".to_string(), description: "Top-left corner Y coordinate".to_string(), items: None }),
                        ("x1".to_string(), PropertyDefinition { prop_type: "number".to_string(), description: "Top-right corner X coordinate".to_string(), items: None }),
                        ("y1".to_string(), PropertyDefinition { prop_type: "number".to_string(), description: "Top-right corner Y coordinate".to_string(), items: None }),
                        ("x2".to_string(), PropertyDefinition { prop_type: "number".to_string(), description: "Bottom-left corner X coordinate".to_string(), items: None }),
                        ("y2".to_string(), PropertyDefinition { prop_type: "number".to_string(), description: "Bottom-left corner Y coordinate".to_string(), items: None }),
                        ("x3".to_string(), PropertyDefinition { prop_type: "number".to_string(), description: "Bottom-right corner X coordinate".to_string(), items: None }),
                        ("y3".to_string(), PropertyDefinition { prop_type: "number".to_string(), description: "Bottom-right corner Y coordinate".to_string(), items: None }),
                        ("interpolation".to_string(), PropertyDefinition { prop_type: "string".to_string(), description: "Interpolation method: linear/cubic (default linear)".to_string(), items: None }),
                    ]),
                    required: vec!["input_file".to_string(), "output_file".to_string()],
                },
            },
            ClaudeTool {
                name: "correct_lens".to_string(),
                description: "Corrects lens distortion (barrel/pincushion) using the FFmpeg lenscorrection filter.".to_string(),
                input_schema: InputSchema {
                    schema_type: "object".to_string(),
                    properties: HashMap::from([
                        ("input_file".to_string(), PropertyDefinition { prop_type: "string".to_string(), description: "Path to the input video file".to_string(), items: None }),
                        ("output_file".to_string(), PropertyDefinition { prop_type: "string".to_string(), description: "Path to save the output video".to_string(), items: None }),
                        ("k1".to_string(), PropertyDefinition { prop_type: "number".to_string(), description: "Barrel distortion coefficient (-1.0–1.0, negative=barrel, positive=pincushion)".to_string(), items: None }),
                        ("k2".to_string(), PropertyDefinition { prop_type: "number".to_string(), description: "Secondary distortion coefficient (-1.0–1.0, default 0.0)".to_string(), items: None }),
                        ("center_x".to_string(), PropertyDefinition { prop_type: "number".to_string(), description: "Distortion center X (0.5 = center)".to_string(), items: None }),
                        ("center_y".to_string(), PropertyDefinition { prop_type: "number".to_string(), description: "Distortion center Y (0.5 = center)".to_string(), items: None }),
                        ("interpolation".to_string(), PropertyDefinition { prop_type: "string".to_string(), description: "Interpolation: nearest/bilinear (default bilinear)".to_string(), items: None }),
                    ]),
                    required: vec!["input_file".to_string(), "output_file".to_string(), "k1".to_string()],
                },
            },
            ClaudeTool {
                name: "apply_shear".to_string(),
                description: "Applies horizontal and/or vertical shear transformation using the FFmpeg shear filter.".to_string(),
                input_schema: InputSchema {
                    schema_type: "object".to_string(),
                    properties: HashMap::from([
                        ("input_file".to_string(), PropertyDefinition { prop_type: "string".to_string(), description: "Path to the input video file".to_string(), items: None }),
                        ("output_file".to_string(), PropertyDefinition { prop_type: "string".to_string(), description: "Path to save the output video".to_string(), items: None }),
                        ("shear_x".to_string(), PropertyDefinition { prop_type: "number".to_string(), description: "Horizontal shear factor (-2.0 to 2.0)".to_string(), items: None }),
                        ("shear_y".to_string(), PropertyDefinition { prop_type: "number".to_string(), description: "Vertical shear factor (-2.0 to 2.0)".to_string(), items: None }),
                        ("fill_color".to_string(), PropertyDefinition { prop_type: "string".to_string(), description: "Fill color for empty areas (default 'black')".to_string(), items: None }),
                        ("interpolation".to_string(), PropertyDefinition { prop_type: "string".to_string(), description: "Interpolation: nearest/bilinear/bicubic (default bilinear)".to_string(), items: None }),
                    ]),
                    required: vec!["input_file".to_string(), "output_file".to_string()],
                },
            },
            // ================================================================
            // BATCH 13 — Temporal Frame Effects
            // ================================================================
            ClaudeTool {
                name: "blend_frames".to_string(),
                description: "Blends adjacent frames for motion blur or dream effects using the FFmpeg tblend filter.".to_string(),
                input_schema: InputSchema {
                    schema_type: "object".to_string(),
                    properties: HashMap::from([
                        ("input_file".to_string(), PropertyDefinition { prop_type: "string".to_string(), description: "Path to the input video file".to_string(), items: None }),
                        ("output_file".to_string(), PropertyDefinition { prop_type: "string".to_string(), description: "Path to save the output video".to_string(), items: None }),
                        ("blend_mode".to_string(), PropertyDefinition { prop_type: "string".to_string(), description: "Blend mode: average/addition/multiply/screen/overlay/grainmerge (default average)".to_string(), items: None }),
                        ("opacity".to_string(), PropertyDefinition { prop_type: "number".to_string(), description: "Blend opacity (0.0–1.0, default 1.0)".to_string(), items: None }),
                    ]),
                    required: vec!["input_file".to_string(), "output_file".to_string()],
                },
            },
            ClaudeTool {
                name: "temporal_median".to_string(),
                description: "Applies temporal median filtering to remove ghosting/flicker using the FFmpeg tmedian filter.".to_string(),
                input_schema: InputSchema {
                    schema_type: "object".to_string(),
                    properties: HashMap::from([
                        ("input_file".to_string(), PropertyDefinition { prop_type: "string".to_string(), description: "Path to the input video file".to_string(), items: None }),
                        ("output_file".to_string(), PropertyDefinition { prop_type: "string".to_string(), description: "Path to save the output video".to_string(), items: None }),
                        ("radius".to_string(), PropertyDefinition { prop_type: "number".to_string(), description: "Number of frames on each side to sample (1–127, default 1)".to_string(), items: None }),
                    ]),
                    required: vec!["input_file".to_string(), "output_file".to_string()],
                },
            },
            ClaudeTool {
                name: "convert_framerate".to_string(),
                description: "Converts video frame rate using the FFmpeg fps filter.".to_string(),
                input_schema: InputSchema {
                    schema_type: "object".to_string(),
                    properties: HashMap::from([
                        ("input_file".to_string(), PropertyDefinition { prop_type: "string".to_string(), description: "Path to the input video file".to_string(), items: None }),
                        ("output_file".to_string(), PropertyDefinition { prop_type: "string".to_string(), description: "Path to save the output video".to_string(), items: None }),
                        ("target_fps".to_string(), PropertyDefinition { prop_type: "number".to_string(), description: "Target frame rate (e.g. 24, 25, 30, 60)".to_string(), items: None }),
                        ("round_mode".to_string(), PropertyDefinition { prop_type: "string".to_string(), description: "Rounding mode: near/up/down/zero/inf (default near)".to_string(), items: None }),
                    ]),
                    required: vec!["input_file".to_string(), "output_file".to_string(), "target_fps".to_string()],
                },
            },
            ClaudeTool {
                name: "tile_frames".to_string(),
                description: "Arranges video frames as a grid/contact sheet image using the FFmpeg tile filter.".to_string(),
                input_schema: InputSchema {
                    schema_type: "object".to_string(),
                    properties: HashMap::from([
                        ("input_file".to_string(), PropertyDefinition { prop_type: "string".to_string(), description: "Path to the input video file".to_string(), items: None }),
                        ("output_file".to_string(), PropertyDefinition { prop_type: "string".to_string(), description: "Path to save the output image (.jpg or .png)".to_string(), items: None }),
                        ("columns".to_string(), PropertyDefinition { prop_type: "number".to_string(), description: "Number of columns in the tile grid (default 4)".to_string(), items: None }),
                        ("rows".to_string(), PropertyDefinition { prop_type: "number".to_string(), description: "Number of rows in the tile grid (default 3)".to_string(), items: None }),
                        ("frame_gap".to_string(), PropertyDefinition { prop_type: "number".to_string(), description: "Pixels between tiles (default 2)".to_string(), items: None }),
                    ]),
                    required: vec!["input_file".to_string(), "output_file".to_string(), "columns".to_string(), "rows".to_string()],
                },
            },
            // ================================================================
            // BATCH 14 — Spatial Audio
            // ================================================================
            ClaudeTool {
                name: "adjust_stereo_width".to_string(),
                description: "Adjusts stereo width and balance using the FFmpeg stereotools filter.".to_string(),
                input_schema: InputSchema {
                    schema_type: "object".to_string(),
                    properties: HashMap::from([
                        ("input_file".to_string(), PropertyDefinition { prop_type: "string".to_string(), description: "Path to the input file".to_string(), items: None }),
                        ("output_file".to_string(), PropertyDefinition { prop_type: "string".to_string(), description: "Path to save the output file".to_string(), items: None }),
                        ("width".to_string(), PropertyDefinition { prop_type: "number".to_string(), description: "Stereo width (0=mono, 1=unchanged, 2=wide, range 0–4)".to_string(), items: None }),
                        ("balance".to_string(), PropertyDefinition { prop_type: "number".to_string(), description: "Balance left/right (-1.0 to 1.0, default 0.0)".to_string(), items: None }),
                        ("mode".to_string(), PropertyDefinition { prop_type: "string".to_string(), description: "Stereo mode: lr>lr/lr>ms/ms>lr/lr>ll/lr>rr/lr>l+r/lr>rl (default lr>lr)".to_string(), items: None }),
                    ]),
                    required: vec!["input_file".to_string(), "output_file".to_string()],
                },
            },
            ClaudeTool {
                name: "apply_stereo_widen".to_string(),
                description: "Widens stereo image using Haas effect via the FFmpeg stereowiden filter.".to_string(),
                input_schema: InputSchema {
                    schema_type: "object".to_string(),
                    properties: HashMap::from([
                        ("input_file".to_string(), PropertyDefinition { prop_type: "string".to_string(), description: "Path to the input file".to_string(), items: None }),
                        ("output_file".to_string(), PropertyDefinition { prop_type: "string".to_string(), description: "Path to save the output file".to_string(), items: None }),
                        ("delay_ms".to_string(), PropertyDefinition { prop_type: "number".to_string(), description: "Delay in milliseconds (1–100, default 20)".to_string(), items: None }),
                        ("feedback".to_string(), PropertyDefinition { prop_type: "number".to_string(), description: "Feedback amount (0–0.9, default 0.3)".to_string(), items: None }),
                        ("crossfeed".to_string(), PropertyDefinition { prop_type: "number".to_string(), description: "Crossfeed amount (0–1, default 0.3)".to_string(), items: None }),
                        ("drymix".to_string(), PropertyDefinition { prop_type: "number".to_string(), description: "Dry mix amount (0–1, default 0.8)".to_string(), items: None }),
                    ]),
                    required: vec!["input_file".to_string(), "output_file".to_string()],
                },
            },
            ClaudeTool {
                name: "mix_audio_channels".to_string(),
                description: "Mixes and routes audio channels using the FFmpeg pan filter.".to_string(),
                input_schema: InputSchema {
                    schema_type: "object".to_string(),
                    properties: HashMap::from([
                        ("input_file".to_string(), PropertyDefinition { prop_type: "string".to_string(), description: "Path to the input file".to_string(), items: None }),
                        ("output_file".to_string(), PropertyDefinition { prop_type: "string".to_string(), description: "Path to save the output file".to_string(), items: None }),
                        ("channel_layout".to_string(), PropertyDefinition { prop_type: "string".to_string(), description: "Output channel layout e.g. 'stereo', 'mono', '5.1'".to_string(), items: None }),
                        ("channel_mix".to_string(), PropertyDefinition { prop_type: "string".to_string(), description: "Pipe-separated channel expressions e.g. 'c0=0.5*c0+0.5*c1|c1=0.5*c0+0.5*c1'".to_string(), items: None }),
                    ]),
                    required: vec!["input_file".to_string(), "output_file".to_string(), "channel_layout".to_string(), "channel_mix".to_string()],
                },
            },

            // ================================================================
            // PHASE D — Professional Finishing Tools
            // ================================================================

            ClaudeTool {
                name: "adjust_color_temperature".to_string(),
                description: "Adjusts the color temperature (white balance) of a video using the FFmpeg colortemperature filter. Use to make footage warmer (lower Kelvin) or cooler (higher Kelvin).".to_string(),
                input_schema: InputSchema {
                    schema_type: "object".to_string(),
                    properties: HashMap::from([
                        ("input_file".to_string(), PropertyDefinition { prop_type: "string".to_string(), description: "Path to the input video file".to_string(), items: None }),
                        ("output_file".to_string(), PropertyDefinition { prop_type: "string".to_string(), description: "Path to save the output file".to_string(), items: None }),
                        ("temperature".to_string(), PropertyDefinition { prop_type: "number".to_string(), description: "Color temperature in Kelvin (1000–40000). Lower = warmer/orange, higher = cooler/blue. Default 6500.".to_string(), items: None }),
                        ("mix".to_string(), PropertyDefinition { prop_type: "number".to_string(), description: "Blend factor with the original (0–1). Default 1.0 = fully applied.".to_string(), items: None }),
                    ]),
                    required: vec!["input_file".to_string(), "output_file".to_string()],
                },
            },

            ClaudeTool {
                name: "adjust_vibrance".to_string(),
                description: "Adjusts vibrance (selective saturation boost for muted colours) using the FFmpeg vibrance filter. Unlike saturation, vibrance avoids over-saturating already vivid colours.".to_string(),
                input_schema: InputSchema {
                    schema_type: "object".to_string(),
                    properties: HashMap::from([
                        ("input_file".to_string(), PropertyDefinition { prop_type: "string".to_string(), description: "Path to the input video file".to_string(), items: None }),
                        ("output_file".to_string(), PropertyDefinition { prop_type: "string".to_string(), description: "Path to save the output file".to_string(), items: None }),
                        ("intensity".to_string(), PropertyDefinition { prop_type: "number".to_string(), description: "Vibrance intensity (-2.0–2.0). Positive boosts muted colours, negative desaturates. Default 0.".to_string(), items: None }),
                        ("red_balance".to_string(), PropertyDefinition { prop_type: "number".to_string(), description: "Red channel balance adjustment (-10–10). Default 1.0.".to_string(), items: None }),
                        ("green_balance".to_string(), PropertyDefinition { prop_type: "number".to_string(), description: "Green channel balance adjustment (-10–10). Default 1.0.".to_string(), items: None }),
                        ("blue_balance".to_string(), PropertyDefinition { prop_type: "number".to_string(), description: "Blue channel balance adjustment (-10–10). Default 1.0.".to_string(), items: None }),
                    ]),
                    required: vec!["input_file".to_string(), "output_file".to_string()],
                },
            },

            ClaudeTool {
                name: "remove_flicker".to_string(),
                description: "Removes temporal flicker from video (e.g. old film, timelapse, fluorescent lighting) using the FFmpeg deflicker filter by averaging luminance over a window of frames.".to_string(),
                input_schema: InputSchema {
                    schema_type: "object".to_string(),
                    properties: HashMap::from([
                        ("input_file".to_string(), PropertyDefinition { prop_type: "string".to_string(), description: "Path to the input video file".to_string(), items: None }),
                        ("output_file".to_string(), PropertyDefinition { prop_type: "string".to_string(), description: "Path to save the output file".to_string(), items: None }),
                        ("size".to_string(), PropertyDefinition { prop_type: "number".to_string(), description: "Number of frames to average over (2–129). Default 5.".to_string(), items: None }),
                        ("mode".to_string(), PropertyDefinition { prop_type: "string".to_string(), description: "Averaging mode: am (arithmetic mean), gm (geometric), hm (harmonic), qm (quadratic), cm (cubic), pm (power), median. Default 'am'.".to_string(), items: None }),
                    ]),
                    required: vec!["input_file".to_string(), "output_file".to_string()],
                },
            },

            ClaudeTool {
                name: "denoise_video_bm3d".to_string(),
                description: "Applies BM3D (Block-Matching 3D) denoising to video — a high-quality spatial denoiser that preserves detail while removing noise/grain.".to_string(),
                input_schema: InputSchema {
                    schema_type: "object".to_string(),
                    properties: HashMap::from([
                        ("input_file".to_string(), PropertyDefinition { prop_type: "string".to_string(), description: "Path to the input video file".to_string(), items: None }),
                        ("output_file".to_string(), PropertyDefinition { prop_type: "string".to_string(), description: "Path to save the output file".to_string(), items: None }),
                        ("sigma".to_string(), PropertyDefinition { prop_type: "number".to_string(), description: "Noise standard deviation (0.1–999.9). Higher = more aggressive denoising. Default 1.0.".to_string(), items: None }),
                        ("block_size".to_string(), PropertyDefinition { prop_type: "number".to_string(), description: "Block size for matching (8–64). Default 16.".to_string(), items: None }),
                        ("mode".to_string(), PropertyDefinition { prop_type: "string".to_string(), description: "Denoising mode: 'basic' (single-pass) or 'final' (two-pass, higher quality). Default 'basic'.".to_string(), items: None }),
                    ]),
                    required: vec!["input_file".to_string(), "output_file".to_string()],
                },
            },

            ClaudeTool {
                name: "deshake_video".to_string(),
                description: "Stabilises shaky handheld video using the FFmpeg deshake filter, which compensates for camera motion by analysing a region of interest.".to_string(),
                input_schema: InputSchema {
                    schema_type: "object".to_string(),
                    properties: HashMap::from([
                        ("input_file".to_string(), PropertyDefinition { prop_type: "string".to_string(), description: "Path to the input video file".to_string(), items: None }),
                        ("output_file".to_string(), PropertyDefinition { prop_type: "string".to_string(), description: "Path to save the stabilised output file".to_string(), items: None }),
                        ("x".to_string(), PropertyDefinition { prop_type: "number".to_string(), description: "X offset of motion-detection region (-1 = auto). Default -1.".to_string(), items: None }),
                        ("y".to_string(), PropertyDefinition { prop_type: "number".to_string(), description: "Y offset of motion-detection region (-1 = auto). Default -1.".to_string(), items: None }),
                        ("w".to_string(), PropertyDefinition { prop_type: "number".to_string(), description: "Width of motion-detection region (-1 = auto). Default -1.".to_string(), items: None }),
                        ("h".to_string(), PropertyDefinition { prop_type: "number".to_string(), description: "Height of motion-detection region (-1 = auto). Default -1.".to_string(), items: None }),
                        ("rx".to_string(), PropertyDefinition { prop_type: "number".to_string(), description: "Maximum pixels to compensate horizontally (default 16).".to_string(), items: None }),
                        ("ry".to_string(), PropertyDefinition { prop_type: "number".to_string(), description: "Maximum pixels to compensate vertically (default 16).".to_string(), items: None }),
                    ]),
                    required: vec!["input_file".to_string(), "output_file".to_string()],
                },
            },
        ]
    }

    /// Filter tools by name (for dynamic tool selection)
    /// Returns only the tools whose names are in the provided list
    pub fn filter_tools_by_name(tool_names: &[String]) -> Vec<ClaudeTool> {
        crate::tool_registry::ToolRegistry::filter_claude_tools_for_profile(
            crate::tool_registry::AgentExecutionProfile::FullProduction,
            tool_names,
        )
    }
}
