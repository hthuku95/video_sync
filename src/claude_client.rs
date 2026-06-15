use backoff::{future::retry, ExponentialBackoff};
use reqwest::Client;
use serde::{Deserialize, Serialize};
use serde_json::Value;
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
                name: "fetch_website_image".to_string(),
                description: "Fetch the hero/og:image from a website URL. Use this when a user provides a website URL (e.g. netflix.com, stripe.com) and you need to extract its visual for use in a Blender landing page animation or product mockup. Returns the image URL that you can pass to blender_generate_scene's reference_image_url parameter.".to_string(),
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

            ClaudeTool {
                name: "measure_lufs".to_string(),
                description: "Measures integrated loudness (LUFS), loudness range (LRA), and true peak of an audio or video file using the EBU R128 standard. Analysis only — no output file produced.".to_string(),
                input_schema: InputSchema {
                    schema_type: "object".to_string(),
                    properties: HashMap::from([
                        ("input_file".to_string(), PropertyDefinition { prop_type: "string".to_string(), description: "Path to the audio or video file to measure".to_string(), items: None }),
                        ("target_lufs".to_string(), PropertyDefinition { prop_type: "number".to_string(), description: "Target loudness level for reference (default -23 LUFS = EBU R128 broadcast standard). YouTube uses -14 LUFS.".to_string(), items: None }),
                    ]),
                    required: vec!["input_file".to_string()],
                },
            },

            ClaudeTool {
                name: "parametric_eq".to_string(),
                description: "Applies a multi-band parametric equalizer to audio using the FFmpeg anequalizer filter. Supports peak, shelf, notch, and pass filters per channel.".to_string(),
                input_schema: InputSchema {
                    schema_type: "object".to_string(),
                    properties: HashMap::from([
                        ("input_file".to_string(), PropertyDefinition { prop_type: "string".to_string(), description: "Path to the input audio/video file".to_string(), items: None }),
                        ("output_file".to_string(), PropertyDefinition { prop_type: "string".to_string(), description: "Path to save the processed output file".to_string(), items: None }),
                        ("eq_params".to_string(), PropertyDefinition { prop_type: "string".to_string(), description: "Pipe-separated EQ band specs. Each band: 'c<ch> f=<hz> w=<bw> g=<db> t=<type>' where type: 0=LPF,1=HPF,2=BPF,3=BPW,4=AP,5=Peak,6=Notch,7=LSF,8=HSF. E.g. 'c0 f=1000 w=200 g=6 t=5|c0 f=100 w=50 g=-3 t=7'".to_string(), items: None }),
                    ]),
                    required: vec!["input_file".to_string(), "output_file".to_string(), "eq_params".to_string()],
                },
            },

            ClaudeTool {
                name: "audio_limiter".to_string(),
                description: "Applies a brickwall limiter to audio to prevent peaks from exceeding a target level using the FFmpeg alimiter filter. Essential for mastering and broadcast delivery.".to_string(),
                input_schema: InputSchema {
                    schema_type: "object".to_string(),
                    properties: HashMap::from([
                        ("input_file".to_string(), PropertyDefinition { prop_type: "string".to_string(), description: "Path to the input audio/video file".to_string(), items: None }),
                        ("output_file".to_string(), PropertyDefinition { prop_type: "string".to_string(), description: "Path to save the limited output file".to_string(), items: None }),
                        ("limit_db".to_string(), PropertyDefinition { prop_type: "number".to_string(), description: "Peak ceiling in dBFS (-10 to 0). Default -1.0 dBFS.".to_string(), items: None }),
                        ("attack_ms".to_string(), PropertyDefinition { prop_type: "number".to_string(), description: "Attack time in milliseconds. Default 5ms.".to_string(), items: None }),
                        ("release_ms".to_string(), PropertyDefinition { prop_type: "number".to_string(), description: "Release time in milliseconds. Default 50ms.".to_string(), items: None }),
                        ("auto_sc".to_string(), PropertyDefinition { prop_type: "boolean".to_string(), description: "Enable auto-level sidechain. Default false.".to_string(), items: None }),
                    ]),
                    required: vec!["input_file".to_string(), "output_file".to_string()],
                },
            },

            ClaudeTool {
                name: "reduce_sibilance".to_string(),
                description: "Reduces harsh sibilance ('ess' sounds) in vocal recordings using the FFmpeg deesser filter. Targets the high-frequency energy of sibilant consonants.".to_string(),
                input_schema: InputSchema {
                    schema_type: "object".to_string(),
                    properties: HashMap::from([
                        ("input_file".to_string(), PropertyDefinition { prop_type: "string".to_string(), description: "Path to the input audio/video file".to_string(), items: None }),
                        ("output_file".to_string(), PropertyDefinition { prop_type: "string".to_string(), description: "Path to save the de-essed output file".to_string(), items: None }),
                        ("split_hz".to_string(), PropertyDefinition { prop_type: "number".to_string(), description: "Crossover frequency in Hz separating low from high band (default 8500 Hz).".to_string(), items: None }),
                        ("threshold".to_string(), PropertyDefinition { prop_type: "number".to_string(), description: "Sibilance detection threshold (0–1). Lower = more aggressive. Default 0.1.".to_string(), items: None }),
                        ("mode".to_string(), PropertyDefinition { prop_type: "string".to_string(), description: "Processing mode: 'split' (process only high band) or 'wide' (process full signal). Default 'split'.".to_string(), items: None }),
                    ]),
                    required: vec!["input_file".to_string(), "output_file".to_string()],
                },
            },

            ClaudeTool {
                name: "denoise_speech_rnn".to_string(),
                description: "Removes background noise from speech/voice recordings using a neural RNN model (FFmpeg arnndn filter). Very effective for voice-overs, interviews, and dialogue.".to_string(),
                input_schema: InputSchema {
                    schema_type: "object".to_string(),
                    properties: HashMap::from([
                        ("input_file".to_string(), PropertyDefinition { prop_type: "string".to_string(), description: "Path to the input audio/video file".to_string(), items: None }),
                        ("output_file".to_string(), PropertyDefinition { prop_type: "string".to_string(), description: "Path to save the denoised output file".to_string(), items: None }),
                        ("model_file".to_string(), PropertyDefinition { prop_type: "string".to_string(), description: "Optional path to a .rnnn model file. Leave empty to use the built-in model.".to_string(), items: None }),
                        ("mix".to_string(), PropertyDefinition { prop_type: "number".to_string(), description: "Mix ratio (0–1): 0 = original audio, 1 = fully denoised. Default 1.0.".to_string(), items: None }),
                    ]),
                    required: vec!["input_file".to_string(), "output_file".to_string()],
                },
            },

            // ================================================================
            // PHASE E — Vectorscope, Waveform, Grid, LumaKey, Binaural, Modulation
            // ================================================================

            ClaudeTool {
                name: "analyze_vectorscope".to_string(),
                description: "Renders a vectorscope visualisation frame from a video — used in colour grading to check chroma distribution and saturation levels.".to_string(),
                input_schema: InputSchema {
                    schema_type: "object".to_string(),
                    properties: HashMap::from([
                        ("input_file".to_string(), PropertyDefinition { prop_type: "string".to_string(), description: "Path to the input video file".to_string(), items: None }),
                        ("output_file".to_string(), PropertyDefinition { prop_type: "string".to_string(), description: "Path to save the vectorscope image (PNG/JPEG)".to_string(), items: None }),
                        ("mode".to_string(), PropertyDefinition { prop_type: "string".to_string(), description: "Display mode: color, color2, color3, color4, color5, gray, tint, phase, hphase (default 'color').".to_string(), items: None }),
                    ]),
                    required: vec!["input_file".to_string(), "output_file".to_string()],
                },
            },

            ClaudeTool {
                name: "analyze_waveform".to_string(),
                description: "Renders a waveform monitor frame from a video — used in colour grading to check luma/chroma levels and exposure.".to_string(),
                input_schema: InputSchema {
                    schema_type: "object".to_string(),
                    properties: HashMap::from([
                        ("input_file".to_string(), PropertyDefinition { prop_type: "string".to_string(), description: "Path to the input video file".to_string(), items: None }),
                        ("output_file".to_string(), PropertyDefinition { prop_type: "string".to_string(), description: "Path to save the waveform image (PNG/JPEG)".to_string(), items: None }),
                        ("mode".to_string(), PropertyDefinition { prop_type: "string".to_string(), description: "Scope layout: row (default) or column.".to_string(), items: None }),
                        ("filter_type".to_string(), PropertyDefinition { prop_type: "string".to_string(), description: "Filter component: lowpass (luma, default), flat, aflat, chroma, color, acolor.".to_string(), items: None }),
                    ]),
                    required: vec!["input_file".to_string(), "output_file".to_string()],
                },
            },

            ClaudeTool {
                name: "draw_grid".to_string(),
                description: "Draws a regular grid over a video using the FFmpeg drawgrid filter. Useful for composition guides, rule-of-thirds overlays, or technical monitoring.".to_string(),
                input_schema: InputSchema {
                    schema_type: "object".to_string(),
                    properties: HashMap::from([
                        ("input_file".to_string(), PropertyDefinition { prop_type: "string".to_string(), description: "Path to the input video file".to_string(), items: None }),
                        ("output_file".to_string(), PropertyDefinition { prop_type: "string".to_string(), description: "Path to save the output file".to_string(), items: None }),
                        ("width".to_string(), PropertyDefinition { prop_type: "number".to_string(), description: "Grid cell width in pixels. Default 100.".to_string(), items: None }),
                        ("height".to_string(), PropertyDefinition { prop_type: "number".to_string(), description: "Grid cell height in pixels. Default 100.".to_string(), items: None }),
                        ("thickness".to_string(), PropertyDefinition { prop_type: "number".to_string(), description: "Line thickness in pixels. Default 1.".to_string(), items: None }),
                        ("color".to_string(), PropertyDefinition { prop_type: "string".to_string(), description: "Grid line colour (FFmpeg colour name or hex). Default 'white@0.5'.".to_string(), items: None }),
                    ]),
                    required: vec!["input_file".to_string(), "output_file".to_string()],
                },
            },

            ClaudeTool {
                name: "grid_stack_videos".to_string(),
                description: "Stacks multiple videos in a grid layout using the FFmpeg xstack filter. E.g. 4 videos in a 2×2 grid. Input count and layout are configurable.".to_string(),
                input_schema: InputSchema {
                    schema_type: "object".to_string(),
                    properties: HashMap::from([
                        ("input_files".to_string(), PropertyDefinition {
                            prop_type: "array".to_string(),
                            description: "Array of input video file paths (minimum 2)".to_string(),
                            items: Some(Box::new(PropertyDefinition {
                                prop_type: "string".to_string(),
                                description: "Path to a video file".to_string(),
                                items: None,
                            })),
                        }),
                        ("output_file".to_string(), PropertyDefinition { prop_type: "string".to_string(), description: "Path to save the stacked output video".to_string(), items: None }),
                        ("layout".to_string(), PropertyDefinition { prop_type: "string".to_string(), description: "xstack layout string e.g. '0_0|w0_0|0_h0|w0_h0' for 2×2. Leave empty for auto-layout.".to_string(), items: None }),
                    ]),
                    required: vec!["input_files".to_string(), "output_file".to_string()],
                },
            },

            ClaudeTool {
                name: "luma_key".to_string(),
                description: "Applies a luma key to make dark or bright regions transparent using the FFmpeg lumakey filter. Useful for title cards and compositing over backgrounds.".to_string(),
                input_schema: InputSchema {
                    schema_type: "object".to_string(),
                    properties: HashMap::from([
                        ("input_file".to_string(), PropertyDefinition { prop_type: "string".to_string(), description: "Path to the input video file".to_string(), items: None }),
                        ("output_file".to_string(), PropertyDefinition { prop_type: "string".to_string(), description: "Path to save the output video with alpha channel".to_string(), items: None }),
                        ("threshold".to_string(), PropertyDefinition { prop_type: "number".to_string(), description: "Luma threshold (0–1). Pixels near this value become transparent. Default 0.1.".to_string(), items: None }),
                        ("tolerance".to_string(), PropertyDefinition { prop_type: "number".to_string(), description: "Tolerance around the threshold (0–1). Default 0.1.".to_string(), items: None }),
                        ("softness".to_string(), PropertyDefinition { prop_type: "number".to_string(), description: "Edge softness (0–1). 0 = hard edge. Default 0.".to_string(), items: None }),
                    ]),
                    required: vec!["input_file".to_string(), "output_file".to_string()],
                },
            },

            ClaudeTool {
                name: "render_binaural".to_string(),
                description: "Virtualises multichannel or stereo audio for headphone playback using the FFmpeg headphone filter (HRTF-based binaural rendering).".to_string(),
                input_schema: InputSchema {
                    schema_type: "object".to_string(),
                    properties: HashMap::from([
                        ("input_file".to_string(), PropertyDefinition { prop_type: "string".to_string(), description: "Path to the input audio/video file".to_string(), items: None }),
                        ("output_file".to_string(), PropertyDefinition { prop_type: "string".to_string(), description: "Path to save the binaural output".to_string(), items: None }),
                        ("hrir_type".to_string(), PropertyDefinition { prop_type: "string".to_string(), description: "HRIR type: 'stereo' (built-in stereo HRIR, default) or 'multich' (built-in multichannel HRIR).".to_string(), items: None }),
                    ]),
                    required: vec!["input_file".to_string(), "output_file".to_string()],
                },
            },

            ClaudeTool {
                name: "add_vibrato".to_string(),
                description: "Adds a vibrato effect to audio using the FFmpeg vibrato filter — a periodic pitch modulation that gives a singing/wavering quality.".to_string(),
                input_schema: InputSchema {
                    schema_type: "object".to_string(),
                    properties: HashMap::from([
                        ("input_file".to_string(), PropertyDefinition { prop_type: "string".to_string(), description: "Path to the input audio/video file".to_string(), items: None }),
                        ("output_file".to_string(), PropertyDefinition { prop_type: "string".to_string(), description: "Path to save the output file".to_string(), items: None }),
                        ("frequency".to_string(), PropertyDefinition { prop_type: "number".to_string(), description: "Modulation frequency in Hz (0.1–20000). Default 5 Hz.".to_string(), items: None }),
                        ("depth".to_string(), PropertyDefinition { prop_type: "number".to_string(), description: "Modulation depth (0–1). Default 0.5.".to_string(), items: None }),
                    ]),
                    required: vec!["input_file".to_string(), "output_file".to_string()],
                },
            },

            ClaudeTool {
                name: "add_tremolo".to_string(),
                description: "Adds a tremolo effect to audio using the FFmpeg tremolo filter — a periodic amplitude modulation that gives a pulsing/shimmering quality.".to_string(),
                input_schema: InputSchema {
                    schema_type: "object".to_string(),
                    properties: HashMap::from([
                        ("input_file".to_string(), PropertyDefinition { prop_type: "string".to_string(), description: "Path to the input audio/video file".to_string(), items: None }),
                        ("output_file".to_string(), PropertyDefinition { prop_type: "string".to_string(), description: "Path to save the output file".to_string(), items: None }),
                        ("frequency".to_string(), PropertyDefinition { prop_type: "number".to_string(), description: "Modulation frequency in Hz (0.1–20000). Default 5 Hz.".to_string(), items: None }),
                        ("depth".to_string(), PropertyDefinition { prop_type: "number".to_string(), description: "Modulation depth (0–1). Default 0.5.".to_string(), items: None }),
                    ]),
                    required: vec!["input_file".to_string(), "output_file".to_string()],
                },
            },

            ClaudeTool {
                name: "add_flanger".to_string(),
                description: "Adds a flanger effect to audio using the FFmpeg flanger filter — a comb-filtering effect created by mixing a slightly delayed copy of the signal.".to_string(),
                input_schema: InputSchema {
                    schema_type: "object".to_string(),
                    properties: HashMap::from([
                        ("input_file".to_string(), PropertyDefinition { prop_type: "string".to_string(), description: "Path to the input audio/video file".to_string(), items: None }),
                        ("output_file".to_string(), PropertyDefinition { prop_type: "string".to_string(), description: "Path to save the output file".to_string(), items: None }),
                        ("delay".to_string(), PropertyDefinition { prop_type: "number".to_string(), description: "Base delay in milliseconds (0–30). Default 0.".to_string(), items: None }),
                        ("depth".to_string(), PropertyDefinition { prop_type: "number".to_string(), description: "Sweep depth in ms (0–10). Default 2.".to_string(), items: None }),
                        ("speed".to_string(), PropertyDefinition { prop_type: "number".to_string(), description: "Sweep speed in Hz (0.1–10). Default 0.5.".to_string(), items: None }),
                        ("shape".to_string(), PropertyDefinition { prop_type: "string".to_string(), description: "LFO shape: 'sinusoidal' (default) or 'triangular'.".to_string(), items: None }),
                    ]),
                    required: vec!["input_file".to_string(), "output_file".to_string()],
                },
            },

            ClaudeTool {
                name: "denoise_audio_nlm".to_string(),
                description: "Denoises audio using Non-Local Means (NLM) algorithm via the FFmpeg anlmdn filter. Effective for broadband noise without introducing musical noise artifacts.".to_string(),
                input_schema: InputSchema {
                    schema_type: "object".to_string(),
                    properties: HashMap::from([
                        ("input_file".to_string(), PropertyDefinition { prop_type: "string".to_string(), description: "Path to the input audio/video file".to_string(), items: None }),
                        ("output_file".to_string(), PropertyDefinition { prop_type: "string".to_string(), description: "Path to save the denoised output".to_string(), items: None }),
                        ("strength".to_string(), PropertyDefinition { prop_type: "number".to_string(), description: "Denoising strength (0.00001–10). Default 0.0001.".to_string(), items: None }),
                        ("patch_size".to_string(), PropertyDefinition { prop_type: "number".to_string(), description: "Patch size in seconds (0–100). Default 0.002.".to_string(), items: None }),
                        ("research_size".to_string(), PropertyDefinition { prop_type: "number".to_string(), description: "Research window in seconds (0–100). Default 0.002.".to_string(), items: None }),
                    ]),
                    required: vec!["input_file".to_string(), "output_file".to_string()],
                },
            },

            // ================================================================
            // PHASE F — Niche/Specialised Tools
            // ================================================================

            ClaudeTool {
                name: "displace_video".to_string(),
                description: "Displaces pixels in a video using x and y displacement map videos (FFmpeg displace filter). Creates warping and distortion effects.".to_string(),
                input_schema: InputSchema {
                    schema_type: "object".to_string(),
                    properties: HashMap::from([
                        ("input_file".to_string(), PropertyDefinition { prop_type: "string".to_string(), description: "Path to the main input video".to_string(), items: None }),
                        ("xmap_file".to_string(), PropertyDefinition { prop_type: "string".to_string(), description: "Path to the horizontal displacement map video".to_string(), items: None }),
                        ("ymap_file".to_string(), PropertyDefinition { prop_type: "string".to_string(), description: "Path to the vertical displacement map video".to_string(), items: None }),
                        ("output_file".to_string(), PropertyDefinition { prop_type: "string".to_string(), description: "Path to save the displaced output".to_string(), items: None }),
                        ("edge".to_string(), PropertyDefinition { prop_type: "string".to_string(), description: "Edge handling: smear (default), wrap, mirror, blank.".to_string(), items: None }),
                    ]),
                    required: vec!["input_file".to_string(), "xmap_file".to_string(), "ymap_file".to_string(), "output_file".to_string()],
                },
            },

            ClaudeTool {
                name: "decimate_frames".to_string(),
                description: "Removes duplicate frames from video to reduce frame rate (FFmpeg decimate filter). Useful for converting 30fps telecine content or fixing duplicated frames.".to_string(),
                input_schema: InputSchema {
                    schema_type: "object".to_string(),
                    properties: HashMap::from([
                        ("input_file".to_string(), PropertyDefinition { prop_type: "string".to_string(), description: "Path to the input video file".to_string(), items: None }),
                        ("output_file".to_string(), PropertyDefinition { prop_type: "string".to_string(), description: "Path to save the decimated output".to_string(), items: None }),
                        ("cycle".to_string(), PropertyDefinition { prop_type: "number".to_string(), description: "Number of frames to analyze per cycle (2–25). Default 5.".to_string(), items: None }),
                        ("dupthresh".to_string(), PropertyDefinition { prop_type: "number".to_string(), description: "Duplicate detection threshold (0–100). Default 1.1.".to_string(), items: None }),
                        ("scthresh".to_string(), PropertyDefinition { prop_type: "number".to_string(), description: "Scene-change threshold (0–100). Default 15.".to_string(), items: None }),
                    ]),
                    required: vec!["input_file".to_string(), "output_file".to_string()],
                },
            },

            ClaudeTool {
                name: "denoise_video_owden".to_string(),
                description: "Applies Overcomplete Wavelet denoising (owdenoise) to video. Good for heavy noise reduction while preserving texture.".to_string(),
                input_schema: InputSchema {
                    schema_type: "object".to_string(),
                    properties: HashMap::from([
                        ("input_file".to_string(), PropertyDefinition { prop_type: "string".to_string(), description: "Path to the input video file".to_string(), items: None }),
                        ("output_file".to_string(), PropertyDefinition { prop_type: "string".to_string(), description: "Path to save the denoised output".to_string(), items: None }),
                        ("luma_strength".to_string(), PropertyDefinition { prop_type: "number".to_string(), description: "Luma denoising strength (0–1000). Default 10.".to_string(), items: None }),
                        ("chroma_strength".to_string(), PropertyDefinition { prop_type: "number".to_string(), description: "Chroma denoising strength (0–1000). Default 10.".to_string(), items: None }),
                    ]),
                    required: vec!["input_file".to_string(), "output_file".to_string()],
                },
            },

            ClaudeTool {
                name: "despill_video".to_string(),
                description: "Removes colour spill (e.g. green or blue screen halo) from a keyed subject using the FFmpeg despill filter.".to_string(),
                input_schema: InputSchema {
                    schema_type: "object".to_string(),
                    properties: HashMap::from([
                        ("input_file".to_string(), PropertyDefinition { prop_type: "string".to_string(), description: "Path to the input video file".to_string(), items: None }),
                        ("output_file".to_string(), PropertyDefinition { prop_type: "string".to_string(), description: "Path to save the despilled output".to_string(), items: None }),
                        ("spill_type".to_string(), PropertyDefinition { prop_type: "string".to_string(), description: "Spill colour to remove: 'green' (default) or 'blue'.".to_string(), items: None }),
                        ("mix".to_string(), PropertyDefinition { prop_type: "number".to_string(), description: "Mix of despilled and original (0–1). Default 0.5.".to_string(), items: None }),
                        ("expand".to_string(), PropertyDefinition { prop_type: "number".to_string(), description: "Expansion of despill region (0–1). Default 0.".to_string(), items: None }),
                    ]),
                    required: vec!["input_file".to_string(), "output_file".to_string()],
                },
            },

            ClaudeTool {
                name: "remap_pixels".to_string(),
                description: "Remaps pixels using x and y coordinate map videos (FFmpeg remap filter). Creates custom geometric distortions and pixel relocations.".to_string(),
                input_schema: InputSchema {
                    schema_type: "object".to_string(),
                    properties: HashMap::from([
                        ("input_file".to_string(), PropertyDefinition { prop_type: "string".to_string(), description: "Path to the main input video".to_string(), items: None }),
                        ("xmap_file".to_string(), PropertyDefinition { prop_type: "string".to_string(), description: "Path to the x-coordinate map video (32-bit float or 16-bit)".to_string(), items: None }),
                        ("ymap_file".to_string(), PropertyDefinition { prop_type: "string".to_string(), description: "Path to the y-coordinate map video".to_string(), items: None }),
                        ("output_file".to_string(), PropertyDefinition { prop_type: "string".to_string(), description: "Path to save the remapped output".to_string(), items: None }),
                        ("fill".to_string(), PropertyDefinition { prop_type: "string".to_string(), description: "Fill colour for out-of-bounds pixels (FFmpeg colour name or hex). Default 'black'.".to_string(), items: None }),
                    ]),
                    required: vec!["input_file".to_string(), "xmap_file".to_string(), "ymap_file".to_string(), "output_file".to_string()],
                },
            },

            ClaudeTool {
                name: "adjust_exposure".to_string(),
                description: "Adjusts the exposure (brightness and black point) of a video using the FFmpeg exposure filter.".to_string(),
                input_schema: InputSchema {
                    schema_type: "object".to_string(),
                    properties: HashMap::from([
                        ("input_file".to_string(), PropertyDefinition { prop_type: "string".to_string(), description: "Path to the input video file".to_string(), items: None }),
                        ("output_file".to_string(), PropertyDefinition { prop_type: "string".to_string(), description: "Path to save the output file".to_string(), items: None }),
                        ("exposure".to_string(), PropertyDefinition { prop_type: "number".to_string(), description: "Exposure value in EV stops (-3 to 3). Positive = brighter. Default 0.".to_string(), items: None }),
                        ("black".to_string(), PropertyDefinition { prop_type: "number".to_string(), description: "Black level pedestal (0–1). Default 0.".to_string(), items: None }),
                    ]),
                    required: vec!["input_file".to_string(), "output_file".to_string()],
                },
            },

            ClaudeTool {
                name: "measure_vmaf".to_string(),
                description: "Measures VMAF (Video Multimethod Assessment Fusion) perceptual quality score between a distorted and a reference video. Analysis only — requires libvmaf in FFmpeg build.".to_string(),
                input_schema: InputSchema {
                    schema_type: "object".to_string(),
                    properties: HashMap::from([
                        ("distorted_file".to_string(), PropertyDefinition { prop_type: "string".to_string(), description: "Path to the distorted/compressed video to evaluate".to_string(), items: None }),
                        ("reference_file".to_string(), PropertyDefinition { prop_type: "string".to_string(), description: "Path to the reference/original video".to_string(), items: None }),
                        ("model_path".to_string(), PropertyDefinition { prop_type: "string".to_string(), description: "Optional path to VMAF model file (.json). Leave empty to use default model.".to_string(), items: None }),
                    ]),
                    required: vec!["distorted_file".to_string(), "reference_file".to_string()],
                },
            },

            ClaudeTool {
                name: "shift_audio_frequency".to_string(),
                description: "Shifts all audio frequencies by a constant amount in Hz using the FFmpeg afreqshift filter. Produces pitch-shifting without time stretching.".to_string(),
                input_schema: InputSchema {
                    schema_type: "object".to_string(),
                    properties: HashMap::from([
                        ("input_file".to_string(), PropertyDefinition { prop_type: "string".to_string(), description: "Path to the input audio/video file".to_string(), items: None }),
                        ("output_file".to_string(), PropertyDefinition { prop_type: "string".to_string(), description: "Path to save the output".to_string(), items: None }),
                        ("shift".to_string(), PropertyDefinition { prop_type: "number".to_string(), description: "Frequency shift in Hz. Positive = shift up, negative = shift down. Default 0.".to_string(), items: None }),
                    ]),
                    required: vec!["input_file".to_string(), "output_file".to_string(), "shift".to_string()],
                },
            },

            ClaudeTool {
                name: "apply_audio_pulsator".to_string(),
                description: "Applies a pulsating amplitude effect to stereo audio using the FFmpeg apulsator filter. Creates a rhythmic pumping or heartbeat-style modulation.".to_string(),
                input_schema: InputSchema {
                    schema_type: "object".to_string(),
                    properties: HashMap::from([
                        ("input_file".to_string(), PropertyDefinition { prop_type: "string".to_string(), description: "Path to the input audio/video file".to_string(), items: None }),
                        ("output_file".to_string(), PropertyDefinition { prop_type: "string".to_string(), description: "Path to save the output".to_string(), items: None }),
                        ("hz".to_string(), PropertyDefinition { prop_type: "number".to_string(), description: "Pulse rate in Hz (0.01–100). Default 2.".to_string(), items: None }),
                        ("amount".to_string(), PropertyDefinition { prop_type: "number".to_string(), description: "Modulation depth (0–1). Default 1.".to_string(), items: None }),
                        ("offset_l".to_string(), PropertyDefinition { prop_type: "number".to_string(), description: "Left channel phase offset (0–1). Default 0.".to_string(), items: None }),
                        ("offset_r".to_string(), PropertyDefinition { prop_type: "number".to_string(), description: "Right channel phase offset (0–1). Default 0.5.".to_string(), items: None }),
                        ("mode".to_string(), PropertyDefinition { prop_type: "string".to_string(), description: "LFO waveform: sine (default), triangle, square, sawup, sawdown.".to_string(), items: None }),
                    ]),
                    required: vec!["input_file".to_string(), "output_file".to_string()],
                },
            },

            ClaudeTool {
                name: "enhance_dialogue".to_string(),
                description: "Enhances speech/dialogue clarity in audio using the FFmpeg dialoguenhance filter. Improves intelligibility of voice without affecting music or effects.".to_string(),
                input_schema: InputSchema {
                    schema_type: "object".to_string(),
                    properties: HashMap::from([
                        ("input_file".to_string(), PropertyDefinition { prop_type: "string".to_string(), description: "Path to the input audio/video file".to_string(), items: None }),
                        ("output_file".to_string(), PropertyDefinition { prop_type: "string".to_string(), description: "Path to save the output".to_string(), items: None }),
                        ("original".to_string(), PropertyDefinition { prop_type: "number".to_string(), description: "Amount of original signal (0–1). Default 0.5.".to_string(), items: None }),
                        ("expand".to_string(), PropertyDefinition { prop_type: "number".to_string(), description: "Enhancement strength (1–3). Default 2.".to_string(), items: None }),
                    ]),
                    required: vec!["input_file".to_string(), "output_file".to_string()],
                },
            },

            ClaudeTool {
                name: "split_audio_channels".to_string(),
                description: "Extracts a single named channel from a multichannel audio stream to a mono output file using FFmpeg channelsplit.".to_string(),
                input_schema: InputSchema {
                    schema_type: "object".to_string(),
                    properties: HashMap::from([
                        ("input_file".to_string(), PropertyDefinition { prop_type: "string".to_string(), description: "Path to the input audio/video file".to_string(), items: None }),
                        ("output_file".to_string(), PropertyDefinition { prop_type: "string".to_string(), description: "Path to save the extracted mono channel".to_string(), items: None }),
                        ("channel_layout".to_string(), PropertyDefinition { prop_type: "string".to_string(), description: "Input channel layout e.g. 'stereo', '5.1'. Default 'stereo'.".to_string(), items: None }),
                        ("channel".to_string(), PropertyDefinition { prop_type: "string".to_string(), description: "Channel name to extract e.g. 'FL' (front-left), 'FR', 'FC', 'LFE', 'BL', 'BR'. Default 'FL'.".to_string(), items: None }),
                    ]),
                    required: vec!["input_file".to_string(), "output_file".to_string()],
                },
            },

            ClaudeTool {
                name: "map_audio_channels".to_string(),
                description: "Remaps audio channels to different output positions using the FFmpeg channelmap filter.".to_string(),
                input_schema: InputSchema {
                    schema_type: "object".to_string(),
                    properties: HashMap::from([
                        ("input_file".to_string(), PropertyDefinition { prop_type: "string".to_string(), description: "Path to the input audio/video file".to_string(), items: None }),
                        ("output_file".to_string(), PropertyDefinition { prop_type: "string".to_string(), description: "Path to save the remapped output".to_string(), items: None }),
                        ("channel_map".to_string(), PropertyDefinition { prop_type: "string".to_string(), description: "Channel mapping e.g. 'FL-FL|FR-FR' (output-input pairs). Default 'FL-FL|FR-FR'.".to_string(), items: None }),
                        ("channel_layout".to_string(), PropertyDefinition { prop_type: "string".to_string(), description: "Output channel layout e.g. 'stereo'. Default 'stereo'.".to_string(), items: None }),
                    ]),
                    required: vec!["input_file".to_string(), "output_file".to_string()],
                },
            },

            ClaudeTool {
                name: "merge_audio_inputs".to_string(),
                description: "Merges multiple mono or stereo audio files into a single multichannel audio file using the FFmpeg amerge filter.".to_string(),
                input_schema: InputSchema {
                    schema_type: "object".to_string(),
                    properties: HashMap::from([
                        ("input_files".to_string(), PropertyDefinition {
                            prop_type: "array".to_string(),
                            description: "Array of input audio/video file paths to merge (minimum 2)".to_string(),
                            items: Some(Box::new(PropertyDefinition {
                                prop_type: "string".to_string(),
                                description: "Path to an audio or video file".to_string(),
                                items: None,
                            })),
                        }),
                        ("output_file".to_string(), PropertyDefinition { prop_type: "string".to_string(), description: "Path to save the merged multichannel audio output".to_string(), items: None }),
                    ]),
                    required: vec!["input_files".to_string(), "output_file".to_string()],
                },
            },

            ClaudeTool {
                name: "apply_crossfeed".to_string(),
                description: "Applies crossfeed processing to stereo audio for comfortable headphone listening — reduces stereo width and adds inter-aural crosstalk.".to_string(),
                input_schema: InputSchema {
                    schema_type: "object".to_string(),
                    properties: HashMap::from([
                        ("input_file".to_string(), PropertyDefinition { prop_type: "string".to_string(), description: "Path to the input audio/video file".to_string(), items: None }),
                        ("output_file".to_string(), PropertyDefinition { prop_type: "string".to_string(), description: "Path to save the crossfeed output".to_string(), items: None }),
                        ("strength".to_string(), PropertyDefinition { prop_type: "number".to_string(), description: "Crossfeed strength (0–1). Default 0.5.".to_string(), items: None }),
                        ("slope".to_string(), PropertyDefinition { prop_type: "number".to_string(), description: "Filter slope (0.01–1.0). Default 0.5.".to_string(), items: None }),
                        ("level_in".to_string(), PropertyDefinition { prop_type: "number".to_string(), description: "Input gain (0.015625–64). Default 1.".to_string(), items: None }),
                        ("level_out".to_string(), PropertyDefinition { prop_type: "number".to_string(), description: "Output gain (0.015625–64). Default 1.".to_string(), items: None }),
                    ]),
                    required: vec!["input_file".to_string(), "output_file".to_string()],
                },
            },

            ClaudeTool {
                name: "apply_extrastereo".to_string(),
                description: "Increases stereo separation beyond the original recording using the FFmpeg extrastereo filter. Makes mixes sound wider.".to_string(),
                input_schema: InputSchema {
                    schema_type: "object".to_string(),
                    properties: HashMap::from([
                        ("input_file".to_string(), PropertyDefinition { prop_type: "string".to_string(), description: "Path to the input audio/video file".to_string(), items: None }),
                        ("output_file".to_string(), PropertyDefinition { prop_type: "string".to_string(), description: "Path to save the output".to_string(), items: None }),
                        ("multiplier".to_string(), PropertyDefinition { prop_type: "number".to_string(), description: "Stereo separation multiplier (-10–10). Default 2.5.".to_string(), items: None }),
                        ("clipping".to_string(), PropertyDefinition { prop_type: "boolean".to_string(), description: "Enable clipping prevention. Default false.".to_string(), items: None }),
                    ]),
                    required: vec!["input_file".to_string(), "output_file".to_string()],
                },
            },

            ClaudeTool {
                name: "apply_firequalizer".to_string(),
                description: "Applies a linear-phase FIR equalizer using arbitrary frequency gain entries (FFmpeg firequalizer). More accurate than IIR for mastering.".to_string(),
                input_schema: InputSchema {
                    schema_type: "object".to_string(),
                    properties: HashMap::from([
                        ("input_file".to_string(), PropertyDefinition { prop_type: "string".to_string(), description: "Path to the input audio/video file".to_string(), items: None }),
                        ("output_file".to_string(), PropertyDefinition { prop_type: "string".to_string(), description: "Path to save the equalised output".to_string(), items: None }),
                        ("gain_entry".to_string(), PropertyDefinition { prop_type: "string".to_string(), description: "Semicolon-separated frequency/gain entries e.g. 'entry(0,0);entry(1000,-6);entry(4000,0)'. Frequency in Hz, gain in dB.".to_string(), items: None }),
                    ]),
                    required: vec!["input_file".to_string(), "output_file".to_string(), "gain_entry".to_string()],
                },
            },

            ClaudeTool {
                name: "apply_biquad".to_string(),
                description: "Applies a direct-form II biquad IIR filter to audio using user-supplied coefficients (FFmpeg biquad). For advanced DSP filter design.".to_string(),
                input_schema: InputSchema {
                    schema_type: "object".to_string(),
                    properties: HashMap::from([
                        ("input_file".to_string(), PropertyDefinition { prop_type: "string".to_string(), description: "Path to the input audio/video file".to_string(), items: None }),
                        ("output_file".to_string(), PropertyDefinition { prop_type: "string".to_string(), description: "Path to save the filtered output".to_string(), items: None }),
                        ("b0".to_string(), PropertyDefinition { prop_type: "number".to_string(), description: "Numerator coefficient b0. Default 1.".to_string(), items: None }),
                        ("b1".to_string(), PropertyDefinition { prop_type: "number".to_string(), description: "Numerator coefficient b1. Default 0.".to_string(), items: None }),
                        ("b2".to_string(), PropertyDefinition { prop_type: "number".to_string(), description: "Numerator coefficient b2. Default 0.".to_string(), items: None }),
                        ("a0".to_string(), PropertyDefinition { prop_type: "number".to_string(), description: "Denominator coefficient a0. Default 1.".to_string(), items: None }),
                        ("a1".to_string(), PropertyDefinition { prop_type: "number".to_string(), description: "Denominator coefficient a1. Default 0.".to_string(), items: None }),
                        ("a2".to_string(), PropertyDefinition { prop_type: "number".to_string(), description: "Denominator coefficient a2. Default 0.".to_string(), items: None }),
                    ]),
                    required: vec!["input_file".to_string(), "output_file".to_string()],
                },
            },

            ClaudeTool {
                name: "filter_bandpass".to_string(),
                description: "Applies a bandpass filter that passes frequencies within a band and attenuates those outside using the FFmpeg bandpass filter.".to_string(),
                input_schema: InputSchema {
                    schema_type: "object".to_string(),
                    properties: HashMap::from([
                        ("input_file".to_string(), PropertyDefinition { prop_type: "string".to_string(), description: "Path to the input audio/video file".to_string(), items: None }),
                        ("output_file".to_string(), PropertyDefinition { prop_type: "string".to_string(), description: "Path to save the filtered output".to_string(), items: None }),
                        ("frequency".to_string(), PropertyDefinition { prop_type: "number".to_string(), description: "Centre frequency in Hz. Default 3000.".to_string(), items: None }),
                        ("width".to_string(), PropertyDefinition { prop_type: "number".to_string(), description: "Filter bandwidth. Default 200.".to_string(), items: None }),
                        ("width_type".to_string(), PropertyDefinition { prop_type: "string".to_string(), description: "Width type: h=Hz (default), q=Q-factor, o=octaves, s=slope.".to_string(), items: None }),
                    ]),
                    required: vec!["input_file".to_string(), "output_file".to_string()],
                },
            },

            ClaudeTool {
                name: "filter_bandreject".to_string(),
                description: "Applies a band-reject (notch) filter that attenuates a specific frequency band using the FFmpeg bandreject filter.".to_string(),
                input_schema: InputSchema {
                    schema_type: "object".to_string(),
                    properties: HashMap::from([
                        ("input_file".to_string(), PropertyDefinition { prop_type: "string".to_string(), description: "Path to the input audio/video file".to_string(), items: None }),
                        ("output_file".to_string(), PropertyDefinition { prop_type: "string".to_string(), description: "Path to save the filtered output".to_string(), items: None }),
                        ("frequency".to_string(), PropertyDefinition { prop_type: "number".to_string(), description: "Centre frequency to reject in Hz. Default 3000.".to_string(), items: None }),
                        ("width".to_string(), PropertyDefinition { prop_type: "number".to_string(), description: "Rejection bandwidth. Default 200.".to_string(), items: None }),
                        ("width_type".to_string(), PropertyDefinition { prop_type: "string".to_string(), description: "Width type: h=Hz (default), q=Q-factor, o=octaves, s=slope.".to_string(), items: None }),
                    ]),
                    required: vec!["input_file".to_string(), "output_file".to_string()],
                },
            },

            ClaudeTool {
                name: "boost_sub_bass".to_string(),
                description: "Boosts sub-bass frequencies using the FFmpeg asubboost filter. Adds perceived low-end warmth and weight to audio.".to_string(),
                input_schema: InputSchema {
                    schema_type: "object".to_string(),
                    properties: HashMap::from([
                        ("input_file".to_string(), PropertyDefinition { prop_type: "string".to_string(), description: "Path to the input audio/video file".to_string(), items: None }),
                        ("output_file".to_string(), PropertyDefinition { prop_type: "string".to_string(), description: "Path to save the boosted output".to_string(), items: None }),
                        ("dry".to_string(), PropertyDefinition { prop_type: "number".to_string(), description: "Dry (original) signal level (0–1). Default 1.".to_string(), items: None }),
                        ("wet".to_string(), PropertyDefinition { prop_type: "number".to_string(), description: "Wet (boosted) signal level (0–1). Default 1.".to_string(), items: None }),
                        ("freq".to_string(), PropertyDefinition { prop_type: "number".to_string(), description: "Sub-bass cutoff frequency in Hz (10–200). Default 20.".to_string(), items: None }),
                        ("decay".to_string(), PropertyDefinition { prop_type: "number".to_string(), description: "Decay factor (0–1). Default 0.7.".to_string(), items: None }),
                    ]),
                    required: vec!["input_file".to_string(), "output_file".to_string()],
                },
            },

            // ================================================================
            // PHASE G — AI/ML Filters
            // ================================================================

            ClaudeTool {
                name: "detect_objects_dnn".to_string(),
                description: "Runs DNN-based object detection on a video using FFmpeg dnn_detect filter. Draws bounding boxes around detected objects.".to_string(),
                input_schema: InputSchema {
                    schema_type: "object".to_string(),
                    properties: HashMap::from([
                        ("input_file".to_string(), PropertyDefinition { prop_type: "string".to_string(), description: "Path to the input video file".to_string(), items: None }),
                        ("output_file".to_string(), PropertyDefinition { prop_type: "string".to_string(), description: "Path to save the annotated output video".to_string(), items: None }),
                        ("model".to_string(), PropertyDefinition { prop_type: "string".to_string(), description: "Path to the DNN model file (e.g. yolov3.weights or model.onnx)".to_string(), items: None }),
                        ("backend".to_string(), PropertyDefinition { prop_type: "string".to_string(), description: "DNN backend: native, openvino, tensorflow, pytorch, onnx. Default 'native'.".to_string(), items: None }),
                        ("confidence".to_string(), PropertyDefinition { prop_type: "number".to_string(), description: "Minimum confidence threshold (0–1). Default 0.5.".to_string(), items: None }),
                        ("labels".to_string(), PropertyDefinition { prop_type: "string".to_string(), description: "Optional path to labels file for class names".to_string(), items: None }),
                    ]),
                    required: vec!["input_file".to_string(), "output_file".to_string(), "model".to_string()],
                },
            },

            ClaudeTool {
                name: "classify_frames_dnn".to_string(),
                description: "Runs DNN-based image classification on video frames using FFmpeg dnn_classify filter. Overlays predicted class label on each frame.".to_string(),
                input_schema: InputSchema {
                    schema_type: "object".to_string(),
                    properties: HashMap::from([
                        ("input_file".to_string(), PropertyDefinition { prop_type: "string".to_string(), description: "Path to the input video file".to_string(), items: None }),
                        ("output_file".to_string(), PropertyDefinition { prop_type: "string".to_string(), description: "Path to save the classified output video".to_string(), items: None }),
                        ("model".to_string(), PropertyDefinition { prop_type: "string".to_string(), description: "Path to the DNN classification model file".to_string(), items: None }),
                        ("backend".to_string(), PropertyDefinition { prop_type: "string".to_string(), description: "DNN backend: native, openvino, tensorflow, pytorch, onnx. Default 'native'.".to_string(), items: None }),
                        ("labels".to_string(), PropertyDefinition { prop_type: "string".to_string(), description: "Optional path to labels file for class names".to_string(), items: None }),
                    ]),
                    required: vec!["input_file".to_string(), "output_file".to_string(), "model".to_string()],
                },
            },

            ClaudeTool {
                name: "upscale_super_resolution".to_string(),
                description: "AI-powered video upscaling using FFmpeg sr (super-resolution) filter. Increases resolution using DNN-based super-resolution models.".to_string(),
                input_schema: InputSchema {
                    schema_type: "object".to_string(),
                    properties: HashMap::from([
                        ("input_file".to_string(), PropertyDefinition { prop_type: "string".to_string(), description: "Path to the input video file".to_string(), items: None }),
                        ("output_file".to_string(), PropertyDefinition { prop_type: "string".to_string(), description: "Path to save the upscaled output video".to_string(), items: None }),
                        ("scale_factor".to_string(), PropertyDefinition { prop_type: "number".to_string(), description: "Upscale multiplier: 2 or 4. Default 2.".to_string(), items: None }),
                        ("model".to_string(), PropertyDefinition { prop_type: "string".to_string(), description: "Optional path to a custom super-resolution model file".to_string(), items: None }),
                        ("backend".to_string(), PropertyDefinition { prop_type: "string".to_string(), description: "DNN backend: native, openvino, tensorflow. Default 'native'.".to_string(), items: None }),
                    ]),
                    required: vec!["input_file".to_string(), "output_file".to_string()],
                },
            },

            ClaudeTool {
                name: "remove_rain_ai".to_string(),
                description: "Removes rain streaks from video using FFmpeg derain AI filter (DNN-based). Requires a trained derain model.".to_string(),
                input_schema: InputSchema {
                    schema_type: "object".to_string(),
                    properties: HashMap::from([
                        ("input_file".to_string(), PropertyDefinition { prop_type: "string".to_string(), description: "Path to the input video file".to_string(), items: None }),
                        ("output_file".to_string(), PropertyDefinition { prop_type: "string".to_string(), description: "Path to save the derrained output video".to_string(), items: None }),
                        ("model".to_string(), PropertyDefinition { prop_type: "string".to_string(), description: "Path to the derain DNN model file".to_string(), items: None }),
                        ("backend".to_string(), PropertyDefinition { prop_type: "string".to_string(), description: "DNN backend: native, openvino, tensorflow. Default 'native'.".to_string(), items: None }),
                    ]),
                    required: vec!["input_file".to_string(), "output_file".to_string(), "model".to_string()],
                },
            },

            ClaudeTool {
                name: "detect_frozen_frames".to_string(),
                description: "Detects frozen/stuck frames in a video using FFmpeg freezedetect filter. Returns timestamps and durations of freeze events. Analysis-only — no output file.".to_string(),
                input_schema: InputSchema {
                    schema_type: "object".to_string(),
                    properties: HashMap::from([
                        ("input_file".to_string(), PropertyDefinition { prop_type: "string".to_string(), description: "Path to the input video file to analyze".to_string(), items: None }),
                        ("noise_db".to_string(), PropertyDefinition { prop_type: "number".to_string(), description: "Noise threshold in dB (negative). Default -60.".to_string(), items: None }),
                        ("duration".to_string(), PropertyDefinition { prop_type: "number".to_string(), description: "Minimum freeze duration in seconds. Default 2.0.".to_string(), items: None }),
                    ]),
                    required: vec!["input_file".to_string()],
                },
            },

            ClaudeTool {
                name: "apply_edgedetect".to_string(),
                description: "Detects and visualises edges in a video using FFmpeg edgedetect filter. Useful for stylised looks, motion analysis, and visual effects.".to_string(),
                input_schema: InputSchema {
                    schema_type: "object".to_string(),
                    properties: HashMap::from([
                        ("input_file".to_string(), PropertyDefinition { prop_type: "string".to_string(), description: "Path to the input video file".to_string(), items: None }),
                        ("output_file".to_string(), PropertyDefinition { prop_type: "string".to_string(), description: "Path to save the edge-detected output video".to_string(), items: None }),
                        ("low".to_string(), PropertyDefinition { prop_type: "number".to_string(), description: "Low hysteresis threshold (0–1). Default 0.0625.".to_string(), items: None }),
                        ("high".to_string(), PropertyDefinition { prop_type: "number".to_string(), description: "High hysteresis threshold (0–1). Default 0.1875.".to_string(), items: None }),
                        ("mode".to_string(), PropertyDefinition { prop_type: "string".to_string(), description: "Edge mode: wires, colormix, canny. Default 'wires'.".to_string(), items: None }),
                    ]),
                    required: vec!["input_file".to_string(), "output_file".to_string()],
                },
            },

            // ================================================================
            // PHASE H — Codec / Format Depth
            // ================================================================

            ClaudeTool {
                name: "encode_vp9".to_string(),
                description: "Encodes video to VP9 using libvpx-vp9 with Opus audio. Best open codec for web delivery with superior compression vs H.264.".to_string(),
                input_schema: InputSchema {
                    schema_type: "object".to_string(),
                    properties: HashMap::from([
                        ("input_file".to_string(), PropertyDefinition { prop_type: "string".to_string(), description: "Path to the input video file".to_string(), items: None }),
                        ("output_file".to_string(), PropertyDefinition { prop_type: "string".to_string(), description: "Path to save the VP9 encoded output (should be .webm or .mkv)".to_string(), items: None }),
                        ("crf".to_string(), PropertyDefinition { prop_type: "number".to_string(), description: "Constant quality factor (0–63). Lower = better quality. Default 31.".to_string(), items: None }),
                        ("bitrate".to_string(), PropertyDefinition { prop_type: "string".to_string(), description: "Target bitrate e.g. '2M'. Leave empty for CRF-only mode (recommended). Default empty.".to_string(), items: None }),
                        ("speed".to_string(), PropertyDefinition { prop_type: "number".to_string(), description: "CPU usage / speed preset (0–5). Higher = faster but lower quality. Default 2.".to_string(), items: None }),
                        ("threads".to_string(), PropertyDefinition { prop_type: "number".to_string(), description: "Encoding thread count. Default 4.".to_string(), items: None }),
                    ]),
                    required: vec!["input_file".to_string(), "output_file".to_string()],
                },
            },

            ClaudeTool {
                name: "encode_av1".to_string(),
                description: "Encodes video to AV1 using libaom-av1 or libsvtav1. Next-generation codec with ~30% better compression than VP9/H.265.".to_string(),
                input_schema: InputSchema {
                    schema_type: "object".to_string(),
                    properties: HashMap::from([
                        ("input_file".to_string(), PropertyDefinition { prop_type: "string".to_string(), description: "Path to the input video file".to_string(), items: None }),
                        ("output_file".to_string(), PropertyDefinition { prop_type: "string".to_string(), description: "Path to save the AV1 encoded output (should be .webm or .mkv)".to_string(), items: None }),
                        ("crf".to_string(), PropertyDefinition { prop_type: "number".to_string(), description: "Constant quality factor (0–63). Default 30.".to_string(), items: None }),
                        ("speed".to_string(), PropertyDefinition { prop_type: "number".to_string(), description: "CPU speed preset (0–8 for libaom, 0–12 for svtav1). Default 4.".to_string(), items: None }),
                        ("threads".to_string(), PropertyDefinition { prop_type: "number".to_string(), description: "Encoding thread count. Default 4.".to_string(), items: None }),
                        ("encoder".to_string(), PropertyDefinition { prop_type: "string".to_string(), description: "AV1 encoder: libaom-av1 (best quality, slow) or libsvtav1 (fast). Default libaom-av1.".to_string(), items: None }),
                    ]),
                    required: vec!["input_file".to_string(), "output_file".to_string()],
                },
            },

            ClaudeTool {
                name: "encode_hevc".to_string(),
                description: "Encodes video to H.265/HEVC using libx265. Up to 50% better compression than H.264 at equivalent quality.".to_string(),
                input_schema: InputSchema {
                    schema_type: "object".to_string(),
                    properties: HashMap::from([
                        ("input_file".to_string(), PropertyDefinition { prop_type: "string".to_string(), description: "Path to the input video file".to_string(), items: None }),
                        ("output_file".to_string(), PropertyDefinition { prop_type: "string".to_string(), description: "Path to save the HEVC encoded output (should be .mp4 or .mkv)".to_string(), items: None }),
                        ("crf".to_string(), PropertyDefinition { prop_type: "number".to_string(), description: "Constant Rate Factor (0–51). Default 28. Lower = better quality.".to_string(), items: None }),
                        ("preset".to_string(), PropertyDefinition { prop_type: "string".to_string(), description: "Encoding speed preset: ultrafast, superfast, veryfast, faster, fast, medium, slow, slower, veryslow. Default medium.".to_string(), items: None }),
                        ("tune".to_string(), PropertyDefinition { prop_type: "string".to_string(), description: "Tuning: grain, zerolatency, fastdecode, animation. Leave empty for default.".to_string(), items: None }),
                    ]),
                    required: vec!["input_file".to_string(), "output_file".to_string()],
                },
            },

            ClaudeTool {
                name: "encode_opus".to_string(),
                description: "Encodes audio to Opus format using libopus. Best-in-class lossy audio codec for streaming, VoIP, and music.".to_string(),
                input_schema: InputSchema {
                    schema_type: "object".to_string(),
                    properties: HashMap::from([
                        ("input_file".to_string(), PropertyDefinition { prop_type: "string".to_string(), description: "Path to the input audio or video file".to_string(), items: None }),
                        ("output_file".to_string(), PropertyDefinition { prop_type: "string".to_string(), description: "Path to save the Opus audio output (should be .opus or .ogg)".to_string(), items: None }),
                        ("bitrate_kbps".to_string(), PropertyDefinition { prop_type: "number".to_string(), description: "Target bitrate in kbps (6–510). Default 128.".to_string(), items: None }),
                        ("vbr".to_string(), PropertyDefinition { prop_type: "string".to_string(), description: "Variable bitrate mode: true or false. Default true.".to_string(), items: None }),
                        ("compression".to_string(), PropertyDefinition { prop_type: "number".to_string(), description: "Compression level (0–10). Higher = slower but smaller. Default 10.".to_string(), items: None }),
                    ]),
                    required: vec!["input_file".to_string(), "output_file".to_string()],
                },
            },

            ClaudeTool {
                name: "encode_hdr10".to_string(),
                description: "Encodes video to HDR10 using libx265 with mastering display and MaxCLL/MaxFALL metadata for HDR-capable screens.".to_string(),
                input_schema: InputSchema {
                    schema_type: "object".to_string(),
                    properties: HashMap::from([
                        ("input_file".to_string(), PropertyDefinition { prop_type: "string".to_string(), description: "Path to the input HDR video file (should already be in HDR colour space)".to_string(), items: None }),
                        ("output_file".to_string(), PropertyDefinition { prop_type: "string".to_string(), description: "Path to save the HDR10 encoded output".to_string(), items: None }),
                        ("crf".to_string(), PropertyDefinition { prop_type: "number".to_string(), description: "Constant Rate Factor (0–51). Default 22.".to_string(), items: None }),
                        ("preset".to_string(), PropertyDefinition { prop_type: "string".to_string(), description: "x265 preset. Default slow.".to_string(), items: None }),
                        ("master_display".to_string(), PropertyDefinition { prop_type: "string".to_string(), description: "Mastering display primaries string for x265, e.g. 'G(13250,34500)B(7500,3000)R(34000,16000)WP(15635,16450)L(10000000,1)'. Leave empty for Rec.2020 defaults.".to_string(), items: None }),
                        ("max_cll".to_string(), PropertyDefinition { prop_type: "string".to_string(), description: "MaxCLL,MaxFALL values e.g. '1000,400'. Leave empty for default.".to_string(), items: None }),
                    ]),
                    required: vec!["input_file".to_string(), "output_file".to_string()],
                },
            },

            ClaudeTool {
                name: "encode_nvenc".to_string(),
                description: "Hardware-accelerated video encoding using NVIDIA NVENC (CUDA GPU). Extremely fast encoding with near-software quality.".to_string(),
                input_schema: InputSchema {
                    schema_type: "object".to_string(),
                    properties: HashMap::from([
                        ("input_file".to_string(), PropertyDefinition { prop_type: "string".to_string(), description: "Path to the input video file".to_string(), items: None }),
                        ("output_file".to_string(), PropertyDefinition { prop_type: "string".to_string(), description: "Path to save the NVENC encoded output".to_string(), items: None }),
                        ("codec".to_string(), PropertyDefinition { prop_type: "string".to_string(), description: "Codec: h264 (default), hevc, av1. Selects h264_nvenc / hevc_nvenc / av1_nvenc.".to_string(), items: None }),
                        ("preset".to_string(), PropertyDefinition { prop_type: "string".to_string(), description: "NVENC preset: p1 (fastest) to p7 (best quality). Default p4 (balanced).".to_string(), items: None }),
                        ("bitrate".to_string(), PropertyDefinition { prop_type: "string".to_string(), description: "Target bitrate e.g. '8M'. Leave empty to use CQ only.".to_string(), items: None }),
                        ("cq".to_string(), PropertyDefinition { prop_type: "number".to_string(), description: "Constant quality value (0–51). Default 23.".to_string(), items: None }),
                    ]),
                    required: vec!["input_file".to_string(), "output_file".to_string()],
                },
            },

            ClaudeTool {
                name: "encode_vaapi".to_string(),
                description: "Hardware-accelerated encoding using Intel/AMD VAAPI GPU. Fast encoding on Linux with Intel or AMD GPUs.".to_string(),
                input_schema: InputSchema {
                    schema_type: "object".to_string(),
                    properties: HashMap::from([
                        ("input_file".to_string(), PropertyDefinition { prop_type: "string".to_string(), description: "Path to the input video file".to_string(), items: None }),
                        ("output_file".to_string(), PropertyDefinition { prop_type: "string".to_string(), description: "Path to save the VAAPI encoded output".to_string(), items: None }),
                        ("codec".to_string(), PropertyDefinition { prop_type: "string".to_string(), description: "Codec: h264 (default), hevc, vp9, av1.".to_string(), items: None }),
                        ("quality".to_string(), PropertyDefinition { prop_type: "number".to_string(), description: "QP quality value (1–51). Lower = better. Default 23.".to_string(), items: None }),
                        ("profile".to_string(), PropertyDefinition { prop_type: "string".to_string(), description: "Encoding profile: high (default), main, baseline.".to_string(), items: None }),
                    ]),
                    required: vec!["input_file".to_string(), "output_file".to_string()],
                },
            },

            ClaudeTool {
                name: "encode_qsv".to_string(),
                description: "Hardware-accelerated encoding using Intel Quick Sync Video (QSV). Very fast encoding on systems with Intel integrated or Arc GPUs.".to_string(),
                input_schema: InputSchema {
                    schema_type: "object".to_string(),
                    properties: HashMap::from([
                        ("input_file".to_string(), PropertyDefinition { prop_type: "string".to_string(), description: "Path to the input video file".to_string(), items: None }),
                        ("output_file".to_string(), PropertyDefinition { prop_type: "string".to_string(), description: "Path to save the QSV encoded output".to_string(), items: None }),
                        ("codec".to_string(), PropertyDefinition { prop_type: "string".to_string(), description: "Codec: h264 (default), hevc, av1, vp9.".to_string(), items: None }),
                        ("preset".to_string(), PropertyDefinition { prop_type: "string".to_string(), description: "Speed preset: veryfast, faster, fast, medium (default), slow, slower, veryslow.".to_string(), items: None }),
                        ("bitrate".to_string(), PropertyDefinition { prop_type: "string".to_string(), description: "Target bitrate e.g. '6M'. Leave empty for auto.".to_string(), items: None }),
                    ]),
                    required: vec!["input_file".to_string(), "output_file".to_string()],
                },
            },

            ClaudeTool {
                name: "encode_prores".to_string(),
                description: "Encodes video to Apple ProRes using prores_ks. Professional intermediate codec for editing workflows and Apple ecosystem delivery.".to_string(),
                input_schema: InputSchema {
                    schema_type: "object".to_string(),
                    properties: HashMap::from([
                        ("input_file".to_string(), PropertyDefinition { prop_type: "string".to_string(), description: "Path to the input video file".to_string(), items: None }),
                        ("output_file".to_string(), PropertyDefinition { prop_type: "string".to_string(), description: "Path to save the ProRes output (should be .mov)".to_string(), items: None }),
                        ("profile".to_string(), PropertyDefinition { prop_type: "number".to_string(), description: "ProRes profile: 0=Proxy, 1=LT, 2=Standard, 3=HQ, 4=4444, 5=4444XQ. Default 3 (HQ).".to_string(), items: None }),
                        ("vendor".to_string(), PropertyDefinition { prop_type: "string".to_string(), description: "4-char vendor tag. Default 'apl0' (Apple).".to_string(), items: None }),
                    ]),
                    required: vec!["input_file".to_string(), "output_file".to_string()],
                },
            },

            ClaudeTool {
                name: "encode_dnxhd".to_string(),
                description: "Encodes video to Avid DNxHD/DNxHR using the dnxhd codec. Professional intermediate codec for Avid Media Composer and post-production workflows.".to_string(),
                input_schema: InputSchema {
                    schema_type: "object".to_string(),
                    properties: HashMap::from([
                        ("input_file".to_string(), PropertyDefinition { prop_type: "string".to_string(), description: "Path to the input video file".to_string(), items: None }),
                        ("output_file".to_string(), PropertyDefinition { prop_type: "string".to_string(), description: "Path to save the DNxHD/DNxHR output (should be .mxf or .mov)".to_string(), items: None }),
                        ("profile".to_string(), PropertyDefinition { prop_type: "string".to_string(), description: "DNxHR profile: dnxhr_lb (low bandwidth), dnxhr_sq (standard, default), dnxhr_hq (high quality), dnxhr_hqx (10-bit HQ), dnxhr_444 (4:4:4).".to_string(), items: None }),
                    ]),
                    required: vec!["input_file".to_string(), "output_file".to_string()],
                },
            },

            ClaudeTool {
                name: "encode_gif".to_string(),
                description: "Creates a high-quality animated GIF using FFmpeg 2-pass palette optimisation (palettegen + paletteuse). Much better quality than naive GIF conversion.".to_string(),
                input_schema: InputSchema {
                    schema_type: "object".to_string(),
                    properties: HashMap::from([
                        ("input_file".to_string(), PropertyDefinition { prop_type: "string".to_string(), description: "Path to the input video file".to_string(), items: None }),
                        ("output_file".to_string(), PropertyDefinition { prop_type: "string".to_string(), description: "Path to save the animated GIF".to_string(), items: None }),
                        ("fps".to_string(), PropertyDefinition { prop_type: "number".to_string(), description: "Frames per second for the GIF. Default 15.".to_string(), items: None }),
                        ("scale".to_string(), PropertyDefinition { prop_type: "number".to_string(), description: "Output width in pixels (height auto-calculated). Default 480.".to_string(), items: None }),
                        ("loop_count".to_string(), PropertyDefinition { prop_type: "number".to_string(), description: "Loop count: 0 = infinite (default), -1 = no loop, N = loop N times.".to_string(), items: None }),
                    ]),
                    required: vec!["input_file".to_string(), "output_file".to_string()],
                },
            },

            ClaudeTool {
                name: "encode_webm".to_string(),
                description: "Encodes video to WebM container format (VP8/VP9 video + Vorbis/Opus audio). Open web format supported by all modern browsers.".to_string(),
                input_schema: InputSchema {
                    schema_type: "object".to_string(),
                    properties: HashMap::from([
                        ("input_file".to_string(), PropertyDefinition { prop_type: "string".to_string(), description: "Path to the input video file".to_string(), items: None }),
                        ("output_file".to_string(), PropertyDefinition { prop_type: "string".to_string(), description: "Path to save the WebM output".to_string(), items: None }),
                        ("video_codec".to_string(), PropertyDefinition { prop_type: "string".to_string(), description: "Video codec: vp8 (default, fast) or vp9 (better quality/compression).".to_string(), items: None }),
                        ("audio_codec".to_string(), PropertyDefinition { prop_type: "string".to_string(), description: "Audio codec: vorbis (default) or opus (better quality).".to_string(), items: None }),
                        ("crf".to_string(), PropertyDefinition { prop_type: "number".to_string(), description: "Constant quality factor. VP8: 4–63 (default 10), VP9: 0–63 (default 31).".to_string(), items: None }),
                        ("bitrate".to_string(), PropertyDefinition { prop_type: "string".to_string(), description: "Target bitrate e.g. '1M'. Leave empty for CRF-only.".to_string(), items: None }),
                    ]),
                    required: vec!["input_file".to_string(), "output_file".to_string()],
                },
            },

            // ================================================================
            // PHASE I — Long-tail sweep, Batch 1
            // ================================================================

            ClaudeTool {
                name: "zoom_pan".to_string(),
                description: "Ken Burns zoom-and-pan effect using FFmpeg zoompan filter.".to_string(),
                input_schema: InputSchema {
                    schema_type: "object".to_string(),
                    properties: HashMap::from([
                        ("input_file".to_string(), PropertyDefinition { prop_type: "string".to_string(), description: "Path to the input video or image file".to_string(), items: None }),
                        ("output_file".to_string(), PropertyDefinition { prop_type: "string".to_string(), description: "Path to save the zoompan output".to_string(), items: None }),
                        ("zoom".to_string(), PropertyDefinition { prop_type: "number".to_string(), description: "Zoom level (>1.0). Default 1.5.".to_string(), items: None }),
                        ("x_expr".to_string(), PropertyDefinition { prop_type: "string".to_string(), description: "X position expression. Default centres frame.".to_string(), items: None }),
                        ("y_expr".to_string(), PropertyDefinition { prop_type: "string".to_string(), description: "Y position expression. Default centres frame.".to_string(), items: None }),
                        ("duration_frames".to_string(), PropertyDefinition { prop_type: "number".to_string(), description: "Duration in frames. Default 125.".to_string(), items: None }),
                        ("fps".to_string(), PropertyDefinition { prop_type: "number".to_string(), description: "Output frame rate. Default 25.".to_string(), items: None }),
                    ]),
                    required: vec!["input_file".to_string(), "output_file".to_string()],
                },
            },

            ClaudeTool {
                name: "chromatic_aberration".to_string(),
                description: "Shifts R/B colour channels to simulate chromatic aberration/lens fringing using FFmpeg rgbashift filter.".to_string(),
                input_schema: InputSchema {
                    schema_type: "object".to_string(),
                    properties: HashMap::from([
                        ("input_file".to_string(), PropertyDefinition { prop_type: "string".to_string(), description: "Path to the input video file".to_string(), items: None }),
                        ("output_file".to_string(), PropertyDefinition { prop_type: "string".to_string(), description: "Path to save the output".to_string(), items: None }),
                        ("rh".to_string(), PropertyDefinition { prop_type: "number".to_string(), description: "Red horizontal shift px. Default 5.".to_string(), items: None }),
                        ("rv".to_string(), PropertyDefinition { prop_type: "number".to_string(), description: "Red vertical shift px. Default 0.".to_string(), items: None }),
                        ("bh".to_string(), PropertyDefinition { prop_type: "number".to_string(), description: "Blue horizontal shift px. Default -5.".to_string(), items: None }),
                        ("bv".to_string(), PropertyDefinition { prop_type: "number".to_string(), description: "Blue vertical shift px. Default 0.".to_string(), items: None }),
                    ]),
                    required: vec!["input_file".to_string(), "output_file".to_string()],
                },
            },

            ClaudeTool {
                name: "temporal_blend".to_string(),
                description: "Blends consecutive frames using FFmpeg tblend filter. Creates motion blur, ghosting, and painterly effects.".to_string(),
                input_schema: InputSchema {
                    schema_type: "object".to_string(),
                    properties: HashMap::from([
                        ("input_file".to_string(), PropertyDefinition { prop_type: "string".to_string(), description: "Path to the input video file".to_string(), items: None }),
                        ("output_file".to_string(), PropertyDefinition { prop_type: "string".to_string(), description: "Path to save the blended output".to_string(), items: None }),
                        ("mode".to_string(), PropertyDefinition { prop_type: "string".to_string(), description: "Blend mode: average (default), addition, multiply, screen, overlay, difference.".to_string(), items: None }),
                        ("opacity".to_string(), PropertyDefinition { prop_type: "number".to_string(), description: "Blend opacity (0–1). Default 1.0.".to_string(), items: None }),
                    ]),
                    required: vec!["input_file".to_string(), "output_file".to_string()],
                },
            },

            ClaudeTool {
                name: "motion_interpolate".to_string(),
                description: "Motion-compensated frame interpolation using FFmpeg minterpolate. Creates smooth slow-motion or high-FPS output from lower-FPS footage.".to_string(),
                input_schema: InputSchema {
                    schema_type: "object".to_string(),
                    properties: HashMap::from([
                        ("input_file".to_string(), PropertyDefinition { prop_type: "string".to_string(), description: "Path to the input video file".to_string(), items: None }),
                        ("output_file".to_string(), PropertyDefinition { prop_type: "string".to_string(), description: "Path to save the interpolated output".to_string(), items: None }),
                        ("target_fps".to_string(), PropertyDefinition { prop_type: "number".to_string(), description: "Target output frame rate. Default 60.".to_string(), items: None }),
                        ("mi_mode".to_string(), PropertyDefinition { prop_type: "string".to_string(), description: "Interpolation mode: mci (default), blend, dup.".to_string(), items: None }),
                    ]),
                    required: vec!["input_file".to_string(), "output_file".to_string()],
                },
            },

            ClaudeTool {
                name: "correct_lens_simple".to_string(),
                description: "Corrects barrel or pincushion lens distortion using FFmpeg lenscorrection filter (k1/k2 coefficients only).".to_string(),
                input_schema: InputSchema {
                    schema_type: "object".to_string(),
                    properties: HashMap::from([
                        ("input_file".to_string(), PropertyDefinition { prop_type: "string".to_string(), description: "Path to the input video file".to_string(), items: None }),
                        ("output_file".to_string(), PropertyDefinition { prop_type: "string".to_string(), description: "Path to save the corrected output".to_string(), items: None }),
                        ("k1".to_string(), PropertyDefinition { prop_type: "number".to_string(), description: "Barrel distortion coefficient (negative=barrel, positive=pincushion). Default -0.1.".to_string(), items: None }),
                        ("k2".to_string(), PropertyDefinition { prop_type: "number".to_string(), description: "Secondary distortion coefficient. Default 0.0.".to_string(), items: None }),
                    ]),
                    required: vec!["input_file".to_string(), "output_file".to_string()],
                },
            },

            ClaudeTool {
                name: "deinterlace_yadif".to_string(),
                description: "Removes interlacing from broadcast/capture footage using FFmpeg yadif filter with full mode/parity control.".to_string(),
                input_schema: InputSchema {
                    schema_type: "object".to_string(),
                    properties: HashMap::from([
                        ("input_file".to_string(), PropertyDefinition { prop_type: "string".to_string(), description: "Path to the interlaced input video file".to_string(), items: None }),
                        ("output_file".to_string(), PropertyDefinition { prop_type: "string".to_string(), description: "Path to save the deinterlaced output".to_string(), items: None }),
                        ("mode".to_string(), PropertyDefinition { prop_type: "number".to_string(), description: "Mode: 0=send frame (default), 1=send field, 2=send frame nospatial, 3=send field nospatial.".to_string(), items: None }),
                        ("parity".to_string(), PropertyDefinition { prop_type: "number".to_string(), description: "Parity: -1=auto (default), 0=TFF, 1=BFF.".to_string(), items: None }),
                    ]),
                    required: vec!["input_file".to_string(), "output_file".to_string()],
                },
            },

            ClaudeTool {
                name: "correct_perspective_linear".to_string(),
                description: "Fixes perspective/keystone distortion using FFmpeg perspective filter (linear interpolation). Correct tilted screens, whiteboards, and angled shots.".to_string(),
                input_schema: InputSchema {
                    schema_type: "object".to_string(),
                    properties: HashMap::from([
                        ("input_file".to_string(), PropertyDefinition { prop_type: "string".to_string(), description: "Path to the input video file".to_string(), items: None }),
                        ("output_file".to_string(), PropertyDefinition { prop_type: "string".to_string(), description: "Path to save the corrected output".to_string(), items: None }),
                        ("x0".to_string(), PropertyDefinition { prop_type: "number".to_string(), description: "Top-left X".to_string(), items: None }),
                        ("y0".to_string(), PropertyDefinition { prop_type: "number".to_string(), description: "Top-left Y".to_string(), items: None }),
                        ("x1".to_string(), PropertyDefinition { prop_type: "number".to_string(), description: "Top-right X".to_string(), items: None }),
                        ("y1".to_string(), PropertyDefinition { prop_type: "number".to_string(), description: "Top-right Y".to_string(), items: None }),
                        ("x2".to_string(), PropertyDefinition { prop_type: "number".to_string(), description: "Bottom-left X".to_string(), items: None }),
                        ("y2".to_string(), PropertyDefinition { prop_type: "number".to_string(), description: "Bottom-left Y".to_string(), items: None }),
                        ("x3".to_string(), PropertyDefinition { prop_type: "number".to_string(), description: "Bottom-right X".to_string(), items: None }),
                        ("y3".to_string(), PropertyDefinition { prop_type: "number".to_string(), description: "Bottom-right Y".to_string(), items: None }),
                    ]),
                    required: vec!["input_file".to_string(), "output_file".to_string(), "x0".to_string(), "y0".to_string(), "x1".to_string(), "y1".to_string(), "x2".to_string(), "y2".to_string(), "x3".to_string(), "y3".to_string()],
                },
            },

            ClaudeTool {
                name: "colorize_video".to_string(),
                description: "Colorizes grayscale/desaturated video with a colour tint using FFmpeg colorize filter. Add sepia, cool blue, or any colour cast.".to_string(),
                input_schema: InputSchema {
                    schema_type: "object".to_string(),
                    properties: HashMap::from([
                        ("input_file".to_string(), PropertyDefinition { prop_type: "string".to_string(), description: "Path to the input video file".to_string(), items: None }),
                        ("output_file".to_string(), PropertyDefinition { prop_type: "string".to_string(), description: "Path to save the colorized output".to_string(), items: None }),
                        ("hue".to_string(), PropertyDefinition { prop_type: "number".to_string(), description: "Colour hue in degrees (0–360). Default 210.".to_string(), items: None }),
                        ("saturation".to_string(), PropertyDefinition { prop_type: "number".to_string(), description: "Colour saturation (0–1). Default 0.5.".to_string(), items: None }),
                        ("lightness".to_string(), PropertyDefinition { prop_type: "number".to_string(), description: "Lightness adjustment (-1–1). Default 0.".to_string(), items: None }),
                    ]),
                    required: vec!["input_file".to_string(), "output_file".to_string()],
                },
            },

            ClaudeTool {
                name: "denoise_hqdn3d".to_string(),
                description: "Fast video denoising using FFmpeg hqdn3d (High Quality 3D Denoiser). Great balance of speed vs quality for luma and chroma noise.".to_string(),
                input_schema: InputSchema {
                    schema_type: "object".to_string(),
                    properties: HashMap::from([
                        ("input_file".to_string(), PropertyDefinition { prop_type: "string".to_string(), description: "Path to the input video file".to_string(), items: None }),
                        ("output_file".to_string(), PropertyDefinition { prop_type: "string".to_string(), description: "Path to save the denoised output".to_string(), items: None }),
                        ("luma_spatial".to_string(), PropertyDefinition { prop_type: "number".to_string(), description: "Luma spatial strength. Default 4.0.".to_string(), items: None }),
                        ("chroma_spatial".to_string(), PropertyDefinition { prop_type: "number".to_string(), description: "Chroma spatial strength. Default 3.0.".to_string(), items: None }),
                        ("luma_tmp".to_string(), PropertyDefinition { prop_type: "number".to_string(), description: "Luma temporal strength. Default 6.0.".to_string(), items: None }),
                        ("chroma_tmp".to_string(), PropertyDefinition { prop_type: "number".to_string(), description: "Chroma temporal strength. Default 4.5.".to_string(), items: None }),
                    ]),
                    required: vec!["input_file".to_string(), "output_file".to_string()],
                },
            },

            ClaudeTool {
                name: "add_echo".to_string(),
                description: "Adds echo/delay effect to audio using FFmpeg aecho filter.".to_string(),
                input_schema: InputSchema {
                    schema_type: "object".to_string(),
                    properties: HashMap::from([
                        ("input_file".to_string(), PropertyDefinition { prop_type: "string".to_string(), description: "Path to the input audio/video file".to_string(), items: None }),
                        ("output_file".to_string(), PropertyDefinition { prop_type: "string".to_string(), description: "Path to save the echo output".to_string(), items: None }),
                        ("in_gain".to_string(), PropertyDefinition { prop_type: "number".to_string(), description: "Input gain (0–1). Default 0.6.".to_string(), items: None }),
                        ("out_gain".to_string(), PropertyDefinition { prop_type: "number".to_string(), description: "Output gain (0–1). Default 0.3.".to_string(), items: None }),
                        ("delays".to_string(), PropertyDefinition { prop_type: "string".to_string(), description: "Delay times ms, pipe-separated. Default '1000'.".to_string(), items: None }),
                        ("decays".to_string(), PropertyDefinition { prop_type: "string".to_string(), description: "Decay factors, pipe-separated. Default '0.5'.".to_string(), items: None }),
                    ]),
                    required: vec!["input_file".to_string(), "output_file".to_string()],
                },
            },

            ClaudeTool {
                name: "noise_gate".to_string(),
                description: "Silences audio below a threshold using FFmpeg agate filter. Removes background hiss between speech.".to_string(),
                input_schema: InputSchema {
                    schema_type: "object".to_string(),
                    properties: HashMap::from([
                        ("input_file".to_string(), PropertyDefinition { prop_type: "string".to_string(), description: "Path to the input audio/video file".to_string(), items: None }),
                        ("output_file".to_string(), PropertyDefinition { prop_type: "string".to_string(), description: "Path to save the gated output".to_string(), items: None }),
                        ("threshold".to_string(), PropertyDefinition { prop_type: "number".to_string(), description: "Gate threshold (0–1). Default 0.01.".to_string(), items: None }),
                        ("range".to_string(), PropertyDefinition { prop_type: "number".to_string(), description: "Range factor (0–1). Default 0.06125.".to_string(), items: None }),
                        ("attack".to_string(), PropertyDefinition { prop_type: "number".to_string(), description: "Attack ms. Default 20.".to_string(), items: None }),
                        ("release".to_string(), PropertyDefinition { prop_type: "number".to_string(), description: "Release ms. Default 250.".to_string(), items: None }),
                    ]),
                    required: vec!["input_file".to_string(), "output_file".to_string()],
                },
            },

            ClaudeTool {
                name: "compress_dynamics".to_string(),
                description: "Dynamic range compression using FFmpeg acompressor. Reduces peaks and raises quiet parts for consistent audio levels.".to_string(),
                input_schema: InputSchema {
                    schema_type: "object".to_string(),
                    properties: HashMap::from([
                        ("input_file".to_string(), PropertyDefinition { prop_type: "string".to_string(), description: "Path to the input audio/video file".to_string(), items: None }),
                        ("output_file".to_string(), PropertyDefinition { prop_type: "string".to_string(), description: "Path to save the compressed output".to_string(), items: None }),
                        ("threshold".to_string(), PropertyDefinition { prop_type: "number".to_string(), description: "Threshold (0–1). Default 0.125.".to_string(), items: None }),
                        ("ratio".to_string(), PropertyDefinition { prop_type: "number".to_string(), description: "Compression ratio. Default 4.".to_string(), items: None }),
                        ("attack".to_string(), PropertyDefinition { prop_type: "number".to_string(), description: "Attack ms. Default 20.".to_string(), items: None }),
                        ("release".to_string(), PropertyDefinition { prop_type: "number".to_string(), description: "Release ms. Default 250.".to_string(), items: None }),
                        ("makeup".to_string(), PropertyDefinition { prop_type: "number".to_string(), description: "Makeup gain. Default 1.0.".to_string(), items: None }),
                    ]),
                    required: vec!["input_file".to_string(), "output_file".to_string()],
                },
            },

            ClaudeTool {
                name: "add_chorus".to_string(),
                description: "Chorus effect using FFmpeg chorus filter. Adds shimmering doubled-voice character to vocals and instruments.".to_string(),
                input_schema: InputSchema {
                    schema_type: "object".to_string(),
                    properties: HashMap::from([
                        ("input_file".to_string(), PropertyDefinition { prop_type: "string".to_string(), description: "Path to the input audio/video file".to_string(), items: None }),
                        ("output_file".to_string(), PropertyDefinition { prop_type: "string".to_string(), description: "Path to save the chorus output".to_string(), items: None }),
                        ("in_gain".to_string(), PropertyDefinition { prop_type: "number".to_string(), description: "Input gain. Default 0.4.".to_string(), items: None }),
                        ("out_gain".to_string(), PropertyDefinition { prop_type: "number".to_string(), description: "Output gain. Default 0.4.".to_string(), items: None }),
                        ("delays".to_string(), PropertyDefinition { prop_type: "string".to_string(), description: "Delay ms. Default '55'.".to_string(), items: None }),
                        ("decays".to_string(), PropertyDefinition { prop_type: "string".to_string(), description: "Decay. Default '0.4'.".to_string(), items: None }),
                        ("speeds".to_string(), PropertyDefinition { prop_type: "string".to_string(), description: "Mod speed Hz. Default '0.25'.".to_string(), items: None }),
                        ("depths".to_string(), PropertyDefinition { prop_type: "string".to_string(), description: "Mod depth. Default '2'.".to_string(), items: None }),
                    ]),
                    required: vec!["input_file".to_string(), "output_file".to_string()],
                },
            },

            ClaudeTool {
                name: "widen_stereo".to_string(),
                description: "Widens stereo field using FFmpeg stereowiden filter for a broader soundstage.".to_string(),
                input_schema: InputSchema {
                    schema_type: "object".to_string(),
                    properties: HashMap::from([
                        ("input_file".to_string(), PropertyDefinition { prop_type: "string".to_string(), description: "Path to the input audio/video file".to_string(), items: None }),
                        ("output_file".to_string(), PropertyDefinition { prop_type: "string".to_string(), description: "Path to save the widened output".to_string(), items: None }),
                        ("delay".to_string(), PropertyDefinition { prop_type: "number".to_string(), description: "Delay ms (0–90). Default 20.".to_string(), items: None }),
                        ("feedback".to_string(), PropertyDefinition { prop_type: "number".to_string(), description: "Feedback (0–1). Default 0.".to_string(), items: None }),
                        ("crossfeed".to_string(), PropertyDefinition { prop_type: "number".to_string(), description: "Crossfeed (0–1). Default 0.".to_string(), items: None }),
                        ("drymix".to_string(), PropertyDefinition { prop_type: "number".to_string(), description: "Dry mix (0–1). Default 0.8.".to_string(), items: None }),
                    ]),
                    required: vec!["input_file".to_string(), "output_file".to_string()],
                },
            },

            ClaudeTool {
                name: "normalize_speech".to_string(),
                description: "Normalises speech volume using FFmpeg speechnorm filter. Evens out quiet and loud passages in spoken audio.".to_string(),
                input_schema: InputSchema {
                    schema_type: "object".to_string(),
                    properties: HashMap::from([
                        ("input_file".to_string(), PropertyDefinition { prop_type: "string".to_string(), description: "Path to the input audio/video file".to_string(), items: None }),
                        ("output_file".to_string(), PropertyDefinition { prop_type: "string".to_string(), description: "Path to save the normalised output".to_string(), items: None }),
                        ("peak".to_string(), PropertyDefinition { prop_type: "number".to_string(), description: "Peak target (0–1). Default 0.95.".to_string(), items: None }),
                        ("strength".to_string(), PropertyDefinition { prop_type: "number".to_string(), description: "Normalisation strength (0–1). Default 0.8.".to_string(), items: None }),
                    ]),
                    required: vec!["input_file".to_string(), "output_file".to_string()],
                },
            },

            ClaudeTool {
                name: "remove_silence_simple".to_string(),
                description: "Removes silent segments from audio using FFmpeg silenceremove. Tightens podcasts, interviews, and lectures.".to_string(),
                input_schema: InputSchema {
                    schema_type: "object".to_string(),
                    properties: HashMap::from([
                        ("input_file".to_string(), PropertyDefinition { prop_type: "string".to_string(), description: "Path to the input audio/video file".to_string(), items: None }),
                        ("output_file".to_string(), PropertyDefinition { prop_type: "string".to_string(), description: "Path to save the silence-removed output".to_string(), items: None }),
                        ("threshold".to_string(), PropertyDefinition { prop_type: "number".to_string(), description: "Silence threshold (0–1). Default 0.02.".to_string(), items: None }),
                        ("duration".to_string(), PropertyDefinition { prop_type: "number".to_string(), description: "Min silence duration s. Default 0.5.".to_string(), items: None }),
                        ("periods".to_string(), PropertyDefinition { prop_type: "number".to_string(), description: "Start periods. Default 1.".to_string(), items: None }),
                    ]),
                    required: vec!["input_file".to_string(), "output_file".to_string()],
                },
            },

            ClaudeTool {
                name: "soft_clip_audio".to_string(),
                description: "Soft audio clipping using FFmpeg asoftclip. Prevents harsh digital distortion by saturating peaks gently.".to_string(),
                input_schema: InputSchema {
                    schema_type: "object".to_string(),
                    properties: HashMap::from([
                        ("input_file".to_string(), PropertyDefinition { prop_type: "string".to_string(), description: "Path to the input audio/video file".to_string(), items: None }),
                        ("output_file".to_string(), PropertyDefinition { prop_type: "string".to_string(), description: "Path to save the output".to_string(), items: None }),
                        ("clip_type".to_string(), PropertyDefinition { prop_type: "string".to_string(), description: "Clip curve: tanh (default), atan, cubic, exp, alg, quintic, sin, erf.".to_string(), items: None }),
                        ("param".to_string(), PropertyDefinition { prop_type: "number".to_string(), description: "Clipping parameter. Default 1.0.".to_string(), items: None }),
                    ]),
                    required: vec!["input_file".to_string(), "output_file".to_string()],
                },
            },

            ClaudeTool {
                name: "segment_video".to_string(),
                description: "Splits video into fixed-duration segments using FFmpeg segment muxer. Creates streaming chunks, chapter files, or upload-ready pieces.".to_string(),
                input_schema: InputSchema {
                    schema_type: "object".to_string(),
                    properties: HashMap::from([
                        ("input_file".to_string(), PropertyDefinition { prop_type: "string".to_string(), description: "Path to the input video file".to_string(), items: None }),
                        ("output_pattern".to_string(), PropertyDefinition { prop_type: "string".to_string(), description: "Output pattern with %03d, e.g. 'segment_%03d.mp4'".to_string(), items: None }),
                        ("segment_time".to_string(), PropertyDefinition { prop_type: "number".to_string(), description: "Duration per segment in seconds. Default 60.".to_string(), items: None }),
                        ("reset_timestamps".to_string(), PropertyDefinition { prop_type: "string".to_string(), description: "Reset timestamps per segment: true or false. Default true.".to_string(), items: None }),
                    ]),
                    required: vec!["input_file".to_string(), "output_pattern".to_string()],
                },
            },

            ClaudeTool {
                name: "pad_video_time".to_string(),
                description: "Adds padding frames (black or coloured) at the start/end of a video using FFmpeg tpad filter.".to_string(),
                input_schema: InputSchema {
                    schema_type: "object".to_string(),
                    properties: HashMap::from([
                        ("input_file".to_string(), PropertyDefinition { prop_type: "string".to_string(), description: "Path to the input video file".to_string(), items: None }),
                        ("output_file".to_string(), PropertyDefinition { prop_type: "string".to_string(), description: "Path to save the padded output".to_string(), items: None }),
                        ("start_duration".to_string(), PropertyDefinition { prop_type: "number".to_string(), description: "Seconds of padding before video. Default 0.".to_string(), items: None }),
                        ("stop_duration".to_string(), PropertyDefinition { prop_type: "number".to_string(), description: "Seconds of padding after video. Default 0.".to_string(), items: None }),
                        ("color".to_string(), PropertyDefinition { prop_type: "string".to_string(), description: "Pad colour: 'black' (default), 'white', '#ff0000', etc.".to_string(), items: None }),
                    ]),
                    required: vec!["input_file".to_string(), "output_file".to_string()],
                },
            },
            // ================================================================
            // PHASE I BATCH 10 — stabilize_video_2pass, lut_rgb, hsvhold, convert_pixel_format,
            //                    setsar, random_frames, visualize_cqt, visualize_frequencies,
            //                    audio_iir, audio_expression, convert_audio_format,
            //                    cross_correlate, audio_multiply, audio_contrast, decode_hdcd
            // ================================================================
            ClaudeTool {
                name: "stabilize_video_2pass".to_string(),
                description: "Two-pass video stabilization using vidstabdetect + vidstabtransform. Pass 1 analyzes shake, pass 2 applies correction. More accurate than single-pass deshake.".to_string(),
                input_schema: InputSchema {
                    schema_type: "object".to_string(),
                    properties: HashMap::from([
                        ("input_file".to_string(), PropertyDefinition { prop_type: "string".to_string(), description: "Path to input video file".to_string(), items: None }),
                        ("output_file".to_string(), PropertyDefinition { prop_type: "string".to_string(), description: "Path to output stabilized video file".to_string(), items: None }),
                        ("shakiness".to_string(), PropertyDefinition { prop_type: "number".to_string(), description: "Shakiness level 1-10 (default 5)".to_string(), items: None }),
                        ("accuracy".to_string(), PropertyDefinition { prop_type: "number".to_string(), description: "Detection accuracy 1-15 (default 15)".to_string(), items: None }),
                        ("smoothing".to_string(), PropertyDefinition { prop_type: "number".to_string(), description: "Smoothing frames (default 10)".to_string(), items: None }),
                        ("zoom".to_string(), PropertyDefinition { prop_type: "number".to_string(), description: "Additional zoom percent, 0=auto (default 0)".to_string(), items: None }),
                    ]),
                    required: vec!["input_file".to_string(), "output_file".to_string()],
                },
            },
            ClaudeTool {
                name: "apply_lut_rgb".to_string(),
                description: "Apply per-pixel RGB expression LUT using lutrgb filter. Create custom color transformations using math expressions on R, G, B channels (val=current value, maxval=255, etc.)".to_string(),
                input_schema: InputSchema {
                    schema_type: "object".to_string(),
                    properties: HashMap::from([
                        ("input_file".to_string(), PropertyDefinition { prop_type: "string".to_string(), description: "Path to input video file".to_string(), items: None }),
                        ("output_file".to_string(), PropertyDefinition { prop_type: "string".to_string(), description: "Path to output video file".to_string(), items: None }),
                        ("r_expr".to_string(), PropertyDefinition { prop_type: "string".to_string(), description: "Expression for red channel (default: val). E.g. 'val*1.2', 'maxval-val' for invert".to_string(), items: None }),
                        ("g_expr".to_string(), PropertyDefinition { prop_type: "string".to_string(), description: "Expression for green channel (default: val)".to_string(), items: None }),
                        ("b_expr".to_string(), PropertyDefinition { prop_type: "string".to_string(), description: "Expression for blue channel (default: val)".to_string(), items: None }),
                    ]),
                    required: vec!["input_file".to_string(), "output_file".to_string()],
                },
            },
            ClaudeTool {
                name: "apply_hsvhold".to_string(),
                description: "Selective color hold using HSV: keep only pixels near a specific hue, turn all others to greyscale. Great for highlight-one-color cinematic effect.".to_string(),
                input_schema: InputSchema {
                    schema_type: "object".to_string(),
                    properties: HashMap::from([
                        ("input_file".to_string(), PropertyDefinition { prop_type: "string".to_string(), description: "Path to input video file".to_string(), items: None }),
                        ("output_file".to_string(), PropertyDefinition { prop_type: "string".to_string(), description: "Path to output video file".to_string(), items: None }),
                        ("hue".to_string(), PropertyDefinition { prop_type: "number".to_string(), description: "Target hue to keep, 0-360 degrees (default 0=red)".to_string(), items: None }),
                        ("white".to_string(), PropertyDefinition { prop_type: "number".to_string(), description: "White threshold 0-1 (default 0.01 — protect near-white pixels)".to_string(), items: None }),
                        ("black".to_string(), PropertyDefinition { prop_type: "number".to_string(), description: "Black threshold 0-1 (default 0.01 — protect near-black pixels)".to_string(), items: None }),
                        ("similarity".to_string(), PropertyDefinition { prop_type: "number".to_string(), description: "Hue similarity radius 0-1 (default 0.01 — narrow; increase for wider range)".to_string(), items: None }),
                        ("blend".to_string(), PropertyDefinition { prop_type: "number".to_string(), description: "Blend factor 0-1 for soft edge (default 0=hard edge)".to_string(), items: None }),
                    ]),
                    required: vec!["input_file".to_string(), "output_file".to_string()],
                },
            },
            ClaudeTool {
                name: "convert_pixel_format".to_string(),
                description: "Convert video pixel format using FFmpeg format filter. Useful for compatibility, HDR→SDR, or ensuring specific codec requirements (e.g. yuv420p for web).".to_string(),
                input_schema: InputSchema {
                    schema_type: "object".to_string(),
                    properties: HashMap::from([
                        ("input_file".to_string(), PropertyDefinition { prop_type: "string".to_string(), description: "Path to input video file".to_string(), items: None }),
                        ("output_file".to_string(), PropertyDefinition { prop_type: "string".to_string(), description: "Path to output video file".to_string(), items: None }),
                        ("pix_fmt".to_string(), PropertyDefinition { prop_type: "string".to_string(), description: "Target pixel format (default: yuv420p). Common: yuv444p, nv12, rgb24, gbrp, p010le (HDR)".to_string(), items: None }),
                    ]),
                    required: vec!["input_file".to_string(), "output_file".to_string()],
                },
            },
            ClaudeTool {
                name: "apply_setsar".to_string(),
                description: "Set sample aspect ratio (SAR/PAR) of video without re-encoding pixels. Used to fix anamorphic footage or set correct display ratio for broadcast formats.".to_string(),
                input_schema: InputSchema {
                    schema_type: "object".to_string(),
                    properties: HashMap::from([
                        ("input_file".to_string(), PropertyDefinition { prop_type: "string".to_string(), description: "Path to input video file".to_string(), items: None }),
                        ("output_file".to_string(), PropertyDefinition { prop_type: "string".to_string(), description: "Path to output video file".to_string(), items: None }),
                        ("sar".to_string(), PropertyDefinition { prop_type: "string".to_string(), description: "Sample aspect ratio as fraction string (default: 1/1 = square pixels). E.g. '16/15' for NTSC".to_string(), items: None }),
                    ]),
                    required: vec!["input_file".to_string(), "output_file".to_string()],
                },
            },
            ClaudeTool {
                name: "apply_random_frames".to_string(),
                description: "Randomly reorder frames using FFmpeg random filter. Creates a glitch/scrambled visual effect by selecting frames from a rolling window in random order.".to_string(),
                input_schema: InputSchema {
                    schema_type: "object".to_string(),
                    properties: HashMap::from([
                        ("input_file".to_string(), PropertyDefinition { prop_type: "string".to_string(), description: "Path to input video file".to_string(), items: None }),
                        ("output_file".to_string(), PropertyDefinition { prop_type: "string".to_string(), description: "Path to output video file".to_string(), items: None }),
                        ("frames".to_string(), PropertyDefinition { prop_type: "number".to_string(), description: "Size of random window in frames (default 30)".to_string(), items: None }),
                        ("seed".to_string(), PropertyDefinition { prop_type: "number".to_string(), description: "Random seed (-1 for random, default -1)".to_string(), items: None }),
                    ]),
                    required: vec!["input_file".to_string(), "output_file".to_string()],
                },
            },
            ClaudeTool {
                name: "visualize_cqt".to_string(),
                description: "Render Constant-Q Transform (CQT) spectrum visualization video from audio. Creates a musical frequency visualization with piano-roll-like frequency axis.".to_string(),
                input_schema: InputSchema {
                    schema_type: "object".to_string(),
                    properties: HashMap::from([
                        ("input_file".to_string(), PropertyDefinition { prop_type: "string".to_string(), description: "Path to input audio or video file".to_string(), items: None }),
                        ("output_file".to_string(), PropertyDefinition { prop_type: "string".to_string(), description: "Path to output video visualization file".to_string(), items: None }),
                        ("width".to_string(), PropertyDefinition { prop_type: "number".to_string(), description: "Output video width (default 1920)".to_string(), items: None }),
                        ("height".to_string(), PropertyDefinition { prop_type: "number".to_string(), description: "Output video height (default 1080)".to_string(), items: None }),
                        ("bar_h".to_string(), PropertyDefinition { prop_type: "number".to_string(), description: "Bar area height in pixels (default 20)".to_string(), items: None }),
                        ("axis_h".to_string(), PropertyDefinition { prop_type: "number".to_string(), description: "Axis area height in pixels (default 30)".to_string(), items: None }),
                    ]),
                    required: vec!["input_file".to_string(), "output_file".to_string()],
                },
            },
            ClaudeTool {
                name: "visualize_frequencies".to_string(),
                description: "Render frequency spectrum visualization video from audio using showfreqs. Displays frequency content over time as a line or bar chart with configurable scale.".to_string(),
                input_schema: InputSchema {
                    schema_type: "object".to_string(),
                    properties: HashMap::from([
                        ("input_file".to_string(), PropertyDefinition { prop_type: "string".to_string(), description: "Path to input audio or video file".to_string(), items: None }),
                        ("output_file".to_string(), PropertyDefinition { prop_type: "string".to_string(), description: "Path to output video visualization file".to_string(), items: None }),
                        ("width".to_string(), PropertyDefinition { prop_type: "number".to_string(), description: "Output width (default 1024)".to_string(), items: None }),
                        ("height".to_string(), PropertyDefinition { prop_type: "number".to_string(), description: "Output height (default 512)".to_string(), items: None }),
                        ("mode".to_string(), PropertyDefinition { prop_type: "string".to_string(), description: "Display mode: line, bar, dot (default line)".to_string(), items: None }),
                        ("ascale".to_string(), PropertyDefinition { prop_type: "string".to_string(), description: "Amplitude scale: lin, sqrt, cbrt, log (default log)".to_string(), items: None }),
                    ]),
                    required: vec!["input_file".to_string(), "output_file".to_string()],
                },
            },
            ClaudeTool {
                name: "apply_audio_iir".to_string(),
                description: "Apply custom IIR (Infinite Impulse Response) filter to audio using aiir. Allows designing arbitrary digital filters by specifying zeros, poles, and gains.".to_string(),
                input_schema: InputSchema {
                    schema_type: "object".to_string(),
                    properties: HashMap::from([
                        ("input_file".to_string(), PropertyDefinition { prop_type: "string".to_string(), description: "Path to input audio or video file".to_string(), items: None }),
                        ("output_file".to_string(), PropertyDefinition { prop_type: "string".to_string(), description: "Path to output file".to_string(), items: None }),
                        ("zeros".to_string(), PropertyDefinition { prop_type: "string".to_string(), description: "IIR filter zeros coefficients (default 1)".to_string(), items: None }),
                        ("poles".to_string(), PropertyDefinition { prop_type: "string".to_string(), description: "IIR filter poles coefficients (default 1)".to_string(), items: None }),
                        ("gains".to_string(), PropertyDefinition { prop_type: "string".to_string(), description: "IIR filter gain coefficients (default 1)".to_string(), items: None }),
                    ]),
                    required: vec!["input_file".to_string(), "output_file".to_string()],
                },
            },
            ClaudeTool {
                name: "apply_audio_expression".to_string(),
                description: "Apply per-sample audio expression using aeval. Transform audio samples with math expressions. Can create ring modulation, distortion, bit manipulation, and other creative effects.".to_string(),
                input_schema: InputSchema {
                    schema_type: "object".to_string(),
                    properties: HashMap::from([
                        ("input_file".to_string(), PropertyDefinition { prop_type: "string".to_string(), description: "Path to input audio or video file".to_string(), items: None }),
                        ("output_file".to_string(), PropertyDefinition { prop_type: "string".to_string(), description: "Path to output file".to_string(), items: None }),
                        ("exprs".to_string(), PropertyDefinition { prop_type: "string".to_string(), description: "Per-sample expression(s). E.g. 'val*0.5' for -6dB, 'val*sin(2*PI*440*t)' for ring mod".to_string(), items: None }),
                    ]),
                    required: vec!["input_file".to_string(), "output_file".to_string()],
                },
            },
            ClaudeTool {
                name: "convert_audio_format".to_string(),
                description: "Force specific audio sample format, sample rate, and channel layout using aformat. Useful for codec compatibility and ensuring downstream tools receive expected audio format.".to_string(),
                input_schema: InputSchema {
                    schema_type: "object".to_string(),
                    properties: HashMap::from([
                        ("input_file".to_string(), PropertyDefinition { prop_type: "string".to_string(), description: "Path to input audio or video file".to_string(), items: None }),
                        ("output_file".to_string(), PropertyDefinition { prop_type: "string".to_string(), description: "Path to output file".to_string(), items: None }),
                        ("sample_fmts".to_string(), PropertyDefinition { prop_type: "string".to_string(), description: "Target sample format (e.g. s16, s32, fltp, flt). Leave empty to keep current".to_string(), items: None }),
                        ("sample_rates".to_string(), PropertyDefinition { prop_type: "string".to_string(), description: "Target sample rate in Hz (e.g. 44100, 48000). Leave empty to keep current".to_string(), items: None }),
                        ("channel_layouts".to_string(), PropertyDefinition { prop_type: "string".to_string(), description: "Target channel layout (e.g. stereo, mono, 5.1). Leave empty to keep current".to_string(), items: None }),
                    ]),
                    required: vec!["input_file".to_string(), "output_file".to_string()],
                },
            },
            ClaudeTool {
                name: "apply_cross_correlate".to_string(),
                description: "Cross-correlate two audio streams using axcorrelate. Useful for measuring similarity between audio signals, finding time alignment, or mixing correlated audio.".to_string(),
                input_schema: InputSchema {
                    schema_type: "object".to_string(),
                    properties: HashMap::from([
                        ("input_file".to_string(), PropertyDefinition { prop_type: "string".to_string(), description: "Path to first input audio file".to_string(), items: None }),
                        ("secondary_file".to_string(), PropertyDefinition { prop_type: "string".to_string(), description: "Path to second input audio file".to_string(), items: None }),
                        ("output_file".to_string(), PropertyDefinition { prop_type: "string".to_string(), description: "Path to output file".to_string(), items: None }),
                        ("size".to_string(), PropertyDefinition { prop_type: "number".to_string(), description: "Segment size in samples (default 256)".to_string(), items: None }),
                        ("algo".to_string(), PropertyDefinition { prop_type: "string".to_string(), description: "Algorithm: slow or fast (default fast)".to_string(), items: None }),
                    ]),
                    required: vec!["input_file".to_string(), "secondary_file".to_string(), "output_file".to_string()],
                },
            },
            ClaudeTool {
                name: "apply_audio_multiply".to_string(),
                description: "Ring modulation — multiply two audio streams sample by sample using amultiply. Creates metallic, robotic, bell-like timbres typical of ring modulators.".to_string(),
                input_schema: InputSchema {
                    schema_type: "object".to_string(),
                    properties: HashMap::from([
                        ("input_file".to_string(), PropertyDefinition { prop_type: "string".to_string(), description: "Path to first input audio file (carrier)".to_string(), items: None }),
                        ("secondary_file".to_string(), PropertyDefinition { prop_type: "string".to_string(), description: "Path to second input audio file (modulator)".to_string(), items: None }),
                        ("output_file".to_string(), PropertyDefinition { prop_type: "string".to_string(), description: "Path to output file".to_string(), items: None }),
                    ]),
                    required: vec!["input_file".to_string(), "secondary_file".to_string(), "output_file".to_string()],
                },
            },
            ClaudeTool {
                name: "apply_audio_contrast".to_string(),
                description: "Enhance audio contrast using acontrast filter. Increases perceived loudness and punch without clipping, similar to soft saturation or tape-style harmonic enhancement.".to_string(),
                input_schema: InputSchema {
                    schema_type: "object".to_string(),
                    properties: HashMap::from([
                        ("input_file".to_string(), PropertyDefinition { prop_type: "string".to_string(), description: "Path to input audio or video file".to_string(), items: None }),
                        ("output_file".to_string(), PropertyDefinition { prop_type: "string".to_string(), description: "Path to output file".to_string(), items: None }),
                        ("contrast".to_string(), PropertyDefinition { prop_type: "number".to_string(), description: "Contrast level 0-100 (default 33)".to_string(), items: None }),
                    ]),
                    required: vec!["input_file".to_string(), "output_file".to_string()],
                },
            },
            ClaudeTool {
                name: "decode_hdcd".to_string(),
                description: "Decode HDCD (High Definition Compatible Digital) encoded audio. HDCD is a lossless encoding that extends dynamic range on standard CDs. Required for proper playback of HDCD masters.".to_string(),
                input_schema: InputSchema {
                    schema_type: "object".to_string(),
                    properties: HashMap::from([
                        ("input_file".to_string(), PropertyDefinition { prop_type: "string".to_string(), description: "Path to input HDCD audio file".to_string(), items: None }),
                        ("output_file".to_string(), PropertyDefinition { prop_type: "string".to_string(), description: "Path to output decoded file".to_string(), items: None }),
                        ("disable_autoconvert".to_string(), PropertyDefinition { prop_type: "boolean".to_string(), description: "Disable automatic format conversion (default false)".to_string(), items: None }),
                        ("process_stereo".to_string(), PropertyDefinition { prop_type: "boolean".to_string(), description: "Process both channels together as stereo pair (default false)".to_string(), items: None }),
                        ("force_pe".to_string(), PropertyDefinition { prop_type: "boolean".to_string(), description: "Force peak extend processing (default false)".to_string(), items: None }),
                    ]),
                    required: vec!["input_file".to_string(), "output_file".to_string()],
                },
            },
            // ================================================================
            // PHASE I BATCH 9 — scale_to_reference, fieldorder, optimize_gif_palette,
            //                   hsv_key, lut_yuv, freezeframes, draw_signal_graph, video_entropy,
            //                   compensation_delay, earwax, allpass, highshelf, lowshelf,
            //                   surround_upmix, detect_volume_levels
            // ================================================================

            ClaudeTool {
                name: "scale_to_reference".to_string(),
                description: "Scales a video to exactly match the dimensions of a reference video using FFmpeg scale2ref. Useful when compositing or comparing two clips that need to be the same size — avoids manual measurement of the reference resolution.".to_string(),
                input_schema: InputSchema {
                    schema_type: "object".to_string(),
                    properties: HashMap::from([
                        ("input_file".to_string(), PropertyDefinition { prop_type: "string".to_string(), description: "Path to the video to scale".to_string(), items: None }),
                        ("ref_file".to_string(), PropertyDefinition { prop_type: "string".to_string(), description: "Path to the reference video whose dimensions will be matched".to_string(), items: None }),
                        ("output_file".to_string(), PropertyDefinition { prop_type: "string".to_string(), description: "Path for output scaled video".to_string(), items: None }),
                        ("flags".to_string(), PropertyDefinition { prop_type: "string".to_string(), description: "Scaling algorithm (default: bilinear). Options: bilinear, bicubic, lanczos, area, neighbor".to_string(), items: None }),
                    ]),
                    required: vec!["input_file".to_string(), "ref_file".to_string(), "output_file".to_string()],
                },
            },

            ClaudeTool {
                name: "apply_fieldorder".to_string(),
                description: "Changes the field order of interlaced video using FFmpeg fieldorder. Converts between top-field-first (TFF) and bottom-field-first (BFF). Use when footage has the wrong field order causing combing artifacts on interlaced displays.".to_string(),
                input_schema: InputSchema {
                    schema_type: "object".to_string(),
                    properties: HashMap::from([
                        ("input_file".to_string(), PropertyDefinition { prop_type: "string".to_string(), description: "Path to input interlaced video".to_string(), items: None }),
                        ("output_file".to_string(), PropertyDefinition { prop_type: "string".to_string(), description: "Path for output video".to_string(), items: None }),
                        ("order".to_string(), PropertyDefinition { prop_type: "string".to_string(), description: "Target field order: tff (top field first, default) or bff (bottom field first)".to_string(), items: None }),
                    ]),
                    required: vec!["input_file".to_string(), "output_file".to_string()],
                },
            },

            ClaudeTool {
                name: "optimize_gif_palette".to_string(),
                description: "Creates a high-quality optimised GIF using FFmpeg two-pass palettegen+paletteuse pipeline. Far better quality than simple GIF conversion — the palette is built from the actual content and dithering is applied. Use for creating shareable animated GIFs.".to_string(),
                input_schema: InputSchema {
                    schema_type: "object".to_string(),
                    properties: HashMap::from([
                        ("input_file".to_string(), PropertyDefinition { prop_type: "string".to_string(), description: "Path to input video file".to_string(), items: None }),
                        ("output_file".to_string(), PropertyDefinition { prop_type: "string".to_string(), description: "Path for output .gif file".to_string(), items: None }),
                        ("width".to_string(), PropertyDefinition { prop_type: "number".to_string(), description: "Output width in pixels (default: 320). Height is auto-calculated to preserve aspect ratio".to_string(), items: None }),
                        ("fps".to_string(), PropertyDefinition { prop_type: "number".to_string(), description: "Output frame rate (default: 10). Higher = smoother but larger file".to_string(), items: None }),
                        ("stats_mode".to_string(), PropertyDefinition { prop_type: "string".to_string(), description: "Palette generation mode: diff (default, optimises for frame differences), full (whole frame), single (first frame only)".to_string(), items: None }),
                    ]),
                    required: vec!["input_file".to_string(), "output_file".to_string()],
                },
            },

            ClaudeTool {
                name: "apply_hsv_key".to_string(),
                description: "Removes pixels from video based on HSV colour space using FFmpeg hsvkey. More precise than standard chroma key for complex or desaturated backgrounds — lets you key by hue, saturation, and brightness independently.".to_string(),
                input_schema: InputSchema {
                    schema_type: "object".to_string(),
                    properties: HashMap::from([
                        ("input_file".to_string(), PropertyDefinition { prop_type: "string".to_string(), description: "Path to input video file".to_string(), items: None }),
                        ("output_file".to_string(), PropertyDefinition { prop_type: "string".to_string(), description: "Path for output video file".to_string(), items: None }),
                        ("hue".to_string(), PropertyDefinition { prop_type: "number".to_string(), description: "Target hue 0–360 degrees (default: 0 = red)".to_string(), items: None }),
                        ("saturation".to_string(), PropertyDefinition { prop_type: "number".to_string(), description: "Target saturation 0–1 (default: 0)".to_string(), items: None }),
                        ("value".to_string(), PropertyDefinition { prop_type: "number".to_string(), description: "Target value/brightness 0–1 (default: 0)".to_string(), items: None }),
                        ("similarity".to_string(), PropertyDefinition { prop_type: "number".to_string(), description: "HSV distance tolerance 0–1 (default: 0.1). Higher = more pixels keyed".to_string(), items: None }),
                        ("blend".to_string(), PropertyDefinition { prop_type: "number".to_string(), description: "Blend factor 0–1 (default: 0.0). Higher = softer edge transition".to_string(), items: None }),
                    ]),
                    required: vec!["input_file".to_string(), "output_file".to_string()],
                },
            },

            ClaudeTool {
                name: "apply_lut_yuv".to_string(),
                description: "Applies per-pixel colour transformations in YUV space using FFmpeg lutyuv. Allows custom mathematical expressions for each channel (Y=luma, U/V=chroma). Use for creative colour effects: invert luma, zero out chroma, remap tones with expressions like 'negval', 'maxval-val', 'val*2'.".to_string(),
                input_schema: InputSchema {
                    schema_type: "object".to_string(),
                    properties: HashMap::from([
                        ("input_file".to_string(), PropertyDefinition { prop_type: "string".to_string(), description: "Path to input video file".to_string(), items: None }),
                        ("output_file".to_string(), PropertyDefinition { prop_type: "string".to_string(), description: "Path for output video file".to_string(), items: None }),
                        ("y_expr".to_string(), PropertyDefinition { prop_type: "string".to_string(), description: "Expression for Y (luma) channel (default: val = passthrough). e.g. 'negval' to invert, 'val/2' to halve".to_string(), items: None }),
                        ("u_expr".to_string(), PropertyDefinition { prop_type: "string".to_string(), description: "Expression for U (Cb) channel (default: val). e.g. '128' to zero out blue-yellow chroma".to_string(), items: None }),
                        ("v_expr".to_string(), PropertyDefinition { prop_type: "string".to_string(), description: "Expression for V (Cr) channel (default: val). e.g. '128' to zero out red-green chroma".to_string(), items: None }),
                    ]),
                    required: vec!["input_file".to_string(), "output_file".to_string()],
                },
            },

            ClaudeTool {
                name: "apply_freezeframes".to_string(),
                description: "Replaces a range of video frames with a freeze frame (a copy of a specified source frame) using FFmpeg freezeframes. Use to freeze-frame a moment, create a pause effect, or replace damaged frames with a clean reference frame.".to_string(),
                input_schema: InputSchema {
                    schema_type: "object".to_string(),
                    properties: HashMap::from([
                        ("input_file".to_string(), PropertyDefinition { prop_type: "string".to_string(), description: "Path to input video file".to_string(), items: None }),
                        ("output_file".to_string(), PropertyDefinition { prop_type: "string".to_string(), description: "Path for output video file".to_string(), items: None }),
                        ("first".to_string(), PropertyDefinition { prop_type: "number".to_string(), description: "First frame number to replace (0-indexed)".to_string(), items: None }),
                        ("last".to_string(), PropertyDefinition { prop_type: "number".to_string(), description: "Last frame number to replace (inclusive)".to_string(), items: None }),
                        ("replace".to_string(), PropertyDefinition { prop_type: "number".to_string(), description: "Frame number to use as the freeze source (default: 0 = first frame)".to_string(), items: None }),
                    ]),
                    required: vec!["input_file".to_string(), "output_file".to_string(), "first".to_string(), "last".to_string()],
                },
            },

            ClaudeTool {
                name: "draw_signal_graph".to_string(),
                description: "Draws a scrolling graph of a video signal statistic over time using FFmpeg signalstats+drawgraph. Visualises metrics like YAVG (luma average), YMAX, YMIN, UAVG, VAVG, SATAVG over the full video duration. Useful for monitoring exposure, colour balance trends, or quality control.".to_string(),
                input_schema: InputSchema {
                    schema_type: "object".to_string(),
                    properties: HashMap::from([
                        ("input_file".to_string(), PropertyDefinition { prop_type: "string".to_string(), description: "Path to input video file".to_string(), items: None }),
                        ("output_file".to_string(), PropertyDefinition { prop_type: "string".to_string(), description: "Path for output video file with graph overlay".to_string(), items: None }),
                        ("signal".to_string(), PropertyDefinition { prop_type: "string".to_string(), description: "Signal to graph (default: YAVG). Options: YAVG, YMAX, YMIN, YDIF, UAVG, VAVG, SATAVG, HUEMED, HUEAVG".to_string(), items: None }),
                        ("width".to_string(), PropertyDefinition { prop_type: "number".to_string(), description: "Graph width in pixels (default: 1280)".to_string(), items: None }),
                        ("height".to_string(), PropertyDefinition { prop_type: "number".to_string(), description: "Graph height in pixels (default: 256)".to_string(), items: None }),
                    ]),
                    required: vec!["input_file".to_string(), "output_file".to_string()],
                },
            },

            ClaudeTool {
                name: "measure_video_entropy".to_string(),
                description: "Measures frame-level entropy of a video using FFmpeg entropy filter. Higher entropy = more visual complexity/detail; lower = flatter/compressed. Reports normalised entropy values per channel. Analysis only — no output file.".to_string(),
                input_schema: InputSchema {
                    schema_type: "object".to_string(),
                    properties: HashMap::from([
                        ("input_file".to_string(), PropertyDefinition { prop_type: "string".to_string(), description: "Path to input video file".to_string(), items: None }),
                    ]),
                    required: vec!["input_file".to_string()],
                },
            },

            ClaudeTool {
                name: "apply_compensation_delay".to_string(),
                description: "Applies a precise time-alignment delay to audio using FFmpeg compensationdelay. Specified in physical distance (mm, cm, m) at a given temperature — the filter converts to exact sample delay based on the speed of sound. Essential for aligning multiple microphones placed at different distances.".to_string(),
                input_schema: InputSchema {
                    schema_type: "object".to_string(),
                    properties: HashMap::from([
                        ("input_file".to_string(), PropertyDefinition { prop_type: "string".to_string(), description: "Path to input audio/video file".to_string(), items: None }),
                        ("output_file".to_string(), PropertyDefinition { prop_type: "string".to_string(), description: "Path for output file".to_string(), items: None }),
                        ("mm".to_string(), PropertyDefinition { prop_type: "number".to_string(), description: "Delay distance in millimetres (default: 0)".to_string(), items: None }),
                        ("cm".to_string(), PropertyDefinition { prop_type: "number".to_string(), description: "Delay distance in centimetres (default: 0)".to_string(), items: None }),
                        ("m".to_string(), PropertyDefinition { prop_type: "number".to_string(), description: "Delay distance in metres (default: 0)".to_string(), items: None }),
                        ("dry".to_string(), PropertyDefinition { prop_type: "number".to_string(), description: "Dry (undelayed) signal mix 0–1 (default: 0)".to_string(), items: None }),
                        ("wet".to_string(), PropertyDefinition { prop_type: "number".to_string(), description: "Wet (delayed) signal mix 0–1 (default: 1)".to_string(), items: None }),
                        ("temp".to_string(), PropertyDefinition { prop_type: "number".to_string(), description: "Temperature in Celsius to calculate speed of sound (default: 20°C)".to_string(), items: None }),
                    ]),
                    required: vec!["input_file".to_string(), "output_file".to_string()],
                },
            },

            ClaudeTool {
                name: "apply_earwax".to_string(),
                description: "Applies the earwax effect using FFmpeg earwax — a simple 3D audio enhancement that makes stereo recordings sound more immersive when listening through headphones by processing the stereo image to simulate speaker separation.".to_string(),
                input_schema: InputSchema {
                    schema_type: "object".to_string(),
                    properties: HashMap::from([
                        ("input_file".to_string(), PropertyDefinition { prop_type: "string".to_string(), description: "Path to input audio/video file".to_string(), items: None }),
                        ("output_file".to_string(), PropertyDefinition { prop_type: "string".to_string(), description: "Path for output file".to_string(), items: None }),
                    ]),
                    required: vec!["input_file".to_string(), "output_file".to_string()],
                },
            },

            ClaudeTool {
                name: "apply_allpass_filter".to_string(),
                description: "Applies a two-pole all-pass filter using FFmpeg allpass. Passes all frequencies unchanged in amplitude but alters their phase relationship. Use to correct phase issues between microphones, create phase-based effects, or shift the stereo image in creative mixing.".to_string(),
                input_schema: InputSchema {
                    schema_type: "object".to_string(),
                    properties: HashMap::from([
                        ("input_file".to_string(), PropertyDefinition { prop_type: "string".to_string(), description: "Path to input audio/video file".to_string(), items: None }),
                        ("output_file".to_string(), PropertyDefinition { prop_type: "string".to_string(), description: "Path for output file".to_string(), items: None }),
                        ("frequency".to_string(), PropertyDefinition { prop_type: "number".to_string(), description: "Centre frequency in Hz where maximum phase shift occurs (default: 3000)".to_string(), items: None }),
                        ("width".to_string(), PropertyDefinition { prop_type: "number".to_string(), description: "Filter width — meaning depends on width_type (default: 0.707 in Q mode)".to_string(), items: None }),
                        ("width_type".to_string(), PropertyDefinition { prop_type: "string".to_string(), description: "Width type: q=Q factor (default), h=Hz, o=octaves, s=slope, k=kHz".to_string(), items: None }),
                        ("mix".to_string(), PropertyDefinition { prop_type: "number".to_string(), description: "Wet/dry mix 0–1 (default: 1.0 = fully processed)".to_string(), items: None }),
                    ]),
                    required: vec!["input_file".to_string(), "output_file".to_string(), "frequency".to_string()],
                },
            },

            ClaudeTool {
                name: "apply_highshelf".to_string(),
                description: "Applies a high-shelf EQ using FFmpeg highshelf. Boosts or cuts all frequencies above the shelf frequency. Use for adding air and brightness (boost above 8kHz), rolling off harsh highs, or de-emphasising tape hiss.".to_string(),
                input_schema: InputSchema {
                    schema_type: "object".to_string(),
                    properties: HashMap::from([
                        ("input_file".to_string(), PropertyDefinition { prop_type: "string".to_string(), description: "Path to input audio/video file".to_string(), items: None }),
                        ("output_file".to_string(), PropertyDefinition { prop_type: "string".to_string(), description: "Path for output file".to_string(), items: None }),
                        ("frequency".to_string(), PropertyDefinition { prop_type: "number".to_string(), description: "Shelf frequency in Hz — boost/cut starts here (e.g. 8000, 12000)".to_string(), items: None }),
                        ("gain".to_string(), PropertyDefinition { prop_type: "number".to_string(), description: "Gain in dB (positive = boost, negative = cut)".to_string(), items: None }),
                        ("width".to_string(), PropertyDefinition { prop_type: "number".to_string(), description: "Shelf slope/width (default: 0.5 in slope mode). Lower = gentler slope".to_string(), items: None }),
                        ("width_type".to_string(), PropertyDefinition { prop_type: "string".to_string(), description: "Width type: s=slope (default), h=Hz, q=Q, o=octaves, k=kHz".to_string(), items: None }),
                        ("poles".to_string(), PropertyDefinition { prop_type: "number".to_string(), description: "Filter order/poles 1 or 2 (default: 2). 1=gentler slope, 2=steeper".to_string(), items: None }),
                    ]),
                    required: vec!["input_file".to_string(), "output_file".to_string(), "frequency".to_string(), "gain".to_string()],
                },
            },

            ClaudeTool {
                name: "apply_lowshelf".to_string(),
                description: "Applies a low-shelf EQ using FFmpeg lowshelf. Boosts or cuts all frequencies below the shelf frequency. Use for adding warmth and body (boost below 200Hz), removing low-end rumble, or tightening the bass.".to_string(),
                input_schema: InputSchema {
                    schema_type: "object".to_string(),
                    properties: HashMap::from([
                        ("input_file".to_string(), PropertyDefinition { prop_type: "string".to_string(), description: "Path to input audio/video file".to_string(), items: None }),
                        ("output_file".to_string(), PropertyDefinition { prop_type: "string".to_string(), description: "Path for output file".to_string(), items: None }),
                        ("frequency".to_string(), PropertyDefinition { prop_type: "number".to_string(), description: "Shelf frequency in Hz — boost/cut starts here (e.g. 80, 120, 200)".to_string(), items: None }),
                        ("gain".to_string(), PropertyDefinition { prop_type: "number".to_string(), description: "Gain in dB (positive = boost bass, negative = cut bass)".to_string(), items: None }),
                        ("width".to_string(), PropertyDefinition { prop_type: "number".to_string(), description: "Shelf slope/width (default: 0.5 in slope mode)".to_string(), items: None }),
                        ("width_type".to_string(), PropertyDefinition { prop_type: "string".to_string(), description: "Width type: s=slope (default), h=Hz, q=Q, o=octaves, k=kHz".to_string(), items: None }),
                        ("poles".to_string(), PropertyDefinition { prop_type: "number".to_string(), description: "Filter poles 1 or 2 (default: 2)".to_string(), items: None }),
                    ]),
                    required: vec!["input_file".to_string(), "output_file".to_string(), "frequency".to_string(), "gain".to_string()],
                },
            },

            ClaudeTool {
                name: "apply_surround_upmix".to_string(),
                description: "Upmixes stereo audio to surround sound using FFmpeg surround. Intelligently distributes stereo content into 5.1, 7.1 or other surround layouts by analysing channel correlation. Use to prepare stereo content for surround delivery or immersive playback.".to_string(),
                input_schema: InputSchema {
                    schema_type: "object".to_string(),
                    properties: HashMap::from([
                        ("input_file".to_string(), PropertyDefinition { prop_type: "string".to_string(), description: "Path to input audio/video file (stereo)".to_string(), items: None }),
                        ("output_file".to_string(), PropertyDefinition { prop_type: "string".to_string(), description: "Path for output surround audio/video file".to_string(), items: None }),
                        ("chl_out".to_string(), PropertyDefinition { prop_type: "string".to_string(), description: "Output channel layout (default: 5.1). Options: 5.1, 7.1, quadrature, hexagonal, etc.".to_string(), items: None }),
                        ("chl_in".to_string(), PropertyDefinition { prop_type: "string".to_string(), description: "Input channel layout (default: stereo)".to_string(), items: None }),
                        ("level_in".to_string(), PropertyDefinition { prop_type: "number".to_string(), description: "Input gain multiplier (default: 1.0)".to_string(), items: None }),
                        ("level_out".to_string(), PropertyDefinition { prop_type: "number".to_string(), description: "Output gain multiplier (default: 1.0)".to_string(), items: None }),
                    ]),
                    required: vec!["input_file".to_string(), "output_file".to_string()],
                },
            },

            ClaudeTool {
                name: "detect_volume_levels".to_string(),
                description: "Measures the maximum and mean volume levels in audio using FFmpeg volumedetect. Reports max_volume (peak), mean_volume (RMS average), and a histogram of volume distribution. Analysis only — no output file. Use to determine how much headroom exists before applying volume changes.".to_string(),
                input_schema: InputSchema {
                    schema_type: "object".to_string(),
                    properties: HashMap::from([
                        ("input_file".to_string(), PropertyDefinition { prop_type: "string".to_string(), description: "Path to input audio/video file".to_string(), items: None }),
                    ]),
                    required: vec!["input_file".to_string()],
                },
            },

            // ================================================================
            // PHASE I BATCH 8 — extract_alpha, merge_alpha, framestep, swaprect,
            //                   fillborders, chromanr, weave, interlace,
            //                   denoise_audio_fft, loop_audio, dc_shift, dynamic_range,
            //                   single_eq_band, stereotools, asetrate
            // ================================================================

            ClaudeTool {
                name: "extract_alpha_channel".to_string(),
                description: "Extracts the alpha (transparency) channel from a video as a greyscale image/video using FFmpeg alphaextract. White = fully opaque, black = fully transparent. Use to inspect or re-use the transparency mask of footage with an alpha channel.".to_string(),
                input_schema: InputSchema {
                    schema_type: "object".to_string(),
                    properties: HashMap::from([
                        ("input_file".to_string(), PropertyDefinition { prop_type: "string".to_string(), description: "Path to input video with alpha channel".to_string(), items: None }),
                        ("output_file".to_string(), PropertyDefinition { prop_type: "string".to_string(), description: "Path for output greyscale video/image".to_string(), items: None }),
                    ]),
                    required: vec!["input_file".to_string(), "output_file".to_string()],
                },
            },

            ClaudeTool {
                name: "merge_alpha_channel".to_string(),
                description: "Merges a greyscale video as the alpha channel into a colour video using FFmpeg alphamerge. Use to add custom transparency masks to footage — the greyscale video becomes the alpha: white = opaque, black = transparent.".to_string(),
                input_schema: InputSchema {
                    schema_type: "object".to_string(),
                    properties: HashMap::from([
                        ("input_file".to_string(), PropertyDefinition { prop_type: "string".to_string(), description: "Path to base colour video file".to_string(), items: None }),
                        ("alpha_file".to_string(), PropertyDefinition { prop_type: "string".to_string(), description: "Path to greyscale video to use as alpha channel".to_string(), items: None }),
                        ("output_file".to_string(), PropertyDefinition { prop_type: "string".to_string(), description: "Path for output video with alpha channel".to_string(), items: None }),
                    ]),
                    required: vec!["input_file".to_string(), "alpha_file".to_string(), "output_file".to_string()],
                },
            },

            ClaudeTool {
                name: "apply_framestep".to_string(),
                description: "Outputs every Nth frame from a video using FFmpeg framestep. Use to reduce frame rate to a fraction (e.g. step=2 keeps every other frame = half the frame rate), create time-lapse from normal footage, or extract a sparse set of frames.".to_string(),
                input_schema: InputSchema {
                    schema_type: "object".to_string(),
                    properties: HashMap::from([
                        ("input_file".to_string(), PropertyDefinition { prop_type: "string".to_string(), description: "Path to input video file".to_string(), items: None }),
                        ("output_file".to_string(), PropertyDefinition { prop_type: "string".to_string(), description: "Path for output video file".to_string(), items: None }),
                        ("step".to_string(), PropertyDefinition { prop_type: "number".to_string(), description: "Keep every Nth frame (default: 1 = keep all, 2 = keep every other, 3 = keep every 3rd, etc.)".to_string(), items: None }),
                    ]),
                    required: vec!["input_file".to_string(), "output_file".to_string()],
                },
            },

            ClaudeTool {
                name: "apply_swaprect".to_string(),
                description: "Swaps two rectangular regions within a video frame using FFmpeg swaprect. Use for creative effects, privacy blurring/masking by swapping regions, or debugging layout issues.".to_string(),
                input_schema: InputSchema {
                    schema_type: "object".to_string(),
                    properties: HashMap::from([
                        ("input_file".to_string(), PropertyDefinition { prop_type: "string".to_string(), description: "Path to input video file".to_string(), items: None }),
                        ("output_file".to_string(), PropertyDefinition { prop_type: "string".to_string(), description: "Path for output video file".to_string(), items: None }),
                        ("x1".to_string(), PropertyDefinition { prop_type: "number".to_string(), description: "X position of first rectangle".to_string(), items: None }),
                        ("y1".to_string(), PropertyDefinition { prop_type: "number".to_string(), description: "Y position of first rectangle".to_string(), items: None }),
                        ("x2".to_string(), PropertyDefinition { prop_type: "number".to_string(), description: "X position of second rectangle".to_string(), items: None }),
                        ("y2".to_string(), PropertyDefinition { prop_type: "number".to_string(), description: "Y position of second rectangle".to_string(), items: None }),
                        ("w".to_string(), PropertyDefinition { prop_type: "number".to_string(), description: "Width of both rectangles in pixels".to_string(), items: None }),
                        ("h".to_string(), PropertyDefinition { prop_type: "number".to_string(), description: "Height of both rectangles in pixels".to_string(), items: None }),
                    ]),
                    required: vec!["input_file".to_string(), "output_file".to_string(), "w".to_string(), "h".to_string()],
                },
            },

            ClaudeTool {
                name: "apply_fillborders".to_string(),
                description: "Fills the border pixels of a video frame using FFmpeg fillborders. Useful for removing thin black borders, fixing crops, or extending content to fill edges. Modes: smear (extend edge pixels), mirror (reflect inward), wrap (tile), fixed (solid color).".to_string(),
                input_schema: InputSchema {
                    schema_type: "object".to_string(),
                    properties: HashMap::from([
                        ("input_file".to_string(), PropertyDefinition { prop_type: "string".to_string(), description: "Path to input video file".to_string(), items: None }),
                        ("output_file".to_string(), PropertyDefinition { prop_type: "string".to_string(), description: "Path for output video file".to_string(), items: None }),
                        ("left".to_string(), PropertyDefinition { prop_type: "number".to_string(), description: "Pixels to fill on left edge (default: 0)".to_string(), items: None }),
                        ("right".to_string(), PropertyDefinition { prop_type: "number".to_string(), description: "Pixels to fill on right edge (default: 0)".to_string(), items: None }),
                        ("top".to_string(), PropertyDefinition { prop_type: "number".to_string(), description: "Pixels to fill on top edge (default: 0)".to_string(), items: None }),
                        ("bottom".to_string(), PropertyDefinition { prop_type: "number".to_string(), description: "Pixels to fill on bottom edge (default: 0)".to_string(), items: None }),
                        ("mode".to_string(), PropertyDefinition { prop_type: "string".to_string(), description: "Fill mode: smear (extend edge, default), mirror, wrap, fixed (solid color)".to_string(), items: None }),
                        ("color".to_string(), PropertyDefinition { prop_type: "string".to_string(), description: "Colour for fixed mode (default: black). e.g. black, white, 0xFF0000".to_string(), items: None }),
                    ]),
                    required: vec!["input_file".to_string(), "output_file".to_string()],
                },
            },

            ClaudeTool {
                name: "apply_chromanr".to_string(),
                description: "Reduces chroma (colour) noise in video using FFmpeg chromanr. Works in YCbCr space — averages chroma values within a spatial window where they differ less than a threshold. Preserves luma sharpness while smoothing colour noise from high-ISO or compressed footage.".to_string(),
                input_schema: InputSchema {
                    schema_type: "object".to_string(),
                    properties: HashMap::from([
                        ("input_file".to_string(), PropertyDefinition { prop_type: "string".to_string(), description: "Path to input video file".to_string(), items: None }),
                        ("output_file".to_string(), PropertyDefinition { prop_type: "string".to_string(), description: "Path for output video file".to_string(), items: None }),
                        ("thres".to_string(), PropertyDefinition { prop_type: "number".to_string(), description: "Chroma similarity threshold 1.0–200.0 (default: 30.0). Higher = more aggressive averaging".to_string(), items: None }),
                        ("sizew".to_string(), PropertyDefinition { prop_type: "number".to_string(), description: "Averaging window width in pixels (default: 5)".to_string(), items: None }),
                        ("sizeh".to_string(), PropertyDefinition { prop_type: "number".to_string(), description: "Averaging window height in pixels (default: 5)".to_string(), items: None }),
                        ("stepw".to_string(), PropertyDefinition { prop_type: "number".to_string(), description: "Horizontal step size (default: 1)".to_string(), items: None }),
                        ("steph".to_string(), PropertyDefinition { prop_type: "number".to_string(), description: "Vertical step size (default: 1)".to_string(), items: None }),
                    ]),
                    required: vec!["input_file".to_string(), "output_file".to_string()],
                },
            },

            ClaudeTool {
                name: "apply_weave".to_string(),
                description: "Weaves separate fields into full interlaced frames using FFmpeg weave. The inverse of deinterlacing — takes a stream of individual fields and combines pairs into full frames. Use to reconstruct interlaced content from field-extracted footage.".to_string(),
                input_schema: InputSchema {
                    schema_type: "object".to_string(),
                    properties: HashMap::from([
                        ("input_file".to_string(), PropertyDefinition { prop_type: "string".to_string(), description: "Path to input video (stream of fields)".to_string(), items: None }),
                        ("output_file".to_string(), PropertyDefinition { prop_type: "string".to_string(), description: "Path for output interlaced video".to_string(), items: None }),
                        ("first_field".to_string(), PropertyDefinition { prop_type: "string".to_string(), description: "Which field comes first: top (default) or bottom".to_string(), items: None }),
                    ]),
                    required: vec!["input_file".to_string(), "output_file".to_string()],
                },
            },

            ClaudeTool {
                name: "apply_interlace".to_string(),
                description: "Creates interlaced video from progressive input using FFmpeg interlace. Combines two consecutive fields into a single interlaced frame. Use for broadcast delivery that requires interlaced output (PAL 50i, NTSC 29.97i).".to_string(),
                input_schema: InputSchema {
                    schema_type: "object".to_string(),
                    properties: HashMap::from([
                        ("input_file".to_string(), PropertyDefinition { prop_type: "string".to_string(), description: "Path to input progressive video file".to_string(), items: None }),
                        ("output_file".to_string(), PropertyDefinition { prop_type: "string".to_string(), description: "Path for output interlaced video file".to_string(), items: None }),
                        ("scan".to_string(), PropertyDefinition { prop_type: "string".to_string(), description: "Field order: tff (top field first, default) or bff (bottom field first)".to_string(), items: None }),
                        ("lowpass".to_string(), PropertyDefinition { prop_type: "number".to_string(), description: "Vertical low-pass filter 0–2 (default: 1). 0=off, 1=linear, 2=complex".to_string(), items: None }),
                    ]),
                    required: vec!["input_file".to_string(), "output_file".to_string()],
                },
            },

            ClaudeTool {
                name: "denoise_audio_fft".to_string(),
                description: "Reduces background noise using FFmpeg afftdn (FFT-based denoiser). Analyses the audio in the frequency domain and attenuates frequencies that fall below a noise floor. Different from RNN denoising — works on any consistent background noise (fan, room tone, hum) without a model file.".to_string(),
                input_schema: InputSchema {
                    schema_type: "object".to_string(),
                    properties: HashMap::from([
                        ("input_file".to_string(), PropertyDefinition { prop_type: "string".to_string(), description: "Path to input audio/video file".to_string(), items: None }),
                        ("output_file".to_string(), PropertyDefinition { prop_type: "string".to_string(), description: "Path for output file".to_string(), items: None }),
                        ("noise_floor".to_string(), PropertyDefinition { prop_type: "number".to_string(), description: "Noise floor in dB, -100–0 (default: -25.0). Set to the level of background noise in the file".to_string(), items: None }),
                        ("noise_reduction".to_string(), PropertyDefinition { prop_type: "number".to_string(), description: "Noise reduction in dB, 0.01–97 (default: 12.0). How much to attenuate noise frequencies".to_string(), items: None }),
                        ("track_noise".to_string(), PropertyDefinition { prop_type: "boolean".to_string(), description: "Track noise profile over time (default: false). Enable for noise that changes".to_string(), items: None }),
                    ]),
                    required: vec!["input_file".to_string(), "output_file".to_string()],
                },
            },

            ClaudeTool {
                name: "loop_audio".to_string(),
                description: "Loops an audio stream N times using FFmpeg aloop. Use to create a repeated loop from a short clip, extend background music to fill a video, or generate a seamless repeating audio texture.".to_string(),
                input_schema: InputSchema {
                    schema_type: "object".to_string(),
                    properties: HashMap::from([
                        ("input_file".to_string(), PropertyDefinition { prop_type: "string".to_string(), description: "Path to input audio/video file".to_string(), items: None }),
                        ("output_file".to_string(), PropertyDefinition { prop_type: "string".to_string(), description: "Path for output file".to_string(), items: None }),
                        ("loop_count".to_string(), PropertyDefinition { prop_type: "number".to_string(), description: "Number of loops (-1 = infinite, 0 = no loop, N = loop N times). Default: 1".to_string(), items: None }),
                        ("size".to_string(), PropertyDefinition { prop_type: "number".to_string(), description: "Number of samples per loop (0 = whole file). Default: 0".to_string(), items: None }),
                        ("start".to_string(), PropertyDefinition { prop_type: "number".to_string(), description: "Sample offset to start loop from (default: 0)".to_string(), items: None }),
                    ]),
                    required: vec!["input_file".to_string(), "output_file".to_string()],
                },
            },

            ClaudeTool {
                name: "apply_dc_shift".to_string(),
                description: "Applies a DC offset correction to audio using FFmpeg dcshift. DC offset is a constant voltage bias that shifts the waveform away from zero — causes distortion and wastes headroom. Use to fix recordings with DC offset (common with cheap microphones or certain interfaces).".to_string(),
                input_schema: InputSchema {
                    schema_type: "object".to_string(),
                    properties: HashMap::from([
                        ("input_file".to_string(), PropertyDefinition { prop_type: "string".to_string(), description: "Path to input audio/video file".to_string(), items: None }),
                        ("output_file".to_string(), PropertyDefinition { prop_type: "string".to_string(), description: "Path for output file".to_string(), items: None }),
                        ("shift".to_string(), PropertyDefinition { prop_type: "number".to_string(), description: "DC shift to apply -1.0–1.0. Use negative value to cancel positive DC offset (default: 0.0)".to_string(), items: None }),
                        ("limitergain".to_string(), PropertyDefinition { prop_type: "number".to_string(), description: "Optional limiter gain 0.0–1.0 to prevent clipping (0 = disabled). Default: 0.0".to_string(), items: None }),
                    ]),
                    required: vec!["input_file".to_string(), "output_file".to_string(), "shift".to_string()],
                },
            },

            ClaudeTool {
                name: "measure_dynamic_range".to_string(),
                description: "Measures the dynamic range of audio using FFmpeg drmeter. Reports DR (crest factor), peak levels, and RMS values per channel. Higher DR = more dynamic audio. Use for mastering quality control or loudness analysis.".to_string(),
                input_schema: InputSchema {
                    schema_type: "object".to_string(),
                    properties: HashMap::from([
                        ("input_file".to_string(), PropertyDefinition { prop_type: "string".to_string(), description: "Path to input audio/video file".to_string(), items: None }),
                    ]),
                    required: vec!["input_file".to_string()],
                },
            },

            ClaudeTool {
                name: "apply_single_eq_band".to_string(),
                description: "Applies a single-band parametric equalizer using FFmpeg equalizer. Unlike the multi-band parametric_eq, this is simpler — one band at a chosen frequency, width, and gain. Good for targeted corrections: cut a resonance, boost presence, reduce mud.".to_string(),
                input_schema: InputSchema {
                    schema_type: "object".to_string(),
                    properties: HashMap::from([
                        ("input_file".to_string(), PropertyDefinition { prop_type: "string".to_string(), description: "Path to input audio/video file".to_string(), items: None }),
                        ("output_file".to_string(), PropertyDefinition { prop_type: "string".to_string(), description: "Path for output file".to_string(), items: None }),
                        ("frequency".to_string(), PropertyDefinition { prop_type: "number".to_string(), description: "Centre frequency in Hz (e.g. 1000 = 1kHz)".to_string(), items: None }),
                        ("width".to_string(), PropertyDefinition { prop_type: "number".to_string(), description: "Band width (meaning depends on width_type). Default: 1.0 octave".to_string(), items: None }),
                        ("gain".to_string(), PropertyDefinition { prop_type: "number".to_string(), description: "Gain in dB, -900–900 (positive = boost, negative = cut)".to_string(), items: None }),
                        ("width_type".to_string(), PropertyDefinition { prop_type: "string".to_string(), description: "Width interpretation: h=Hz, q=Q factor, o=octaves (default), s=slope, k=kHz".to_string(), items: None }),
                    ]),
                    required: vec!["input_file".to_string(), "output_file".to_string(), "frequency".to_string(), "gain".to_string()],
                },
            },

            ClaudeTool {
                name: "apply_stereotools".to_string(),
                description: "Professional stereo field manipulation using FFmpeg stereotools. Offers independent level/balance for each channel, phase inversion, channel muting, soft clipping, and mode switching (LR↔MS, mono, swap, etc.). More flexible than stereowiden for professional stereo work.".to_string(),
                input_schema: InputSchema {
                    schema_type: "object".to_string(),
                    properties: HashMap::from([
                        ("input_file".to_string(), PropertyDefinition { prop_type: "string".to_string(), description: "Path to input audio/video file".to_string(), items: None }),
                        ("output_file".to_string(), PropertyDefinition { prop_type: "string".to_string(), description: "Path for output file".to_string(), items: None }),
                        ("level_in".to_string(), PropertyDefinition { prop_type: "number".to_string(), description: "Input gain 0.015–64.0 (default: 1.0)".to_string(), items: None }),
                        ("level_out".to_string(), PropertyDefinition { prop_type: "number".to_string(), description: "Output gain 0.015–64.0 (default: 1.0)".to_string(), items: None }),
                        ("balance_in".to_string(), PropertyDefinition { prop_type: "number".to_string(), description: "Input balance -1.0–1.0 (default: 0.0 = centre)".to_string(), items: None }),
                        ("balance_out".to_string(), PropertyDefinition { prop_type: "number".to_string(), description: "Output balance -1.0–1.0 (default: 0.0 = centre)".to_string(), items: None }),
                        ("softclip".to_string(), PropertyDefinition { prop_type: "boolean".to_string(), description: "Soft clip output (default: false)".to_string(), items: None }),
                        ("mutel".to_string(), PropertyDefinition { prop_type: "boolean".to_string(), description: "Mute left channel (default: false)".to_string(), items: None }),
                        ("muter".to_string(), PropertyDefinition { prop_type: "boolean".to_string(), description: "Mute right channel (default: false)".to_string(), items: None }),
                        ("phasel".to_string(), PropertyDefinition { prop_type: "boolean".to_string(), description: "Invert phase on left channel (default: false)".to_string(), items: None }),
                        ("phaser".to_string(), PropertyDefinition { prop_type: "boolean".to_string(), description: "Invert phase on right channel (default: false)".to_string(), items: None }),
                        ("mode".to_string(), PropertyDefinition { prop_type: "string".to_string(), description: "Channel mode: lr>lr (default), lr>ms, ms>lr, lr>ll, lr>rr, lr>l+r, lr>rl, ms>ll, ms>rr, ms>l+r".to_string(), items: None }),
                    ]),
                    required: vec!["input_file".to_string(), "output_file".to_string()],
                },
            },

            ClaudeTool {
                name: "apply_asetrate".to_string(),
                description: "Changes the audio sample rate metadata without resampling using FFmpeg asetrate. This shifts both pitch and speed together (like speeding up/slowing down a tape). Use for pitch-shifting effects, creating lo-fi chipmunk/slowed+reverb audio styles, or fixing wrongly-tagged sample rates.".to_string(),
                input_schema: InputSchema {
                    schema_type: "object".to_string(),
                    properties: HashMap::from([
                        ("input_file".to_string(), PropertyDefinition { prop_type: "string".to_string(), description: "Path to input audio/video file".to_string(), items: None }),
                        ("output_file".to_string(), PropertyDefinition { prop_type: "string".to_string(), description: "Path for output file".to_string(), items: None }),
                        ("sample_rate".to_string(), PropertyDefinition { prop_type: "number".to_string(), description: "New sample rate in Hz. Setting higher than original = slower/lower pitch (e.g. 88200 = half speed). Lower = faster/higher pitch (e.g. 22050 = double speed). Default: 44100".to_string(), items: None }),
                    ]),
                    required: vec!["input_file".to_string(), "output_file".to_string()],
                },
            },

            // ================================================================
            // PHASE I BATCH 7 — xfade_transition, color_key, monochrome, maskedmerge,
            //                   convert_360_video, fix_banding, greyedge, fade_video,
            //                   normalize_loudness, dynamic_audio_normalize, resample_audio,
            //                   trim_audio, crystalizer, multiband_compress, super_equalizer
            // ================================================================

            ClaudeTool {
                name: "apply_xfade_transition".to_string(),
                description: "Cross-fades between two video clips using FFmpeg xfade. Supports many transition types: fade, dissolve, wipeleft/right/up/down, slideleft/right/up/down, circlecrop, rectcrop, distance, fadeblack, fadewhite, radial, smoothleft/right/up/down, circleopen/close, horzopen/close, vertopen/close, diagtl/tr/bl/br, hlslice, hrslice, vuslice, vdslice.".to_string(),
                input_schema: InputSchema {
                    schema_type: "object".to_string(),
                    properties: HashMap::from([
                        ("input_file".to_string(), PropertyDefinition { prop_type: "string".to_string(), description: "Path to first input video file".to_string(), items: None }),
                        ("secondary_file".to_string(), PropertyDefinition { prop_type: "string".to_string(), description: "Path to second input video file".to_string(), items: None }),
                        ("output_file".to_string(), PropertyDefinition { prop_type: "string".to_string(), description: "Path for output video file".to_string(), items: None }),
                        ("transition".to_string(), PropertyDefinition { prop_type: "string".to_string(), description: "Transition type (default: fade). Options: fade, dissolve, wipeleft, wiperight, wipeup, wipedown, slideleft, slideright, slideup, slidedown, circlecrop, rectcrop, distance, fadeblack, fadewhite, radial, smoothleft, smoothright, smoothup, smoothdown, circleopen, circleclose, horzopen, horzclose, vertopen, vertclose, diagtl, diagtr, diagbl, diagbr, hlslice, hrslice, vuslice, vdslice".to_string(), items: None }),
                        ("duration".to_string(), PropertyDefinition { prop_type: "number".to_string(), description: "Duration of transition in seconds (default: 1.0)".to_string(), items: None }),
                        ("offset".to_string(), PropertyDefinition { prop_type: "number".to_string(), description: "Seconds into first clip where transition starts (default: 0.0)".to_string(), items: None }),
                    ]),
                    required: vec!["input_file".to_string(), "secondary_file".to_string(), "output_file".to_string()],
                },
            },

            ClaudeTool {
                name: "apply_color_key".to_string(),
                description: "Removes a specific colour from a video using FFmpeg colorkey, making those pixels transparent. Unlike chroma key (which uses hue/saturation), colorkey matches an exact colour value. Use to key out flat colour backgrounds.".to_string(),
                input_schema: InputSchema {
                    schema_type: "object".to_string(),
                    properties: HashMap::from([
                        ("input_file".to_string(), PropertyDefinition { prop_type: "string".to_string(), description: "Path to input video file".to_string(), items: None }),
                        ("output_file".to_string(), PropertyDefinition { prop_type: "string".to_string(), description: "Path for output video file".to_string(), items: None }),
                        ("color".to_string(), PropertyDefinition { prop_type: "string".to_string(), description: "Colour to key out as hex (default: 0x00FF00 green). e.g. 0xFF0000 for red, black, white".to_string(), items: None }),
                        ("similarity".to_string(), PropertyDefinition { prop_type: "number".to_string(), description: "Similarity radius 0.0–1.0 (default: 0.1). Higher = more pixels keyed out".to_string(), items: None }),
                        ("blend".to_string(), PropertyDefinition { prop_type: "number".to_string(), description: "Blend factor 0.0–1.0 (default: 0.0). Higher = softer edges".to_string(), items: None }),
                    ]),
                    required: vec!["input_file".to_string(), "output_file".to_string()],
                },
            },

            ClaudeTool {
                name: "apply_monochrome".to_string(),
                description: "Converts video to a stylised monochrome/black-and-white using FFmpeg monochrome. Unlike simple desaturation, lets you choose which colour bias to retain and how hard the conversion is — useful for cinematic B&W looks.".to_string(),
                input_schema: InputSchema {
                    schema_type: "object".to_string(),
                    properties: HashMap::from([
                        ("input_file".to_string(), PropertyDefinition { prop_type: "string".to_string(), description: "Path to input video file".to_string(), items: None }),
                        ("output_file".to_string(), PropertyDefinition { prop_type: "string".to_string(), description: "Path for output video file".to_string(), items: None }),
                        ("cb".to_string(), PropertyDefinition { prop_type: "number".to_string(), description: "Blue-yellow chroma bias -1.0–1.0 (default: 0.0). Positive = warmer tones brighter".to_string(), items: None }),
                        ("cr".to_string(), PropertyDefinition { prop_type: "number".to_string(), description: "Red-green chroma bias -1.0–1.0 (default: 0.0)".to_string(), items: None }),
                        ("size".to_string(), PropertyDefinition { prop_type: "number".to_string(), description: "Size of the colour band 0.0–1.0 (default: 1.0)".to_string(), items: None }),
                    ]),
                    required: vec!["input_file".to_string(), "output_file".to_string()],
                },
            },

            ClaudeTool {
                name: "apply_maskedmerge".to_string(),
                description: "Blends two video streams together using a third stream as a mask using FFmpeg maskedmerge. Pixels from the overlay where the mask is white; pixels from the base where mask is black. Use for precise compositing with custom alpha masks.".to_string(),
                input_schema: InputSchema {
                    schema_type: "object".to_string(),
                    properties: HashMap::from([
                        ("input_file".to_string(), PropertyDefinition { prop_type: "string".to_string(), description: "Path to base video file".to_string(), items: None }),
                        ("overlay_file".to_string(), PropertyDefinition { prop_type: "string".to_string(), description: "Path to overlay video file (shown where mask is white)".to_string(), items: None }),
                        ("mask_file".to_string(), PropertyDefinition { prop_type: "string".to_string(), description: "Path to mask video file (white = show overlay, black = show base)".to_string(), items: None }),
                        ("output_file".to_string(), PropertyDefinition { prop_type: "string".to_string(), description: "Path for output video file".to_string(), items: None }),
                        ("planes".to_string(), PropertyDefinition { prop_type: "number".to_string(), description: "Bitmask of planes to process (default: 15 = all)".to_string(), items: None }),
                    ]),
                    required: vec!["input_file".to_string(), "overlay_file".to_string(), "mask_file".to_string(), "output_file".to_string()],
                },
            },

            ClaudeTool {
                name: "convert_360_video".to_string(),
                description: "Converts 360° video between different projection formats using FFmpeg v360. Handles equirectangular, cubemap, flat (rectilinear), fisheye, stereographic, mercator, ball, hammer, sinusoidal, healpix, tetrahedron and more. Use to extract a normal-looking crop from a 360° camera, or to convert between VR formats.".to_string(),
                input_schema: InputSchema {
                    schema_type: "object".to_string(),
                    properties: HashMap::from([
                        ("input_file".to_string(), PropertyDefinition { prop_type: "string".to_string(), description: "Path to input 360° video file".to_string(), items: None }),
                        ("output_file".to_string(), PropertyDefinition { prop_type: "string".to_string(), description: "Path for output video file".to_string(), items: None }),
                        ("input_fmt".to_string(), PropertyDefinition { prop_type: "string".to_string(), description: "Input projection format (default: equirect). Options: equirect, c3x2, c6x1, c1x6, eac, equiangular, flat, gnomonic, dfisheye, pannini, cylindrical, perspective, tetrahedron, ball, hammer, sinusoidal, fisheye, mercator, stereographic, healpix".to_string(), items: None }),
                        ("output_fmt".to_string(), PropertyDefinition { prop_type: "string".to_string(), description: "Output projection format (default: flat). Same options as input_fmt".to_string(), items: None }),
                        ("width".to_string(), PropertyDefinition { prop_type: "number".to_string(), description: "Output width in pixels (default: 1920)".to_string(), items: None }),
                        ("height".to_string(), PropertyDefinition { prop_type: "number".to_string(), description: "Output height in pixels (default: 1080)".to_string(), items: None }),
                        ("h_fov".to_string(), PropertyDefinition { prop_type: "number".to_string(), description: "Horizontal field of view in degrees (default: 90.0 for flat output)".to_string(), items: None }),
                        ("v_fov".to_string(), PropertyDefinition { prop_type: "number".to_string(), description: "Vertical field of view in degrees (default: 90.0 for flat output)".to_string(), items: None }),
                    ]),
                    required: vec!["input_file".to_string(), "output_file".to_string()],
                },
            },

            ClaudeTool {
                name: "fix_banding".to_string(),
                description: "Fixes colour banding artifacts in gradients using FFmpeg gradfun (gradient dithering). Adds subtle dithering noise to smooth areas to break up visible colour bands, particularly helpful for footage that was graded aggressively or encoded at low bit depth.".to_string(),
                input_schema: InputSchema {
                    schema_type: "object".to_string(),
                    properties: HashMap::from([
                        ("input_file".to_string(), PropertyDefinition { prop_type: "string".to_string(), description: "Path to input video file".to_string(), items: None }),
                        ("output_file".to_string(), PropertyDefinition { prop_type: "string".to_string(), description: "Path for output video file".to_string(), items: None }),
                        ("strength".to_string(), PropertyDefinition { prop_type: "number".to_string(), description: "Dithering strength 0.51–65.0 (default: 1.2). Higher = more aggressive dithering".to_string(), items: None }),
                        ("radius".to_string(), PropertyDefinition { prop_type: "number".to_string(), description: "Gradient detection radius 4–32 pixels (default: 16)".to_string(), items: None }),
                    ]),
                    required: vec!["input_file".to_string(), "output_file".to_string()],
                },
            },

            ClaudeTool {
                name: "apply_greyedge".to_string(),
                description: "Auto white-balances a video using FFmpeg greyedge (grey edge assumption). Analyses the image edges to estimate the scene illuminant and corrects the white balance automatically. Useful when footage has a colour cast from mixed or unknown lighting.".to_string(),
                input_schema: InputSchema {
                    schema_type: "object".to_string(),
                    properties: HashMap::from([
                        ("input_file".to_string(), PropertyDefinition { prop_type: "string".to_string(), description: "Path to input video file".to_string(), items: None }),
                        ("output_file".to_string(), PropertyDefinition { prop_type: "string".to_string(), description: "Path for output video file".to_string(), items: None }),
                        ("difford".to_string(), PropertyDefinition { prop_type: "number".to_string(), description: "Order of differentiation 0–2 (default: 1). 0=grey world, 1=grey edge 1st order, 2=grey edge 2nd order".to_string(), items: None }),
                        ("minknorm".to_string(), PropertyDefinition { prop_type: "number".to_string(), description: "Minkowski norm 0–20 (default: 1). 0=max norm, 1=L1, 2=L2 (Euclidean)".to_string(), items: None }),
                        ("sigma".to_string(), PropertyDefinition { prop_type: "number".to_string(), description: "Gaussian blur sigma before differentiation 0.0–200.0 (default: 1.0)".to_string(), items: None }),
                    ]),
                    required: vec!["input_file".to_string(), "output_file".to_string()],
                },
            },

            ClaudeTool {
                name: "apply_fade_video".to_string(),
                description: "Applies a video fade-in or fade-out using FFmpeg fade. Can fade from/to any colour (default black). Use for smooth intros, outros, or scene transitions.".to_string(),
                input_schema: InputSchema {
                    schema_type: "object".to_string(),
                    properties: HashMap::from([
                        ("input_file".to_string(), PropertyDefinition { prop_type: "string".to_string(), description: "Path to input video file".to_string(), items: None }),
                        ("output_file".to_string(), PropertyDefinition { prop_type: "string".to_string(), description: "Path for output video file".to_string(), items: None }),
                        ("fade_type".to_string(), PropertyDefinition { prop_type: "string".to_string(), description: "Fade direction: in (brighten from color) or out (darken to color). Default: in".to_string(), items: None }),
                        ("start_time".to_string(), PropertyDefinition { prop_type: "number".to_string(), description: "Timestamp in seconds where the fade starts (default: 0.0)".to_string(), items: None }),
                        ("duration".to_string(), PropertyDefinition { prop_type: "number".to_string(), description: "Duration of the fade in seconds (default: 1.0)".to_string(), items: None }),
                        ("color".to_string(), PropertyDefinition { prop_type: "string".to_string(), description: "Colour to fade from/to (default: black). e.g. black, white, 0x000000".to_string(), items: None }),
                    ]),
                    required: vec!["input_file".to_string(), "output_file".to_string()],
                },
            },

            ClaudeTool {
                name: "normalize_loudness".to_string(),
                description: "Normalises audio to a target integrated loudness using FFmpeg loudnorm (EBU R128 standard). Unlike simple volume adjustment, this dynamic normaliser preserves dynamics while meeting broadcast/streaming loudness targets (-23 LUFS for broadcast, -14 LUFS for Spotify/YouTube, -16 LUFS for podcasts).".to_string(),
                input_schema: InputSchema {
                    schema_type: "object".to_string(),
                    properties: HashMap::from([
                        ("input_file".to_string(), PropertyDefinition { prop_type: "string".to_string(), description: "Path to input audio/video file".to_string(), items: None }),
                        ("output_file".to_string(), PropertyDefinition { prop_type: "string".to_string(), description: "Path for output file".to_string(), items: None }),
                        ("i".to_string(), PropertyDefinition { prop_type: "number".to_string(), description: "Target integrated loudness in LUFS (default: -23.0 = EBU broadcast. -14 for streaming, -16 for podcasts)".to_string(), items: None }),
                        ("lra".to_string(), PropertyDefinition { prop_type: "number".to_string(), description: "Target loudness range in LU (default: 7.0, range: 1–20)".to_string(), items: None }),
                        ("tp".to_string(), PropertyDefinition { prop_type: "number".to_string(), description: "Maximum true peak in dBFS (default: -2.0)".to_string(), items: None }),
                    ]),
                    required: vec!["input_file".to_string(), "output_file".to_string()],
                },
            },

            ClaudeTool {
                name: "dynamic_audio_normalize".to_string(),
                description: "Applies dynamic per-frame normalisation to audio using FFmpeg dynaudnorm. Analyses short frames independently and brings each to a target peak level, smoothed with a Gaussian window. Useful to level out dialogue that varies wildly in volume while preserving natural dynamics.".to_string(),
                input_schema: InputSchema {
                    schema_type: "object".to_string(),
                    properties: HashMap::from([
                        ("input_file".to_string(), PropertyDefinition { prop_type: "string".to_string(), description: "Path to input audio/video file".to_string(), items: None }),
                        ("output_file".to_string(), PropertyDefinition { prop_type: "string".to_string(), description: "Path for output file".to_string(), items: None }),
                        ("frame_len".to_string(), PropertyDefinition { prop_type: "number".to_string(), description: "Frame length in milliseconds 10–8000 (default: 500)".to_string(), items: None }),
                        ("gausssize".to_string(), PropertyDefinition { prop_type: "number".to_string(), description: "Gaussian smoothing window size, must be odd 3–301 (default: 31)".to_string(), items: None }),
                        ("peak".to_string(), PropertyDefinition { prop_type: "number".to_string(), description: "Target peak value 0.0–1.0 (default: 0.95)".to_string(), items: None }),
                        ("max_gain".to_string(), PropertyDefinition { prop_type: "number".to_string(), description: "Maximum gain factor 1.0–100.0 (default: 10.0)".to_string(), items: None }),
                        ("rms".to_string(), PropertyDefinition { prop_type: "number".to_string(), description: "RMS-based target, 0=disabled 0.0–1.0 (default: 0.0)".to_string(), items: None }),
                        ("coupling".to_string(), PropertyDefinition { prop_type: "boolean".to_string(), description: "Couple all channels together (default: true). False = independent per-channel normalisation".to_string(), items: None }),
                    ]),
                    required: vec!["input_file".to_string(), "output_file".to_string()],
                },
            },

            ClaudeTool {
                name: "resample_audio".to_string(),
                description: "Resamples audio to a different sample rate using FFmpeg aresample. Use to convert 48kHz broadcast audio to 44.1kHz for CD/music, or vice versa, or to fix mismatched sample rates between clips.".to_string(),
                input_schema: InputSchema {
                    schema_type: "object".to_string(),
                    properties: HashMap::from([
                        ("input_file".to_string(), PropertyDefinition { prop_type: "string".to_string(), description: "Path to input audio/video file".to_string(), items: None }),
                        ("output_file".to_string(), PropertyDefinition { prop_type: "string".to_string(), description: "Path for output file".to_string(), items: None }),
                        ("sample_rate".to_string(), PropertyDefinition { prop_type: "number".to_string(), description: "Target sample rate in Hz (default: 44100). Common: 22050, 44100, 48000, 96000".to_string(), items: None }),
                        ("resampler".to_string(), PropertyDefinition { prop_type: "string".to_string(), description: "Resampler library: swr (default, fast) or soxr (high quality)".to_string(), items: None }),
                    ]),
                    required: vec!["input_file".to_string(), "output_file".to_string()],
                },
            },

            ClaudeTool {
                name: "trim_audio".to_string(),
                description: "Trims an audio stream to a time range using FFmpeg atrim. Unlike file-level trimming, this operates at the filter level and resets timestamps to start from zero. Use to precisely cut audio with frame accuracy.".to_string(),
                input_schema: InputSchema {
                    schema_type: "object".to_string(),
                    properties: HashMap::from([
                        ("input_file".to_string(), PropertyDefinition { prop_type: "string".to_string(), description: "Path to input audio/video file".to_string(), items: None }),
                        ("output_file".to_string(), PropertyDefinition { prop_type: "string".to_string(), description: "Path for output file".to_string(), items: None }),
                        ("start".to_string(), PropertyDefinition { prop_type: "number".to_string(), description: "Start time in seconds (default: 0.0)".to_string(), items: None }),
                        ("end".to_string(), PropertyDefinition { prop_type: "number".to_string(), description: "End time in seconds (0 = use duration instead)".to_string(), items: None }),
                        ("duration".to_string(), PropertyDefinition { prop_type: "number".to_string(), description: "Duration in seconds from start (0 = use end instead)".to_string(), items: None }),
                    ]),
                    required: vec!["input_file".to_string(), "output_file".to_string()],
                },
            },

            ClaudeTool {
                name: "apply_crystalizer".to_string(),
                description: "Applies audio crystalizer effect using FFmpeg crystalizer — enhances transients and detail by boosting frequency contrasts between frames. Creates a 'hyper-detailed' or 'glassy' audio texture. Use for music production enhancement or creative sound design.".to_string(),
                input_schema: InputSchema {
                    schema_type: "object".to_string(),
                    properties: HashMap::from([
                        ("input_file".to_string(), PropertyDefinition { prop_type: "string".to_string(), description: "Path to input audio/video file".to_string(), items: None }),
                        ("output_file".to_string(), PropertyDefinition { prop_type: "string".to_string(), description: "Path for output file".to_string(), items: None }),
                        ("i".to_string(), PropertyDefinition { prop_type: "number".to_string(), description: "Intensity 0.0–10.0 (default: 2.0). Higher = more pronounced effect".to_string(), items: None }),
                        ("clip".to_string(), PropertyDefinition { prop_type: "boolean".to_string(), description: "Clip output to prevent distortion (default: false)".to_string(), items: None }),
                    ]),
                    required: vec!["input_file".to_string(), "output_file".to_string()],
                },
            },

            ClaudeTool {
                name: "multiband_compress".to_string(),
                description: "Applies multiband dynamic range compression using FFmpeg mcompand. Splits audio into frequency bands (separated by crossover frequencies) and compresses each independently. More transparent than single-band compression for music mastering, broadcast, or podcast finishing.".to_string(),
                input_schema: InputSchema {
                    schema_type: "object".to_string(),
                    properties: HashMap::from([
                        ("input_file".to_string(), PropertyDefinition { prop_type: "string".to_string(), description: "Path to input audio/video file".to_string(), items: None }),
                        ("output_file".to_string(), PropertyDefinition { prop_type: "string".to_string(), description: "Path for output file".to_string(), items: None }),
                        ("params".to_string(), PropertyDefinition { prop_type: "string".to_string(), description: "mcompand band spec string. Format: 'attacks decays points gain [crossover_freq attacks decays points gain ...]'. Example: '0.005 0.1 -47/-40 -34/-34 -17/-17 0/0 2 500 0.003 0.05 -47/-40 -34/-34 -17/-17 0/0 2'".to_string(), items: None }),
                    ]),
                    required: vec!["input_file".to_string(), "output_file".to_string()],
                },
            },

            ClaudeTool {
                name: "apply_super_equalizer".to_string(),
                description: "Applies an 18-band graphic equalizer using FFmpeg superequalizer. Bands: 65Hz, 92Hz, 131Hz, 185Hz, 262Hz, 370Hz, 523Hz, 740Hz, 1kHz, 1.5kHz, 2kHz, 3kHz, 4.4kHz, 6.2kHz, 8.8kHz, 12.5kHz, 17.5kHz, 24kHz. Gain range 0.0–20.0 (unity = 10.0).".to_string(),
                input_schema: InputSchema {
                    schema_type: "object".to_string(),
                    properties: HashMap::from([
                        ("input_file".to_string(), PropertyDefinition { prop_type: "string".to_string(), description: "Path to input audio/video file".to_string(), items: None }),
                        ("output_file".to_string(), PropertyDefinition { prop_type: "string".to_string(), description: "Path for output file".to_string(), items: None }),
                        ("bands".to_string(), PropertyDefinition { prop_type: "string".to_string(), description: "18-band gain string (default: all 10.0). Format: '1b=V:2b=V:3b=V:...:18b=V' where V is 0.0–20.0 and 10.0 is unity (no change). Example to boost bass: '1b=14:2b=13:3b=12:4b=11:5b=10:6b=10:7b=10:8b=10:9b=10:10b=10:11b=10:12b=10:13b=10:14b=10:15b=10:16b=10:17b=10:18b=10'".to_string(), items: None }),
                    ]),
                    required: vec!["input_file".to_string(), "output_file".to_string()],
                },
            },

            // ================================================================
            // PHASE I BATCH 6 — colormatrix, chromashift, cas, nlmeans_video, spp, pp,
            //                   mestimate, midequalizer, median_spatial,
            //                   acrusher, atempo, asetnsamples, apad, asubcut, asupercut
            // ================================================================

            ClaudeTool {
                name: "apply_colormatrix".to_string(),
                description: "Converts video between colour matrix standards (bt601, bt709, smpte240m, fcc, bt2020) using FFmpeg colormatrix. Use to fix colour space metadata mismatches.".to_string(),
                input_schema: InputSchema {
                    schema_type: "object".to_string(),
                    properties: HashMap::from([
                        ("input_file".to_string(), PropertyDefinition { prop_type: "string".to_string(), description: "Path to input video".to_string(), items: None }),
                        ("output_file".to_string(), PropertyDefinition { prop_type: "string".to_string(), description: "Path for output video".to_string(), items: None }),
                        ("src".to_string(), PropertyDefinition { prop_type: "string".to_string(), description: "Source colour matrix: bt601, bt709, smpte240m, fcc, bt2020 (default bt601)".to_string(), items: None }),
                        ("dst".to_string(), PropertyDefinition { prop_type: "string".to_string(), description: "Target colour matrix: bt601, bt709, smpte240m, fcc, bt2020 (default bt709)".to_string(), items: None }),
                    ]),
                    required: vec!["input_file".to_string(), "output_file".to_string()],
                },
            },

            ClaudeTool {
                name: "apply_chromashift".to_string(),
                description: "Shifts chroma (colour) channels horizontally and vertically for chromatic aberration or colour-fringing effects using FFmpeg chromashift.".to_string(),
                input_schema: InputSchema {
                    schema_type: "object".to_string(),
                    properties: HashMap::from([
                        ("input_file".to_string(), PropertyDefinition { prop_type: "string".to_string(), description: "Path to input video".to_string(), items: None }),
                        ("output_file".to_string(), PropertyDefinition { prop_type: "string".to_string(), description: "Path for output video".to_string(), items: None }),
                        ("cbh".to_string(), PropertyDefinition { prop_type: "number".to_string(), description: "Cb channel horizontal shift in pixels (negative = left, default 0)".to_string(), items: None }),
                        ("cbv".to_string(), PropertyDefinition { prop_type: "number".to_string(), description: "Cb channel vertical shift (default 0)".to_string(), items: None }),
                        ("crh".to_string(), PropertyDefinition { prop_type: "number".to_string(), description: "Cr channel horizontal shift (default 0)".to_string(), items: None }),
                        ("crv".to_string(), PropertyDefinition { prop_type: "number".to_string(), description: "Cr channel vertical shift (default 0)".to_string(), items: None }),
                    ]),
                    required: vec!["input_file".to_string(), "output_file".to_string()],
                },
            },

            ClaudeTool {
                name: "apply_cas".to_string(),
                description: "Applies Contrast Adaptive Sharpening (CAS) — AMD FidelityFX-style adaptive sharpening that boosts local detail without halos.".to_string(),
                input_schema: InputSchema {
                    schema_type: "object".to_string(),
                    properties: HashMap::from([
                        ("input_file".to_string(), PropertyDefinition { prop_type: "string".to_string(), description: "Path to input video".to_string(), items: None }),
                        ("output_file".to_string(), PropertyDefinition { prop_type: "string".to_string(), description: "Path for output video".to_string(), items: None }),
                        ("strength".to_string(), PropertyDefinition { prop_type: "number".to_string(), description: "Sharpening strength (0.0–1.0, default 0.0)".to_string(), items: None }),
                        ("planes".to_string(), PropertyDefinition { prop_type: "number".to_string(), description: "Bitmask of planes to process (default 7 = YUV luma+chroma)".to_string(), items: None }),
                    ]),
                    required: vec!["input_file".to_string(), "output_file".to_string()],
                },
            },

            ClaudeTool {
                name: "apply_nlmeans_video".to_string(),
                description: "Applies Non-Local Means denoising (nlmeans) for high-quality video noise reduction by comparing patch similarity across the frame.".to_string(),
                input_schema: InputSchema {
                    schema_type: "object".to_string(),
                    properties: HashMap::from([
                        ("input_file".to_string(), PropertyDefinition { prop_type: "string".to_string(), description: "Path to input video".to_string(), items: None }),
                        ("output_file".to_string(), PropertyDefinition { prop_type: "string".to_string(), description: "Path for output video".to_string(), items: None }),
                        ("s".to_string(), PropertyDefinition { prop_type: "number".to_string(), description: "Denoising strength (default 1.0; higher = more smoothing)".to_string(), items: None }),
                        ("p".to_string(), PropertyDefinition { prop_type: "number".to_string(), description: "Patch size radius in pixels (default 3 → 7x7 patch)".to_string(), items: None }),
                        ("pc".to_string(), PropertyDefinition { prop_type: "number".to_string(), description: "Chroma patch size radius (default = luma p)".to_string(), items: None }),
                        ("r".to_string(), PropertyDefinition { prop_type: "number".to_string(), description: "Research window radius (default 7 → 15x15 search area)".to_string(), items: None }),
                        ("rc".to_string(), PropertyDefinition { prop_type: "number".to_string(), description: "Chroma research window radius (default = luma r)".to_string(), items: None }),
                    ]),
                    required: vec!["input_file".to_string(), "output_file".to_string()],
                },
            },

            ClaudeTool {
                name: "apply_spp".to_string(),
                description: "Applies Simple Post-Processing (spp) — DCT-based deblocking and denoising filter, good for cleaning up compressed video artefacts.".to_string(),
                input_schema: InputSchema {
                    schema_type: "object".to_string(),
                    properties: HashMap::from([
                        ("input_file".to_string(), PropertyDefinition { prop_type: "string".to_string(), description: "Path to input video".to_string(), items: None }),
                        ("output_file".to_string(), PropertyDefinition { prop_type: "string".to_string(), description: "Path for output video".to_string(), items: None }),
                        ("quality".to_string(), PropertyDefinition { prop_type: "number".to_string(), description: "Filter quality 1–6 (higher = slower but better, default 3)".to_string(), items: None }),
                        ("qp".to_string(), PropertyDefinition { prop_type: "number".to_string(), description: "Fixed QP value for strength (0 = use from stream, default 0)".to_string(), items: None }),
                        ("mode".to_string(), PropertyDefinition { prop_type: "string".to_string(), description: "Threshold mode: hard or soft (default hard)".to_string(), items: None }),
                    ]),
                    required: vec!["input_file".to_string(), "output_file".to_string()],
                },
            },

            ClaudeTool {
                name: "apply_pp".to_string(),
                description: "Applies FFmpeg pp (postprocess) filter — a collection of deblocking, deringing, and noise-reduction subfilters. Specify subfilters like 'hb/vb/dr' or 'default'.".to_string(),
                input_schema: InputSchema {
                    schema_type: "object".to_string(),
                    properties: HashMap::from([
                        ("input_file".to_string(), PropertyDefinition { prop_type: "string".to_string(), description: "Path to input video".to_string(), items: None }),
                        ("output_file".to_string(), PropertyDefinition { prop_type: "string".to_string(), description: "Path for output video".to_string(), items: None }),
                        ("subfilters".to_string(), PropertyDefinition { prop_type: "string".to_string(), description: "Subfilter string e.g. 'hb/vb/dr' (horiz/vert deblock + derinig) or 'default'. See FFmpeg pp docs for options.".to_string(), items: None }),
                    ]),
                    required: vec!["input_file".to_string(), "output_file".to_string()],
                },
            },

            ClaudeTool {
                name: "apply_mestimate".to_string(),
                description: "Estimates and visualises motion vectors using FFmpeg mestimate filter — useful for motion analysis and optical flow visualisation.".to_string(),
                input_schema: InputSchema {
                    schema_type: "object".to_string(),
                    properties: HashMap::from([
                        ("input_file".to_string(), PropertyDefinition { prop_type: "string".to_string(), description: "Path to input video".to_string(), items: None }),
                        ("output_file".to_string(), PropertyDefinition { prop_type: "string".to_string(), description: "Path for output video".to_string(), items: None }),
                        ("method".to_string(), PropertyDefinition { prop_type: "string".to_string(), description: "Motion estimation method: esa, tss, tdls, ntss, fss, ds, hexbs, epzs, umh (default esa)".to_string(), items: None }),
                        ("mb_size".to_string(), PropertyDefinition { prop_type: "number".to_string(), description: "Macroblock size (default 16)".to_string(), items: None }),
                        ("search_param".to_string(), PropertyDefinition { prop_type: "number".to_string(), description: "Search parameter (default 7)".to_string(), items: None }),
                    ]),
                    required: vec!["input_file".to_string(), "output_file".to_string()],
                },
            },

            ClaudeTool {
                name: "apply_midequalizer".to_string(),
                description: "Matches midtone exposure between two video streams using midequalizer — useful for colour-matching shots from different cameras.".to_string(),
                input_schema: InputSchema {
                    schema_type: "object".to_string(),
                    properties: HashMap::from([
                        ("input_file".to_string(), PropertyDefinition { prop_type: "string".to_string(), description: "Path to the reference video (first input)".to_string(), items: None }),
                        ("secondary_file".to_string(), PropertyDefinition { prop_type: "string".to_string(), description: "Path to the video to be matched (second input)".to_string(), items: None }),
                        ("output_file".to_string(), PropertyDefinition { prop_type: "string".to_string(), description: "Path for output video".to_string(), items: None }),
                        ("planes".to_string(), PropertyDefinition { prop_type: "number".to_string(), description: "Bitmask of planes to equalise (default 15)".to_string(), items: None }),
                    ]),
                    required: vec!["input_file".to_string(), "secondary_file".to_string(), "output_file".to_string()],
                },
            },

            ClaudeTool {
                name: "apply_median_spatial".to_string(),
                description: "Applies a spatio-temporal median filter across frames to remove outlier pixels (impulse noise, specks). More powerful than a single-frame median.".to_string(),
                input_schema: InputSchema {
                    schema_type: "object".to_string(),
                    properties: HashMap::from([
                        ("input_file".to_string(), PropertyDefinition { prop_type: "string".to_string(), description: "Path to input video".to_string(), items: None }),
                        ("output_file".to_string(), PropertyDefinition { prop_type: "string".to_string(), description: "Path for output video".to_string(), items: None }),
                        ("radius".to_string(), PropertyDefinition { prop_type: "number".to_string(), description: "Spatial filter radius (1–127, default 1 → 3x3)".to_string(), items: None }),
                        ("radiusV".to_string(), PropertyDefinition { prop_type: "number".to_string(), description: "Vertical radius (default = radius)".to_string(), items: None }),
                        ("percentile".to_string(), PropertyDefinition { prop_type: "number".to_string(), description: "Percentile of sorted values to use (0.0–1.0, default 0.5 = median)".to_string(), items: None }),
                        ("planes".to_string(), PropertyDefinition { prop_type: "number".to_string(), description: "Bitmask of planes (default 15)".to_string(), items: None }),
                    ]),
                    required: vec!["input_file".to_string(), "output_file".to_string()],
                },
            },

            ClaudeTool {
                name: "apply_acrusher".to_string(),
                description: "Applies bit-crusher/lo-fi distortion using FFmpeg acrusher — reduces bit depth and sample quality for lo-fi, vintage, or glitch-art audio effects.".to_string(),
                input_schema: InputSchema {
                    schema_type: "object".to_string(),
                    properties: HashMap::from([
                        ("input_file".to_string(), PropertyDefinition { prop_type: "string".to_string(), description: "Path to input audio/video".to_string(), items: None }),
                        ("output_file".to_string(), PropertyDefinition { prop_type: "string".to_string(), description: "Path for output file".to_string(), items: None }),
                        ("level_in".to_string(), PropertyDefinition { prop_type: "number".to_string(), description: "Input gain (default 1.0)".to_string(), items: None }),
                        ("level_out".to_string(), PropertyDefinition { prop_type: "number".to_string(), description: "Output gain (default 1.0)".to_string(), items: None }),
                        ("bits".to_string(), PropertyDefinition { prop_type: "number".to_string(), description: "Target bit depth (1–64, default 8)".to_string(), items: None }),
                        ("mix".to_string(), PropertyDefinition { prop_type: "number".to_string(), description: "Dry/wet mix (0.0–1.0, default 0.5)".to_string(), items: None }),
                        ("mode".to_string(), PropertyDefinition { prop_type: "string".to_string(), description: "lin or log quantisation mode (default log)".to_string(), items: None }),
                        ("dc".to_string(), PropertyDefinition { prop_type: "number".to_string(), description: "DC bias (default 1.0)".to_string(), items: None }),
                        ("aa".to_string(), PropertyDefinition { prop_type: "number".to_string(), description: "Antialiasing (0.0–1.0, default 0.5)".to_string(), items: None }),
                        ("samples".to_string(), PropertyDefinition { prop_type: "number".to_string(), description: "Sample reduction factor (default 1.0)".to_string(), items: None }),
                        ("lfo".to_string(), PropertyDefinition { prop_type: "boolean".to_string(), description: "Enable LFO modulation on bit depth (default false)".to_string(), items: None }),
                        ("lforange".to_string(), PropertyDefinition { prop_type: "number".to_string(), description: "LFO range in bits (default 20.0)".to_string(), items: None }),
                        ("lforate".to_string(), PropertyDefinition { prop_type: "number".to_string(), description: "LFO rate in Hz (default 0.3)".to_string(), items: None }),
                    ]),
                    required: vec!["input_file".to_string(), "output_file".to_string()],
                },
            },

            ClaudeTool {
                name: "apply_atempo".to_string(),
                description: "Changes audio playback speed/tempo without altering pitch using FFmpeg atempo. Supports 0.5x–100x; chains filters automatically for extreme values.".to_string(),
                input_schema: InputSchema {
                    schema_type: "object".to_string(),
                    properties: HashMap::from([
                        ("input_file".to_string(), PropertyDefinition { prop_type: "string".to_string(), description: "Path to input audio/video".to_string(), items: None }),
                        ("output_file".to_string(), PropertyDefinition { prop_type: "string".to_string(), description: "Path for output file".to_string(), items: None }),
                        ("tempo".to_string(), PropertyDefinition { prop_type: "number".to_string(), description: "Speed multiplier: 0.5 = half speed, 2.0 = double speed (0.5–100, default 1.0)".to_string(), items: None }),
                    ]),
                    required: vec!["input_file".to_string(), "output_file".to_string(), "tempo".to_string()],
                },
            },

            ClaudeTool {
                name: "apply_asetnsamples".to_string(),
                description: "Sets a fixed number of audio samples per output frame using asetnsamples — ensures consistent frame sizes for downstream processing.".to_string(),
                input_schema: InputSchema {
                    schema_type: "object".to_string(),
                    properties: HashMap::from([
                        ("input_file".to_string(), PropertyDefinition { prop_type: "string".to_string(), description: "Path to input audio/video".to_string(), items: None }),
                        ("output_file".to_string(), PropertyDefinition { prop_type: "string".to_string(), description: "Path for output file".to_string(), items: None }),
                        ("nb_samples".to_string(), PropertyDefinition { prop_type: "number".to_string(), description: "Number of samples per output frame (default 1024)".to_string(), items: None }),
                        ("pad".to_string(), PropertyDefinition { prop_type: "boolean".to_string(), description: "Pad last frame with silence if needed (default true)".to_string(), items: None }),
                    ]),
                    required: vec!["input_file".to_string(), "output_file".to_string()],
                },
            },

            ClaudeTool {
                name: "apply_apad".to_string(),
                description: "Pads audio with silence at the end using apad — useful for ensuring a minimum output duration or adding a tail of silence after content.".to_string(),
                input_schema: InputSchema {
                    schema_type: "object".to_string(),
                    properties: HashMap::from([
                        ("input_file".to_string(), PropertyDefinition { prop_type: "string".to_string(), description: "Path to input audio/video".to_string(), items: None }),
                        ("output_file".to_string(), PropertyDefinition { prop_type: "string".to_string(), description: "Path for output file".to_string(), items: None }),
                        ("pad_dur".to_string(), PropertyDefinition { prop_type: "number".to_string(), description: "Duration of silence to add in seconds (default 0)".to_string(), items: None }),
                        ("whole_dur".to_string(), PropertyDefinition { prop_type: "number".to_string(), description: "Pad until total duration reaches this value in seconds (default 0)".to_string(), items: None }),
                        ("pad_len".to_string(), PropertyDefinition { prop_type: "number".to_string(), description: "Number of samples to add (default 0, overridden by pad_dur)".to_string(), items: None }),
                        ("whole_len".to_string(), PropertyDefinition { prop_type: "number".to_string(), description: "Pad until total sample count reaches this (default 0)".to_string(), items: None }),
                    ]),
                    required: vec!["input_file".to_string(), "output_file".to_string()],
                },
            },

            ClaudeTool {
                name: "apply_asubcut".to_string(),
                description: "Cuts sub-bass frequencies below a cutoff frequency using asubcut high-pass filter — removes rumble, HVAC noise, and low-end handling noise.".to_string(),
                input_schema: InputSchema {
                    schema_type: "object".to_string(),
                    properties: HashMap::from([
                        ("input_file".to_string(), PropertyDefinition { prop_type: "string".to_string(), description: "Path to input audio/video".to_string(), items: None }),
                        ("output_file".to_string(), PropertyDefinition { prop_type: "string".to_string(), description: "Path for output file".to_string(), items: None }),
                        ("cutoff".to_string(), PropertyDefinition { prop_type: "number".to_string(), description: "Cutoff frequency in Hz (default 20.0)".to_string(), items: None }),
                        ("order".to_string(), PropertyDefinition { prop_type: "number".to_string(), description: "Filter order 3–20 (higher = steeper rolloff, default 10)".to_string(), items: None }),
                        ("level".to_string(), PropertyDefinition { prop_type: "number".to_string(), description: "Output level (default 1.0)".to_string(), items: None }),
                    ]),
                    required: vec!["input_file".to_string(), "output_file".to_string()],
                },
            },

            ClaudeTool {
                name: "apply_asupercut".to_string(),
                description: "Cuts super-treble frequencies above a cutoff using asupercut low-pass filter — removes ultrasonic artefacts and reduces aliasing in high-frequency content.".to_string(),
                input_schema: InputSchema {
                    schema_type: "object".to_string(),
                    properties: HashMap::from([
                        ("input_file".to_string(), PropertyDefinition { prop_type: "string".to_string(), description: "Path to input audio/video".to_string(), items: None }),
                        ("output_file".to_string(), PropertyDefinition { prop_type: "string".to_string(), description: "Path for output file".to_string(), items: None }),
                        ("cutoff".to_string(), PropertyDefinition { prop_type: "number".to_string(), description: "Cutoff frequency in Hz (default 20000.0)".to_string(), items: None }),
                        ("order".to_string(), PropertyDefinition { prop_type: "number".to_string(), description: "Filter order 3–20 (default 10)".to_string(), items: None }),
                        ("level".to_string(), PropertyDefinition { prop_type: "number".to_string(), description: "Output level (default 1.0)".to_string(), items: None }),
                    ]),
                    required: vec!["input_file".to_string(), "output_file".to_string()],
                },
            },

            // ================================================================
            // PHASE I BATCH 5 — threshold, maskedclamp, roberts, sobel, prewitt, kirsch,
            //                   video_limiter, bilateral, unsharp_mask, lagfun, tinterlace,
            //                   datascope, fspp, haas, aemphasis
            // ================================================================

            ClaudeTool {
                name: "apply_threshold".to_string(),
                description: "Applies pixel-value thresholding to a video — pixels below a floor clip to black, above a ceiling clip to max. Useful for creating high-contrast or graphic looks.".to_string(),
                input_schema: InputSchema {
                    schema_type: "object".to_string(),
                    properties: HashMap::from([
                        ("input_file".to_string(), PropertyDefinition { prop_type: "string".to_string(), description: "Path to the input video file".to_string(), items: None }),
                        ("output_file".to_string(), PropertyDefinition { prop_type: "string".to_string(), description: "Path for the output video file".to_string(), items: None }),
                        ("planes".to_string(), PropertyDefinition { prop_type: "number".to_string(), description: "Bitmask of planes to process (default 15 = all RGBA)".to_string(), items: None }),
                    ]),
                    required: vec!["input_file".to_string(), "output_file".to_string()],
                },
            },

            ClaudeTool {
                name: "apply_maskedclamp".to_string(),
                description: "Clamps each pixel of the input video between a dark and bright reference stream using FFmpeg maskedclamp filter. Useful for HDR tone mapping workflows and masking-based operations.".to_string(),
                input_schema: InputSchema {
                    schema_type: "object".to_string(),
                    properties: HashMap::from([
                        ("input_file".to_string(), PropertyDefinition { prop_type: "string".to_string(), description: "Path to the input video file".to_string(), items: None }),
                        ("output_file".to_string(), PropertyDefinition { prop_type: "string".to_string(), description: "Path for the output video file".to_string(), items: None }),
                        ("undershoot".to_string(), PropertyDefinition { prop_type: "number".to_string(), description: "Allowed undershoot below minimum (default 0)".to_string(), items: None }),
                        ("overshoot".to_string(), PropertyDefinition { prop_type: "number".to_string(), description: "Allowed overshoot above maximum (default 0)".to_string(), items: None }),
                        ("planes".to_string(), PropertyDefinition { prop_type: "number".to_string(), description: "Bitmask of planes to process (default 15)".to_string(), items: None }),
                    ]),
                    required: vec!["input_file".to_string(), "output_file".to_string()],
                },
            },

            ClaudeTool {
                name: "apply_roberts".to_string(),
                description: "Applies Roberts cross edge detection — highlights sharp edges/transitions in video using the Roberts operator. Good for stylised or graphic effects.".to_string(),
                input_schema: InputSchema {
                    schema_type: "object".to_string(),
                    properties: HashMap::from([
                        ("input_file".to_string(), PropertyDefinition { prop_type: "string".to_string(), description: "Path to the input video file".to_string(), items: None }),
                        ("output_file".to_string(), PropertyDefinition { prop_type: "string".to_string(), description: "Path for the output video file".to_string(), items: None }),
                        ("planes".to_string(), PropertyDefinition { prop_type: "number".to_string(), description: "Bitmask of planes to apply (default 15)".to_string(), items: None }),
                        ("scale".to_string(), PropertyDefinition { prop_type: "number".to_string(), description: "Amplification scale (default 1.0)".to_string(), items: None }),
                        ("delta".to_string(), PropertyDefinition { prop_type: "number".to_string(), description: "Offset added to output (default 0.0)".to_string(), items: None }),
                    ]),
                    required: vec!["input_file".to_string(), "output_file".to_string()],
                },
            },

            ClaudeTool {
                name: "apply_sobel".to_string(),
                description: "Applies Sobel edge detection — detects edges in video using horizontal and vertical Sobel gradients. Popular for motion graphics and visual effects.".to_string(),
                input_schema: InputSchema {
                    schema_type: "object".to_string(),
                    properties: HashMap::from([
                        ("input_file".to_string(), PropertyDefinition { prop_type: "string".to_string(), description: "Path to the input video file".to_string(), items: None }),
                        ("output_file".to_string(), PropertyDefinition { prop_type: "string".to_string(), description: "Path for the output video file".to_string(), items: None }),
                        ("planes".to_string(), PropertyDefinition { prop_type: "number".to_string(), description: "Bitmask of planes (default 15)".to_string(), items: None }),
                        ("scale".to_string(), PropertyDefinition { prop_type: "number".to_string(), description: "Amplification scale (default 1.0)".to_string(), items: None }),
                        ("delta".to_string(), PropertyDefinition { prop_type: "number".to_string(), description: "Offset added to output (default 0.0)".to_string(), items: None }),
                    ]),
                    required: vec!["input_file".to_string(), "output_file".to_string()],
                },
            },

            ClaudeTool {
                name: "apply_prewitt".to_string(),
                description: "Applies Prewitt edge detection operator — extracts edges from video using the Prewitt gradient method.".to_string(),
                input_schema: InputSchema {
                    schema_type: "object".to_string(),
                    properties: HashMap::from([
                        ("input_file".to_string(), PropertyDefinition { prop_type: "string".to_string(), description: "Path to the input video file".to_string(), items: None }),
                        ("output_file".to_string(), PropertyDefinition { prop_type: "string".to_string(), description: "Path for the output video file".to_string(), items: None }),
                        ("planes".to_string(), PropertyDefinition { prop_type: "number".to_string(), description: "Bitmask of planes (default 15)".to_string(), items: None }),
                        ("scale".to_string(), PropertyDefinition { prop_type: "number".to_string(), description: "Amplification scale (default 1.0)".to_string(), items: None }),
                        ("delta".to_string(), PropertyDefinition { prop_type: "number".to_string(), description: "Offset (default 0.0)".to_string(), items: None }),
                    ]),
                    required: vec!["input_file".to_string(), "output_file".to_string()],
                },
            },

            ClaudeTool {
                name: "apply_kirsch".to_string(),
                description: "Applies Kirsch edge detection — uses the Kirsch compass kernel to detect edges in 8 orientations, producing clean edge maps.".to_string(),
                input_schema: InputSchema {
                    schema_type: "object".to_string(),
                    properties: HashMap::from([
                        ("input_file".to_string(), PropertyDefinition { prop_type: "string".to_string(), description: "Path to the input video file".to_string(), items: None }),
                        ("output_file".to_string(), PropertyDefinition { prop_type: "string".to_string(), description: "Path for the output video file".to_string(), items: None }),
                        ("planes".to_string(), PropertyDefinition { prop_type: "number".to_string(), description: "Bitmask of planes (default 15)".to_string(), items: None }),
                        ("scale".to_string(), PropertyDefinition { prop_type: "number".to_string(), description: "Amplification scale (default 1.0)".to_string(), items: None }),
                        ("delta".to_string(), PropertyDefinition { prop_type: "number".to_string(), description: "Offset (default 0.0)".to_string(), items: None }),
                    ]),
                    required: vec!["input_file".to_string(), "output_file".to_string()],
                },
            },

            ClaudeTool {
                name: "apply_video_limiter".to_string(),
                description: "Clamps video pixel values to a [min, max] range using FFmpeg limiter filter. Useful for broadcast-legal signal range enforcement (e.g. 16-235 for Y′CbCr).".to_string(),
                input_schema: InputSchema {
                    schema_type: "object".to_string(),
                    properties: HashMap::from([
                        ("input_file".to_string(), PropertyDefinition { prop_type: "string".to_string(), description: "Path to the input video file".to_string(), items: None }),
                        ("output_file".to_string(), PropertyDefinition { prop_type: "string".to_string(), description: "Path for the output video file".to_string(), items: None }),
                        ("min".to_string(), PropertyDefinition { prop_type: "number".to_string(), description: "Minimum pixel value (0-65535, default 0)".to_string(), items: None }),
                        ("max".to_string(), PropertyDefinition { prop_type: "number".to_string(), description: "Maximum pixel value (0-65535, default 65535)".to_string(), items: None }),
                        ("planes".to_string(), PropertyDefinition { prop_type: "number".to_string(), description: "Bitmask of planes to process (default 15)".to_string(), items: None }),
                    ]),
                    required: vec!["input_file".to_string(), "output_file".to_string()],
                },
            },

            ClaudeTool {
                name: "apply_bilateral".to_string(),
                description: "Applies bilateral filter for edge-preserving noise reduction — smooths uniform areas while keeping sharp edges intact. Great for skin smoothing and denoise without losing detail.".to_string(),
                input_schema: InputSchema {
                    schema_type: "object".to_string(),
                    properties: HashMap::from([
                        ("input_file".to_string(), PropertyDefinition { prop_type: "string".to_string(), description: "Path to the input video file".to_string(), items: None }),
                        ("output_file".to_string(), PropertyDefinition { prop_type: "string".to_string(), description: "Path for the output video file".to_string(), items: None }),
                        ("sigmaS".to_string(), PropertyDefinition { prop_type: "number".to_string(), description: "Spatial sigma — controls how far nearby pixels influence the filter (0.1–512, default 0.1)".to_string(), items: None }),
                        ("sigmaR".to_string(), PropertyDefinition { prop_type: "number".to_string(), description: "Range sigma — controls how similar pixels must be in colour (0.1–1.0, default 0.1)".to_string(), items: None }),
                        ("planes".to_string(), PropertyDefinition { prop_type: "number".to_string(), description: "Bitmask of planes to filter (default 1 = luma only)".to_string(), items: None }),
                    ]),
                    required: vec!["input_file".to_string(), "output_file".to_string()],
                },
            },

            ClaudeTool {
                name: "apply_unsharp_mask".to_string(),
                description: "Applies unsharp mask for precise sharpening or blurring of luma and chroma planes independently. More controlled than simple sharpen — adjust luma and chroma kernel sizes and amounts.".to_string(),
                input_schema: InputSchema {
                    schema_type: "object".to_string(),
                    properties: HashMap::from([
                        ("input_file".to_string(), PropertyDefinition { prop_type: "string".to_string(), description: "Path to the input video file".to_string(), items: None }),
                        ("output_file".to_string(), PropertyDefinition { prop_type: "string".to_string(), description: "Path for the output video file".to_string(), items: None }),
                        ("luma_x".to_string(), PropertyDefinition { prop_type: "number".to_string(), description: "Luma kernel width in pixels (odd, 3–23, default 5)".to_string(), items: None }),
                        ("luma_y".to_string(), PropertyDefinition { prop_type: "number".to_string(), description: "Luma kernel height in pixels (odd, 3–23, default 5)".to_string(), items: None }),
                        ("luma_amount".to_string(), PropertyDefinition { prop_type: "number".to_string(), description: "Luma effect amount (-1.5–1.5; positive=sharpen, negative=blur, default 1.0)".to_string(), items: None }),
                        ("chroma_x".to_string(), PropertyDefinition { prop_type: "number".to_string(), description: "Chroma kernel width (odd, 3–23, default 5)".to_string(), items: None }),
                        ("chroma_y".to_string(), PropertyDefinition { prop_type: "number".to_string(), description: "Chroma kernel height (odd, 3–23, default 5)".to_string(), items: None }),
                        ("chroma_amount".to_string(), PropertyDefinition { prop_type: "number".to_string(), description: "Chroma effect amount (default 0.0)".to_string(), items: None }),
                    ]),
                    required: vec!["input_file".to_string(), "output_file".to_string()],
                },
            },

            ClaudeTool {
                name: "apply_lagfun".to_string(),
                description: "Applies lagfun exponential moving average — creates slow-motion ghost trails or motion blur effect by blending frames over time.".to_string(),
                input_schema: InputSchema {
                    schema_type: "object".to_string(),
                    properties: HashMap::from([
                        ("input_file".to_string(), PropertyDefinition { prop_type: "string".to_string(), description: "Path to the input video file".to_string(), items: None }),
                        ("output_file".to_string(), PropertyDefinition { prop_type: "string".to_string(), description: "Path for the output video file".to_string(), items: None }),
                        ("decay".to_string(), PropertyDefinition { prop_type: "number".to_string(), description: "EMA decay factor (0.0–1.0; higher=longer trail, default 0.95)".to_string(), items: None }),
                        ("planes".to_string(), PropertyDefinition { prop_type: "number".to_string(), description: "Bitmask of planes (default 15)".to_string(), items: None }),
                    ]),
                    required: vec!["input_file".to_string(), "output_file".to_string()],
                },
            },

            ClaudeTool {
                name: "apply_tinterlace".to_string(),
                description: "Applies temporal field interlacing modes for broadcast output using FFmpeg tinterlace filter. Converts progressive video to various interlaced formats.".to_string(),
                input_schema: InputSchema {
                    schema_type: "object".to_string(),
                    properties: HashMap::from([
                        ("input_file".to_string(), PropertyDefinition { prop_type: "string".to_string(), description: "Path to the input video file".to_string(), items: None }),
                        ("output_file".to_string(), PropertyDefinition { prop_type: "string".to_string(), description: "Path for the output video file".to_string(), items: None }),
                        ("mode".to_string(), PropertyDefinition { prop_type: "number".to_string(), description: "Interlace mode: 0=merge, 1=drop_even, 2=drop_odd, 3=pad, 4=interleave_top, 5=interleave_bottom, 6=interlacex2, 7=mergex2 (default 0)".to_string(), items: None }),
                        ("flags".to_string(), PropertyDefinition { prop_type: "string".to_string(), description: "Processing flags: vlpf=low-pass filter, cvlpf=chroma+luma LPF (default vlpf)".to_string(), items: None }),
                    ]),
                    required: vec!["input_file".to_string(), "output_file".to_string()],
                },
            },

            ClaudeTool {
                name: "apply_datascope".to_string(),
                description: "Renders a datascope overlay showing raw pixel values at a specified region — useful for colour accuracy analysis and broadcast QC.".to_string(),
                input_schema: InputSchema {
                    schema_type: "object".to_string(),
                    properties: HashMap::from([
                        ("input_file".to_string(), PropertyDefinition { prop_type: "string".to_string(), description: "Path to the input video file".to_string(), items: None }),
                        ("output_file".to_string(), PropertyDefinition { prop_type: "string".to_string(), description: "Path for the output video file".to_string(), items: None }),
                        ("size".to_string(), PropertyDefinition { prop_type: "string".to_string(), description: "Output size (e.g. hd720, 1280x720, default hd720)".to_string(), items: None }),
                        ("x".to_string(), PropertyDefinition { prop_type: "number".to_string(), description: "X position of the pixel region to inspect (default 0)".to_string(), items: None }),
                        ("y".to_string(), PropertyDefinition { prop_type: "number".to_string(), description: "Y position of the pixel region to inspect (default 0)".to_string(), items: None }),
                        ("mode".to_string(), PropertyDefinition { prop_type: "number".to_string(), description: "Display mode: 0=mono, 1=color, 2=color2 (default 0)".to_string(), items: None }),
                        ("opacity".to_string(), PropertyDefinition { prop_type: "number".to_string(), description: "Opacity of datascope overlay (0.0–1.0, default 0.75)".to_string(), items: None }),
                        ("axis".to_string(), PropertyDefinition { prop_type: "boolean".to_string(), description: "Show row/column axis labels (default false)".to_string(), items: None }),
                    ]),
                    required: vec!["input_file".to_string(), "output_file".to_string()],
                },
            },

            ClaudeTool {
                name: "apply_fspp".to_string(),
                description: "Applies Fast Super Pixel (fspp) frequency-domain denoising — removes noise while preserving fine detail via super-pixel blocks.".to_string(),
                input_schema: InputSchema {
                    schema_type: "object".to_string(),
                    properties: HashMap::from([
                        ("input_file".to_string(), PropertyDefinition { prop_type: "string".to_string(), description: "Path to the input video file".to_string(), items: None }),
                        ("output_file".to_string(), PropertyDefinition { prop_type: "string".to_string(), description: "Path for the output video file".to_string(), items: None }),
                        ("quality".to_string(), PropertyDefinition { prop_type: "number".to_string(), description: "Filter quality/iterations (4–5, default 4)".to_string(), items: None }),
                        ("strength".to_string(), PropertyDefinition { prop_type: "number".to_string(), description: "Denoising strength (-15–32, default 0 = auto from QP)".to_string(), items: None }),
                        ("use_bframe_qp".to_string(), PropertyDefinition { prop_type: "boolean".to_string(), description: "Use B-frame QP for strength calculation (default false)".to_string(), items: None }),
                    ]),
                    required: vec!["input_file".to_string(), "output_file".to_string()],
                },
            },

            ClaudeTool {
                name: "apply_haas".to_string(),
                description: "Applies Haas effect — creates stereo widening by introducing a slight delay on one channel, giving perception of space and width without comb filtering.".to_string(),
                input_schema: InputSchema {
                    schema_type: "object".to_string(),
                    properties: HashMap::from([
                        ("input_file".to_string(), PropertyDefinition { prop_type: "string".to_string(), description: "Path to the input audio/video file".to_string(), items: None }),
                        ("output_file".to_string(), PropertyDefinition { prop_type: "string".to_string(), description: "Path for the output file".to_string(), items: None }),
                        ("level_in".to_string(), PropertyDefinition { prop_type: "number".to_string(), description: "Input level gain (default 1.0)".to_string(), items: None }),
                        ("level_out".to_string(), PropertyDefinition { prop_type: "number".to_string(), description: "Output level gain (default 1.0)".to_string(), items: None }),
                        ("side_gain".to_string(), PropertyDefinition { prop_type: "number".to_string(), description: "Side channel gain (default 1.0)".to_string(), items: None }),
                        ("middle_source".to_string(), PropertyDefinition { prop_type: "string".to_string(), description: "Middle source: mid, left, right, side (default mid)".to_string(), items: None }),
                        ("middle_phase".to_string(), PropertyDefinition { prop_type: "boolean".to_string(), description: "Invert middle phase (default false)".to_string(), items: None }),
                        ("left_delay".to_string(), PropertyDefinition { prop_type: "number".to_string(), description: "Left channel delay in ms (0–40, default 2.5)".to_string(), items: None }),
                        ("left_balance".to_string(), PropertyDefinition { prop_type: "number".to_string(), description: "Left channel balance (-1.0–1.0, default -1.0)".to_string(), items: None }),
                        ("right_delay".to_string(), PropertyDefinition { prop_type: "number".to_string(), description: "Right channel delay in ms (0–40, default 2.5)".to_string(), items: None }),
                        ("right_balance".to_string(), PropertyDefinition { prop_type: "number".to_string(), description: "Right channel balance (-1.0–1.0, default 1.0)".to_string(), items: None }),
                    ]),
                    required: vec!["input_file".to_string(), "output_file".to_string()],
                },
            },

            ClaudeTool {
                name: "apply_aemphasis".to_string(),
                description: "Applies audio emphasis/de-emphasis curves (RIAA, CD, 50FM, 75FM etc) — reproduces or applies pre-emphasis used in vinyl/tape/FM recording standards.".to_string(),
                input_schema: InputSchema {
                    schema_type: "object".to_string(),
                    properties: HashMap::from([
                        ("input_file".to_string(), PropertyDefinition { prop_type: "string".to_string(), description: "Path to the input audio/video file".to_string(), items: None }),
                        ("output_file".to_string(), PropertyDefinition { prop_type: "string".to_string(), description: "Path for the output file".to_string(), items: None }),
                        ("level_in".to_string(), PropertyDefinition { prop_type: "number".to_string(), description: "Input gain (default 1.0)".to_string(), items: None }),
                        ("level_out".to_string(), PropertyDefinition { prop_type: "number".to_string(), description: "Output gain (default 1.0)".to_string(), items: None }),
                        ("mode".to_string(), PropertyDefinition { prop_type: "string".to_string(), description: "Mode: reproduction (remove pre-emphasis) or production (add pre-emphasis) — default reproduction".to_string(), items: None }),
                        ("emph_type".to_string(), PropertyDefinition { prop_type: "string".to_string(), description: "Emphasis curve: riaa, cd, 50fm, 75fm, 50kf, 75kf (default cd)".to_string(), items: None }),
                    ]),
                    required: vec!["input_file".to_string(), "output_file".to_string()],
                },
            },

            // PHASE I BATCH 4 — negate, pixelize, colorlevels, pseudocolor, colorhold,
            //                   shuffleplanes, blackdetect, idet, vstack, hstack,
            //                   setdar, stereo3d, telecine, pullup, thumbnail_select
            // ================================================================

            ClaudeTool {
                name: "apply_negate".to_string(),
                description: "Inverts video colours (negative) via FFmpeg negate filter. Can be applied to individual R/G/B channels independently.".to_string(),
                input_schema: InputSchema {
                    schema_type: "object".to_string(),
                    properties: HashMap::from([
                        ("input_file".to_string(), PropertyDefinition { prop_type: "string".to_string(), description: "Path to the input video file".to_string(), items: None }),
                        ("output_file".to_string(), PropertyDefinition { prop_type: "string".to_string(), description: "Path to save the negated output".to_string(), items: None }),
                        ("components".to_string(), PropertyDefinition { prop_type: "number".to_string(), description: "Bitmask of channels to negate: 1=R, 2=G, 4=B, 8=A. Default 7 (RGB). Use 15 to include alpha.".to_string(), items: None }),
                    ]),
                    required: vec!["input_file".to_string(), "output_file".to_string()],
                },
            },

            ClaudeTool {
                name: "apply_pixelize".to_string(),
                description: "Pixelates/mosaics video via FFmpeg pixelize filter. Creates a blocky pixelation effect used for censorship, privacy masking, or artistic style.".to_string(),
                input_schema: InputSchema {
                    schema_type: "object".to_string(),
                    properties: HashMap::from([
                        ("input_file".to_string(), PropertyDefinition { prop_type: "string".to_string(), description: "Path to the input video file".to_string(), items: None }),
                        ("output_file".to_string(), PropertyDefinition { prop_type: "string".to_string(), description: "Path to save the pixelated output".to_string(), items: None }),
                        ("width".to_string(), PropertyDefinition { prop_type: "number".to_string(), description: "Pixel block width. Default 16. Higher = more blocky.".to_string(), items: None }),
                        ("height".to_string(), PropertyDefinition { prop_type: "number".to_string(), description: "Pixel block height. Default same as width.".to_string(), items: None }),
                        ("mode".to_string(), PropertyDefinition { prop_type: "number".to_string(), description: "Mode: 0=average colour per block (default), 1=replicate top-left pixel.".to_string(), items: None }),
                    ]),
                    required: vec!["input_file".to_string(), "output_file".to_string()],
                },
            },

            ClaudeTool {
                name: "apply_colorlevels".to_string(),
                description: "Clips and remaps input/output colour levels per channel via FFmpeg colorlevels. Like the Levels tool in Photoshop — set black/white points to fix exposure and colour cast.".to_string(),
                input_schema: InputSchema {
                    schema_type: "object".to_string(),
                    properties: HashMap::from([
                        ("input_file".to_string(), PropertyDefinition { prop_type: "string".to_string(), description: "Path to the input video file".to_string(), items: None }),
                        ("output_file".to_string(), PropertyDefinition { prop_type: "string".to_string(), description: "Path to save the levels-adjusted output".to_string(), items: None }),
                        ("rimin".to_string(), PropertyDefinition { prop_type: "number".to_string(), description: "Red input minimum (0–1). Default 0. Raise to set black point.".to_string(), items: None }),
                        ("rimax".to_string(), PropertyDefinition { prop_type: "number".to_string(), description: "Red input maximum (0–1). Default 1. Lower to set white point.".to_string(), items: None }),
                        ("gimin".to_string(), PropertyDefinition { prop_type: "number".to_string(), description: "Green input minimum. Default 0.".to_string(), items: None }),
                        ("gimax".to_string(), PropertyDefinition { prop_type: "number".to_string(), description: "Green input maximum. Default 1.".to_string(), items: None }),
                        ("bimin".to_string(), PropertyDefinition { prop_type: "number".to_string(), description: "Blue input minimum. Default 0.".to_string(), items: None }),
                        ("bimax".to_string(), PropertyDefinition { prop_type: "number".to_string(), description: "Blue input maximum. Default 1.".to_string(), items: None }),
                        ("romin".to_string(), PropertyDefinition { prop_type: "number".to_string(), description: "Output minimum for all channels. Default 0.".to_string(), items: None }),
                        ("romax".to_string(), PropertyDefinition { prop_type: "number".to_string(), description: "Output maximum for all channels. Default 1.".to_string(), items: None }),
                    ]),
                    required: vec!["input_file".to_string(), "output_file".to_string()],
                },
            },

            ClaudeTool {
                name: "apply_pseudocolor".to_string(),
                description: "False-colour visualisation via FFmpeg pseudocolor filter. Maps luminance to a scientific colour palette (magma, inferno, plasma, viridis, turbo). Good for heat maps and analysis overlays.".to_string(),
                input_schema: InputSchema {
                    schema_type: "object".to_string(),
                    properties: HashMap::from([
                        ("input_file".to_string(), PropertyDefinition { prop_type: "string".to_string(), description: "Path to the input video file".to_string(), items: None }),
                        ("output_file".to_string(), PropertyDefinition { prop_type: "string".to_string(), description: "Path to save the false-colour output".to_string(), items: None }),
                        ("preset".to_string(), PropertyDefinition { prop_type: "number".to_string(), description: "Colour palette: 0=magma, 1=inferno, 2=plasma, 3=viridis, 4=turbo, 5=cividis, 6=range1, 7=range2, 8=shadows, 9=highlights. Default 0.".to_string(), items: None }),
                        ("opacity".to_string(), PropertyDefinition { prop_type: "number".to_string(), description: "Opacity of the effect (0–1). Default 1.0.".to_string(), items: None }),
                    ]),
                    required: vec!["input_file".to_string(), "output_file".to_string()],
                },
            },

            ClaudeTool {
                name: "apply_colorhold".to_string(),
                description: "Selective colour effect via FFmpeg colorhold: keeps one specified colour and desaturates everything else to greyscale. Classic movie-poster technique.".to_string(),
                input_schema: InputSchema {
                    schema_type: "object".to_string(),
                    properties: HashMap::from([
                        ("input_file".to_string(), PropertyDefinition { prop_type: "string".to_string(), description: "Path to the input video file".to_string(), items: None }),
                        ("output_file".to_string(), PropertyDefinition { prop_type: "string".to_string(), description: "Path to save the selective-colour output".to_string(), items: None }),
                        ("color".to_string(), PropertyDefinition { prop_type: "string".to_string(), description: "Colour to preserve. Any FFmpeg colour string: red, blue, #ff0000, etc. Default 'red'.".to_string(), items: None }),
                        ("similarity".to_string(), PropertyDefinition { prop_type: "number".to_string(), description: "Colour similarity range (0–1). Default 0.1. Higher = broader match.".to_string(), items: None }),
                        ("blend".to_string(), PropertyDefinition { prop_type: "number".to_string(), description: "Blend between greyscale and original at boundary (0–1). Default 0.".to_string(), items: None }),
                    ]),
                    required: vec!["input_file".to_string(), "output_file".to_string()],
                },
            },

            ClaudeTool {
                name: "apply_shuffleplanes".to_string(),
                description: "Reorders or duplicates video colour planes via FFmpeg shuffleplanes. Can swap R↔B for channel effects, create false colour, or isolate single channels.".to_string(),
                input_schema: InputSchema {
                    schema_type: "object".to_string(),
                    properties: HashMap::from([
                        ("input_file".to_string(), PropertyDefinition { prop_type: "string".to_string(), description: "Path to the input video file".to_string(), items: None }),
                        ("output_file".to_string(), PropertyDefinition { prop_type: "string".to_string(), description: "Path to save the shuffled output".to_string(), items: None }),
                        ("map0".to_string(), PropertyDefinition { prop_type: "number".to_string(), description: "Source plane index for output plane 0. Default 0.".to_string(), items: None }),
                        ("map1".to_string(), PropertyDefinition { prop_type: "number".to_string(), description: "Source plane index for output plane 1. Default 1.".to_string(), items: None }),
                        ("map2".to_string(), PropertyDefinition { prop_type: "number".to_string(), description: "Source plane index for output plane 2. Default 2. Set to 0 to swap R↔B.".to_string(), items: None }),
                        ("map3".to_string(), PropertyDefinition { prop_type: "number".to_string(), description: "Source plane index for output plane 3. Default 3.".to_string(), items: None }),
                    ]),
                    required: vec!["input_file".to_string(), "output_file".to_string()],
                },
            },

            ClaudeTool {
                name: "detect_black_frames".to_string(),
                description: "Detects black/near-black frames via FFmpeg blackdetect. Returns timestamps of black segments — useful for finding ad breaks, chapter boundaries, or tape leader. Analysis only.".to_string(),
                input_schema: InputSchema {
                    schema_type: "object".to_string(),
                    properties: HashMap::from([
                        ("input_file".to_string(), PropertyDefinition { prop_type: "string".to_string(), description: "Path to the input video file".to_string(), items: None }),
                        ("black_min_duration".to_string(), PropertyDefinition { prop_type: "number".to_string(), description: "Minimum duration of black segment in seconds. Default 2.0.".to_string(), items: None }),
                        ("picture_black_ratio_th".to_string(), PropertyDefinition { prop_type: "number".to_string(), description: "Fraction of frame that must be black (0–1). Default 0.98.".to_string(), items: None }),
                        ("pixel_black_th".to_string(), PropertyDefinition { prop_type: "number".to_string(), description: "Pixel luminance threshold below which it counts as black (0–1). Default 0.10.".to_string(), items: None }),
                    ]),
                    required: vec!["input_file".to_string()],
                },
            },

            ClaudeTool {
                name: "detect_interlace_type".to_string(),
                description: "Detects whether video is progressive, top-field-first (TFF), or bottom-field-first (BFF) via FFmpeg idet filter. Use before deinterlacing to choose the correct mode.".to_string(),
                input_schema: InputSchema {
                    schema_type: "object".to_string(),
                    properties: HashMap::from([
                        ("input_file".to_string(), PropertyDefinition { prop_type: "string".to_string(), description: "Path to the input video file".to_string(), items: None }),
                    ]),
                    required: vec!["input_file".to_string()],
                },
            },

            ClaudeTool {
                name: "apply_vstack".to_string(),
                description: "Stacks two videos vertically (one above the other) via FFmpeg vstack filter. Both clips must have the same width.".to_string(),
                input_schema: InputSchema {
                    schema_type: "object".to_string(),
                    properties: HashMap::from([
                        ("input_file".to_string(), PropertyDefinition { prop_type: "string".to_string(), description: "Path to the top video file".to_string(), items: None }),
                        ("secondary_file".to_string(), PropertyDefinition { prop_type: "string".to_string(), description: "Path to the bottom video file".to_string(), items: None }),
                        ("output_file".to_string(), PropertyDefinition { prop_type: "string".to_string(), description: "Path to save the stacked output".to_string(), items: None }),
                        ("shortest".to_string(), PropertyDefinition { prop_type: "boolean".to_string(), description: "End output when the shorter clip ends. Default false.".to_string(), items: None }),
                    ]),
                    required: vec!["input_file".to_string(), "secondary_file".to_string(), "output_file".to_string()],
                },
            },

            ClaudeTool {
                name: "apply_hstack".to_string(),
                description: "Stacks two videos horizontally (side by side) via FFmpeg hstack filter. Both clips must have the same height.".to_string(),
                input_schema: InputSchema {
                    schema_type: "object".to_string(),
                    properties: HashMap::from([
                        ("input_file".to_string(), PropertyDefinition { prop_type: "string".to_string(), description: "Path to the left video file".to_string(), items: None }),
                        ("secondary_file".to_string(), PropertyDefinition { prop_type: "string".to_string(), description: "Path to the right video file".to_string(), items: None }),
                        ("output_file".to_string(), PropertyDefinition { prop_type: "string".to_string(), description: "Path to save the side-by-side output".to_string(), items: None }),
                        ("shortest".to_string(), PropertyDefinition { prop_type: "boolean".to_string(), description: "End output when the shorter clip ends. Default false.".to_string(), items: None }),
                    ]),
                    required: vec!["input_file".to_string(), "secondary_file".to_string(), "output_file".to_string()],
                },
            },

            ClaudeTool {
                name: "apply_setdar".to_string(),
                description: "Sets the display aspect ratio (DAR) via FFmpeg setdar without re-encoding pixels. Corrects wrongly-tagged anamorphic or square-pixel footage.".to_string(),
                input_schema: InputSchema {
                    schema_type: "object".to_string(),
                    properties: HashMap::from([
                        ("input_file".to_string(), PropertyDefinition { prop_type: "string".to_string(), description: "Path to the input video file".to_string(), items: None }),
                        ("output_file".to_string(), PropertyDefinition { prop_type: "string".to_string(), description: "Path to save the DAR-corrected output".to_string(), items: None }),
                        ("dar".to_string(), PropertyDefinition { prop_type: "string".to_string(), description: "Display aspect ratio as fraction: '16/9', '4/3', '2.35', '1/1'. Default '16/9'.".to_string(), items: None }),
                    ]),
                    required: vec!["input_file".to_string(), "output_file".to_string()],
                },
            },

            ClaudeTool {
                name: "apply_stereo3d".to_string(),
                description: "Converts between stereoscopic 3D video formats via FFmpeg stereo3d. Supports side-by-side (SBS), over-under, anaglyphs (red-cyan), and mono extraction.".to_string(),
                input_schema: InputSchema {
                    schema_type: "object".to_string(),
                    properties: HashMap::from([
                        ("input_file".to_string(), PropertyDefinition { prop_type: "string".to_string(), description: "Path to the input stereo 3D video file".to_string(), items: None }),
                        ("output_file".to_string(), PropertyDefinition { prop_type: "string".to_string(), description: "Path to save the converted output".to_string(), items: None }),
                        ("input_format".to_string(), PropertyDefinition { prop_type: "string".to_string(), description: "Input 3D format: sbsl=SBS left-first, sbsr=SBS right-first, abl=above-below, abr=above-below right. Default 'sbsl'.".to_string(), items: None }),
                        ("output_format".to_string(), PropertyDefinition { prop_type: "string".to_string(), description: "Output 3D format: arcd=red-cyan anaglyph, ml=mono left, mr=mono right, sbsl, sbsr, abl. Default 'arcd'.".to_string(), items: None }),
                    ]),
                    required: vec!["input_file".to_string(), "output_file".to_string()],
                },
            },

            ClaudeTool {
                name: "apply_telecine".to_string(),
                description: "Applies 3:2 pulldown telecine (film-to-video) via FFmpeg telecine filter. Converts 24fps film content to 29.97fps broadcast standard by duplicating fields.".to_string(),
                input_schema: InputSchema {
                    schema_type: "object".to_string(),
                    properties: HashMap::from([
                        ("input_file".to_string(), PropertyDefinition { prop_type: "string".to_string(), description: "Path to the input 24fps film video".to_string(), items: None }),
                        ("output_file".to_string(), PropertyDefinition { prop_type: "string".to_string(), description: "Path to save the telecined 29.97fps output".to_string(), items: None }),
                        ("pattern".to_string(), PropertyDefinition { prop_type: "string".to_string(), description: "Pulldown pattern. Default '23' (3:2 pulldown). Can also be '2332' or other sequences.".to_string(), items: None }),
                        ("first_field".to_string(), PropertyDefinition { prop_type: "number".to_string(), description: "First field: 0=top (default), 1=bottom.".to_string(), items: None }),
                    ]),
                    required: vec!["input_file".to_string(), "output_file".to_string()],
                },
            },

            ClaudeTool {
                name: "apply_pullup".to_string(),
                description: "Removes 3:2 pulldown (inverse telecine / IVTC) via FFmpeg pullup filter. Recovers original 24fps from telecined 29.97fps broadcast content.".to_string(),
                input_schema: InputSchema {
                    schema_type: "object".to_string(),
                    properties: HashMap::from([
                        ("input_file".to_string(), PropertyDefinition { prop_type: "string".to_string(), description: "Path to the telecined 29.97fps input video".to_string(), items: None }),
                        ("output_file".to_string(), PropertyDefinition { prop_type: "string".to_string(), description: "Path to save the 24fps progressive output".to_string(), items: None }),
                    ]),
                    required: vec!["input_file".to_string(), "output_file".to_string()],
                },
            },

            ClaudeTool {
                name: "select_thumbnail_frame".to_string(),
                description: "Selects the best representative thumbnail frame from a video using FFmpeg thumbnail filter. Analyses N-frame batches and picks the most representative frame — better than a fixed timestamp.".to_string(),
                input_schema: InputSchema {
                    schema_type: "object".to_string(),
                    properties: HashMap::from([
                        ("input_file".to_string(), PropertyDefinition { prop_type: "string".to_string(), description: "Path to the input video file".to_string(), items: None }),
                        ("output_file".to_string(), PropertyDefinition { prop_type: "string".to_string(), description: "Path to save the thumbnail image (e.g. thumb.jpg)".to_string(), items: None }),
                        ("n".to_string(), PropertyDefinition { prop_type: "number".to_string(), description: "Batch size — analyse every N frames for the best one. Default 100.".to_string(), items: None }),
                    ]),
                    required: vec!["input_file".to_string(), "output_file".to_string()],
                },
            },

            // ================================================================
            // PHASE I BATCH 3 — Blur variants, grain, rotation, geq, CCM, denoisers, LUT3D, SITI, amplify
            // ================================================================

            ClaudeTool {
                name: "apply_gaussian_blur".to_string(),
                description: "Gaussian blur via FFmpeg gblur filter. Smooth, natural-looking blur with configurable sigma (radius). Good for background blur, depth-of-field simulation, and privacy masking.".to_string(),
                input_schema: InputSchema {
                    schema_type: "object".to_string(),
                    properties: HashMap::from([
                        ("input_file".to_string(), PropertyDefinition { prop_type: "string".to_string(), description: "Path to the input video file".to_string(), items: None }),
                        ("output_file".to_string(), PropertyDefinition { prop_type: "string".to_string(), description: "Path to save the blurred output".to_string(), items: None }),
                        ("sigma".to_string(), PropertyDefinition { prop_type: "number".to_string(), description: "Blur sigma (radius). Default 3. Higher = more blur.".to_string(), items: None }),
                        ("steps".to_string(), PropertyDefinition { prop_type: "number".to_string(), description: "Number of blur passes (1–6). Default 1. More passes = closer to true Gaussian.".to_string(), items: None }),
                        ("planes".to_string(), PropertyDefinition { prop_type: "number".to_string(), description: "Planes bitmask. Default 15 (all).".to_string(), items: None }),
                    ]),
                    required: vec!["input_file".to_string(), "output_file".to_string()],
                },
            },

            ClaudeTool {
                name: "apply_box_blur".to_string(),
                description: "Box (average) blur via FFmpeg avgblur filter. Fast rectangular blur. Useful for quick background softening, pixel effect, or motion blur approximation.".to_string(),
                input_schema: InputSchema {
                    schema_type: "object".to_string(),
                    properties: HashMap::from([
                        ("input_file".to_string(), PropertyDefinition { prop_type: "string".to_string(), description: "Path to the input video file".to_string(), items: None }),
                        ("output_file".to_string(), PropertyDefinition { prop_type: "string".to_string(), description: "Path to save the blurred output".to_string(), items: None }),
                        ("size_x".to_string(), PropertyDefinition { prop_type: "number".to_string(), description: "Horizontal kernel size (pixels). Default 3.".to_string(), items: None }),
                        ("size_y".to_string(), PropertyDefinition { prop_type: "number".to_string(), description: "Vertical kernel size (pixels). Default same as size_x.".to_string(), items: None }),
                        ("planes".to_string(), PropertyDefinition { prop_type: "number".to_string(), description: "Planes bitmask. Default 15 (all).".to_string(), items: None }),
                    ]),
                    required: vec!["input_file".to_string(), "output_file".to_string()],
                },
            },

            ClaudeTool {
                name: "apply_smart_blur".to_string(),
                description: "Smart blur via FFmpeg smartblur. Blurs flat regions while preserving edges. Negative luma_threshold = blur edges too; positive = protect edges.".to_string(),
                input_schema: InputSchema {
                    schema_type: "object".to_string(),
                    properties: HashMap::from([
                        ("input_file".to_string(), PropertyDefinition { prop_type: "string".to_string(), description: "Path to the input video file".to_string(), items: None }),
                        ("output_file".to_string(), PropertyDefinition { prop_type: "string".to_string(), description: "Path to save the smart-blurred output".to_string(), items: None }),
                        ("luma_radius".to_string(), PropertyDefinition { prop_type: "number".to_string(), description: "Luma blur radius (0.1–5.0). Default 1.0.".to_string(), items: None }),
                        ("luma_strength".to_string(), PropertyDefinition { prop_type: "number".to_string(), description: "Luma blur strength (-1.0 to 1.0). Negative = blur, positive = sharpen. Default -0.3.".to_string(), items: None }),
                        ("luma_threshold".to_string(), PropertyDefinition { prop_type: "number".to_string(), description: "Edge threshold (-30 to 30). 0 = blur everything. Default 0.".to_string(), items: None }),
                    ]),
                    required: vec!["input_file".to_string(), "output_file".to_string()],
                },
            },

            ClaudeTool {
                name: "add_film_grain".to_string(),
                description: "Adds analog film grain/noise via FFmpeg noise filter. Simulates film texture, adds organic feel to digital footage, or matches grain across clips.".to_string(),
                input_schema: InputSchema {
                    schema_type: "object".to_string(),
                    properties: HashMap::from([
                        ("input_file".to_string(), PropertyDefinition { prop_type: "string".to_string(), description: "Path to the input video file".to_string(), items: None }),
                        ("output_file".to_string(), PropertyDefinition { prop_type: "string".to_string(), description: "Path to save the grain-added output".to_string(), items: None }),
                        ("all_strength".to_string(), PropertyDefinition { prop_type: "number".to_string(), description: "Grain intensity (1–100). Default 8. Higher = more grain.".to_string(), items: None }),
                        ("flags".to_string(), PropertyDefinition { prop_type: "string".to_string(), description: "Grain type flags: a=additive (default), u=uniform, p=temporal (animated grain).".to_string(), items: None }),
                    ]),
                    required: vec!["input_file".to_string(), "output_file".to_string()],
                },
            },

            ClaudeTool {
                name: "apply_rotate_angle".to_string(),
                description: "Rotates video by an arbitrary angle in radians via FFmpeg rotate filter. Unlike rotate_video (90°/180° only), this rotates by any angle (e.g. PI/6 = 30°, PI/4 = 45°).".to_string(),
                input_schema: InputSchema {
                    schema_type: "object".to_string(),
                    properties: HashMap::from([
                        ("input_file".to_string(), PropertyDefinition { prop_type: "string".to_string(), description: "Path to the input video file".to_string(), items: None }),
                        ("output_file".to_string(), PropertyDefinition { prop_type: "string".to_string(), description: "Path to save the rotated output".to_string(), items: None }),
                        ("angle_rad".to_string(), PropertyDefinition { prop_type: "number".to_string(), description: "Rotation angle in radians. PI/6 ≈ 0.5236 (30°), PI/4 ≈ 0.7854 (45°), PI ≈ 3.14159 (180°).".to_string(), items: None }),
                        ("fillcolor".to_string(), PropertyDefinition { prop_type: "string".to_string(), description: "Background fill colour for exposed corners. Default 'black'. Use 'none' for transparent (requires RGBA output).".to_string(), items: None }),
                        ("expand".to_string(), PropertyDefinition { prop_type: "boolean".to_string(), description: "Expand canvas to fit rotated content without cropping. Default false.".to_string(), items: None }),
                    ]),
                    required: vec!["input_file".to_string(), "output_file".to_string(), "angle_rad".to_string()],
                },
            },

            ClaudeTool {
                name: "apply_geq".to_string(),
                description: "Per-pixel generic equation manipulation via FFmpeg geq filter. Write mathematical expressions for each pixel using X,Y coordinates and built-in functions like lum(), r(), g(), b().".to_string(),
                input_schema: InputSchema {
                    schema_type: "object".to_string(),
                    properties: HashMap::from([
                        ("input_file".to_string(), PropertyDefinition { prop_type: "string".to_string(), description: "Path to the input video file".to_string(), items: None }),
                        ("output_file".to_string(), PropertyDefinition { prop_type: "string".to_string(), description: "Path to save the processed output".to_string(), items: None }),
                        ("lum_expr".to_string(), PropertyDefinition { prop_type: "string".to_string(), description: "Luma expression. Default 'lum(X,Y)' (passthrough). Example: 'lum(W-X,Y)' (mirror), 'lum(X,Y)*0.5' (darken).".to_string(), items: None }),
                        ("cb_expr".to_string(), PropertyDefinition { prop_type: "string".to_string(), description: "Cb chroma expression. Default 'cb(X,Y)'.".to_string(), items: None }),
                        ("cr_expr".to_string(), PropertyDefinition { prop_type: "string".to_string(), description: "Cr chroma expression. Default 'cr(X,Y)'.".to_string(), items: None }),
                    ]),
                    required: vec!["input_file".to_string(), "output_file".to_string()],
                },
            },

            ClaudeTool {
                name: "apply_colorchannelmixer".to_string(),
                description: "Colour channel matrix mixer via FFmpeg colorchannelmixer. Controls how much each input channel (R/G/B) contributes to each output channel. Use for colour-grading, channel swap, cross-processing, or precise greyscale conversion.".to_string(),
                input_schema: InputSchema {
                    schema_type: "object".to_string(),
                    properties: HashMap::from([
                        ("input_file".to_string(), PropertyDefinition { prop_type: "string".to_string(), description: "Path to the input video file".to_string(), items: None }),
                        ("output_file".to_string(), PropertyDefinition { prop_type: "string".to_string(), description: "Path to save the colour-mixed output".to_string(), items: None }),
                        ("rr".to_string(), PropertyDefinition { prop_type: "number".to_string(), description: "Red channel: how much Red input goes to Red output. Default 1.0.".to_string(), items: None }),
                        ("rg".to_string(), PropertyDefinition { prop_type: "number".to_string(), description: "Red channel: how much Green input goes to Red output. Default 0.0.".to_string(), items: None }),
                        ("rb".to_string(), PropertyDefinition { prop_type: "number".to_string(), description: "Red channel: how much Blue input goes to Red output. Default 0.0.".to_string(), items: None }),
                        ("gr".to_string(), PropertyDefinition { prop_type: "number".to_string(), description: "Green channel: how much Red input. Default 0.0.".to_string(), items: None }),
                        ("gg".to_string(), PropertyDefinition { prop_type: "number".to_string(), description: "Green channel: how much Green input. Default 1.0.".to_string(), items: None }),
                        ("gb".to_string(), PropertyDefinition { prop_type: "number".to_string(), description: "Green channel: how much Blue input. Default 0.0.".to_string(), items: None }),
                        ("br".to_string(), PropertyDefinition { prop_type: "number".to_string(), description: "Blue channel: how much Red input. Default 0.0.".to_string(), items: None }),
                        ("bg".to_string(), PropertyDefinition { prop_type: "number".to_string(), description: "Blue channel: how much Green input. Default 0.0.".to_string(), items: None }),
                        ("bb".to_string(), PropertyDefinition { prop_type: "number".to_string(), description: "Blue channel: how much Blue input. Default 1.0.".to_string(), items: None }),
                    ]),
                    required: vec!["input_file".to_string(), "output_file".to_string()],
                },
            },

            ClaudeTool {
                name: "apply_atadenoise".to_string(),
                description: "Adaptive temporal averaging denoiser via FFmpeg atadenoise. Excellent for consistent temporal noise, grain, and sensor noise without blurring motion.".to_string(),
                input_schema: InputSchema {
                    schema_type: "object".to_string(),
                    properties: HashMap::from([
                        ("input_file".to_string(), PropertyDefinition { prop_type: "string".to_string(), description: "Path to the input video file".to_string(), items: None }),
                        ("output_file".to_string(), PropertyDefinition { prop_type: "string".to_string(), description: "Path to save the denoised output".to_string(), items: None }),
                        ("window_size".to_string(), PropertyDefinition { prop_type: "number".to_string(), description: "Temporal window size (odd number, 5–129). Default 9. Larger = stronger but slower.".to_string(), items: None }),
                        ("threshold_a".to_string(), PropertyDefinition { prop_type: "number".to_string(), description: "Low threshold (0–1). Default 0.02.".to_string(), items: None }),
                        ("threshold_b".to_string(), PropertyDefinition { prop_type: "number".to_string(), description: "High threshold (0–1). Default 0.04. Higher = more aggressive.".to_string(), items: None }),
                        ("planes".to_string(), PropertyDefinition { prop_type: "number".to_string(), description: "Planes bitmask. Default 7 (YUV).".to_string(), items: None }),
                    ]),
                    required: vec!["input_file".to_string(), "output_file".to_string()],
                },
            },

            ClaudeTool {
                name: "apply_vaguedenoiser".to_string(),
                description: "Wavelet-based video denoiser via FFmpeg vaguedenoiser. Preserves fine detail better than spatial denoisers. Good for broadcast footage and archival.".to_string(),
                input_schema: InputSchema {
                    schema_type: "object".to_string(),
                    properties: HashMap::from([
                        ("input_file".to_string(), PropertyDefinition { prop_type: "string".to_string(), description: "Path to the input video file".to_string(), items: None }),
                        ("output_file".to_string(), PropertyDefinition { prop_type: "string".to_string(), description: "Path to save the denoised output".to_string(), items: None }),
                        ("threshold".to_string(), PropertyDefinition { prop_type: "number".to_string(), description: "Denoising threshold. Default 2.0. Higher = stronger.".to_string(), items: None }),
                        ("method".to_string(), PropertyDefinition { prop_type: "number".to_string(), description: "Thresholding method: 0=soft (default), 1=hard, 2=garrote.".to_string(), items: None }),
                        ("nsteps".to_string(), PropertyDefinition { prop_type: "number".to_string(), description: "Wavelet decomposition steps. Default 6. More = captures larger noise structures.".to_string(), items: None }),
                        ("percent".to_string(), PropertyDefinition { prop_type: "number".to_string(), description: "Percentage of denoising to apply (0–100). Default 85.".to_string(), items: None }),
                        ("planes".to_string(), PropertyDefinition { prop_type: "number".to_string(), description: "Planes bitmask. Default 7 (YUV).".to_string(), items: None }),
                    ]),
                    required: vec!["input_file".to_string(), "output_file".to_string()],
                },
            },

            ClaudeTool {
                name: "apply_fftdnoiz".to_string(),
                description: "FFT-based video denoiser via FFmpeg fftdnoiz. Excellent for uniform additive noise (sensor noise, digitised film). Works in frequency domain.".to_string(),
                input_schema: InputSchema {
                    schema_type: "object".to_string(),
                    properties: HashMap::from([
                        ("input_file".to_string(), PropertyDefinition { prop_type: "string".to_string(), description: "Path to the input video file".to_string(), items: None }),
                        ("output_file".to_string(), PropertyDefinition { prop_type: "string".to_string(), description: "Path to save the denoised output".to_string(), items: None }),
                        ("sigma".to_string(), PropertyDefinition { prop_type: "number".to_string(), description: "Noise sigma (0.1–30). Default 1.0. Higher = stronger denoising.".to_string(), items: None }),
                        ("amount".to_string(), PropertyDefinition { prop_type: "number".to_string(), description: "Amount of denoising applied (0–1). Default 0.96.".to_string(), items: None }),
                        ("block_size".to_string(), PropertyDefinition { prop_type: "number".to_string(), description: "FFT block size. Default 32. Must be power of 2.".to_string(), items: None }),
                        ("overlap".to_string(), PropertyDefinition { prop_type: "number".to_string(), description: "Block overlap ratio (0.2–0.8). Default 0.5. Higher = smoother.".to_string(), items: None }),
                        ("planes".to_string(), PropertyDefinition { prop_type: "number".to_string(), description: "Planes bitmask. Default 7 (YUV).".to_string(), items: None }),
                    ]),
                    required: vec!["input_file".to_string(), "output_file".to_string()],
                },
            },

            ClaudeTool {
                name: "generate_waveform_video".to_string(),
                description: "Renders audio amplitude waveform as a video using FFmpeg showwaves filter. Shows waveform shape over time — useful for audio visualisation, editing guides, and podcast thumbnails.".to_string(),
                input_schema: InputSchema {
                    schema_type: "object".to_string(),
                    properties: HashMap::from([
                        ("input_file".to_string(), PropertyDefinition { prop_type: "string".to_string(), description: "Path to the input audio/video file".to_string(), items: None }),
                        ("output_file".to_string(), PropertyDefinition { prop_type: "string".to_string(), description: "Path to save the waveform video".to_string(), items: None }),
                        ("width".to_string(), PropertyDefinition { prop_type: "number".to_string(), description: "Output width px. Default 1280.".to_string(), items: None }),
                        ("height".to_string(), PropertyDefinition { prop_type: "number".to_string(), description: "Output height px. Default 240.".to_string(), items: None }),
                        ("mode".to_string(), PropertyDefinition { prop_type: "string".to_string(), description: "Waveform mode: line (default), point, p2p (peak-to-peak), cline (centred line).".to_string(), items: None }),
                        ("color".to_string(), PropertyDefinition { prop_type: "string".to_string(), description: "Waveform colour. Default 'white'. Any FFmpeg colour string.".to_string(), items: None }),
                    ]),
                    required: vec!["input_file".to_string(), "output_file".to_string()],
                },
            },

            ClaudeTool {
                name: "apply_lut3d".to_string(),
                description: "Applies a 3D LUT (.cube file) via FFmpeg lut3d filter. More precise than haldclut for colour grading — supports industry-standard .cube files from DaVinci, Lightroom, etc.".to_string(),
                input_schema: InputSchema {
                    schema_type: "object".to_string(),
                    properties: HashMap::from([
                        ("input_file".to_string(), PropertyDefinition { prop_type: "string".to_string(), description: "Path to the input video file".to_string(), items: None }),
                        ("output_file".to_string(), PropertyDefinition { prop_type: "string".to_string(), description: "Path to save the LUT-graded output".to_string(), items: None }),
                        ("lut_file".to_string(), PropertyDefinition { prop_type: "string".to_string(), description: "Path to the .cube LUT file".to_string(), items: None }),
                        ("interp".to_string(), PropertyDefinition { prop_type: "string".to_string(), description: "Interpolation: nearest, trilinear, tetrahedral (default, most accurate), pyramid, prism.".to_string(), items: None }),
                    ]),
                    required: vec!["input_file".to_string(), "output_file".to_string(), "lut_file".to_string()],
                },
            },

            ClaudeTool {
                name: "measure_siti".to_string(),
                description: "Measures Spatial Information (SI) and Temporal Information (TI) via FFmpeg siti filter. Industry standard for video complexity analysis — helps select codec presets and bitrate targets.".to_string(),
                input_schema: InputSchema {
                    schema_type: "object".to_string(),
                    properties: HashMap::from([
                        ("input_file".to_string(), PropertyDefinition { prop_type: "string".to_string(), description: "Path to the input video file".to_string(), items: None }),
                    ]),
                    required: vec!["input_file".to_string()],
                },
            },

            ClaudeTool {
                name: "create_test_pattern".to_string(),
                description: "Generates a standard test pattern video using FFmpeg lavfi source. No input file needed. Useful for testing monitors, audio/video sync, codec chains, and colour accuracy.".to_string(),
                input_schema: InputSchema {
                    schema_type: "object".to_string(),
                    properties: HashMap::from([
                        ("output_file".to_string(), PropertyDefinition { prop_type: "string".to_string(), description: "Path to save the generated test pattern video".to_string(), items: None }),
                        ("width".to_string(), PropertyDefinition { prop_type: "number".to_string(), description: "Output width px. Default 1920.".to_string(), items: None }),
                        ("height".to_string(), PropertyDefinition { prop_type: "number".to_string(), description: "Output height px. Default 1080.".to_string(), items: None }),
                        ("duration".to_string(), PropertyDefinition { prop_type: "number".to_string(), description: "Duration in seconds. Default 10.".to_string(), items: None }),
                        ("pattern".to_string(), PropertyDefinition { prop_type: "string".to_string(), description: "Pattern type: smptebars (default), smptehdbars (HD), testsrc, testsrc2.".to_string(), items: None }),
                        ("framerate".to_string(), PropertyDefinition { prop_type: "number".to_string(), description: "Frame rate. Default 25.".to_string(), items: None }),
                    ]),
                    required: vec!["output_file".to_string()],
                },
            },

            ClaudeTool {
                name: "apply_amplify".to_string(),
                description: "Amplifies pixel differences between consecutive frames via FFmpeg amplify filter. Makes subtle temporal changes visible; creates dramatic motion emphasis or microscopy-style effects.".to_string(),
                input_schema: InputSchema {
                    schema_type: "object".to_string(),
                    properties: HashMap::from([
                        ("input_file".to_string(), PropertyDefinition { prop_type: "string".to_string(), description: "Path to the input video file".to_string(), items: None }),
                        ("output_file".to_string(), PropertyDefinition { prop_type: "string".to_string(), description: "Path to save the amplified output".to_string(), items: None }),
                        ("radius".to_string(), PropertyDefinition { prop_type: "number".to_string(), description: "Temporal radius (frames to compare). Default 2.".to_string(), items: None }),
                        ("factor".to_string(), PropertyDefinition { prop_type: "number".to_string(), description: "Amplification factor. Default 2.0. Higher = more dramatic.".to_string(), items: None }),
                        ("threshold".to_string(), PropertyDefinition { prop_type: "number".to_string(), description: "Minimum change threshold to amplify. Default 10. Lower = amplify subtler motion.".to_string(), items: None }),
                        ("planes".to_string(), PropertyDefinition { prop_type: "number".to_string(), description: "Planes bitmask. Default 7 (YUV).".to_string(), items: None }),
                    ]),
                    required: vec!["input_file".to_string(), "output_file".to_string()],
                },
            },

            // ================================================================
            // PHASE I BATCH 2 — Long-tail: morphology, histogram, convolution
            // ================================================================

            ClaudeTool {
                name: "select_frames".to_string(),
                description: "Selects specific frames from video using FFmpeg select+setpts. Good for extracting keyframes, sampling at intervals, or filtering by frame type.".to_string(),
                input_schema: InputSchema {
                    schema_type: "object".to_string(),
                    properties: HashMap::from([
                        ("input_file".to_string(), PropertyDefinition { prop_type: "string".to_string(), description: "Path to the input video file".to_string(), items: None }),
                        ("output_file".to_string(), PropertyDefinition { prop_type: "string".to_string(), description: "Path to save the selected-frames output".to_string(), items: None }),
                        ("expr".to_string(), PropertyDefinition { prop_type: "string".to_string(), description: "FFmpeg select expression. Default: keyframes only. Examples: 'not(mod(n,30))' = every 30th frame; 'eq(pict_type\\,PICT_TYPE_I)' = I-frames.".to_string(), items: None }),
                        ("fps".to_string(), PropertyDefinition { prop_type: "number".to_string(), description: "Output frame rate. 0 = keep original timing. Default 0.".to_string(), items: None }),
                    ]),
                    required: vec!["input_file".to_string(), "output_file".to_string()],
                },
            },

            ClaudeTool {
                name: "posterize_video".to_string(),
                description: "Posterizes video to N colour levels using FFmpeg posterize filter. Creates a graphic-novel or stencil look.".to_string(),
                input_schema: InputSchema {
                    schema_type: "object".to_string(),
                    properties: HashMap::from([
                        ("input_file".to_string(), PropertyDefinition { prop_type: "string".to_string(), description: "Path to the input video file".to_string(), items: None }),
                        ("output_file".to_string(), PropertyDefinition { prop_type: "string".to_string(), description: "Path to save the posterized output".to_string(), items: None }),
                        ("levels".to_string(), PropertyDefinition { prop_type: "number".to_string(), description: "Number of colour levels per channel (2–64). Default 5. Lower = more stylized.".to_string(), items: None }),
                    ]),
                    required: vec!["input_file".to_string(), "output_file".to_string()],
                },
            },

            ClaudeTool {
                name: "solarize_video".to_string(),
                description: "Applies solarize effect: pixels above threshold are inverted. Classic darkroom/psychedelic look via FFmpeg solarize.".to_string(),
                input_schema: InputSchema {
                    schema_type: "object".to_string(),
                    properties: HashMap::from([
                        ("input_file".to_string(), PropertyDefinition { prop_type: "string".to_string(), description: "Path to the input video file".to_string(), items: None }),
                        ("output_file".to_string(), PropertyDefinition { prop_type: "string".to_string(), description: "Path to save the solarized output".to_string(), items: None }),
                        ("threshold".to_string(), PropertyDefinition { prop_type: "number".to_string(), description: "Luminance threshold 0–255. Pixels above are inverted. Default 128.".to_string(), items: None }),
                    ]),
                    required: vec!["input_file".to_string(), "output_file".to_string()],
                },
            },

            ClaudeTool {
                name: "apply_dilation".to_string(),
                description: "Morphological dilation: expands bright regions and fills dark gaps via FFmpeg dilation filter. Useful for noise cleanup and effect creation.".to_string(),
                input_schema: InputSchema {
                    schema_type: "object".to_string(),
                    properties: HashMap::from([
                        ("input_file".to_string(), PropertyDefinition { prop_type: "string".to_string(), description: "Path to the input video file".to_string(), items: None }),
                        ("output_file".to_string(), PropertyDefinition { prop_type: "string".to_string(), description: "Path to save the dilation output".to_string(), items: None }),
                        ("threshold0".to_string(), PropertyDefinition { prop_type: "number".to_string(), description: "Max change threshold for plane 0. Default 65535.".to_string(), items: None }),
                        ("threshold1".to_string(), PropertyDefinition { prop_type: "number".to_string(), description: "Max change threshold for plane 1. Default 65535.".to_string(), items: None }),
                        ("threshold2".to_string(), PropertyDefinition { prop_type: "number".to_string(), description: "Max change threshold for plane 2. Default 65535.".to_string(), items: None }),
                        ("threshold3".to_string(), PropertyDefinition { prop_type: "number".to_string(), description: "Max change threshold for plane 3. Default 65535.".to_string(), items: None }),
                        ("coordinates".to_string(), PropertyDefinition { prop_type: "number".to_string(), description: "8-bit bitmask of neighbours to check. Default 255 (all 8).".to_string(), items: None }),
                    ]),
                    required: vec!["input_file".to_string(), "output_file".to_string()],
                },
            },

            ClaudeTool {
                name: "apply_erosion".to_string(),
                description: "Morphological erosion: shrinks bright regions and removes small protrusions via FFmpeg erosion filter.".to_string(),
                input_schema: InputSchema {
                    schema_type: "object".to_string(),
                    properties: HashMap::from([
                        ("input_file".to_string(), PropertyDefinition { prop_type: "string".to_string(), description: "Path to the input video file".to_string(), items: None }),
                        ("output_file".to_string(), PropertyDefinition { prop_type: "string".to_string(), description: "Path to save the erosion output".to_string(), items: None }),
                        ("threshold0".to_string(), PropertyDefinition { prop_type: "number".to_string(), description: "Max change threshold for plane 0. Default 65535.".to_string(), items: None }),
                        ("threshold1".to_string(), PropertyDefinition { prop_type: "number".to_string(), description: "Max change threshold for plane 1. Default 65535.".to_string(), items: None }),
                        ("threshold2".to_string(), PropertyDefinition { prop_type: "number".to_string(), description: "Max change threshold for plane 2. Default 65535.".to_string(), items: None }),
                        ("threshold3".to_string(), PropertyDefinition { prop_type: "number".to_string(), description: "Max change threshold for plane 3. Default 65535.".to_string(), items: None }),
                        ("coordinates".to_string(), PropertyDefinition { prop_type: "number".to_string(), description: "8-bit bitmask of neighbours. Default 255 (all 8).".to_string(), items: None }),
                    ]),
                    required: vec!["input_file".to_string(), "output_file".to_string()],
                },
            },

            ClaudeTool {
                name: "apply_median_filter".to_string(),
                description: "Applies median filter for salt-and-pepper noise removal via FFmpeg median filter. Non-linear, preserves edges well.".to_string(),
                input_schema: InputSchema {
                    schema_type: "object".to_string(),
                    properties: HashMap::from([
                        ("input_file".to_string(), PropertyDefinition { prop_type: "string".to_string(), description: "Path to the input video file".to_string(), items: None }),
                        ("output_file".to_string(), PropertyDefinition { prop_type: "string".to_string(), description: "Path to save the filtered output".to_string(), items: None }),
                        ("radius".to_string(), PropertyDefinition { prop_type: "number".to_string(), description: "Kernel radius (1–127). Default 1. Higher = stronger.".to_string(), items: None }),
                        ("planes".to_string(), PropertyDefinition { prop_type: "number".to_string(), description: "Planes to filter (bitmask). Default 15 (all).".to_string(), items: None }),
                    ]),
                    required: vec!["input_file".to_string(), "output_file".to_string()],
                },
            },

            ClaudeTool {
                name: "apply_histogram_eq".to_string(),
                description: "Global histogram equalisation via FFmpeg histeq. Improves contrast by redistributing pixel luminance across the full range.".to_string(),
                input_schema: InputSchema {
                    schema_type: "object".to_string(),
                    properties: HashMap::from([
                        ("input_file".to_string(), PropertyDefinition { prop_type: "string".to_string(), description: "Path to the input video file".to_string(), items: None }),
                        ("output_file".to_string(), PropertyDefinition { prop_type: "string".to_string(), description: "Path to save the equalised output".to_string(), items: None }),
                        ("strength".to_string(), PropertyDefinition { prop_type: "number".to_string(), description: "Equalisation strength (0–1). Default 0.2. Higher = more aggressive.".to_string(), items: None }),
                        ("intensity".to_string(), PropertyDefinition { prop_type: "number".to_string(), description: "Intensity multiplier (0–1). Default 0.21.".to_string(), items: None }),
                        ("antibanding".to_string(), PropertyDefinition { prop_type: "string".to_string(), description: "Anti-banding level: none (default), weak, strong.".to_string(), items: None }),
                    ]),
                    required: vec!["input_file".to_string(), "output_file".to_string()],
                },
            },

            ClaudeTool {
                name: "apply_clahe".to_string(),
                description: "CLAHE (Contrast-Limited Adaptive Histogram Equalisation) via FFmpeg clahe. Better local contrast enhancement than global histeq without overexposing highlights.".to_string(),
                input_schema: InputSchema {
                    schema_type: "object".to_string(),
                    properties: HashMap::from([
                        ("input_file".to_string(), PropertyDefinition { prop_type: "string".to_string(), description: "Path to the input video file".to_string(), items: None }),
                        ("output_file".to_string(), PropertyDefinition { prop_type: "string".to_string(), description: "Path to save the CLAHE output".to_string(), items: None }),
                        ("clip_limit".to_string(), PropertyDefinition { prop_type: "number".to_string(), description: "Clip limit (1–100). Default 25. Higher = more contrast.".to_string(), items: None }),
                        ("nb_tiles_x".to_string(), PropertyDefinition { prop_type: "number".to_string(), description: "Horizontal tile count. Default 8.".to_string(), items: None }),
                        ("nb_tiles_y".to_string(), PropertyDefinition { prop_type: "number".to_string(), description: "Vertical tile count. Default 8.".to_string(), items: None }),
                    ]),
                    required: vec!["input_file".to_string(), "output_file".to_string()],
                },
            },

            ClaudeTool {
                name: "apply_deblock".to_string(),
                description: "Removes block/DCT artefacts from heavily compressed video via FFmpeg deblock filter. Useful for upscaling old H.264/MPEG footage.".to_string(),
                input_schema: InputSchema {
                    schema_type: "object".to_string(),
                    properties: HashMap::from([
                        ("input_file".to_string(), PropertyDefinition { prop_type: "string".to_string(), description: "Path to the input video file".to_string(), items: None }),
                        ("output_file".to_string(), PropertyDefinition { prop_type: "string".to_string(), description: "Path to save the debocked output".to_string(), items: None }),
                        ("filter_type".to_string(), PropertyDefinition { prop_type: "number".to_string(), description: "Filter strength level (1–4). Default 4 = strongest.".to_string(), items: None }),
                        ("block_size".to_string(), PropertyDefinition { prop_type: "number".to_string(), description: "Block size in pixels. Default 8 (matches H.264 DCT blocks).".to_string(), items: None }),
                        ("strength".to_string(), PropertyDefinition { prop_type: "number".to_string(), description: "Alpha/beta/gamma/delta strength (0–1). Default 0.5.".to_string(), items: None }),
                        ("planes".to_string(), PropertyDefinition { prop_type: "number".to_string(), description: "Planes to filter (bitmask). Default 15 (all).".to_string(), items: None }),
                    ]),
                    required: vec!["input_file".to_string(), "output_file".to_string()],
                },
            },

            ClaudeTool {
                name: "adjust_hue_saturation".to_string(),
                description: "Precise hue and saturation adjustment via FFmpeg huesaturation filter. Independent control over hue rotation, saturation, intensity, and lightness.".to_string(),
                input_schema: InputSchema {
                    schema_type: "object".to_string(),
                    properties: HashMap::from([
                        ("input_file".to_string(), PropertyDefinition { prop_type: "string".to_string(), description: "Path to the input video file".to_string(), items: None }),
                        ("output_file".to_string(), PropertyDefinition { prop_type: "string".to_string(), description: "Path to save the adjusted output".to_string(), items: None }),
                        ("hue".to_string(), PropertyDefinition { prop_type: "number".to_string(), description: "Hue rotation in degrees (-180 to 180). Default 0.".to_string(), items: None }),
                        ("saturation".to_string(), PropertyDefinition { prop_type: "number".to_string(), description: "Saturation adjustment (-3 to 3). Default 0. Positive = more vivid.".to_string(), items: None }),
                        ("intensity".to_string(), PropertyDefinition { prop_type: "number".to_string(), description: "Intensity adjustment (-1 to 1). Default 0.".to_string(), items: None }),
                        ("lightness".to_string(), PropertyDefinition { prop_type: "number".to_string(), description: "Lightness offset (-1 to 1). Default 0.".to_string(), items: None }),
                    ]),
                    required: vec!["input_file".to_string(), "output_file".to_string()],
                },
            },

            ClaudeTool {
                name: "apply_convolution".to_string(),
                description: "Applies a custom NxN convolution kernel via FFmpeg convolution filter. Can sharpen, blur, emboss, or detect edges with any kernel matrix.".to_string(),
                input_schema: InputSchema {
                    schema_type: "object".to_string(),
                    properties: HashMap::from([
                        ("input_file".to_string(), PropertyDefinition { prop_type: "string".to_string(), description: "Path to the input video file".to_string(), items: None }),
                        ("output_file".to_string(), PropertyDefinition { prop_type: "string".to_string(), description: "Path to save the filtered output".to_string(), items: None }),
                        ("matrix".to_string(), PropertyDefinition { prop_type: "string".to_string(), description: "Space-separated kernel values. 9 values = 3x3, 25 = 5x5. Default: '0 -1 0 -1 5 -1 0 -1 0' (sharpen). Blur: '1 1 1 1 1 1 1 1 1'. Emboss: '-2 -1 0 -1 1 1 0 1 2'.".to_string(), items: None }),
                        ("rdiv".to_string(), PropertyDefinition { prop_type: "number".to_string(), description: "Normalisation divisor. Default 1.0.".to_string(), items: None }),
                        ("bias".to_string(), PropertyDefinition { prop_type: "number".to_string(), description: "Bias added after convolution. Default 0.".to_string(), items: None }),
                        ("planes".to_string(), PropertyDefinition { prop_type: "number".to_string(), description: "Planes to filter (bitmask). Default 15 (all).".to_string(), items: None }),
                    ]),
                    required: vec!["input_file".to_string(), "output_file".to_string()],
                },
            },

            ClaudeTool {
                name: "reverse_audio".to_string(),
                description: "Reverses the audio stream using FFmpeg areverse filter. Creates backwards/reversed audio effect.".to_string(),
                input_schema: InputSchema {
                    schema_type: "object".to_string(),
                    properties: HashMap::from([
                        ("input_file".to_string(), PropertyDefinition { prop_type: "string".to_string(), description: "Path to the input audio/video file".to_string(), items: None }),
                        ("output_file".to_string(), PropertyDefinition { prop_type: "string".to_string(), description: "Path to save the reversed audio output".to_string(), items: None }),
                    ]),
                    required: vec!["input_file".to_string(), "output_file".to_string()],
                },
            },

            ClaudeTool {
                name: "blend_audio_streams".to_string(),
                description: "Mixes two audio files together using FFmpeg amix filter. Primary + secondary inputs are blended into a single output.".to_string(),
                input_schema: InputSchema {
                    schema_type: "object".to_string(),
                    properties: HashMap::from([
                        ("input_file".to_string(), PropertyDefinition { prop_type: "string".to_string(), description: "Path to the primary audio/video file".to_string(), items: None }),
                        ("secondary_file".to_string(), PropertyDefinition { prop_type: "string".to_string(), description: "Path to the secondary audio file to mix in".to_string(), items: None }),
                        ("output_file".to_string(), PropertyDefinition { prop_type: "string".to_string(), description: "Path to save the mixed output".to_string(), items: None }),
                        ("duration".to_string(), PropertyDefinition { prop_type: "string".to_string(), description: "Output duration: longest (default), shortest, first.".to_string(), items: None }),
                        ("dropout_transition".to_string(), PropertyDefinition { prop_type: "number".to_string(), description: "Seconds to fade out a stream when it ends. Default 2.0.".to_string(), items: None }),
                    ]),
                    required: vec!["input_file".to_string(), "secondary_file".to_string(), "output_file".to_string()],
                },
            },

            ClaudeTool {
                name: "measure_silence".to_string(),
                description: "Detects silent segments using FFmpeg silencedetect. Returns timestamps of silence_start, silence_end, and silence_duration. Analysis only — no output file.".to_string(),
                input_schema: InputSchema {
                    schema_type: "object".to_string(),
                    properties: HashMap::from([
                        ("input_file".to_string(), PropertyDefinition { prop_type: "string".to_string(), description: "Path to the input audio/video file".to_string(), items: None }),
                        ("noise_db".to_string(), PropertyDefinition { prop_type: "number".to_string(), description: "Noise floor in dBFS (negative number). Default -30. e.g. -60 for very quiet silence.".to_string(), items: None }),
                        ("duration_s".to_string(), PropertyDefinition { prop_type: "number".to_string(), description: "Minimum silence duration in seconds. Default 0.5.".to_string(), items: None }),
                    ]),
                    required: vec!["input_file".to_string()],
                },
            },

            ClaudeTool {
                name: "measure_audio_spectrum".to_string(),
                description: "Renders audio frequency spectrum as a video file using FFmpeg showspectrum filter. Useful for visualising frequency content, EQ decisions, and audio analysis.".to_string(),
                input_schema: InputSchema {
                    schema_type: "object".to_string(),
                    properties: HashMap::from([
                        ("input_file".to_string(), PropertyDefinition { prop_type: "string".to_string(), description: "Path to the input audio/video file".to_string(), items: None }),
                        ("output_file".to_string(), PropertyDefinition { prop_type: "string".to_string(), description: "Path to save the spectrum video (e.g. spectrum.mp4)".to_string(), items: None }),
                        ("width".to_string(), PropertyDefinition { prop_type: "number".to_string(), description: "Output width in pixels. Default 1024.".to_string(), items: None }),
                        ("height".to_string(), PropertyDefinition { prop_type: "number".to_string(), description: "Output height in pixels. Default 512.".to_string(), items: None }),
                        ("mode".to_string(), PropertyDefinition { prop_type: "string".to_string(), description: "Display mode: combined, separate, default combined.".to_string(), items: None }),
                        ("color".to_string(), PropertyDefinition { prop_type: "string".to_string(), description: "Colour scheme: intensity (default), fire, moreland, rainbow, etc.".to_string(), items: None }),
                    ]),
                    required: vec!["input_file".to_string(), "output_file".to_string()],
                },
            },

            // ── Workflow Recipes ─────────────────────────────────────────
            ClaudeTool {
                name: "youtube_ready_export".to_string(),
                description: "Multi-step YouTube export pipeline: stabilize → normalize color → loudnorm to −14 LUFS → convert to yuv420p. Use this when the user wants their video ready for YouTube upload in one shot.".to_string(),
                input_schema: InputSchema {
                    schema_type: "object".to_string(),
                    properties: HashMap::from([
                        ("input_file".to_string(), PropertyDefinition { prop_type: "string".to_string(), description: "Path to the input video file".to_string(), items: None }),
                        ("output_file".to_string(), PropertyDefinition { prop_type: "string".to_string(), description: "Path to save the YouTube-ready output (e.g. youtube_ready.mp4)".to_string(), items: None }),
                    ]),
                    required: vec!["input_file".to_string(), "output_file".to_string()],
                },
            },
            ClaudeTool {
                name: "podcast_cleanup".to_string(),
                description: "Multi-step podcast audio cleanup: denoise → de-ess sibilance → limit peaks → loudnorm to −16 LUFS. Use this when the user wants professional-sounding podcast or speech audio.".to_string(),
                input_schema: InputSchema {
                    schema_type: "object".to_string(),
                    properties: HashMap::from([
                        ("input_file".to_string(), PropertyDefinition { prop_type: "string".to_string(), description: "Path to the input audio or video file".to_string(), items: None }),
                        ("output_file".to_string(), PropertyDefinition { prop_type: "string".to_string(), description: "Path to save the cleaned audio (e.g. podcast_clean.wav)".to_string(), items: None }),
                    ]),
                    required: vec!["input_file".to_string(), "output_file".to_string()],
                },
            },
            ClaudeTool {
                name: "cinematic_grade".to_string(),
                description: "Multi-step cinematic color grade: vintage curves → vibrance boost → vignette → film grain. Use this when the user wants a cinematic, film-like look for trailers or highlight reels.".to_string(),
                input_schema: InputSchema {
                    schema_type: "object".to_string(),
                    properties: HashMap::from([
                        ("input_file".to_string(), PropertyDefinition { prop_type: "string".to_string(), description: "Path to the input video file".to_string(), items: None }),
                        ("output_file".to_string(), PropertyDefinition { prop_type: "string".to_string(), description: "Path to save the graded video (e.g. cinematic_output.mp4)".to_string(), items: None }),
                    ]),
                    required: vec!["input_file".to_string(), "output_file".to_string()],
                },
            },
            ClaudeTool {
                name: "create_gif_workflow".to_string(),
                description: "Creates a high-quality optimised GIF: trim segment → scale → palette-optimised GIF. Use when the user wants to create a GIF from a video clip.".to_string(),
                input_schema: InputSchema {
                    schema_type: "object".to_string(),
                    properties: HashMap::from([
                        ("input_file".to_string(), PropertyDefinition { prop_type: "string".to_string(), description: "Path to the input video file".to_string(), items: None }),
                        ("output_file".to_string(), PropertyDefinition { prop_type: "string".to_string(), description: "Path to save the GIF (e.g. output.gif)".to_string(), items: None }),
                        ("start_seconds".to_string(), PropertyDefinition { prop_type: "number".to_string(), description: "Start time in seconds. Default 0.".to_string(), items: None }),
                        ("duration_seconds".to_string(), PropertyDefinition { prop_type: "number".to_string(), description: "Duration of the GIF in seconds. Default 5.".to_string(), items: None }),
                        ("width".to_string(), PropertyDefinition { prop_type: "number".to_string(), description: "Output width in pixels. Default 480.".to_string(), items: None }),
                        ("fps".to_string(), PropertyDefinition { prop_type: "number".to_string(), description: "Frames per second. Default 15.".to_string(), items: None }),
                    ]),
                    required: vec!["input_file".to_string(), "output_file".to_string()],
                },
            },
            ClaudeTool {
                name: "talking_head_cleanup".to_string(),
                description: "Multi-step talking head video cleanup: stabilize → denoise speech → de-ess sibilance → loudnorm to −16 LUFS. Use for YouTube talking head footage, interviews, or screen recordings.".to_string(),
                input_schema: InputSchema {
                    schema_type: "object".to_string(),
                    properties: HashMap::from([
                        ("input_file".to_string(), PropertyDefinition { prop_type: "string".to_string(), description: "Path to the input video file".to_string(), items: None }),
                        ("output_file".to_string(), PropertyDefinition { prop_type: "string".to_string(), description: "Path to save the cleaned video (e.g. talking_head_clean.mp4)".to_string(), items: None }),
                    ]),
                    required: vec!["input_file".to_string(), "output_file".to_string()],
                },
            },

            // ── Cloud Storage Tool ────────────────────────────────────────────────
            ClaudeTool {
                name: "download_from_cloud".to_string(),
                description: "Download a file from a cloud storage URL (GCS presigned URL, R2 presigned URL, or any HTTP URL) to a local file in the outputs/ directory. Use this to retrieve previously generated videos, images, or assets from storage for re-editing, compositing, or quality review. The file is saved to outputs/ and can then be used with any video editing tool.".to_string(),
                input_schema: InputSchema {
                    schema_type: "object".to_string(),
                    properties: HashMap::from([
                        ("url".to_string(), PropertyDefinition { prop_type: "string".to_string(), description: "The presigned URL to download from (GCS or R2 presigned URL)".to_string(), items: None }),
                        ("output_path".to_string(), PropertyDefinition { prop_type: "string".to_string(), description: "Desired output filename inside outputs/ (e.g. 'video.mp4' or 'outputs/video.mp4')".to_string(), items: None }),
                    ]),
                    required: vec!["url".to_string(), "output_path".to_string()],
                },
            },

            // ── BlenderMCPServer tools ────────────────────────────────────────────
            ClaudeTool {
                name: "blender_generate_scene".to_string(),
                description: "Generate a procedural 3D Blender scene as an MP4 clip. Use instead of Pexels for custom, on-brand, or abstract backgrounds.".to_string(),
                input_schema: InputSchema {
                    schema_type: "object".to_string(),
                    properties: HashMap::from([
                        ("prompt".to_string(), PropertyDefinition { prop_type: "string".to_string(), description: "Natural language description of the scene".to_string(), items: None }),
                        ("duration".to_string(), PropertyDefinition { prop_type: "number".to_string(), description: "Clip length in seconds (default: 10)".to_string(), items: None }),
                        ("style".to_string(), PropertyDefinition { prop_type: "string".to_string(), description: "Visual style: 'cinematic' | 'minimal' | 'energetic' | 'calm'".to_string(), items: None }),
                        ("reference_image_url".to_string(), PropertyDefinition { prop_type: "string".to_string(), description: "Optional URL of a reference image for style guidance".to_string(), items: None }),
                        ("include_narration".to_string(), PropertyDefinition { prop_type: "boolean".to_string(), description: "Optional. If true, generate narration audio and a narrated video variant when VibeVoice is configured".to_string(), items: None }),
                        ("narration_text".to_string(), PropertyDefinition { prop_type: "string".to_string(), description: "Optional narration script to speak over the rendered scene".to_string(), items: None }),
                        ("narration_speaker".to_string(), PropertyDefinition { prop_type: "string".to_string(), description: "Optional VibeVoice speaker preset name, e.g. 'Emma' or 'Carter'".to_string(), items: None }),
                    ]),
                    required: vec!["prompt".to_string()],
                },
            },
            ClaudeTool {
                name: "blender_generate_thumbnail".to_string(),
                description: "Generate a 3D rendered YouTube thumbnail image (1280x720 PNG) with text overlay.".to_string(),
                input_schema: InputSchema {
                    schema_type: "object".to_string(),
                    properties: HashMap::from([
                        ("prompt".to_string(), PropertyDefinition { prop_type: "string".to_string(), description: "Description of the thumbnail visual style and content".to_string(), items: None }),
                        ("title_text".to_string(), PropertyDefinition { prop_type: "string".to_string(), description: "Text to overlay on the thumbnail".to_string(), items: None }),
                        ("style".to_string(), PropertyDefinition { prop_type: "string".to_string(), description: "Style: 'youtube' | 'cinematic' | 'minimal'".to_string(), items: None }),
                    ]),
                    required: vec!["prompt".to_string()],
                },
            },
            ClaudeTool {
                name: "blender_generate_title_card".to_string(),
                description: "Generate an animated 3D title card as an MP4 clip. Use for video intros, section headers, and branded openers.".to_string(),
                input_schema: InputSchema {
                    schema_type: "object".to_string(),
                    properties: HashMap::from([
                        ("title".to_string(), PropertyDefinition { prop_type: "string".to_string(), description: "Main title text".to_string(), items: None }),
                        ("subtitle".to_string(), PropertyDefinition { prop_type: "string".to_string(), description: "Secondary/subtitle text".to_string(), items: None }),
                        ("duration".to_string(), PropertyDefinition { prop_type: "number".to_string(), description: "Clip length in seconds (default: 5)".to_string(), items: None }),
                        ("style".to_string(), PropertyDefinition { prop_type: "string".to_string(), description: "Visual style description".to_string(), items: None }),
                    ]),
                    required: vec!["title".to_string()],
                },
            },
            ClaudeTool {
                name: "blender_generate_data_viz".to_string(),
                description: "Generate an animated 3D data visualisation clip (bar chart) from JSON data.".to_string(),
                input_schema: InputSchema {
                    schema_type: "object".to_string(),
                    properties: HashMap::from([
                        ("data_json".to_string(), PropertyDefinition { prop_type: "string".to_string(), description: "JSON array of data points, e.g. [{\"label\":\"Q1\",\"value\":42}]".to_string(), items: None }),
                        ("chart_type".to_string(), PropertyDefinition { prop_type: "string".to_string(), description: "Chart type: 'bar' (default)".to_string(), items: None }),
                        ("title".to_string(), PropertyDefinition { prop_type: "string".to_string(), description: "Chart title text".to_string(), items: None }),
                        ("duration".to_string(), PropertyDefinition { prop_type: "number".to_string(), description: "Clip length in seconds (default: 10)".to_string(), items: None }),
                    ]),
                    required: vec!["data_json".to_string()],
                },
            },
            ClaudeTool {
                name: "blender_generate_lower_third".to_string(),
                description: "Generate an animated lower-third text overlay clip (green-screen) for name plates and captions.".to_string(),
                input_schema: InputSchema {
                    schema_type: "object".to_string(),
                    properties: HashMap::from([
                        ("name_text".to_string(), PropertyDefinition { prop_type: "string".to_string(), description: "Primary lower-third text (name or title)".to_string(), items: None }),
                        ("subtitle_text".to_string(), PropertyDefinition { prop_type: "string".to_string(), description: "Secondary text (role or organisation)".to_string(), items: None }),
                        ("style".to_string(), PropertyDefinition { prop_type: "string".to_string(), description: "Colour/animation style description".to_string(), items: None }),
                        ("duration".to_string(), PropertyDefinition { prop_type: "number".to_string(), description: "Display duration in seconds (default: 5)".to_string(), items: None }),
                    ]),
                    required: vec!["name_text".to_string()],
                },
            },
            ClaudeTool {
                name: "blender_generate_latex".to_string(),
                description: "Generate a LaTeX/Manim mathematical equation animation clip. Ideal for educational math and science videos.".to_string(),
                input_schema: InputSchema {
                    schema_type: "object".to_string(),
                    properties: HashMap::from([
                        ("latex_expression".to_string(), PropertyDefinition { prop_type: "string".to_string(), description: r#"LaTeX math expression, e.g. r"\frac{d}{dt}\int_a^b f(x,t)dx""#.to_string(), items: None }),
                        ("animation_type".to_string(), PropertyDefinition { prop_type: "string".to_string(), description: "Animation style: 'appear' | 'morph' | 'step_by_step'".to_string(), items: None }),
                        ("duration".to_string(), PropertyDefinition { prop_type: "number".to_string(), description: "Clip length in seconds (default: 8)".to_string(), items: None }),
                        ("background_style".to_string(), PropertyDefinition { prop_type: "string".to_string(), description: "Background: 'dark' | 'light' | 'transparent'".to_string(), items: None }),
                        ("include_narration".to_string(), PropertyDefinition { prop_type: "boolean".to_string(), description: "Optional. If true, generate narration audio and a narrated video variant for this math render".to_string(), items: None }),
                        ("narration_text".to_string(), PropertyDefinition { prop_type: "string".to_string(), description: "Optional narration script to speak during the animation".to_string(), items: None }),
                        ("narration_speaker".to_string(), PropertyDefinition { prop_type: "string".to_string(), description: "Optional VibeVoice speaker preset name, e.g. 'Emma' or 'Carter'".to_string(), items: None }),
                    ]),
                    required: vec!["latex_expression".to_string()],
                },
            },
            ClaudeTool {
                name: "blender_generate_ui_mockup".to_string(),
                description: "Generate a 3D device UI mockup animation (iPhone, MacBook, browser, iPad) for app demos and SaaS product videos.".to_string(),
                input_schema: InputSchema {
                    schema_type: "object".to_string(),
                    properties: HashMap::from([
                        ("device".to_string(), PropertyDefinition { prop_type: "string".to_string(), description: "Device frame: 'iPhone' | 'MacBook' | 'browser' | 'iPad'".to_string(), items: None }),
                        ("animation".to_string(), PropertyDefinition { prop_type: "string".to_string(), description: "Animation: 'static' (PNG) | 'reveal' | 'scroll' | 'tilt'".to_string(), items: None }),
                        ("duration".to_string(), PropertyDefinition { prop_type: "number".to_string(), description: "Clip length in seconds (default: 5)".to_string(), items: None }),
                        ("screenshot_url".to_string(), PropertyDefinition { prop_type: "string".to_string(), description: "URL of screenshot to show on device screen".to_string(), items: None }),
                    ]),
                    required: vec!["device".to_string()],
                },
            },
            ClaudeTool {
                name: "blender_generate_flowchart".to_string(),
                description: "Generate an animated Manim flowchart with process boxes, decision diamonds, and arrows. Use for process diagrams, system architecture flows, and explainer videos.".to_string(),
                input_schema: InputSchema {
                    schema_type: "object".to_string(),
                    properties: HashMap::from([
                        ("nodes".to_string(), PropertyDefinition { prop_type: "string".to_string(), description: "JSON array of nodes: [{\"id\":\"start\",\"label\":\"Start\",\"type\":\"start\"},{\"id\":\"step1\",\"label\":\"Process Data\",\"type\":\"process\"},...]".to_string(), items: None }),
                        ("edges".to_string(), PropertyDefinition { prop_type: "string".to_string(), description: "JSON array of connections: [{\"from\":\"start\",\"to\":\"step1\"},{\"from\":\"decide\",\"to\":\"step2\",\"label\":\"Yes\"},...]".to_string(), items: None }),
                        ("title".to_string(), PropertyDefinition { prop_type: "string".to_string(), description: "Chart heading text".to_string(), items: None }),
                        ("duration".to_string(), PropertyDefinition { prop_type: "number".to_string(), description: "Clip length in seconds (default: 12)".to_string(), items: None }),
                        ("style".to_string(), PropertyDefinition { prop_type: "string".to_string(), description: "'dark' | 'light' | 'blue'".to_string(), items: None }),
                    ]),
                    required: vec!["nodes".to_string(), "edges".to_string()],
                },
            },
            ClaudeTool {
                name: "blender_generate_3d_math".to_string(),
                description: "Generate a 3D mathematics animation using Manim's ThreeDScene — ideal for academic content, math tutorials, and STEM explainer videos.".to_string(),
                input_schema: InputSchema {
                    schema_type: "object".to_string(),
                    properties: HashMap::from([
                        ("scene_type".to_string(), PropertyDefinition { prop_type: "string".to_string(), description: "'surface' | 'curve' | 'vector_field' | 'torus'".to_string(), items: None }),
                        ("title".to_string(), PropertyDefinition { prop_type: "string".to_string(), description: "Title text".to_string(), items: None }),
                        ("function".to_string(), PropertyDefinition { prop_type: "string".to_string(), description: "'wave' | 'sin' | 'cos' | 'saddle' | 'paraboloid' | 'ripple'".to_string(), items: None }),
                        ("duration".to_string(), PropertyDefinition { prop_type: "number".to_string(), description: "Clip length in seconds (default: 12)".to_string(), items: None }),
                        ("color".to_string(), PropertyDefinition { prop_type: "string".to_string(), description: "'BLUE' | 'RED' | 'GREEN' | 'GOLD' | 'PURPLE' | 'TEAL'".to_string(), items: None }),
                    ]),
                    required: vec![],
                },
            },
            ClaudeTool {
                name: "blender_generate_code_animation".to_string(),
                description: "Generate an animated code syntax-highlighting clip — ideal for tech tutorials, YouTube programming content, and developer explainer videos.".to_string(),
                input_schema: InputSchema {
                    schema_type: "object".to_string(),
                    properties: HashMap::from([
                        ("code".to_string(), PropertyDefinition { prop_type: "string".to_string(), description: "Source code to display and animate".to_string(), items: None }),
                        ("language".to_string(), PropertyDefinition { prop_type: "string".to_string(), description: "'python' | 'javascript' | 'rust' | 'cpp' | 'java' | 'bash' | 'sql' | 'typescript' | 'go'".to_string(), items: None }),
                        ("title".to_string(), PropertyDefinition { prop_type: "string".to_string(), description: "Heading above code block".to_string(), items: None }),
                        ("highlight_lines".to_string(), PropertyDefinition { prop_type: "string".to_string(), description: "JSON array of 1-indexed line numbers e.g. '[3,7,11]'".to_string(), items: None }),
                        ("reveal_mode".to_string(), PropertyDefinition { prop_type: "string".to_string(), description: "'line_by_line' | 'all_at_once' | 'block'".to_string(), items: None }),
                        ("duration".to_string(), PropertyDefinition { prop_type: "number".to_string(), description: "Clip length in seconds (default: 12)".to_string(), items: None }),
                        ("style".to_string(), PropertyDefinition { prop_type: "string".to_string(), description: "'monokai' | 'dracula' | 'solarized-dark'".to_string(), items: None }),
                    ]),
                    required: vec!["code".to_string()],
                },
            },
            ClaudeTool {
                name: "blender_generate_timeline".to_string(),
                description: "Generate an animated timeline, project roadmap, or Gantt-style clip — great for business explainers, project demos, and history videos.".to_string(),
                input_schema: InputSchema {
                    schema_type: "object".to_string(),
                    properties: HashMap::from([
                        ("events".to_string(), PropertyDefinition { prop_type: "string".to_string(), description: "JSON array: [{\"date\":\"Jan\",\"label\":\"Kickoff\",\"color\":\"BLUE\"},{\"date\":\"Mar\",\"label\":\"Launch\",\"color\":\"GREEN\"},...]".to_string(), items: None }),
                        ("title".to_string(), PropertyDefinition { prop_type: "string".to_string(), description: "Heading text".to_string(), items: None }),
                        ("duration".to_string(), PropertyDefinition { prop_type: "number".to_string(), description: "Clip length in seconds (default: 12)".to_string(), items: None }),
                        ("style".to_string(), PropertyDefinition { prop_type: "string".to_string(), description: "'dark' | 'light' | 'gradient'".to_string(), items: None }),
                        ("orientation".to_string(), PropertyDefinition { prop_type: "string".to_string(), description: "'horizontal' | 'vertical'".to_string(), items: None }),
                    ]),
                    required: vec!["events".to_string()],
                },
            },
            ClaudeTool {
                name: "blender_generate_network_graph".to_string(),
                description: "Generate an animated network or knowledge graph — great for org charts, concept maps, AI/ML topic maps, and relationship visualizations.".to_string(),
                input_schema: InputSchema {
                    schema_type: "object".to_string(),
                    properties: HashMap::from([
                        ("nodes".to_string(), PropertyDefinition { prop_type: "string".to_string(), description: "JSON array: [{\"id\":\"A\",\"label\":\"AI\",\"color\":\"BLUE\"},{\"id\":\"B\",\"label\":\"ML\",\"color\":\"GREEN\"},...]".to_string(), items: None }),
                        ("edges".to_string(), PropertyDefinition { prop_type: "string".to_string(), description: "JSON array: [{\"from\":\"A\",\"to\":\"B\"},{\"from\":\"A\",\"to\":\"C\",\"directed\":true},...]".to_string(), items: None }),
                        ("title".to_string(), PropertyDefinition { prop_type: "string".to_string(), description: "Heading text".to_string(), items: None }),
                        ("layout".to_string(), PropertyDefinition { prop_type: "string".to_string(), description: "'radial' | 'circular' | 'spring'".to_string(), items: None }),
                        ("duration".to_string(), PropertyDefinition { prop_type: "number".to_string(), description: "Clip length in seconds (default: 12)".to_string(), items: None }),
                        ("style".to_string(), PropertyDefinition { prop_type: "string".to_string(), description: "'dark' | 'neon'".to_string(), items: None }),
                    ]),
                    required: vec!["nodes".to_string(), "edges".to_string()],
                },
            },
            ClaudeTool {
                name: "blender_generate_logo_reveal".to_string(),
                description: "Generate a 3D extruded text / logo reveal animation in Blender — the most popular Fiverr motion graphics request.".to_string(),
                input_schema: InputSchema {
                    schema_type: "object".to_string(),
                    properties: HashMap::from([
                        ("text".to_string(), PropertyDefinition { prop_type: "string".to_string(), description: "Brand name or main text to extrude and animate".to_string(), items: None }),
                        ("tagline".to_string(), PropertyDefinition { prop_type: "string".to_string(), description: "Optional secondary line (slogan / subtitle)".to_string(), items: None }),
                        ("style".to_string(), PropertyDefinition { prop_type: "string".to_string(), description: "'extrude_reveal' | 'zoom_in' | 'split' | 'typewriter'".to_string(), items: None }),
                        ("color".to_string(), PropertyDefinition { prop_type: "string".to_string(), description: "JSON RGBA float array e.g. '[0.1, 0.5, 1.0, 1.0]'".to_string(), items: None }),
                        ("bg_color".to_string(), PropertyDefinition { prop_type: "string".to_string(), description: "JSON RGBA float array for background".to_string(), items: None }),
                        ("duration".to_string(), PropertyDefinition { prop_type: "number".to_string(), description: "Clip length in seconds (default: 6)".to_string(), items: None }),
                    ]),
                    required: vec!["text".to_string()],
                },
            },
            ClaudeTool {
                name: "blender_generate_abstract_bg".to_string(),
                description: "Generate an animated abstract background loop in Blender — useful as a video backdrop, intro overlay, or stock footage asset.".to_string(),
                input_schema: InputSchema {
                    schema_type: "object".to_string(),
                    properties: HashMap::from([
                        ("style".to_string(), PropertyDefinition { prop_type: "string".to_string(), description: "'geometric' | 'waves' | 'particles' | 'grid' | 'gradient'".to_string(), items: None }),
                        ("primary_color".to_string(), PropertyDefinition { prop_type: "string".to_string(), description: "JSON RGBA float array e.g. '[0.05, 0.2, 0.8, 1.0]'".to_string(), items: None }),
                        ("secondary_color".to_string(), PropertyDefinition { prop_type: "string".to_string(), description: "JSON RGBA float array e.g. '[0.8, 0.1, 0.5, 1.0]'".to_string(), items: None }),
                        ("duration".to_string(), PropertyDefinition { prop_type: "number".to_string(), description: "Clip length in seconds (default: 8)".to_string(), items: None }),
                    ]),
                    required: vec![],
                },
            },
            ClaudeTool {
                name: "blender_generate_countdown".to_string(),
                description: "Generate a 3D animated countdown timer in Blender — useful for YouTube intros, live-stream countdowns, and event teasers.".to_string(),
                input_schema: InputSchema {
                    schema_type: "object".to_string(),
                    properties: HashMap::from([
                        ("start_number".to_string(), PropertyDefinition { prop_type: "number".to_string(), description: "Count from (e.g. 10, 5, 3)".to_string(), items: None }),
                        ("end_number".to_string(), PropertyDefinition { prop_type: "number".to_string(), description: "Count to (e.g. 1 or 0)".to_string(), items: None }),
                        ("style".to_string(), PropertyDefinition { prop_type: "string".to_string(), description: "'bold' | 'neon' | 'minimal' | 'cinematic'".to_string(), items: None }),
                        ("color".to_string(), PropertyDefinition { prop_type: "string".to_string(), description: "JSON RGBA float array for number material".to_string(), items: None }),
                        ("show_ring".to_string(), PropertyDefinition { prop_type: "string".to_string(), description: "'true' or 'false'".to_string(), items: None }),
                        ("duration".to_string(), PropertyDefinition { prop_type: "number".to_string(), description: "Total clip duration (0 = 1s per count)".to_string(), items: None }),
                    ]),
                    required: vec![],
                },
            },
            ClaudeTool {
                name: "blender_generate_particle_confetti".to_string(),
                description: "Generate an animated particle burst in Blender — confetti, snow, stars, rain, or bubbles. Great for celebration intros, event videos, and festive overlays.".to_string(),
                input_schema: InputSchema {
                    schema_type: "object".to_string(),
                    properties: HashMap::from([
                        ("style".to_string(), PropertyDefinition { prop_type: "string".to_string(), description: "'confetti' | 'snow' | 'stars' | 'rain' | 'bubbles'".to_string(), items: None }),
                        ("count".to_string(), PropertyDefinition { prop_type: "number".to_string(), description: "Number of particles (default: 400)".to_string(), items: None }),
                        ("duration".to_string(), PropertyDefinition { prop_type: "number".to_string(), description: "Clip length in seconds (default: 6)".to_string(), items: None }),
                        ("primary_color".to_string(), PropertyDefinition { prop_type: "string".to_string(), description: "JSON RGBA float array e.g. '[1,0.3,0.1,1]'".to_string(), items: None }),
                        ("secondary_color".to_string(), PropertyDefinition { prop_type: "string".to_string(), description: "JSON RGBA float array for second color".to_string(), items: None }),
                        ("bg_color".to_string(), PropertyDefinition { prop_type: "string".to_string(), description: "JSON RGBA float array for background".to_string(), items: None }),
                    ]),
                    required: vec![],
                },
            },
            ClaudeTool {
                name: "blender_generate_rigid_body_drop".to_string(),
                description: "Generate a physics rigid-body drop animation in Blender — 3D extruded letters or geometric objects fall and collide. Very popular for logo reveals and kinetic titles.".to_string(),
                input_schema: InputSchema {
                    schema_type: "object".to_string(),
                    properties: HashMap::from([
                        ("text".to_string(), PropertyDefinition { prop_type: "string".to_string(), description: "Text to extrude as falling 3D letters (when object_type='text')".to_string(), items: None }),
                        ("object_type".to_string(), PropertyDefinition { prop_type: "string".to_string(), description: "'text' | 'spheres' | 'cubes' | 'mixed'".to_string(), items: None }),
                        ("count".to_string(), PropertyDefinition { prop_type: "number".to_string(), description: "Number of objects if not text (default: 12)".to_string(), items: None }),
                        ("color".to_string(), PropertyDefinition { prop_type: "string".to_string(), description: "JSON RGBA float array".to_string(), items: None }),
                        ("bg_color".to_string(), PropertyDefinition { prop_type: "string".to_string(), description: "JSON RGBA float array for background".to_string(), items: None }),
                        ("style".to_string(), PropertyDefinition { prop_type: "string".to_string(), description: "'dark' | 'bright' | 'neon'".to_string(), items: None }),
                        ("duration".to_string(), PropertyDefinition { prop_type: "number".to_string(), description: "Clip length in seconds (default: 5)".to_string(), items: None }),
                    ]),
                    required: vec![],
                },
            },
            ClaudeTool {
                name: "blender_generate_camera_path".to_string(),
                description: "Generate a smooth camera fly-through or orbit animation in Blender — orbit, helix, arc, dolly zoom, or linear flythrough around a 3D scene.".to_string(),
                input_schema: InputSchema {
                    schema_type: "object".to_string(),
                    properties: HashMap::from([
                        ("path_type".to_string(), PropertyDefinition { prop_type: "string".to_string(), description: "'orbit' | 'helix' | 'arc' | 'dolly_zoom' | 'flythrough'".to_string(), items: None }),
                        ("subject".to_string(), PropertyDefinition { prop_type: "string".to_string(), description: "'spheres' | 'cubes' | 'text' | 'abstract' | 'landscape'".to_string(), items: None }),
                        ("title".to_string(), PropertyDefinition { prop_type: "string".to_string(), description: "Optional 3D text placed in scene".to_string(), items: None }),
                        ("color".to_string(), PropertyDefinition { prop_type: "string".to_string(), description: "JSON RGBA float array for objects".to_string(), items: None }),
                        ("bg_color".to_string(), PropertyDefinition { prop_type: "string".to_string(), description: "JSON RGBA float array for background".to_string(), items: None }),
                        ("style".to_string(), PropertyDefinition { prop_type: "string".to_string(), description: "'cinematic' | 'minimal' | 'neon'".to_string(), items: None }),
                        ("duration".to_string(), PropertyDefinition { prop_type: "number".to_string(), description: "Clip length in seconds (default: 8)".to_string(), items: None }),
                    ]),
                    required: vec![],
                },
            },
            ClaudeTool {
                name: "blender_generate_toon_scene".to_string(),
                description: "Generate an NPR cartoon / toon-shaded Blender scene with bold outlines and flat colours — great for animated explainers, kids content, and stylised brand videos.".to_string(),
                input_schema: InputSchema {
                    schema_type: "object".to_string(),
                    properties: HashMap::from([
                        ("subject".to_string(), PropertyDefinition { prop_type: "string".to_string(), description: "'characters' | 'robots' | 'landscape' | 'abstract' | 'logo'".to_string(), items: None }),
                        ("title".to_string(), PropertyDefinition { prop_type: "string".to_string(), description: "Optional text label in scene".to_string(), items: None }),
                        ("outline_color".to_string(), PropertyDefinition { prop_type: "string".to_string(), description: "JSON RGBA float array for outlines".to_string(), items: None }),
                        ("primary_color".to_string(), PropertyDefinition { prop_type: "string".to_string(), description: "JSON RGBA float array for main objects".to_string(), items: None }),
                        ("bg_color".to_string(), PropertyDefinition { prop_type: "string".to_string(), description: "JSON RGBA float array for background".to_string(), items: None }),
                        ("outline_width".to_string(), PropertyDefinition { prop_type: "number".to_string(), description: "Outline thickness 0.5–5.0 (default: 1.5)".to_string(), items: None }),
                        ("flat_shading".to_string(), PropertyDefinition { prop_type: "string".to_string(), description: "'true' for pure cartoon flat look".to_string(), items: None }),
                        ("duration".to_string(), PropertyDefinition { prop_type: "number".to_string(), description: "Clip length in seconds (default: 6)".to_string(), items: None }),
                    ]),
                    required: vec![],
                },
            },
            ClaudeTool {
                name: "blender_generate_grease_pencil_reveal".to_string(),
                description: "Generate a whiteboard / sketch draw-on text reveal using Blender Grease Pencil — letters appear stroke-by-stroke. Supports whiteboard, neon, sketch, and chalkboard styles.".to_string(),
                input_schema: InputSchema {
                    schema_type: "object".to_string(),
                    properties: HashMap::from([
                        ("text".to_string(), PropertyDefinition { prop_type: "string".to_string(), description: "Text to draw (max 12 characters)".to_string(), items: None }),
                        ("style".to_string(), PropertyDefinition { prop_type: "string".to_string(), description: "'whiteboard' | 'neon' | 'sketch' | 'chalkboard'".to_string(), items: None }),
                        ("color".to_string(), PropertyDefinition { prop_type: "string".to_string(), description: "JSON RGBA float array for strokes".to_string(), items: None }),
                        ("bg_color".to_string(), PropertyDefinition { prop_type: "string".to_string(), description: "JSON RGBA float array for background".to_string(), items: None }),
                        ("stroke_width".to_string(), PropertyDefinition { prop_type: "number".to_string(), description: "Line thickness 10–200 (default: 50)".to_string(), items: None }),
                        ("duration".to_string(), PropertyDefinition { prop_type: "number".to_string(), description: "Clip length in seconds (default: 6)".to_string(), items: None }),
                    ]),
                    required: vec!["text".to_string()],
                },
            },
            ClaudeTool {
                name: "blender_generate_geometry_scatter".to_string(),
                description: "Generate a procedural instance-scatter animation in Blender — objects distributed across a plane, sphere, torus, or grid with animated wave displacement.".to_string(),
                input_schema: InputSchema {
                    schema_type: "object".to_string(),
                    properties: HashMap::from([
                        ("instance_type".to_string(), PropertyDefinition { prop_type: "string".to_string(), description: "'cubes' | 'spheres' | 'stars' | 'arrows' | 'crystals'".to_string(), items: None }),
                        ("surface".to_string(), PropertyDefinition { prop_type: "string".to_string(), description: "'plane' | 'sphere' | 'torus' | 'grid'".to_string(), items: None }),
                        ("count".to_string(), PropertyDefinition { prop_type: "number".to_string(), description: "Number of instances (default: 200)".to_string(), items: None }),
                        ("primary_color".to_string(), PropertyDefinition { prop_type: "string".to_string(), description: "JSON RGBA float array".to_string(), items: None }),
                        ("secondary_color".to_string(), PropertyDefinition { prop_type: "string".to_string(), description: "JSON RGBA float array for second color variant".to_string(), items: None }),
                        ("bg_color".to_string(), PropertyDefinition { prop_type: "string".to_string(), description: "JSON RGBA float array for background".to_string(), items: None }),
                        ("animated".to_string(), PropertyDefinition { prop_type: "string".to_string(), description: "'true' for wave displacement animation".to_string(), items: None }),
                        ("scale".to_string(), PropertyDefinition { prop_type: "number".to_string(), description: "Instance scale multiplier (default: 1.0)".to_string(), items: None }),
                        ("duration".to_string(), PropertyDefinition { prop_type: "number".to_string(), description: "Clip length in seconds (default: 8)".to_string(), items: None }),
                    ]),
                    required: vec![],
                },
            },
            ClaudeTool {
                name: "blender_generate_text_animation".to_string(),
                description: "Generate a kinetic typography / text animation clip using Manim. Supports 8 modes: letter_by_letter, word_by_word, typewriter, wave, zoom_burst, spin_in, color_cycle, highlight_words.".to_string(),
                input_schema: InputSchema {
                    schema_type: "object".to_string(),
                    properties: HashMap::from([
                        ("text".to_string(), PropertyDefinition { prop_type: "string".to_string(), description: "The text to animate".to_string(), items: None }),
                        ("mode".to_string(), PropertyDefinition { prop_type: "string".to_string(), description: "'letter_by_letter' | 'word_by_word' | 'typewriter' | 'wave' | 'zoom_burst' | 'spin_in' | 'color_cycle' | 'highlight_words'".to_string(), items: None }),
                        ("color".to_string(), PropertyDefinition { prop_type: "string".to_string(), description: "Manim colour name e.g. 'BLUE', 'RED', 'YELLOW'".to_string(), items: None }),
                        ("font_size".to_string(), PropertyDefinition { prop_type: "number".to_string(), description: "Font size (default: 48)".to_string(), items: None }),
                        ("duration".to_string(), PropertyDefinition { prop_type: "number".to_string(), description: "Clip length in seconds (default: 8)".to_string(), items: None }),
                    ]),
                    required: vec!["text".to_string()],
                },
            },
            ClaudeTool {
                name: "blender_generate_vector_field".to_string(),
                description: "Generate an animated vector field / flow field visualisation using Manim ArrowVectorField and StreamLines. Great for physics, fluid dynamics, and EM field illustrations.".to_string(),
                input_schema: InputSchema {
                    schema_type: "object".to_string(),
                    properties: HashMap::from([
                        ("field_type".to_string(), PropertyDefinition { prop_type: "string".to_string(), description: "'circular' | 'sink' | 'source' | 'saddle' | 'linear' | 'complex'".to_string(), items: None }),
                        ("title".to_string(), PropertyDefinition { prop_type: "string".to_string(), description: "Optional title label".to_string(), items: None }),
                        ("show_stream_lines".to_string(), PropertyDefinition { prop_type: "string".to_string(), description: "'true' to add animated StreamLines overlay".to_string(), items: None }),
                        ("color".to_string(), PropertyDefinition { prop_type: "string".to_string(), description: "Manim colour name for arrows".to_string(), items: None }),
                        ("duration".to_string(), PropertyDefinition { prop_type: "number".to_string(), description: "Clip length in seconds (default: 10)".to_string(), items: None }),
                    ]),
                    required: vec![],
                },
            },
            ClaudeTool {
                name: "blender_generate_matrix_transform".to_string(),
                description: "Generate a linear algebra matrix transformation animation using Manim LinearTransformationScene — shows how a 2×2 matrix transforms the plane with basis vectors and optional determinant annotation.".to_string(),
                input_schema: InputSchema {
                    schema_type: "object".to_string(),
                    properties: HashMap::from([
                        ("matrix".to_string(), PropertyDefinition { prop_type: "string".to_string(), description: "JSON 2×2 matrix e.g. '[[0,-1],[1,0]]' (default: 90° rotation)".to_string(), items: None }),
                        ("title".to_string(), PropertyDefinition { prop_type: "string".to_string(), description: "Scene title".to_string(), items: None }),
                        ("show_vectors".to_string(), PropertyDefinition { prop_type: "string".to_string(), description: "'true' to show sample vectors being transformed".to_string(), items: None }),
                        ("show_det".to_string(), PropertyDefinition { prop_type: "string".to_string(), description: "'true' to annotate the determinant".to_string(), items: None }),
                        ("duration".to_string(), PropertyDefinition { prop_type: "number".to_string(), description: "Clip length in seconds (default: 12)".to_string(), items: None }),
                    ]),
                    required: vec![],
                },
            },
            ClaudeTool {
                name: "blender_generate_polar_graph".to_string(),
                description: "Generate an animated polar / complex plane graph using Manim PolarPlane, ComplexPlane, or NumberPlane. Supports rose curves, lemniscates, spirals, cardioids, and standard function plots.".to_string(),
                input_schema: InputSchema {
                    schema_type: "object".to_string(),
                    properties: HashMap::from([
                        ("plane_type".to_string(), PropertyDefinition { prop_type: "string".to_string(), description: "'polar' | 'complex' | 'number_plane'".to_string(), items: None }),
                        ("function".to_string(), PropertyDefinition { prop_type: "string".to_string(), description: "'rose' | 'lemniscate' | 'spiral' | 'cardioid' | 'circle' | 'sin' | 'parabola'".to_string(), items: None }),
                        ("title".to_string(), PropertyDefinition { prop_type: "string".to_string(), description: "Scene title".to_string(), items: None }),
                        ("color".to_string(), PropertyDefinition { prop_type: "string".to_string(), description: "Manim colour name for the curve".to_string(), items: None }),
                        ("k_value".to_string(), PropertyDefinition { prop_type: "number".to_string(), description: "k for rose curve (number of petals, default: 4)".to_string(), items: None }),
                        ("duration".to_string(), PropertyDefinition { prop_type: "number".to_string(), description: "Clip length in seconds (default: 12)".to_string(), items: None }),
                    ]),
                    required: vec![],
                },
            },
            ClaudeTool {
                name: "blender_generate_geometry_proof".to_string(),
                description: "Generate an animated geometry proof using Manim — supports Pythagorean theorem, circle area (inscribed polygon limit), triangle angle sum, and boolean shape operations.".to_string(),
                input_schema: InputSchema {
                    schema_type: "object".to_string(),
                    properties: HashMap::from([
                        ("proof_type".to_string(), PropertyDefinition { prop_type: "string".to_string(), description: "'pythagorean' | 'circle_area' | 'triangle_sum' | 'boolean_ops'".to_string(), items: None }),
                        ("title".to_string(), PropertyDefinition { prop_type: "string".to_string(), description: "Scene title".to_string(), items: None }),
                        ("color_a".to_string(), PropertyDefinition { prop_type: "string".to_string(), description: "Manim colour for shape A".to_string(), items: None }),
                        ("color_b".to_string(), PropertyDefinition { prop_type: "string".to_string(), description: "Manim colour for shape B".to_string(), items: None }),
                        ("show_labels".to_string(), PropertyDefinition { prop_type: "string".to_string(), description: "'true' to show formula labels".to_string(), items: None }),
                        ("duration".to_string(), PropertyDefinition { prop_type: "number".to_string(), description: "Clip length in seconds (default: 14)".to_string(), items: None }),
                    ]),
                    required: vec![],
                },
            },
            ClaudeTool {
                name: "blender_generate_animation".to_string(),
                description: "Generate ANY Manim animation from a natural language description using LLM code generation. Use this for kinetic typography, abstract motion graphics, step-by-step explanations, or any creative animation that doesn't fit the specific latex/chart/scene categories.".to_string(),
                input_schema: InputSchema {
                    schema_type: "object".to_string(),
                    properties: HashMap::from([
                        ("description".to_string(), PropertyDefinition { prop_type: "string".to_string(), description: "Natural language description of the animation to generate".to_string(), items: None }),
                        ("duration".to_string(), PropertyDefinition { prop_type: "number".to_string(), description: "Clip length in seconds (default: 10)".to_string(), items: None }),
                        ("background_style".to_string(), PropertyDefinition { prop_type: "string".to_string(), description: "Background: 'dark' (default) | 'light' | 'gradient'".to_string(), items: None }),
                        ("composite_over_scene".to_string(), PropertyDefinition { prop_type: "boolean".to_string(), description: "If true, composite the Manim animation over a Blender 3D background scene".to_string(), items: None }),
                        ("include_narration".to_string(), PropertyDefinition { prop_type: "boolean".to_string(), description: "Optional. If true, generate narration audio and a narrated video variant for this animation".to_string(), items: None }),
                        ("narration_text".to_string(), PropertyDefinition { prop_type: "string".to_string(), description: "Optional narration script to speak during the animation".to_string(), items: None }),
                        ("narration_speaker".to_string(), PropertyDefinition { prop_type: "string".to_string(), description: "Optional VibeVoice speaker preset name, e.g. 'Emma' or 'Carter'".to_string(), items: None }),
                    ]),
                    required: vec!["description".to_string()],
                },
            },
            ClaudeTool {
                name: "blender_generate_chart".to_string(),
                description: "Generate an animated data visualisation clip (bar chart, line chart, pie chart, animated counter, or scatter plot) using Manim. Returns a video URL.".to_string(),
                input_schema: InputSchema {
                    schema_type: "object".to_string(),
                    properties: HashMap::from([
                        ("chart_type".to_string(), PropertyDefinition { prop_type: "string".to_string(), description: "Chart type: 'bar_chart' | 'line_chart' | 'pie_chart' | 'counter' | 'scatter'".to_string(), items: None }),
                        ("title".to_string(), PropertyDefinition { prop_type: "string".to_string(), description: "Chart title text".to_string(), items: None }),
                        ("data".to_string(), PropertyDefinition { prop_type: "string".to_string(), description: "JSON array of data values, e.g. '[42,78,55,90]'".to_string(), items: None }),
                        ("labels".to_string(), PropertyDefinition { prop_type: "string".to_string(), description: "JSON array of labels, e.g. '[\"Q1\",\"Q2\",\"Q3\",\"Q4\"]'".to_string(), items: None }),
                        ("duration".to_string(), PropertyDefinition { prop_type: "number".to_string(), description: "Clip length in seconds (default: 10)".to_string(), items: None }),
                        ("colors".to_string(), PropertyDefinition { prop_type: "string".to_string(), description: "JSON array of Manim colour names, e.g. '[\"BLUE\",\"GREEN\"]'".to_string(), items: None }),
                    ]),
                    required: vec!["chart_type".to_string(), "title".to_string(), "data".to_string()],
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
