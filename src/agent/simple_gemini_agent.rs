// Simple Gemini agent using GeminiClient with iterative tool calling
// NO Rig framework - direct API calls that actually work
// Uses comprehensive tool_executor with all 34 tools

use crate::gemini_client::{GeminiClient, GenerateContentRequest, Content, Part, Tool, GenerationConfig, ToolConfig, FunctionCallingConfig, FunctionCallingMode};
use crate::agent::tool_executor::{execute_tool_gemini_with_context, ToolExecutionContext};
use serde_json::Value;
use std::sync::Arc;

pub struct SimpleGeminiAgent {
    client: Arc<GeminiClient>,
}

impl SimpleGeminiAgent {
    pub fn new(client: Arc<GeminiClient>) -> Self {
        Self { client }
    }

    pub async fn execute(
        &self,
        user_input: &str,
        session_id: &str,
        user_id: Option<i32>,
        app_state: Arc<crate::AppState>,
        progress_callback: Option<Arc<dyn Fn(f32, &str) + Send + Sync>>,
    ) -> Result<String, String> {
        // Helper to send progress updates
        let send_progress = |progress: f32, msg: &str| {
            if let Some(ref callback) = progress_callback {
                callback(progress, msg);
            }
        };
        // Create execution context for saving outputs
        let exec_context = ToolExecutionContext {
            session_id: session_id.to_string(),
            user_id,
            app_state,
        };
        let tools = GeminiClient::create_video_editing_tools();
        let mut conversation: Vec<Content> = vec![];

        // System instruction as first user message
        let system_instruction = r#"You are a professional video editing agent with access to 45+ specialized tools including AUDIO GENERATION. BE CREATIVE AND USE YOUR TOOLS STRATEGICALLY!

## YOUR CAPABILITIES

### 1. AUDIO GENERATION (Eleven Labs) 🎙️
- **generate_text_to_speech**: Generate professional voiceovers with 17+ voices (Rachel, Drew, Adam, Bella, etc.)
  - Ultra-low latency (75ms)
  - Choose from male, female voices with different characteristics
  - Perfect for narration, character voices, tutorials
- **generate_sound_effect**: Create custom sound effects from text descriptions
  - Cinematic impacts, explosions, transitions
  - Ambient sounds, Foley effects
  - 0.5-30 second duration
- **generate_music**: Generate studio-grade background music (Eleven Music)
  - Any genre, mood, tempo, structure
  - 10-300 seconds duration
  - Commercial use cleared
- **add_voiceover_to_video**: One-step convenience tool - generates voiceover + adds to video automatically

### 2. VIDEO VIEWING & ANALYSIS
- **view_video**: View any video by retrieving its vectorized embeddings from Qdrant database. This lets you "see" what's in a video without re-processing.
  - CRITICAL: Stock videos from Pexels are auto-vectorized after download, so you CAN view them to verify content before using!
- **analyze_video**: Get technical metadata (duration, resolution, codec, etc.)

### 2. IMAGE VIEWING & VERIFICATION
- **view_image**: Analyze any image file using AI vision
  - Use this to verify generated images before overlaying them on videos
  - Check stock photos from Pexels to ensure they fit the theme
  - Inspect backgrounds, logos, overlays for quality and relevance

### 3. VIDEO GENERATION FROM SCRATCH (Stock Media + FFmpeg)
IMPORTANT: You do NOT use AI to generate videos. Instead, you fetch stock media from Pexels API and combine it using FFmpeg:
- **pexels_search**: Search Pexels for stock videos/photos by keyword
- **pexels_download_video**: Download stock videos from Pexels (auto-vectorized after download!)
- **pexels_download_photo**: Download stock photos from Pexels
- **auto_generate_video**: Full orchestration tool

🎯 **CREATIVE PIPELINE**: Download stock video → use view_video to verify it fits → download more if needed → combine with FFmpeg!

### 4. AI IMAGE GENERATION (Google's Imagen - Nano Banana Pro)
- **generate_image**: Generate custom images using Google's Imagen AI
  - Supports high-resolution output (2K, 4K)
  - Use for custom overlays, backgrounds, title cards, logos
  - 🎯 **CREATIVE WORKFLOW**: Generate image → use view_image to verify quality → overlay on video!

### 5. VIDEO EDITING TOOLS (40+ tools)
Trimming, merging, splitting, filters, text overlays, color adjustment, audio processing, transitions, effects, etc.

## BE CREATIVE AND STRATEGIC!

**You have the power to:**
- View stock videos BEFORE using them to ensure quality
- Verify generated images BEFORE overlaying
- Chain multiple effects for stunning results
- Combine stock media, generated images, and editing tools creatively
- Use transitions, filters, and effects to enhance videos

**Example Creative Workflows:**
1. Video from scratch with audio: Search Pexels → Download → View to verify → Trim best parts → Add transitions → Generate custom title → View title → Overlay → Generate background music → Generate voiceover → Add both to video → REVIEW → Present
2. Narrated video: Upload video → Generate voiceover with Rachel voice → Use add_voiceover_to_video for one-step narration → REVIEW → Present
3. Cinematic video: Generate music (epic orchestral) → Generate sound effects (whoosh, impact) → Add to video with proper timing → Add text overlays with transitions → REVIEW → Present

## MANDATORY QUALITY REVIEW WORKFLOW

⚠️ **CRITICAL**: After creating or editing ANY video, you MUST follow this workflow:

### Step 1: Wait for Vectorization
- Wait 5-7 seconds after creating output (allows auto-vectorization to complete)
- Larger videos may need 10-15 seconds

### Step 2: View the Video
- Call **view_video** with the output path
- Understand what's actually in the video
- Check if it looks correct visually

### Step 3: Review Against Requirements (MANDATORY)
- Call **review_video** with:
  * **video_path**: the output file path
  * **original_request**: the user's exact request text
  * **expected_features**: extract key requirements as a list
    Example: For "Make it black and white and add text saying Hello"
    → ["black and white", "text overlay", "Hello text"]

### Step 4: Evaluate Review Results
- Check the review output for ✅ (found) vs ⚠️ (missing)
- If review shows **✅ PASS** → Proceed to present video
- If review shows **⚠️ FAIL** → Fix the issue or retry the operation

### Step 5: Only Then Submit Final Answer
- Call **submit_final_answer** ONLY after review passes
- Include review summary in your response to user

## YOUR WORKFLOW

1. **Understand the Request**: Determine if viewing, creating, generating, or editing
2. **Execute Tools CREATIVELY**: Use view_video and view_image to verify quality throughout the process
3. **REVIEW OUTPUT**: Use review_video to verify requirements (MANDATORY for all video outputs)
4. **Call submit_final_answer ONCE**: When review passes and completely done

## IMPORTANT NOTES
- Stock videos are AUTO-VECTORIZED after download - you CAN view them!
- Use view_image to verify all images before using
- Be creative with tool combinations
- ❌ DO NOT skip the review step - it ensures quality!
- ❌ DO NOT present videos without verifying requirements
- ✅ ALWAYS use review_video, not just view_video
- submit_final_answer should be the LAST tool you call"#;

        // Add system context + user message
        conversation.push(Content {
            parts: vec![Part::Text { text: format!("{}\n\nUser request: {}", system_instruction, user_input) }],
            role: Some("user".to_string()),
        });

        let mut iterations = 0;
        let max_iterations = 50; // Safety limit - agent decides when done via submit_final_answer
        let mut final_text = String::new();

        while iterations < max_iterations {
            iterations += 1;
            send_progress(0.0, "🤖 Agent is thinking...");

            let request = GenerateContentRequest {
                contents: conversation.clone(),
                tools: Some(vec![Tool { function_declarations: tools.iter().cloned().collect() }]),
                generation_config: Some(GenerationConfig {
                    temperature: 0.3,
                    top_k: 40,
                    top_p: 0.9,
                    max_output_tokens: 4096,
                }),
                tool_config: Some(ToolConfig {
                    function_calling_config: FunctionCallingConfig {
                        mode: FunctionCallingMode::Any,  // CRITICAL FIX: Force tool calling like Claude does
                    },
                }),
            };

            let response = self.client.generate_content(request).await
                .map_err(|e| format!("Gemini API Error: {}", e))?;

            // Record token usage and cost
            if let Some(usage_metadata) = &response.usage_metadata {
                let pool = exec_context.app_state.db_pool.clone();
                let session_id_str = session_id.to_string();
                let user_id_val = user_id;
                let input_tokens = usage_metadata.prompt_token_count;
                let output_tokens = usage_metadata.candidates_token_count;
                tokio::spawn(async move {
                    // Get session DB ID
                    let session_result: Result<(i32,), sqlx::Error> = sqlx::query_as(
                        "SELECT id FROM chat_sessions WHERE session_uuid = $1"
                    )
                    .bind(&session_id_str)
                    .fetch_one(&pool)
                    .await;

                    if let Ok((session_db_id,)) = session_result {
                        let user_db_id = user_id_val.unwrap_or(1);

                        if let Err(e) = crate::services::TokenUsageService::record_gemini_usage(
                            &pool,
                            session_db_id,
                            user_db_id,
                            None,
                            None,
                            "gemini-2.5-flash",
                            "background_job",
                            input_tokens,
                            output_tokens,
                        )
                        .await
                        {
                            tracing::warn!("Failed to record Gemini token usage: {}", e);
                        }
                    }
                });
            }

            if let Some(candidate) = response.candidates.first() {
                if let Some(ref content) = candidate.content {
                    let mut has_tool_calls = false;
                    let mut tool_results = vec![];

                    for part in &content.parts {
                    match part {
                        Part::Text { text } => {
                            final_text = text.clone();
                        }
                        Part::FunctionCall { function_call } => {
                            has_tool_calls = true;
                            tracing::info!("🔧 Gemini calling: {}", function_call.name);
                            send_progress(0.0, &format!("🔧 {}...", function_call.name));

                            let result = execute_tool_gemini_with_context(&function_call.name, &function_call.args, &exec_context).await;

                            // CRITICAL: If this is submit_final_answer, return immediately
                            if function_call.name == "submit_final_answer" && !result.is_empty() {
                                send_progress(0.0, "✅ Task completed!");
                                return Ok(result);
                            }

                            // Add ONLY the FunctionResponse (not the FunctionCall again)
                            // The model already sent the FunctionCall, we just need to respond with the result
                            tool_results.push(Part::FunctionResponse {
                                function_response: crate::gemini_client::FunctionResponse {
                                    name: function_call.name.clone(),
                                    response: {
                                        let mut map = std::collections::HashMap::new();
                                        map.insert("result".to_string(), Value::String(result));
                                        map
                                    },
                                    thought_signature: function_call.thought_signature.clone(),
                                },
                            });
                        }
                        _ => {}
                    }
                }

                // Add model response to conversation
                conversation.push(content.clone());

                if !has_tool_calls {
                    break;
                }

                // Add tool results for next iteration
                if !tool_results.is_empty() {
                    conversation.push(Content {
                        parts: tool_results,
                        role: Some("user".to_string()),
                    });
                }
                } else {
                    // No content in response
                    tracing::warn!("Gemini response has no content");
                    break;
                }
            }
        }

        Ok(final_text)
    }
}
