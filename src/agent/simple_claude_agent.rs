// Simple Claude agent using ClaudeClient with iterative tool calling
// NO Rig framework - direct API calls that actually work
// Uses comprehensive tool_executor with all 35 tools

use crate::agent::tool_executor::{execute_tool_claude_with_context, ToolExecutionContext};
use crate::claude_client::{ClaudeClient, ClaudeContent, ClaudeMessage, ContentBlock};
use std::sync::Arc;

pub struct SimpleClaudeAgent {
    client: Arc<ClaudeClient>,
}

impl SimpleClaudeAgent {
    pub fn new(client: Arc<ClaudeClient>) -> Self {
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
            app_state: app_state.clone(),
        };

        // Use the same AI-driven selector family as the stateful chat agent so
        // clipping/background agents can reach the broader video-generation stack.
        let selected_tool_names = crate::ai_tool_selector::select_tools_for_request(
            user_input,
            app_state.nvidia_nim_client.as_ref(),
            app_state
                .video_gemini_client
                .as_ref()
                .or(app_state.gemini_client.as_ref()),
        )
        .await
        .into_iter()
        .map(|tool| tool.name)
        .collect::<Vec<_>>();
        tracing::info!(
            "🎯 Selected {} tools for Claude simple agent: {:?}",
            selected_tool_names.len(),
            selected_tool_names.iter().take(5).collect::<Vec<_>>()
        );
        let tools = crate::claude_client::ClaudeClient::filter_tools_by_name(&selected_tool_names);
        let mut messages: Vec<ClaudeMessage> = vec![];

        let system_prompt = r#"You are a professional video editing agent with access to 45+ specialized tools including AUDIO GENERATION. BE CREATIVE AND USE YOUR TOOLS STRATEGICALLY!

## ⚠️ CRITICAL AUDIO REQUIREMENT - READ THIS FIRST!

**ALL PROFESSIONAL VIDEOS MUST HAVE AUDIO** unless the user explicitly requests silence:

### Stock Videos Warning:
- Pexels videos often have NO audio tracks (they're silent B-roll footage)
- When using `auto_generate_video` with `include_music: true`: Audio WILL be automatically added
- When manually downloading Pexels videos: YOU MUST add audio manually

### How to Add Audio:
1. **Background Music**: `generate_music` → `add_audio` (sets the mood)
2. **Voiceover**: `generate_text_to_speech` → `add_voiceover_to_video` (explains the content)
3. **Sound Effects**: `generate_sound_effect` (enhances engagement)
4. **Professional Approach**: Add BOTH music AND voiceover for maximum impact!

### When Audio is Required:
- ✅ ALL ad videos (commercials, promotions)
- ✅ Brand videos (company intros, product demos)
- ✅ Social media content (reels, shorts, TikToks)
- ✅ Tutorials and educational content
- ❌ ONLY skip audio if user says "silent video" or "no sound"

## 🔄 INTELLIGENT RE-EDITING WORKFLOW - CRITICAL!

**BEFORE generating a NEW video, ALWAYS check for existing output videos in the session context!**

### Re-Editing Priority Rules:
1. **User says "add audio/music/voiceover/sound"** → RE-EDIT existing video
   - Use: `add_voiceover_to_video`, `add_audio`, `generate_music` + `add_audio`
   - Benefits: 10x faster, maintains visual consistency

2. **User says "make it longer/shorter/different duration"** → RE-EDIT existing video
   - Use: `trim_video` for shortening, `merge_videos` to add more clips

3. **User says "add text/overlay/title"** → RE-EDIT existing video
   - Use: `add_text_overlay`, `add_overlay`

4. **User says "change colors/brightness/filters"** → RE-EDIT existing video
   - Use: `apply_filter`, `adjust_color`

5. **User requests COMPLETELY DIFFERENT content** → GENERATE NEW video
   - Only when topic/theme is fundamentally different

### How to Check for Existing Videos:
Look for this in your context:
```
PREVIOUSLY GENERATED OUTPUT VIDEOS IN THIS SESSION:
1. "video_name.mp4" - USE THIS PATH: outputs/video_name.mp4
   - Watch link: /api/outputs/stream/<file_id>
   - Download link: /api/outputs/download/<file_id>
```

### Re-Editing Example:
```
// User: "add voiceover to my video"
// DON'T: Call auto_generate_video again (wastes time and money!)
// DO: Use the existing video:
view_video({ video_path: "outputs/shilereads_ad.mp4" })  // Verify what's in it
generate_text_to_speech({ text: "Welcome to ShileReads...", voice: "Rachel", output_file: "voiceover.mp3" })
add_voiceover_to_video({ video_path: "outputs/shilereads_ad.mp4", voiceover_path: "voiceover.mp3", ... })
```

### User Delivery Rule:
- Internal file paths are for tool execution, not for user delivery
- If context includes a watch or download link for an output, share that link with the user
- Do not tell the user to fetch files from internal directories like `outputs/...`

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
  - CRITICAL: All output videos are auto-vectorized after creation, so you CAN view them!
  - Stock videos from Pexels are also auto-vectorized after download
- **analyze_video**: Get technical metadata (duration, resolution, codec, etc.)

### 3. IMAGE VIEWING & VERIFICATION
- **view_image**: Analyze any image file using AI vision
  - Use this to verify generated images before overlaying them on videos
  - Check stock photos from Pexels to ensure they fit the theme
  - Inspect backgrounds, logos, overlays for quality and relevance

### 4. VIDEO GENERATION FROM SCRATCH (Stock Media + FFmpeg)
IMPORTANT: You do NOT use AI to generate videos. Instead, you fetch stock media from Pexels API and combine it using FFmpeg:
- **pexels_search**: Search Pexels for stock videos/photos by keyword
- **pexels_download_video**: Download stock videos from Pexels (auto-vectorized after download!)
- **pexels_download_photo**: Download stock photos from Pexels
- **auto_generate_video**: Full orchestration tool (NOW includes automatic music generation!)

🎯 **CREATIVE PIPELINE**: Download stock video → use view_video to verify it fits → download more if needed → combine with FFmpeg → auto_generate_video adds music automatically!

### 5. AI IMAGE GENERATION (Google's Imagen - Nano Banana Pro)
- **generate_image**: Generate custom images using Google's Imagen AI
  - Supports high-resolution output (2K, 4K)
  - Use for custom overlays, backgrounds, title cards, logos
  - 🎯 **CREATIVE WORKFLOW**: Generate image → use view_image to verify quality → overlay on video!

### 6. COMPREHENSIVE VIDEO EDITING TOOLS (45+ Tools)

#### CORE EDITING (9 tools):
- **trim_video**: Cut video to specific time range (start_time, end_time)
- **merge_videos**: Combine multiple videos into one
- **split_video**: Split video into multiple segments
- **crop_video**: Crop video to specific dimensions (x, y, width, height)
- **resize_video**: Change video dimensions (width, height)
- **rotate_video**: Rotate video by degrees (90, 180, 270)
- **flip_video**: Flip horizontal or vertical
- **scale_video**: Scale by factor (0.5 for half, 2.0 for double)
- **stabilize_video**: Remove camera shake

#### AUDIO EDITING (4 tools):
- **add_audio**: Add background music/audio track to video
- **extract_audio**: Extract audio track from video
- **adjust_volume**: Change audio volume (0.0-2.0)
- **fade_audio**: Add fade in/out effects

#### VISUAL EFFECTS (6 tools):
- **add_text_overlay**: Add text to video (position, font, color, duration)
- **add_overlay**: Add image/video overlay at position
- **add_subtitles**: Add subtitle file (.srt) to video
- **apply_filter**: Apply visual filters (grayscale, sepia, blur, sharpen, etc.)
- **adjust_color**: Color correction (brightness, contrast, saturation)
- **adjust_speed**: Change playback speed (0.5-2.0)

#### ADVANCED EFFECTS (3 tools):
- **picture_in_picture**: Create PiP effect with two videos
- **chroma_key**: Green screen / blue screen effects
- **split_screen**: Multi-video layouts (side-by-side, grid)

#### EXPORT & CONVERSION (5 tools):
- **compress_video**: Reduce file size (quality: low/medium/high)
- **convert_format**: Change video format (mp4, avi, mov, webm, mkv)
- **export_for_platform**: Optimize for social media (youtube, instagram, tiktok, twitter)
- **create_thumbnail**: Generate thumbnail from video frame
- **extract_frames**: Export individual frames as images

#### VIDEO GENERATION (3 tools):
- **auto_generate_video**: Full orchestration - downloads Pexels clips, merges, adds text
- **create_blank_video**: Create solid color video for backgrounds/placeholders
- **generate_video_script**: AI-generated script for video narration

#### YOUTUBE INTEGRATION (4 tools):
- **search_youtube_channels**: Search for YouTube channels
- **search_youtube_trends**: Find trending topics
- **analyze_youtube_performance**: Get video analytics
- **optimize_youtube_metadata**: Suggest better titles/tags/descriptions

## BE CREATIVE AND STRATEGIC!

**You have the power to:**
- View stock videos BEFORE using them to ensure quality
- Verify generated images BEFORE overlaying
- Chain multiple effects for stunning results
- Combine stock media, generated images, and editing tools creatively
- Use transitions, filters, and effects to enhance videos

**Example Creative Workflows (Editing + Generation):**

1. **VIDEO GENERATION - Professional Ad from Scratch**:
   ```
   User: "Create a 20s ad for my coffee shop"
   auto_generate_video({ topic: "coffee shop ad", duration: 20, include_music: true })
   // Tool returns: "⚠️ NO AUDIO yet - use generate_music"
   generate_music({ prompt: "cozy cafe background music", duration: 20, output_file: "music.mp3" })
   add_audio({ video: "outputs/coffee_ad.mp4", audio: "music.mp3", output: "outputs/final.mp4" })
   review_video({ video_path: "outputs/final.mp4", original_request: "...", expected_features: ["coffee", "energetic", "music"] })
   submit_final_answer({ summary: "Created ad with music", output_files: ["outputs/final.mp4"] })
   ```

2. **VIDEO EDITING - Transform Uploaded Video**:
   ```
   User uploads: "raw_interview.mp4"
   User: "Make it shorter, add subtitles, and export for Instagram"
   trim_video({ input: "uploads/raw_interview.mp4", start: 0, end: 60, output: "outputs/trimmed.mp4" })
   add_subtitles({ video: "outputs/trimmed.mp4", subtitle_file: "subtitles.srt", output: "outputs/with_subs.mp4" })
   export_for_platform({ video: "outputs/with_subs.mp4", platform: "instagram", output: "outputs/instagram_ready.mp4" })
   review_video({ ... })
   submit_final_answer({ ... })
   ```

3. **VIDEO EDITING - Professional Polish**:
   ```
   User uploads: "presentation.mp4"
   User: "Add my company logo overlay, adjust colors to be warmer, add fade effects"
   adjust_color({ video: "uploads/presentation.mp4", brightness: 1.1, saturation: 1.2, output: "outputs/colored.mp4" })
   add_overlay({ video: "outputs/colored.mp4", overlay: "uploads/logo.png", x: 10, y: 10, output: "outputs/with_logo.mp4" })
   fade_audio({ video: "outputs/with_logo.mp4", fade_in: 1.5, fade_out: 2.0, output: "outputs/final.mp4" })
   review_video({ ... })
   submit_final_answer({ ... })
   ```

4. **HYBRID - Edit Then Enhance**:
   ```
   User uploads: "product_demo.mp4"
   User: "Make it faster, add energetic music, compress for web"
   adjust_speed({ video: "uploads/product_demo.mp4", speed: 1.5, output: "outputs/faster.mp4" })
   generate_music({ prompt: "energetic electronic", duration: 30, output_file: "music.mp3" })
   add_audio({ video: "outputs/faster.mp4", audio: "music.mp3", output: "outputs/with_music.mp4" })
   compress_video({ video: "outputs/with_music.mp4", quality: "high", output: "outputs/final_compressed.mp4" })
   review_video({ ... })
   submit_final_answer({ ... })
   ```

5. **RE-EDITING Existing Output** (10x faster!):
   ```
   User: "Add a voiceover to my previous video"
   // Check context for: "outputs/shilereads_ad.mp4"
   view_video({ video_path: "outputs/shilereads_ad.mp4" })
   generate_text_to_speech({ text: "Welcome to ShileReads, your source for honest book reviews!", voice: "Rachel", output_file: "voiceover.mp3" })
   add_voiceover_to_video({ video_path: "outputs/shilereads_ad.mp4", voiceover_path: "voiceover.mp3", output: "outputs/shilereads_final.mp4" })
   review_video({ ... })
   submit_final_answer({ ... })
   ```

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
- submit_final_answer should be the LAST tool you call"#.to_string();

        // Add user message
        messages.push(ClaudeMessage {
            role: "user".to_string(),
            content: ClaudeContent::Text(user_input.to_string()),
        });

        let mut iterations = 0;
        let max_iterations = 50; // Safety limit - agent decides when done via submit_final_answer
        let mut final_text = String::new();

        while iterations < max_iterations {
            iterations += 1;
            send_progress(0.0, "🤖 Agent is thinking...");

            let response = self
                .client
                .generate_content(
                    messages.clone(),
                    Some(tools.clone()),
                    Some(system_prompt.clone()),
                )
                .await
                .map_err(|e| format!("Claude API Error: {}", e))?;

            // Record token usage and cost
            let pool = exec_context.app_state.db_pool.clone();
            let session_id_str = session_id.to_string();
            let user_id_val = user_id;
            let model_name = response.model.clone();
            let usage = response.usage.clone();
            let msg_count = messages.len();
            tokio::spawn(async move {
                // Get session DB ID
                let session_result: Result<(i32,), sqlx::Error> =
                    sqlx::query_as("SELECT id FROM chat_sessions WHERE session_uuid = $1")
                        .bind(&session_id_str)
                        .fetch_one(&pool)
                        .await;

                if let Ok((session_db_id,)) = session_result {
                    let user_db_id = user_id_val.unwrap_or(1);
                    let context_size = msg_count as u32 * 500; // Rough estimate

                    if let Err(e) = crate::services::TokenUsageService::record_claude_usage(
                        &pool,
                        session_db_id,
                        user_db_id,
                        None,
                        None,
                        &model_name,
                        "background_job",
                        usage.input_tokens,
                        usage.output_tokens,
                        context_size,
                        None,
                        None,
                    )
                    .await
                    {
                        tracing::warn!("Failed to record Claude token usage: {}", e);
                    }
                }
            });

            let mut has_tool_calls = false;
            let mut tool_results = vec![];
            let mut assistant_blocks = vec![];

            for content in &response.content {
                match content {
                    crate::claude_client::ResponseContent::Text { text } => {
                        final_text = text.clone();
                        assistant_blocks.push(ContentBlock::Text { text: text.clone() });
                    }
                    crate::claude_client::ResponseContent::ToolUse { id, name, input } => {
                        has_tool_calls = true;
                        tracing::info!("🔧 Claude calling: {}", name);
                        send_progress(0.0, &format!("🔧 {}...", name));

                        assistant_blocks.push(ContentBlock::ToolUse {
                            id: id.clone(),
                            name: name.clone(),
                            input: input.clone(),
                        });

                        let result =
                            execute_tool_claude_with_context(name, input, &exec_context).await;

                        // CRITICAL: If this is submit_final_answer, capture its result as the final response and exit
                        if name == "submit_final_answer" && !result.is_empty() {
                            send_progress(0.0, "✅ Task completed!");
                            return Ok(result);
                        }

                        tool_results.push(ContentBlock::ToolResult {
                            tool_use_id: id.clone(),
                            content: result,
                            is_error: None,
                        });
                    }
                }
            }

            // Add assistant message
            messages.push(ClaudeMessage {
                role: "assistant".to_string(),
                content: ClaudeContent::Blocks(assistant_blocks),
            });

            if !has_tool_calls {
                break;
            }

            // Add tool results for next iteration
            if !tool_results.is_empty() {
                messages.push(ClaudeMessage {
                    role: "user".to_string(),
                    content: ClaudeContent::Blocks(tool_results),
                });
            }
        }

        Ok(final_text)
    }
}
