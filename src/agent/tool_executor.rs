// Comprehensive tool executor for all 35+ video editing tools
// Maps tool names to actual video processing function calls

use serde_json::Value;
use std::collections::HashMap;
use tokio::fs::File;
use tokio::io::AsyncWriteExt;
use std::sync::Arc;
use crate::AppState;
use base64::prelude::BASE64_STANDARD;
use base64::Engine;
use std::time::Duration;

/// Retry function with exponential backoff for handling vectorization delays
async fn retry_with_exponential_backoff<F, Fut, T, E>(
    mut operation: F,
    max_retries: u32,
    initial_delay_ms: u64,
) -> Result<T, E>
where
    F: FnMut() -> Fut,
    Fut: std::future::Future<Output = Result<T, E>>,
{
    let mut delay = initial_delay_ms;
    for attempt in 0..max_retries {
        match operation().await {
            Ok(result) => return Ok(result),
            Err(e) => {
                if attempt == max_retries - 1 {
                    return Err(e);
                }
                tokio::time::sleep(Duration::from_millis(delay)).await;
                delay *= 2; // Exponential backoff
            }
        }
    }
    unreachable!()
}

/// Helper function to ensure all output files are in the outputs/ directory
fn ensure_outputs_directory(file_path: &str) -> String {
    // If path is already in outputs/ or starts with outputs/, return as is
    if file_path.starts_with("outputs/") || file_path.starts_with("./outputs/") {
        return file_path.to_string();
    }

    // If path is absolute or contains directory separators, extract just the filename
    let filename = std::path::Path::new(file_path)
        .file_name()
        .and_then(|f| f.to_str())
        .unwrap_or(file_path);

    // Return path with outputs/ prefix
    format!("outputs/{}", filename)
}

/// Context needed for tool execution to save outputs to DB and vectorize them
pub struct ToolExecutionContext {
    pub session_id: String,
    pub user_id: Option<i32>,
    pub app_state: Arc<AppState>,
}

/// Execute a tool with full context - saves outputs to DB and vectorizes them
pub async fn execute_tool_claude_with_context(
    name: &str,
    args: &Value,
    ctx: &ToolExecutionContext,
) -> String {
    // Handle special tools that need AppState access
    if name == "view_video" {
        return execute_view_video_with_state_claude(args, ctx).await;
    }
    if name == "review_video" {
        return execute_review_video_with_state_claude(args, ctx).await;
    }
    if name == "view_image" {
        return execute_view_image_with_state_claude(args, ctx).await;
    }
    if name == "generate_text_to_speech" {
        return execute_generate_text_to_speech_with_state_claude(args, ctx).await;
    }
    if name == "generate_sound_effect" {
        return execute_generate_sound_effect_with_state_claude(args, ctx).await;
    }
    if name == "generate_music" {
        return execute_generate_music_with_state_claude(args, ctx).await;
    }
    if name == "add_voiceover_to_video" {
        return execute_add_voiceover_to_video_with_state_claude(args, ctx).await;
    }
    if name == "set_chat_title" {
        return execute_set_chat_title_with_state_claude(args, ctx).await;
    }

    // YouTube integration tools (READ-ONLY research tools)
    if name == "optimize_youtube_metadata" {
        return execute_optimize_youtube_metadata_with_state_claude(args, ctx).await;
    }
    if name == "analyze_youtube_performance" {
        return execute_analyze_youtube_performance_with_state_claude(args, ctx).await;
    }
    if name == "suggest_content_ideas" {
        return execute_suggest_content_ideas_with_state_claude(args, ctx).await;
    }
    if name == "search_youtube_trends" {
        return execute_search_youtube_trends_with_state_claude(args, ctx).await;
    }
    if name == "search_youtube_channels" {
        return execute_search_youtube_channels_with_state_claude(args, ctx).await;
    }

    // Execute the tool first
    let result = execute_tool_claude(name, args).await;

    // Auto-vectorize downloaded stock videos from Pexels
    if name == "pexels_download_video" && !result.starts_with("❌") {
        if let Some(output_path) = extract_output_path_from_args(args) {
            let ctx_clone = (ctx.session_id.clone(), ctx.user_id, ctx.app_state.clone(), output_path.clone());
            tokio::spawn(async move {
                let (session_id, user_id, app_state, output_path) = ctx_clone;
                tracing::info!("🎬 Auto-vectorizing stock video: {}", output_path);
                if let Err(e) = crate::services::VideoVectorizationService::process_video_for_vectorization(
                    &output_path,
                    &uuid::Uuid::new_v4().to_string(),
                    &session_id,
                    user_id,
                    &app_state,
                ).await {
                    tracing::warn!("Failed to vectorize stock video {}: {}", output_path, e);
                } else {
                    tracing::info!("✅ Stock video vectorized: {}", output_path);
                }
            });
        }
    }

    // If tool succeeded and created an output file, save it to DB and vectorize
    if !result.starts_with("❌") && !result.starts_with("Error") {
        if let Some(output_path) = extract_output_path_from_args(args) {
            // Save to PostgreSQL in background (non-blocking)
            let ctx_clone = (ctx.session_id.clone(), ctx.user_id, ctx.app_state.clone(), output_path.clone(), name.to_string());
            tokio::spawn(async move {
                let (session_id, user_id, app_state, output_path, tool_name) = ctx_clone;

                // Get session and user IDs from database
                if let Ok(session_db_id) = get_session_db_id(&session_id, &app_state).await {
                    let user_db_id = user_id.unwrap_or(1); // Default to user 1 if not authenticated

                    // Save to PostgreSQL
                    if let Err(e) = crate::services::output_video::OutputVideoService::save_output_video(
                        &app_state.db_pool,
                        session_db_id,
                        user_db_id,
                        None,
                        &output_path,
                        &tool_name,
                        None,
                        &tool_name,
                        Some("Video created by AI agent"),
                    ).await {
                        tracing::warn!("Failed to save output video to DB: {}", e);
                    } else {
                        tracing::info!("✅ Saved output video to PostgreSQL: {}", output_path);
                    }

                    // Vectorize the output video
                    if let Err(e) = crate::services::VideoVectorizationService::process_video_for_vectorization(
                        &output_path,
                        &uuid::Uuid::new_v4().to_string(),
                        &session_id,
                        Some(user_db_id),
                        &app_state,
                    ).await {
                        tracing::warn!("Failed to vectorize output video: {}", e);
                    } else {
                        tracing::info!("✅ Vectorized output video: {}", output_path);
                    }
                }
            });
        }
    }

    result
}

/// Execute a tool with full context for Gemini
pub async fn execute_tool_gemini_with_context(
    name: &str,
    args: &HashMap<String, Value>,
    ctx: &ToolExecutionContext,
) -> String {
    // Handle special tools that need AppState access
    if name == "view_video" {
        return execute_view_video_with_state_gemini(args, ctx).await;
    }
    if name == "review_video" {
        return execute_review_video_with_state_gemini(args, ctx).await;
    }
    if name == "view_image" {
        return execute_view_image_with_state_gemini(args, ctx).await;
    }
    if name == "generate_text_to_speech" {
        return execute_generate_text_to_speech_with_state_gemini(args, ctx).await;
    }
    if name == "generate_sound_effect" {
        return execute_generate_sound_effect_with_state_gemini(args, ctx).await;
    }
    if name == "generate_music" {
        return execute_generate_music_with_state_gemini(args, ctx).await;
    }
    if name == "add_voiceover_to_video" {
        return execute_add_voiceover_to_video_with_state_gemini(args, ctx).await;
    }
    if name == "set_chat_title" {
        return execute_set_chat_title_with_state_gemini(args, ctx).await;
    }

    // YouTube integration tools (READ-ONLY research tools)
    if name == "optimize_youtube_metadata" {
        return execute_optimize_youtube_metadata_with_state_gemini(args, ctx).await;
    }
    if name == "analyze_youtube_performance" {
        return execute_analyze_youtube_performance_with_state_gemini(args, ctx).await;
    }
    if name == "suggest_content_ideas" {
        return execute_suggest_content_ideas_with_state_gemini(args, ctx).await;
    }
    if name == "search_youtube_trends" {
        return execute_search_youtube_trends_with_state_gemini(args, ctx).await;
    }
    if name == "search_youtube_channels" {
        return execute_search_youtube_channels_with_state_gemini(args, ctx).await;
    }

    // Execute the tool first
    let result = execute_tool_gemini(name, args).await;

    // Auto-vectorize downloaded stock videos from Pexels
    if name == "pexels_download_video" && !result.starts_with("❌") {
        if let Some(output_path) = extract_output_path_from_gemini_args(args) {
            let ctx_clone = (ctx.session_id.clone(), ctx.user_id, ctx.app_state.clone(), output_path.clone());
            tokio::spawn(async move {
                let (session_id, user_id, app_state, output_path) = ctx_clone;
                tracing::info!("🎬 Auto-vectorizing stock video: {}", output_path);
                if let Err(e) = crate::services::VideoVectorizationService::process_video_for_vectorization(
                    &output_path,
                    &uuid::Uuid::new_v4().to_string(),
                    &session_id,
                    user_id,
                    &app_state,
                ).await {
                    tracing::warn!("Failed to vectorize stock video {}: {}", output_path, e);
                } else {
                    tracing::info!("✅ Stock video vectorized: {}", output_path);
                }
            });
        }
    }

    // If tool succeeded and created an output file, save it to DB and vectorize
    if !result.starts_with("❌") && !result.starts_with("Error") {
        if let Some(output_path) = extract_output_path_from_gemini_args(args) {
            // Save to PostgreSQL and vectorize in background
            let ctx_clone = (ctx.session_id.clone(), ctx.user_id, ctx.app_state.clone(), output_path.clone(), name.to_string());
            tokio::spawn(async move {
                let (session_id, user_id, app_state, output_path, tool_name) = ctx_clone;

                if let Ok(session_db_id) = get_session_db_id(&session_id, &app_state).await {
                    let user_db_id = user_id.unwrap_or(1);

                    // Save to PostgreSQL
                    if let Err(e) = crate::services::output_video::OutputVideoService::save_output_video(
                        &app_state.db_pool,
                        session_db_id,
                        user_db_id,
                        None,
                        &output_path,
                        &tool_name,
                        None,
                        &tool_name,
                        Some("Video created by AI agent"),
                    ).await {
                        tracing::warn!("Failed to save output video to DB: {}", e);
                    } else {
                        tracing::info!("✅ Saved output video to PostgreSQL: {}", output_path);
                    }

                    // Vectorize the output video
                    if let Err(e) = crate::services::VideoVectorizationService::process_video_for_vectorization(
                        &output_path,
                        &uuid::Uuid::new_v4().to_string(),
                        &session_id,
                        Some(user_db_id),
                        &app_state,
                    ).await {
                        tracing::warn!("Failed to vectorize output video: {}", e);
                    } else {
                        tracing::info!("✅ Vectorized output video: {}", output_path);
                    }
                }
            });
        }
    }

    result
}

/// Extract output file path from tool arguments
fn extract_output_path_from_args(args: &Value) -> Option<String> {
    args.get("output_file")
        .or_else(|| args.get("output_path"))
        .or_else(|| args.get("output"))
        .and_then(|v| v.as_str())
        .map(|s| s.to_string())
}

/// Extract output file path from Gemini-style arguments
fn extract_output_path_from_gemini_args(args: &HashMap<String, Value>) -> Option<String> {
    args.get("output_file")
        .or_else(|| args.get("output_path"))
        .or_else(|| args.get("output"))
        .and_then(|v| v.as_str())
        .map(|s| s.to_string())
}

/// Get database session ID from UUID session string
async fn get_session_db_id(session_uuid: &str, app_state: &Arc<AppState>) -> Result<i32, String> {
    sqlx::query_scalar::<_, i32>("SELECT id FROM chat_sessions WHERE session_uuid = $1")
        .bind(session_uuid)
        .fetch_one(&app_state.db_pool)
        .await
        .map_err(|e| format!("Failed to get session DB ID: {}", e))
}

/// Execute a tool by name with the provided arguments (for Claude - uses Value)
pub async fn execute_tool_claude(name: &str, args: &Value) -> String {
    match name {
        // Core operations
        "trim_video" => execute_trim_video_claude(args),
        "merge_videos" => execute_merge_videos_claude(args),
        "analyze_video" => execute_analyze_video_claude(args),
        "split_video" => execute_split_video_claude(args),

        // Visual effects
        "add_text_overlay" => execute_add_text_overlay_claude(args),
        "apply_filter" => execute_apply_filter_claude(args),
        "add_overlay" => execute_add_overlay_claude(args),
        "adjust_color" => execute_adjust_color_claude(args),
        "add_subtitles" => execute_add_subtitles_claude(args),

        // Transform operations
        "resize_video" => execute_resize_video_claude(args),
        "crop_video" => execute_crop_video_claude(args),
        "rotate_video" => execute_rotate_video_claude(args),
        "adjust_speed" => execute_adjust_speed_claude(args),
        "flip_video" => execute_flip_video_claude(args),
        "scale_video" => execute_scale_video_claude(args),

        // Audio operations
        "extract_audio" => execute_extract_audio_claude(args),
        "add_audio" => execute_add_audio_claude(args),
        "adjust_volume" => execute_adjust_volume_claude(args),
        "fade_audio" => execute_fade_audio_claude(args),

        // Export operations
        "convert_format" => execute_convert_format_claude(args),
        "compress_video" => execute_compress_video_claude(args),
        "export_for_platform" => execute_export_for_platform_claude(args),
        "create_thumbnail" => execute_create_thumbnail_claude(args),
        "extract_frames" => execute_extract_frames_claude(args),

        // Advanced operations
        "picture_in_picture" => execute_picture_in_picture_claude(args),
        "chroma_key" => execute_chroma_key_claude(args),
        "split_screen" => execute_split_screen_claude(args),
        "stabilize_video" => execute_stabilize_video_claude(args),

        // AI/Generation tools
        "pexels_search" => execute_pexels_search_claude(args).await,
        "pexels_download_video" => execute_pexels_download_video_claude(args).await,
        "pexels_download_photo" => execute_pexels_download_photo_claude(args).await,
        "pexels_get_trending" => execute_pexels_get_trending_claude(args).await,
        "pexels_get_curated" => execute_pexels_get_curated_claude(args).await,
        "analyze_image" => execute_analyze_image_claude(args).await,
        "generate_text_to_speech" => execute_generate_text_to_speech_placeholder_claude(args).await,
        "generate_sound_effect" => execute_generate_sound_effect_placeholder_claude(args).await,
        "generate_music" => execute_generate_music_placeholder_claude(args).await,
        "add_voiceover_to_video" => execute_add_voiceover_placeholder_claude(args).await,
        "generate_video_script" => execute_generate_video_script_claude(args).await,
        "create_blank_video" => execute_create_blank_video_claude(args),
        "generate_image" => execute_generate_image_claude(args).await,
        "auto_generate_video" => execute_auto_generate_video_claude(args).await,
        "view_video" => execute_view_video_claude(args).await,
        "review_video" => execute_review_video_claude(args).await,
        "view_image" => execute_view_image_claude(args).await,

        // Control tools
        "submit_final_answer" => execute_submit_final_answer_claude(args),

        _ => format!("❌ Unknown tool: {}", name),
    }
}

/// Execute a tool by name with the provided arguments (for Gemini - uses HashMap)
pub async fn execute_tool_gemini(name: &str, args: &HashMap<String, Value>) -> String {
    match name {
        // Core operations
        "trim_video" => execute_trim_video_gemini(args),
        "merge_videos" => execute_merge_videos_gemini(args),
        "analyze_video" => execute_analyze_video_gemini(args),
        "split_video" => execute_split_video_gemini(args),

        // Visual effects
        "add_text_overlay" => execute_add_text_overlay_gemini(args),
        "apply_filter" => execute_apply_filter_gemini(args),
        "add_overlay" => execute_add_overlay_gemini(args),
        "adjust_color" => execute_adjust_color_gemini(args),
        "add_subtitles" => execute_add_subtitles_gemini(args),

        // Transform operations
        "resize_video" => execute_resize_video_gemini(args),
        "crop_video" => execute_crop_video_gemini(args),
        "rotate_video" => execute_rotate_video_gemini(args),
        "adjust_speed" => execute_adjust_speed_gemini(args),
        "flip_video" => execute_flip_video_gemini(args),
        "scale_video" => execute_scale_video_gemini(args),

        // Audio operations
        "extract_audio" => execute_extract_audio_gemini(args),
        "add_audio" => execute_add_audio_gemini(args),
        "adjust_volume" => execute_adjust_volume_gemini(args),
        "fade_audio" => execute_fade_audio_gemini(args),

        // Export operations
        "convert_format" => execute_convert_format_gemini(args),
        "compress_video" => execute_compress_video_gemini(args),
        "export_for_platform" => execute_export_for_platform_gemini(args),
        "create_thumbnail" => execute_create_thumbnail_gemini(args),
        "extract_frames" => execute_extract_frames_gemini(args),

        // Advanced operations
        "picture_in_picture" => execute_picture_in_picture_gemini(args),
        "chroma_key" => execute_chroma_key_gemini(args),
        "split_screen" => execute_split_screen_gemini(args),
        "stabilize_video" => execute_stabilize_video_gemini(args),

        // AI/Generation tools
        "pexels_search" => execute_pexels_search_gemini(args).await,
        "pexels_download_video" => execute_pexels_download_video_gemini(args).await,
        "pexels_download_photo" => execute_pexels_download_photo_gemini(args).await,
        "pexels_get_trending" => execute_pexels_get_trending_gemini(args).await,
        "pexels_get_curated" => execute_pexels_get_curated_gemini(args).await,
        "analyze_image" => execute_analyze_image_gemini(args).await,
        "generate_text_to_speech" => execute_generate_text_to_speech_placeholder_gemini(args).await,
        "generate_sound_effect" => execute_generate_sound_effect_placeholder_gemini(args).await,
        "generate_music" => execute_generate_music_placeholder_gemini(args).await,
        "add_voiceover_to_video" => execute_add_voiceover_placeholder_gemini(args).await,
        "generate_video_script" => execute_generate_video_script_gemini(args).await,
        "create_blank_video" => execute_create_blank_video_gemini(args),
        "generate_image" => execute_generate_image_gemini(args).await,
        "auto_generate_video" => execute_auto_generate_video_gemini(args).await,
        "view_video" => execute_view_video_gemini(args).await,
        "review_video" => execute_review_video_gemini(args).await,
        "view_image" => execute_view_image_gemini(args).await,

        // Control tools
        "submit_final_answer" => execute_submit_final_answer_gemini(args),

        _ => format!("❌ Unknown tool: {}", name),
    }
}

// Helper function to download file from URL
async fn download_file_from_url(url: &str, output_path: &str) -> Result<(), String> {
    let client = reqwest::Client::new();

    let response = client
        .get(url)
        .send()
        .await
        .map_err(|e| format!("Failed to send request: {}", e))?;

    if !response.status().is_success() {
        return Err(format!("HTTP error: {}", response.status()));
    }

    let bytes = response
        .bytes()
        .await
        .map_err(|e| format!("Failed to read response body: {}", e))?;

    let mut file = File::create(output_path)
        .await
        .map_err(|e| format!("Failed to create file: {}", e))?;

    file.write_all(&bytes)
        .await
        .map_err(|e| format!("Failed to write file: {}", e))?;

    Ok(())
}

// ============================================================================
// CLAUDE TOOL EXECUTORS (args: &Value)
// ============================================================================

fn execute_trim_video_claude(args: &Value) -> String {
    let input = args["input_file"].as_str().unwrap_or("");
    let output_raw = args["output_file"].as_str().unwrap_or("");
    let output = ensure_outputs_directory(output_raw);
    let start = args["start_seconds"].as_f64().unwrap_or(0.0);
    let end = args["end_seconds"].as_f64().unwrap_or(0.0);
    crate::core::trim_video(input, &output, start, end).unwrap_or_else(|e| e)
}

fn execute_merge_videos_claude(args: &Value) -> String {
    let input_files: Vec<String> = args["input_files"].as_array()
        .map(|arr| arr.iter().filter_map(|v| v.as_str()).map(|s| s.to_string()).collect())
        .unwrap_or_default();
    let output_raw = args["output_file"].as_str().unwrap_or("");
    let output = ensure_outputs_directory(output_raw);
    crate::core::merge_videos(&input_files, &output).unwrap_or_else(|e| e)
}

fn execute_analyze_video_claude(args: &Value) -> String {
    let input = args["input_file"].as_str().unwrap_or("");
    match crate::core::analyze_video(input) {
        Ok(metadata) => serde_json::to_string_pretty(&metadata)
            .unwrap_or_else(|_| "Failed to serialize metadata".to_string()),
        Err(e) => e,
    }
}

fn execute_split_video_claude(args: &Value) -> String {
    let input = args["input_file"].as_str().unwrap_or("");
    let output_prefix = args["output_prefix"].as_str().unwrap_or("");
    let segment_duration = args["segment_duration"].as_f64().unwrap_or(10.0);
    crate::core::split_video(input, output_prefix, segment_duration).unwrap_or_else(|e| e)
}

fn execute_add_text_overlay_claude(args: &Value) -> String {
    let input = args["input_file"].as_str().unwrap_or("");
    let output_raw = args["output_file"].as_str().unwrap_or("");
    let output = ensure_outputs_directory(output_raw);
    let text = args["text"].as_str().unwrap_or("");
    let x = &args["x"].as_u64().unwrap_or(960).to_string();
    let y = &args["y"].as_u64().unwrap_or(540).to_string();
    let font_file = args.get("font_file").and_then(|v| v.as_str())
        .unwrap_or("/usr/share/fonts/truetype/dejavu/DejaVuSans-Bold.ttf");
    let font_size = args.get("font_size").and_then(|v| v.as_u64()).unwrap_or(48) as u32;
    let color = args.get("color").and_then(|v| v.as_str()).unwrap_or("white");
    let start_time = args.get("start_time").and_then(|v| v.as_f64()).unwrap_or(0.0);
    let end_time = args.get("end_time").and_then(|v| v.as_f64()).unwrap_or(999999.0);
    crate::visual::add_text_overlay(input, &output, text, x, y, font_file, font_size, color, start_time, end_time)
        .unwrap_or_else(|e| e)
}

fn execute_apply_filter_claude(args: &Value) -> String {
    let input = args["input_file"].as_str().unwrap_or("");
    let output_raw = args["output_file"].as_str().unwrap_or("");
    let output = ensure_outputs_directory(output_raw);
    let filter = args["filter_type"].as_str().unwrap_or("");
    let intensity = args.get("intensity").and_then(|v| v.as_f64()).unwrap_or(1.0);
    crate::visual::apply_filter(input, &output, filter, intensity).unwrap_or_else(|e| e)
}

fn execute_add_overlay_claude(args: &Value) -> String {
    let input = args["input_file"].as_str().unwrap_or("");
    let output_raw = args["output_file"].as_str().unwrap_or("");
    let output = ensure_outputs_directory(output_raw);
    let overlay = args["overlay_file"].as_str().unwrap_or("");
    let x = args["x"].as_u64().unwrap_or(0) as u32;
    let y = args["y"].as_u64().unwrap_or(0) as u32;
    crate::visual::add_overlay(input, overlay, &output, x, y).unwrap_or_else(|e| e)
}

fn execute_adjust_color_claude(args: &Value) -> String {
    let input = args["input_file"].as_str().unwrap_or("");
    let output_raw = args["output_file"].as_str().unwrap_or("");
    let output = ensure_outputs_directory(output_raw);
    let brightness = args.get("brightness").and_then(|v| v.as_f64()).unwrap_or(0.0);
    let contrast = args.get("contrast").and_then(|v| v.as_f64()).unwrap_or(0.0);
    let saturation = args.get("saturation").and_then(|v| v.as_f64()).unwrap_or(0.0);
    // Note: hue is not supported by adjust_color function (only brightness, contrast, saturation)
    crate::visual::adjust_color(input, &output, brightness, contrast, saturation).unwrap_or_else(|e| e)
}

fn execute_add_subtitles_claude(args: &Value) -> String {
    let input = args["input_file"].as_str().unwrap_or("");
    let output_raw = args["output_file"].as_str().unwrap_or("");
    let output = ensure_outputs_directory(output_raw);
    let subtitle_text = args["subtitle_text"].as_str().unwrap_or("");
    // Note: add_subtitles only takes (input, subtitle, output) - font_size and color not supported
    crate::visual::add_subtitles(input, subtitle_text, &output).unwrap_or_else(|e| e)
}

fn execute_resize_video_claude(args: &Value) -> String {
    let input = args["input_file"].as_str().unwrap_or("");
    let output_raw = args["output_file"].as_str().unwrap_or("");
    let output = ensure_outputs_directory(output_raw);
    let width = args["width"].as_u64().unwrap_or(1920) as u32;
    let height = args["height"].as_u64().unwrap_or(1080) as u32;
    crate::transform::resize_video(input, &output, width, height).unwrap_or_else(|e| e)
}

fn execute_crop_video_claude(args: &Value) -> String {
    let input = args["input_file"].as_str().unwrap_or("");
    let output_raw = args["output_file"].as_str().unwrap_or("");
    let output = ensure_outputs_directory(output_raw);
    let x = args["x"].as_u64().unwrap_or(0) as u32;
    let y = args["y"].as_u64().unwrap_or(0) as u32;
    let width = args["width"].as_u64().unwrap_or(1920) as u32;
    let height = args["height"].as_u64().unwrap_or(1080) as u32;
    crate::transform::crop_video(input, &output, width, height, x, y).unwrap_or_else(|e| e)
}

fn execute_rotate_video_claude(args: &Value) -> String {
    let input = args["input_file"].as_str().unwrap_or("");
    let output_raw = args["output_file"].as_str().unwrap_or("");
    let output = ensure_outputs_directory(output_raw);
    let degrees = args["degrees"].as_f64().unwrap_or(0.0);
    let angle_str = format!("{}", degrees as i32);
    crate::transform::rotate_video(input, &output, &angle_str).unwrap_or_else(|e| e)
}

fn execute_adjust_speed_claude(args: &Value) -> String {
    let input = args["input_file"].as_str().unwrap_or("");
    let output_raw = args["output_file"].as_str().unwrap_or("");
    let output = ensure_outputs_directory(output_raw);
    let speed_factor = args["speed_factor"].as_f64().unwrap_or(1.0);
    crate::transform::adjust_speed(input, &output, speed_factor).unwrap_or_else(|e| e)
}

fn execute_flip_video_claude(args: &Value) -> String {
    let input = args["input_file"].as_str().unwrap_or("");
    let output_raw = args["output_file"].as_str().unwrap_or("");
    let output = ensure_outputs_directory(output_raw);
    let direction = args["direction"].as_str().unwrap_or("horizontal");
    crate::transform::flip_video(input, &output, direction).unwrap_or_else(|e| e)
}

fn execute_scale_video_claude(args: &Value) -> String {
    let input = args["input_file"].as_str().unwrap_or("");
    let output_raw = args["output_file"].as_str().unwrap_or("");
    let output = ensure_outputs_directory(output_raw);
    let scale_factor = args["scale_factor"].as_f64().unwrap_or(1.0);
    let algorithm = "bicubic"; // Default scaling algorithm
    crate::transform::scale_video(input, &output, scale_factor, algorithm).unwrap_or_else(|e| e)
}

fn execute_extract_audio_claude(args: &Value) -> String {
    let input = args["input_file"].as_str().unwrap_or("");
    let output_raw = args["output_file"].as_str().unwrap_or("");
    let output = ensure_outputs_directory(output_raw);
    let format = args["format"].as_str().unwrap_or("mp3");
    crate::audio::extract_audio(input, &output, format).unwrap_or_else(|e| e)
}

fn execute_add_audio_claude(args: &Value) -> String {
    let input = args["input_file"].as_str().unwrap_or("");
    let output_raw = args["output_file"].as_str().unwrap_or("");
    let output = ensure_outputs_directory(output_raw);
    let audio_file = args["audio_file"].as_str().unwrap_or("");
    // Note: add_audio signature is (video, audio, output) - no replace parameter
    crate::audio::add_audio(input, audio_file, &output).unwrap_or_else(|e| e)
}

fn execute_adjust_volume_claude(args: &Value) -> String {
    let input = args["input_file"].as_str().unwrap_or("");
    let output_raw = args["output_file"].as_str().unwrap_or("");
    let output = ensure_outputs_directory(output_raw);
    let volume_factor = args["volume_factor"].as_f64().unwrap_or(1.0);
    crate::audio::adjust_volume(input, &output, volume_factor).unwrap_or_else(|e| e)
}

fn execute_fade_audio_claude(args: &Value) -> String {
    let input = args["input_file"].as_str().unwrap_or("");
    let output_raw = args["output_file"].as_str().unwrap_or("");
    let output = ensure_outputs_directory(output_raw);
    let fade_in_duration = args["fade_in_duration"].as_f64().unwrap_or(0.0);
    let fade_out_duration = args["fade_out_duration"].as_f64().unwrap_or(0.0);
    // fade_audio requires total duration as 5th parameter - use analyze_video to get it or estimate
    let duration = 60.0; // Default estimate - ideally should analyze video first
    crate::audio::fade_audio(input, &output, fade_in_duration, fade_out_duration, duration).unwrap_or_else(|e| e)
}

fn execute_convert_format_claude(args: &Value) -> String {
    let input = args["input_file"].as_str().unwrap_or("");
    let output_raw = args["output_file"].as_str().unwrap_or("");
    let output = ensure_outputs_directory(output_raw);
    let format = args["format"].as_str().unwrap_or("mp4");
    crate::export::convert_format(input, &output, format).unwrap_or_else(|e| e)
}

fn execute_compress_video_claude(args: &Value) -> String {
    let input = args["input_file"].as_str().unwrap_or("");
    let output_raw = args["output_file"].as_str().unwrap_or("");
    let output = ensure_outputs_directory(output_raw);
    let quality = args["quality"].as_str().unwrap_or("medium");
    crate::export::compress_video(input, &output, quality).unwrap_or_else(|e| e)
}

fn execute_export_for_platform_claude(args: &Value) -> String {
    let input = args["input_file"].as_str().unwrap_or("");
    let output_raw = args["output_file"].as_str().unwrap_or("");
    let output = ensure_outputs_directory(output_raw);
    let platform = args["platform"].as_str().unwrap_or("youtube");
    crate::export::export_for_platform(input, &output, platform).unwrap_or_else(|e| e)
}

fn execute_create_thumbnail_claude(args: &Value) -> String {
    let input = args["input_file"].as_str().unwrap_or("");
    let output_raw = args["output_file"].as_str().unwrap_or("");
    let output = ensure_outputs_directory(output_raw);
    let timestamp = args["timestamp"].as_f64().unwrap_or(0.0);
    // Note: create_thumbnail only takes 3 params (input, output, timestamp) - width/height not supported
    crate::transform::create_thumbnail(input, &output, timestamp).unwrap_or_else(|e| e)
}

fn execute_extract_frames_claude(args: &Value) -> String {
    let input = args["input_file"].as_str().unwrap_or("");
    let output_dir = args["output_dir"].as_str().unwrap_or("");
    let frame_rate = args.get("frame_rate").and_then(|v| v.as_f64()).unwrap_or(1.0);
    let format = args.get("format").and_then(|v| v.as_str()).unwrap_or("png");
    crate::export::extract_frames(input, output_dir, frame_rate, format).unwrap_or_else(|e| e)
}

fn execute_picture_in_picture_claude(args: &Value) -> String {
    let main_video = args["main_video"].as_str().unwrap_or("");
    let pip_video = args["pip_video"].as_str().unwrap_or("");
    let output_raw = args["output_file"].as_str().unwrap_or("");
    let output = ensure_outputs_directory(output_raw);
    let x = args["x"].as_u64().unwrap_or(0).to_string();
    let y = args["y"].as_u64().unwrap_or(0).to_string();
    // Note: scale parameter is not supported by picture_in_picture function
    crate::advanced::picture_in_picture(main_video, pip_video, &output, &x, &y).unwrap_or_else(|e| e)
}

fn execute_chroma_key_claude(args: &Value) -> String {
    let input = args["input_file"].as_str().unwrap_or("");
    let background = args["background_file"].as_str().unwrap_or("");
    let output_raw = args["output_file"].as_str().unwrap_or("");
    let output = ensure_outputs_directory(output_raw);
    let key_color = args.get("key_color").and_then(|v| v.as_str()).unwrap_or("green");
    let similarity = args.get("similarity").and_then(|v| v.as_f64()).unwrap_or(0.3) as f32;
    let blend = 0.1f32; // Default blend value for smooth edges
    crate::advanced::chroma_key(input, background, &output, key_color, similarity, blend).unwrap_or_else(|e| e)
}

fn execute_split_screen_claude(args: &Value) -> String {
    let video1 = args["video1"].as_str().unwrap_or("");
    let video2 = args["video2"].as_str().unwrap_or("");
    let output_raw = args["output_file"].as_str().unwrap_or("");
    let output = ensure_outputs_directory(output_raw);
    let orientation = args["orientation"].as_str().unwrap_or("horizontal");
    crate::advanced::split_screen(video1, video2, &output, orientation).unwrap_or_else(|e| e)
}

fn execute_stabilize_video_claude(args: &Value) -> String {
    let input = args["input_file"].as_str().unwrap_or("");
    let output_raw = args["output_file"].as_str().unwrap_or("");
    let output = ensure_outputs_directory(output_raw);
    let strength = args["strength"].as_u64().unwrap_or(5) as u32;
    crate::transform::stabilize_video(input, &output, strength).unwrap_or_else(|e| e)
}

async fn execute_pexels_search_claude(args: &Value) -> String {
    let query = args["query"].as_str().unwrap_or("");
    let media_type = args["media_type"].as_str().unwrap_or("videos");
    let per_page = args.get("per_page").and_then(|v| v.as_u64()).unwrap_or(15) as i32;

    if query.is_empty() {
        return "❌ Error: query is required for Pexels search".to_string();
    }

    // Get Pexels API key from environment
    let api_key = match std::env::var("PEXELS_API_KEY") {
        Ok(key) if !key.is_empty() => key,
        _ => return "❌ Error: PEXELS_API_KEY environment variable not set".to_string(),
    };

    let pexels_client = crate::pexels_client::PexelsClient::new(api_key);

    match media_type {
        "videos" => {
            match pexels_client.search_videos(query, Some(per_page), None, None, None, None, None).await {
                Ok(response) => {
                    serde_json::to_string_pretty(&response)
                        .unwrap_or_else(|_| format!("❌ Failed to serialize Pexels response"))
                }
                Err(e) => format!("❌ Pexels search failed: {}", e),
            }
        }
        "photos" => {
            match pexels_client.search_photos(query, Some(per_page), None, None, None, None).await {
                Ok(response) => {
                    serde_json::to_string_pretty(&response)
                        .unwrap_or_else(|_| format!("❌ Failed to serialize Pexels response"))
                }
                Err(e) => format!("❌ Pexels search failed: {}", e),
            }
        }
        _ => format!("❌ Invalid media_type: {}. Use 'videos' or 'photos'", media_type),
    }
}

async fn execute_pexels_download_video_claude(args: &Value) -> String {
    let video_url = args["video_url"].as_str().unwrap_or("");
    let output_file = args["output_file"].as_str().unwrap_or("");

    if video_url.is_empty() || output_file.is_empty() {
        return "❌ Error: video_url and output_file are required".to_string();
    }

    match download_file_from_url(video_url, output_file).await {
        Ok(_) => format!("✅ Successfully downloaded video from Pexels to: {}", output_file),
        Err(e) => format!("❌ Failed to download video: {}", e),
    }
}

async fn execute_pexels_download_photo_claude(args: &Value) -> String {
    let photo_url = args["photo_url"].as_str().unwrap_or("");
    let output_file = args["output_file"].as_str().unwrap_or("");

    if photo_url.is_empty() || output_file.is_empty() {
        return "❌ Error: photo_url and output_file are required".to_string();
    }

    match download_file_from_url(photo_url, output_file).await {
        Ok(_) => format!("✅ Successfully downloaded photo from Pexels to: {}", output_file),
        Err(e) => format!("❌ Failed to download photo: {}", e),
    }
}

async fn execute_pexels_get_trending_claude(args: &Value) -> String {
    let per_page = args.get("per_page").and_then(|v| v.as_u64()).unwrap_or(15) as i32;

    // Get Pexels API key from environment
    let api_key = match std::env::var("PEXELS_API_KEY") {
        Ok(key) if !key.is_empty() => key,
        _ => return "❌ Error: PEXELS_API_KEY environment variable not set".to_string(),
    };

    let pexels_client = crate::pexels_client::PexelsClient::new(api_key);

    match pexels_client.get_trending_videos(Some(per_page), None).await {
        Ok(response) => {
            serde_json::to_string_pretty(&response)
                .unwrap_or_else(|_| format!("❌ Failed to serialize trending videos response"))
        }
        Err(e) => format!("❌ Failed to get trending videos: {}", e),
    }
}

async fn execute_pexels_get_curated_claude(args: &Value) -> String {
    let per_page = args.get("per_page").and_then(|v| v.as_u64()).unwrap_or(15) as i32;

    // Get Pexels API key from environment
    let api_key = match std::env::var("PEXELS_API_KEY") {
        Ok(key) if !key.is_empty() => key,
        _ => return "❌ Error: PEXELS_API_KEY environment variable not set".to_string(),
    };

    let pexels_client = crate::pexels_client::PexelsClient::new(api_key);

    match pexels_client.get_curated_photos(Some(per_page), None).await {
        Ok(response) => {
            serde_json::to_string_pretty(&response)
                .unwrap_or_else(|_| format!("❌ Failed to serialize curated photos response"))
        }
        Err(e) => format!("❌ Failed to get curated photos: {}", e),
    }
}

async fn execute_analyze_image_claude(args: &Value) -> String {
    let image_path = args["image_path"].as_str().unwrap_or("");
    let analysis_type = args.get("analysis_type").and_then(|v| v.as_str()).unwrap_or("general");

    if image_path.is_empty() {
        return "❌ Error: image_path is required".to_string();
    }

    // Check if file exists
    if tokio::fs::metadata(image_path).await.is_err() {
        return format!("❌ Error: Image file not found: {}", image_path);
    }

    // Get Gemini API key from environment
    let api_key = match std::env::var("GEMINI_API_KEY").or_else(|_| std::env::var("GOOGLE_API_KEY")) {
        Ok(key) if !key.is_empty() => key,
        _ => return "❌ Error: GEMINI_API_KEY or GOOGLE_API_KEY environment variable not set".to_string(),
    };

    let gemini_client = crate::gemini_client::GeminiClient::new(api_key);

    // Create analysis prompt based on type
    let prompt = match analysis_type {
        "detailed" => "Provide a detailed analysis of this image, including: composition, lighting, colors, subjects, objects, mood, style, and any text or graphics present.",
        "objects" => "List and describe all objects visible in this image with their positions and characteristics.",
        "colors" => "Analyze the color palette of this image, identifying dominant colors, color harmony, and mood created by the colors.",
        _ => "Describe what you see in this image in detail.",
    };

    match gemini_client.analyze_video_content(image_path, Some(prompt.to_string())).await {
        Ok(analysis) => {
            format!("🖼️ **Image Analysis: {}**\n\nType: {}\n\n{}", image_path, analysis_type, analysis)
        }
        Err(e) => format!("❌ Failed to analyze image: {}", e),
    }
}

async fn execute_generate_text_to_speech_claude(args: &Value) -> String {
    let text = args["text"].as_str().unwrap_or("");
    let output_file = args["output_file"].as_str().unwrap_or("");
    let voice = args.get("voice").and_then(|v| v.as_str()).unwrap_or("neutral");
    let _speed = args.get("speed").and_then(|v| v.as_f64()).unwrap_or(1.0);

    if text.is_empty() || output_file.is_empty() {
        return "❌ Error: text and output_file are required".to_string();
    }

    // Get Gemini API key
    let api_key = match std::env::var("GEMINI_API_KEY").or_else(|_| std::env::var("GOOGLE_API_KEY")) {
        Ok(key) if !key.is_empty() => key,
        _ => return "❌ Error: GEMINI_API_KEY or GOOGLE_API_KEY environment variable not set".to_string(),
    };

    // Map voice preference to Gemini voice names
    let voice_name = match voice.to_lowercase().as_str() {
        "male" => "Kore",
        "female" => "Aoede",
        "neutral" => "Puck",
        _ => "Puck",
    };

    // Build TTS request for Gemini 2.5 Flash TTS
    let request = serde_json::json!({
        "contents": [{
            "parts": [{
                "text": text
            }],
            "role": "user"
        }],
        "generationConfig": {
            "response_modalities": ["AUDIO"],
            "speech_config": {
                "voice_config": {
                    "prebuilt_voice_config": {
                        "voice_name": voice_name
                    }
                }
            }
        }
    });

    let client = reqwest::Client::new();
    let url = format!("https://generativelanguage.googleapis.com/v1beta/models/gemini-2.5-flash-preview-tts:generateContent?key={}", api_key);

    match client.post(&url)
        .header("Content-Type", "application/json")
        .json(&request)
        .send()
        .await
    {
        Ok(response) if response.status().is_success() => {
            match response.text().await {
                Ok(response_text) => {
                    // Parse response to extract audio data
                    if let Ok(json_response) = serde_json::from_str::<serde_json::Value>(&response_text) {
                        if let Some(candidates) = json_response["candidates"].as_array() {
                            if let Some(candidate) = candidates.first() {
                                if let Some(content) = candidate.get("content") {
                                    if let Some(parts) = content["parts"].as_array() {
                                        for part in parts {
                                            if let Some(inline_data) = part.get("inlineData") {
                                                if let Some(data) = inline_data["data"].as_str() {
                                                    // Decode base64 audio and save
                                                    match BASE64_STANDARD.decode(data) {
                                                        Ok(audio_bytes) => {
                                                            match tokio::fs::write(&output_file, &audio_bytes).await {
                                                                Ok(_) => return format!("✅ Successfully generated speech audio and saved to: {}", output_file),
                                                                Err(e) => return format!("❌ Failed to save audio file: {}", e),
                                                            }
                                                        }
                                                        Err(e) => return format!("❌ Failed to decode audio data: {}", e),
                                                    }
                                                }
                                            }
                                        }
                                    }
                                }
                            }
                        }
                    }
                    format!("❌ No audio data found in TTS response")
                }
                Err(e) => format!("❌ Failed to read TTS response: {}", e),
            }
        }
        Ok(response) => {
            let status = response.status();
            let error_text = response.text().await.unwrap_or_else(|_| "Unknown error".to_string());
            format!("❌ TTS API error ({}): {}", status, error_text)
        }
        Err(e) => format!("❌ Failed to call TTS API: {}", e),
    }
}

async fn execute_generate_video_script_claude(args: &Value) -> String {
    let topic = args["topic"].as_str().unwrap_or("");
    let duration = args["duration"].as_f64().unwrap_or(60.0);
    let style = args.get("style").and_then(|v| v.as_str()).unwrap_or("educational");
    let tone = args.get("tone").and_then(|v| v.as_str()).unwrap_or("professional");

    if topic.is_empty() {
        return "❌ Error: topic is required".to_string();
    }

    // Get Gemini API key
    let api_key = match std::env::var("GEMINI_API_KEY").or_else(|_| std::env::var("GOOGLE_API_KEY")) {
        Ok(key) if !key.is_empty() => key,
        _ => return "❌ Error: GEMINI_API_KEY or GOOGLE_API_KEY environment variable not set".to_string(),
    };

    let gemini_client = crate::gemini_client::GeminiClient::new(api_key);

    match gemini_client.generate_video_script(
        style,
        topic,
        &format!("Create a {} video about {}", style, topic),
        duration as u32,
        Some(tone),
        Some(style),
    ).await {
        Ok(script) => {
            format!("📝 **Video Script Generated**\n\nTopic: {}\nDuration: {:.0}s\nStyle: {}\nTone: {}\n\n{}",
                topic, duration, style, tone, script)
        }
        Err(e) => format!("❌ Failed to generate video script: {}", e),
    }
}

fn execute_create_blank_video_claude(args: &Value) -> String {
    let output_raw = args["output_file"].as_str().unwrap_or("");
    let output = ensure_outputs_directory(output_raw);
    let duration = args["duration"].as_f64().unwrap_or(10.0);
    let width = args["width"].as_u64().unwrap_or(1920) as u32;
    let height = args["height"].as_u64().unwrap_or(1080) as u32;
    let color = args.get("color").and_then(|v| v.as_str()).unwrap_or("black");
    crate::utils::create_blank_video(&output, duration, width, height, color).unwrap_or_else(|e| e)
}

fn execute_submit_final_answer_claude(args: &Value) -> String {
    let summary = args["summary"].as_str().unwrap_or("Task completed");
    let output_files = args.get("output_files").and_then(|v| v.as_array())
        .map(|arr| arr.iter().filter_map(|v| v.as_str()).collect::<Vec<_>>())
        .unwrap_or_default();

    let mut response = format!("✅ {}\n\n", summary);

    if !output_files.is_empty() {
        response.push_str("📥 **Your edited videos are ready!**\n\n");
        for file_path in output_files {
            // Generate deterministic file ID from path (same as download endpoint uses)
            let file_id = generate_file_id_from_path(file_path);
            let file_name = std::path::Path::new(file_path)
                .file_name()
                .and_then(|n| n.to_str())
                .unwrap_or("video.mp4");

            // Create download, stream, and YouTube upload URLs (frontend will convert to buttons)
            response.push_str(&format!("**{}**\n", file_name));
            response.push_str(&format!("Download: `/api/outputs/download/{}`\n", file_id));
            response.push_str(&format!("Stream: `/api/outputs/stream/{}`\n", file_id));
            response.push_str(&format!("YouTube: `{}|{}`\n\n", file_path, file_name));
        }
    }

    response
}

/// Generate deterministic file ID from path (matches output.rs logic)
fn generate_file_id_from_path(path: &str) -> String {
    use std::collections::hash_map::DefaultHasher;
    use std::hash::{Hash, Hasher};

    let mut hasher = DefaultHasher::new();
    path.hash(&mut hasher);
    format!("{:x}", hasher.finish())
}

// ============================================================================
// GEMINI TOOL EXECUTORS (args: &HashMap<String, Value>)
// ============================================================================

fn execute_trim_video_gemini(args: &HashMap<String, Value>) -> String {
    let input = args.get("input_file").and_then(|v| v.as_str()).unwrap_or("");
    let output = args.get("output_file").and_then(|v| v.as_str()).unwrap_or("");
    let start = args.get("start_seconds").and_then(|v| v.as_f64()).unwrap_or(0.0);
    let end = args.get("end_seconds").and_then(|v| v.as_f64()).unwrap_or(0.0);
    crate::core::trim_video(input, &output, start, end).unwrap_or_else(|e| e)
}

fn execute_merge_videos_gemini(args: &HashMap<String, Value>) -> String {
    let input_files: Vec<String> = args.get("input_files").and_then(|v| v.as_array())
        .map(|arr| arr.iter().filter_map(|v| v.as_str()).map(|s| s.to_string()).collect())
        .unwrap_or_default();
    let output = args.get("output_file").and_then(|v| v.as_str()).unwrap_or("");
    crate::core::merge_videos(&input_files, &output).unwrap_or_else(|e| e)
}

fn execute_analyze_video_gemini(args: &HashMap<String, Value>) -> String {
    let input = args.get("input_file").and_then(|v| v.as_str()).unwrap_or("");
    match crate::core::analyze_video(input) {
        Ok(metadata) => serde_json::to_string_pretty(&metadata)
            .unwrap_or_else(|_| "Failed to serialize metadata".to_string()),
        Err(e) => e,
    }
}

fn execute_split_video_gemini(args: &HashMap<String, Value>) -> String {
    let input = args.get("input_file").and_then(|v| v.as_str()).unwrap_or("");
    let output_prefix = args.get("output_prefix").and_then(|v| v.as_str()).unwrap_or("");
    let segment_duration = args.get("segment_duration").and_then(|v| v.as_f64()).unwrap_or(10.0);
    crate::core::split_video(input, output_prefix, segment_duration).unwrap_or_else(|e| e)
}

fn execute_add_text_overlay_gemini(args: &HashMap<String, Value>) -> String {
    let input = args.get("input_file").and_then(|v| v.as_str()).unwrap_or("");
    let output = args.get("output_file").and_then(|v| v.as_str()).unwrap_or("");
    let text = args.get("text").and_then(|v| v.as_str()).unwrap_or("");
    let x = &args.get("x").and_then(|v| v.as_u64()).unwrap_or(960).to_string();
    let y = &args.get("y").and_then(|v| v.as_u64()).unwrap_or(540).to_string();
    let font_file = args.get("font_file").and_then(|v| v.as_str())
        .unwrap_or("/usr/share/fonts/truetype/dejavu/DejaVuSans-Bold.ttf");
    let font_size = args.get("font_size").and_then(|v| v.as_u64()).unwrap_or(48) as u32;
    let color = args.get("color").and_then(|v| v.as_str()).unwrap_or("white");
    let start_time = args.get("start_time").and_then(|v| v.as_f64()).unwrap_or(0.0);
    let end_time = args.get("end_time").and_then(|v| v.as_f64()).unwrap_or(999999.0);
    crate::visual::add_text_overlay(input, &output, text, x, y, font_file, font_size, color, start_time, end_time)
        .unwrap_or_else(|e| e)
}

fn execute_apply_filter_gemini(args: &HashMap<String, Value>) -> String {
    let input = args.get("input_file").and_then(|v| v.as_str()).unwrap_or("");
    let output = args.get("output_file").and_then(|v| v.as_str()).unwrap_or("");
    let filter = args.get("filter_type").and_then(|v| v.as_str()).unwrap_or("");
    let intensity = args.get("intensity").and_then(|v| v.as_f64()).unwrap_or(1.0);
    crate::visual::apply_filter(input, &output, filter, intensity).unwrap_or_else(|e| e)
}

fn execute_add_overlay_gemini(args: &HashMap<String, Value>) -> String {
    let input = args.get("input_file").and_then(|v| v.as_str()).unwrap_or("");
    let output = args.get("output_file").and_then(|v| v.as_str()).unwrap_or("");
    let overlay = args.get("overlay_file").and_then(|v| v.as_str()).unwrap_or("");
    let x = args.get("x").and_then(|v| v.as_u64()).unwrap_or(0) as u32;
    let y = args.get("y").and_then(|v| v.as_u64()).unwrap_or(0) as u32;
    crate::visual::add_overlay(input, overlay, &output, x, y).unwrap_or_else(|e| e)
}

fn execute_adjust_color_gemini(args: &HashMap<String, Value>) -> String {
    let input = args.get("input_file").and_then(|v| v.as_str()).unwrap_or("");
    let output = args.get("output_file").and_then(|v| v.as_str()).unwrap_or("");
    let brightness = args.get("brightness").and_then(|v| v.as_f64()).unwrap_or(0.0);
    let contrast = args.get("contrast").and_then(|v| v.as_f64()).unwrap_or(0.0);
    let saturation = args.get("saturation").and_then(|v| v.as_f64()).unwrap_or(0.0);
    // Note: hue is not supported by adjust_color function (only brightness, contrast, saturation)
    crate::visual::adjust_color(input, &output, brightness, contrast, saturation).unwrap_or_else(|e| e)
}

fn execute_add_subtitles_gemini(args: &HashMap<String, Value>) -> String {
    let input = args.get("input_file").and_then(|v| v.as_str()).unwrap_or("");
    let output = args.get("output_file").and_then(|v| v.as_str()).unwrap_or("");
    let subtitle_text = args.get("subtitle_text").and_then(|v| v.as_str()).unwrap_or("");
    // Note: add_subtitles only takes (input, subtitle, output) - font_size and color not supported
    crate::visual::add_subtitles(input, subtitle_text, output).unwrap_or_else(|e| e)
}

fn execute_resize_video_gemini(args: &HashMap<String, Value>) -> String {
    let input = args.get("input_file").and_then(|v| v.as_str()).unwrap_or("");
    let output = args.get("output_file").and_then(|v| v.as_str()).unwrap_or("");
    let width = args.get("width").and_then(|v| v.as_u64()).unwrap_or(1920) as u32;
    let height = args.get("height").and_then(|v| v.as_u64()).unwrap_or(1080) as u32;
    crate::transform::resize_video(input, &output, width, height).unwrap_or_else(|e| e)
}

fn execute_crop_video_gemini(args: &HashMap<String, Value>) -> String {
    let input = args.get("input_file").and_then(|v| v.as_str()).unwrap_or("");
    let output = args.get("output_file").and_then(|v| v.as_str()).unwrap_or("");
    let x = args.get("x").and_then(|v| v.as_u64()).unwrap_or(0) as u32;
    let y = args.get("y").and_then(|v| v.as_u64()).unwrap_or(0) as u32;
    let width = args.get("width").and_then(|v| v.as_u64()).unwrap_or(1920) as u32;
    let height = args.get("height").and_then(|v| v.as_u64()).unwrap_or(1080) as u32;
    crate::transform::crop_video(input, &output, width, height, x, y).unwrap_or_else(|e| e)
}

fn execute_rotate_video_gemini(args: &HashMap<String, Value>) -> String {
    let input = args.get("input_file").and_then(|v| v.as_str()).unwrap_or("");
    let output = args.get("output_file").and_then(|v| v.as_str()).unwrap_or("");
    let degrees = args.get("degrees").and_then(|v| v.as_f64()).unwrap_or(0.0);
    let angle_str = format!("{}", degrees as i32);
    crate::transform::rotate_video(input, &output, &angle_str).unwrap_or_else(|e| e)
}

fn execute_adjust_speed_gemini(args: &HashMap<String, Value>) -> String {
    let input = args.get("input_file").and_then(|v| v.as_str()).unwrap_or("");
    let output = args.get("output_file").and_then(|v| v.as_str()).unwrap_or("");
    let speed_factor = args.get("speed_factor").and_then(|v| v.as_f64()).unwrap_or(1.0);
    crate::transform::adjust_speed(input, &output, speed_factor).unwrap_or_else(|e| e)
}

fn execute_flip_video_gemini(args: &HashMap<String, Value>) -> String {
    let input = args.get("input_file").and_then(|v| v.as_str()).unwrap_or("");
    let output = args.get("output_file").and_then(|v| v.as_str()).unwrap_or("");
    let direction = args.get("direction").and_then(|v| v.as_str()).unwrap_or("horizontal");
    crate::transform::flip_video(input, &output, direction).unwrap_or_else(|e| e)
}

fn execute_scale_video_gemini(args: &HashMap<String, Value>) -> String {
    let input = args.get("input_file").and_then(|v| v.as_str()).unwrap_or("");
    let output = args.get("output_file").and_then(|v| v.as_str()).unwrap_or("");
    let scale_factor = args.get("scale_factor").and_then(|v| v.as_f64()).unwrap_or(1.0);
    let algorithm = "bicubic"; // Default scaling algorithm
    crate::transform::scale_video(input, &output, scale_factor, algorithm).unwrap_or_else(|e| e)
}

fn execute_extract_audio_gemini(args: &HashMap<String, Value>) -> String {
    let input = args.get("input_file").and_then(|v| v.as_str()).unwrap_or("");
    let output = args.get("output_file").and_then(|v| v.as_str()).unwrap_or("");
    let format = args.get("format").and_then(|v| v.as_str()).unwrap_or("mp3");
    crate::audio::extract_audio(input, &output, format).unwrap_or_else(|e| e)
}

fn execute_add_audio_gemini(args: &HashMap<String, Value>) -> String {
    let input = args.get("input_file").and_then(|v| v.as_str()).unwrap_or("");
    let output = args.get("output_file").and_then(|v| v.as_str()).unwrap_or("");
    let audio_file = args.get("audio_file").and_then(|v| v.as_str()).unwrap_or("");
    // Note: add_audio signature is (video, audio, output) - no replace parameter
    crate::audio::add_audio(input, audio_file, output).unwrap_or_else(|e| e)
}

fn execute_adjust_volume_gemini(args: &HashMap<String, Value>) -> String {
    let input = args.get("input_file").and_then(|v| v.as_str()).unwrap_or("");
    let output = args.get("output_file").and_then(|v| v.as_str()).unwrap_or("");
    let volume_factor = args.get("volume_factor").and_then(|v| v.as_f64()).unwrap_or(1.0);
    crate::audio::adjust_volume(input, &output, volume_factor).unwrap_or_else(|e| e)
}

fn execute_fade_audio_gemini(args: &HashMap<String, Value>) -> String {
    let input = args.get("input_file").and_then(|v| v.as_str()).unwrap_or("");
    let output = args.get("output_file").and_then(|v| v.as_str()).unwrap_or("");
    let fade_in_duration = args.get("fade_in_duration").and_then(|v| v.as_f64()).unwrap_or(0.0);
    let fade_out_duration = args.get("fade_out_duration").and_then(|v| v.as_f64()).unwrap_or(0.0);
    // fade_audio requires total duration as 5th parameter - use analyze_video to get it or estimate
    let duration = 60.0; // Default estimate - ideally should analyze video first
    crate::audio::fade_audio(input, &output, fade_in_duration, fade_out_duration, duration).unwrap_or_else(|e| e)
}

fn execute_convert_format_gemini(args: &HashMap<String, Value>) -> String {
    let input = args.get("input_file").and_then(|v| v.as_str()).unwrap_or("");
    let output = args.get("output_file").and_then(|v| v.as_str()).unwrap_or("");
    let format = args.get("format").and_then(|v| v.as_str()).unwrap_or("mp4");
    crate::export::convert_format(input, &output, format).unwrap_or_else(|e| e)
}

fn execute_compress_video_gemini(args: &HashMap<String, Value>) -> String {
    let input = args.get("input_file").and_then(|v| v.as_str()).unwrap_or("");
    let output = args.get("output_file").and_then(|v| v.as_str()).unwrap_or("");
    let quality = args.get("quality").and_then(|v| v.as_str()).unwrap_or("medium");
    crate::export::compress_video(input, &output, quality).unwrap_or_else(|e| e)
}

fn execute_export_for_platform_gemini(args: &HashMap<String, Value>) -> String {
    let input = args.get("input_file").and_then(|v| v.as_str()).unwrap_or("");
    let output = args.get("output_file").and_then(|v| v.as_str()).unwrap_or("");
    let platform = args.get("platform").and_then(|v| v.as_str()).unwrap_or("youtube");
    crate::export::export_for_platform(input, &output, platform).unwrap_or_else(|e| e)
}

fn execute_create_thumbnail_gemini(args: &HashMap<String, Value>) -> String {
    let input = args.get("input_file").and_then(|v| v.as_str()).unwrap_or("");
    let output = args.get("output_file").and_then(|v| v.as_str()).unwrap_or("");
    let timestamp = args.get("timestamp").and_then(|v| v.as_f64()).unwrap_or(0.0);
    // Note: create_thumbnail only takes 3 params (input, output, timestamp) - width/height not supported
    crate::transform::create_thumbnail(input, &output, timestamp).unwrap_or_else(|e| e)
}

fn execute_extract_frames_gemini(args: &HashMap<String, Value>) -> String {
    let input = args.get("input_file").and_then(|v| v.as_str()).unwrap_or("");
    let output_dir = args.get("output_dir").and_then(|v| v.as_str()).unwrap_or("");
    let frame_rate = args.get("frame_rate").and_then(|v| v.as_f64()).unwrap_or(1.0);
    let format = args.get("format").and_then(|v| v.as_str()).unwrap_or("png");
    crate::export::extract_frames(input, output_dir, frame_rate, format).unwrap_or_else(|e| e)
}

fn execute_picture_in_picture_gemini(args: &HashMap<String, Value>) -> String {
    let main_video = args.get("main_video").and_then(|v| v.as_str()).unwrap_or("");
    let pip_video = args.get("pip_video").and_then(|v| v.as_str()).unwrap_or("");
    let output = args.get("output_file").and_then(|v| v.as_str()).unwrap_or("");
    let x = args.get("x").and_then(|v| v.as_u64()).unwrap_or(0).to_string();
    let y = args.get("y").and_then(|v| v.as_u64()).unwrap_or(0).to_string();
    // Note: scale parameter is not supported by picture_in_picture function
    crate::advanced::picture_in_picture(main_video, pip_video, &output, &x, &y).unwrap_or_else(|e| e)
}

fn execute_chroma_key_gemini(args: &HashMap<String, Value>) -> String {
    let input = args.get("input_file").and_then(|v| v.as_str()).unwrap_or("");
    let background = args.get("background_file").and_then(|v| v.as_str()).unwrap_or("");
    let output = args.get("output_file").and_then(|v| v.as_str()).unwrap_or("");
    let key_color = args.get("key_color").and_then(|v| v.as_str()).unwrap_or("green");
    let similarity = args.get("similarity").and_then(|v| v.as_f64()).unwrap_or(0.3) as f32;
    let blend = 0.1f32; // Default blend value for smooth edges
    crate::advanced::chroma_key(input, background, &output, key_color, similarity, blend).unwrap_or_else(|e| e)
}

fn execute_split_screen_gemini(args: &HashMap<String, Value>) -> String {
    let video1 = args.get("video1").and_then(|v| v.as_str()).unwrap_or("");
    let video2 = args.get("video2").and_then(|v| v.as_str()).unwrap_or("");
    let output = args.get("output_file").and_then(|v| v.as_str()).unwrap_or("");
    let orientation = args.get("orientation").and_then(|v| v.as_str()).unwrap_or("horizontal");
    crate::advanced::split_screen(video1, video2, &output, orientation).unwrap_or_else(|e| e)
}

fn execute_stabilize_video_gemini(args: &HashMap<String, Value>) -> String {
    let input = args.get("input_file").and_then(|v| v.as_str()).unwrap_or("");
    let output = args.get("output_file").and_then(|v| v.as_str()).unwrap_or("");
    let strength = args.get("strength").and_then(|v| v.as_u64()).unwrap_or(5) as u32;
    crate::transform::stabilize_video(input, &output, strength).unwrap_or_else(|e| e)
}

async fn execute_pexels_search_gemini(args: &HashMap<String, Value>) -> String {
    let query = args.get("query").and_then(|v| v.as_str()).unwrap_or("");
    let media_type = args.get("media_type").and_then(|v| v.as_str()).unwrap_or("videos");
    let per_page = args.get("per_page").and_then(|v| v.as_u64()).unwrap_or(15) as i32;

    if query.is_empty() {
        return "❌ Error: query is required for Pexels search".to_string();
    }

    // Get Pexels API key from environment
    let api_key = match std::env::var("PEXELS_API_KEY") {
        Ok(key) if !key.is_empty() => key,
        _ => return "❌ Error: PEXELS_API_KEY environment variable not set".to_string(),
    };

    let pexels_client = crate::pexels_client::PexelsClient::new(api_key);

    match media_type {
        "videos" => {
            match pexels_client.search_videos(query, Some(per_page), None, None, None, None, None).await {
                Ok(response) => {
                    serde_json::to_string_pretty(&response)
                        .unwrap_or_else(|_| format!("❌ Failed to serialize Pexels response"))
                }
                Err(e) => format!("❌ Pexels search failed: {}", e),
            }
        }
        "photos" => {
            match pexels_client.search_photos(query, Some(per_page), None, None, None, None).await {
                Ok(response) => {
                    serde_json::to_string_pretty(&response)
                        .unwrap_or_else(|_| format!("❌ Failed to serialize Pexels response"))
                }
                Err(e) => format!("❌ Pexels search failed: {}", e),
            }
        }
        _ => format!("❌ Invalid media_type: {}. Use 'videos' or 'photos'", media_type),
    }
}

async fn execute_pexels_download_video_gemini(args: &HashMap<String, Value>) -> String {
    let video_url = args.get("video_url").and_then(|v| v.as_str()).unwrap_or("");
    let output_file_raw = args.get("output_file").and_then(|v| v.as_str()).unwrap_or("");
    let output_file = ensure_outputs_directory(output_file_raw);

    if video_url.is_empty() || output_file.is_empty() {
        return "❌ Error: video_url and output_file are required".to_string();
    }

    match download_file_from_url(video_url, &output_file).await {
        Ok(_) => format!("✅ Successfully downloaded video from Pexels to: {}", output_file),
        Err(e) => format!("❌ Failed to download video: {}", e),
    }
}

async fn execute_pexels_download_photo_gemini(args: &HashMap<String, Value>) -> String {
    let photo_url = args.get("photo_url").and_then(|v| v.as_str()).unwrap_or("");
    let output_file_raw = args.get("output_file").and_then(|v| v.as_str()).unwrap_or("");
    let output_file = ensure_outputs_directory(output_file_raw);

    if photo_url.is_empty() || output_file.is_empty() {
        return "❌ Error: photo_url and output_file are required".to_string();
    }

    match download_file_from_url(photo_url, &output_file).await {
        Ok(_) => format!("✅ Successfully downloaded photo from Pexels to: {}", output_file),
        Err(e) => format!("❌ Failed to download photo: {}", e),
    }
}

async fn execute_pexels_get_trending_gemini(args: &HashMap<String, Value>) -> String {
    let per_page = args.get("per_page").and_then(|v| v.as_u64()).unwrap_or(15) as i32;

    // Get Pexels API key from environment
    let api_key = match std::env::var("PEXELS_API_KEY") {
        Ok(key) if !key.is_empty() => key,
        _ => return "❌ Error: PEXELS_API_KEY environment variable not set".to_string(),
    };

    let pexels_client = crate::pexels_client::PexelsClient::new(api_key);

    match pexels_client.get_trending_videos(Some(per_page), None).await {
        Ok(response) => {
            serde_json::to_string_pretty(&response)
                .unwrap_or_else(|_| format!("❌ Failed to serialize trending videos response"))
        }
        Err(e) => format!("❌ Failed to get trending videos: {}", e),
    }
}

async fn execute_pexels_get_curated_gemini(args: &HashMap<String, Value>) -> String {
    let per_page = args.get("per_page").and_then(|v| v.as_u64()).unwrap_or(15) as i32;

    // Get Pexels API key from environment
    let api_key = match std::env::var("PEXELS_API_KEY") {
        Ok(key) if !key.is_empty() => key,
        _ => return "❌ Error: PEXELS_API_KEY environment variable not set".to_string(),
    };

    let pexels_client = crate::pexels_client::PexelsClient::new(api_key);

    match pexels_client.get_curated_photos(Some(per_page), None).await {
        Ok(response) => {
            serde_json::to_string_pretty(&response)
                .unwrap_or_else(|_| format!("❌ Failed to serialize curated photos response"))
        }
        Err(e) => format!("❌ Failed to get curated photos: {}", e),
    }
}

async fn execute_analyze_image_gemini(args: &HashMap<String, Value>) -> String {
    let image_path = args.get("image_path").and_then(|v| v.as_str()).unwrap_or("");
    let analysis_type = args.get("analysis_type").and_then(|v| v.as_str()).unwrap_or("general");

    if image_path.is_empty() {
        return "❌ Error: image_path is required".to_string();
    }

    // Check if file exists
    if tokio::fs::metadata(image_path).await.is_err() {
        return format!("❌ Error: Image file not found: {}", image_path);
    }

    // Get Gemini API key from environment
    let api_key = match std::env::var("GEMINI_API_KEY").or_else(|_| std::env::var("GOOGLE_API_KEY")) {
        Ok(key) if !key.is_empty() => key,
        _ => return "❌ Error: GEMINI_API_KEY or GOOGLE_API_KEY environment variable not set".to_string(),
    };

    let gemini_client = crate::gemini_client::GeminiClient::new(api_key);

    // Create analysis prompt based on type
    let prompt = match analysis_type {
        "detailed" => "Provide a detailed analysis of this image, including: composition, lighting, colors, subjects, objects, mood, style, and any text or graphics present.",
        "objects" => "List and describe all objects visible in this image with their positions and characteristics.",
        "colors" => "Analyze the color palette of this image, identifying dominant colors, color harmony, and mood created by the colors.",
        _ => "Describe what you see in this image in detail.",
    };

    match gemini_client.analyze_video_content(image_path, Some(prompt.to_string())).await {
        Ok(analysis) => {
            format!("🖼️ **Image Analysis: {}**\n\nType: {}\n\n{}", image_path, analysis_type, analysis)
        }
        Err(e) => format!("❌ Failed to analyze image: {}", e),
    }
}

async fn execute_generate_text_to_speech_gemini(args: &HashMap<String, Value>) -> String {
    let text = args.get("text").and_then(|v| v.as_str()).unwrap_or("");
    let output_file_raw = args.get("output_file").and_then(|v| v.as_str()).unwrap_or("");
    let output_file = ensure_outputs_directory(output_file_raw);
    let voice = args.get("voice").and_then(|v| v.as_str()).unwrap_or("neutral");
    let _speed = args.get("speed").and_then(|v| v.as_f64()).unwrap_or(1.0);

    if text.is_empty() || output_file.is_empty() {
        return "❌ Error: text and output_file are required".to_string();
    }

    // Get Gemini API key
    let api_key = match std::env::var("GEMINI_API_KEY").or_else(|_| std::env::var("GOOGLE_API_KEY")) {
        Ok(key) if !key.is_empty() => key,
        _ => return "❌ Error: GEMINI_API_KEY or GOOGLE_API_KEY environment variable not set".to_string(),
    };

    // Map voice preference to Gemini voice names
    let voice_name = match voice.to_lowercase().as_str() {
        "male" => "Kore",
        "female" => "Aoede",
        "neutral" => "Puck",
        _ => "Puck",
    };

    // Build TTS request for Gemini 2.5 Flash TTS
    let request = serde_json::json!({
        "contents": [{
            "parts": [{
                "text": text
            }],
            "role": "user"
        }],
        "generationConfig": {
            "response_modalities": ["AUDIO"],
            "speech_config": {
                "voice_config": {
                    "prebuilt_voice_config": {
                        "voice_name": voice_name
                    }
                }
            }
        }
    });

    let client = reqwest::Client::new();
    let url = format!("https://generativelanguage.googleapis.com/v1beta/models/gemini-2.5-flash-preview-tts:generateContent?key={}", api_key);

    match client.post(&url)
        .header("Content-Type", "application/json")
        .json(&request)
        .send()
        .await
    {
        Ok(response) if response.status().is_success() => {
            match response.text().await {
                Ok(response_text) => {
                    // Parse response to extract audio data
                    if let Ok(json_response) = serde_json::from_str::<serde_json::Value>(&response_text) {
                        if let Some(candidates) = json_response["candidates"].as_array() {
                            if let Some(candidate) = candidates.first() {
                                if let Some(content) = candidate.get("content") {
                                    if let Some(parts) = content["parts"].as_array() {
                                        for part in parts {
                                            if let Some(inline_data) = part.get("inlineData") {
                                                if let Some(data) = inline_data["data"].as_str() {
                                                    // Decode base64 audio and save
                                                    match BASE64_STANDARD.decode(data) {
                                                        Ok(audio_bytes) => {
                                                            match tokio::fs::write(&output_file, &audio_bytes).await {
                                                                Ok(_) => return format!("✅ Successfully generated speech audio and saved to: {}", output_file),
                                                                Err(e) => return format!("❌ Failed to save audio file: {}", e),
                                                            }
                                                        }
                                                        Err(e) => return format!("❌ Failed to decode audio data: {}", e),
                                                    }
                                                }
                                            }
                                        }
                                    }
                                }
                            }
                        }
                    }
                    format!("❌ No audio data found in TTS response")
                }
                Err(e) => format!("❌ Failed to read TTS response: {}", e),
            }
        }
        Ok(response) => {
            let status = response.status();
            let error_text = response.text().await.unwrap_or_else(|_| "Unknown error".to_string());
            format!("❌ TTS API error ({}): {}", status, error_text)
        }
        Err(e) => format!("❌ Failed to call TTS API: {}", e),
    }
}

async fn execute_generate_video_script_gemini(args: &HashMap<String, Value>) -> String {
    let topic = args.get("topic").and_then(|v| v.as_str()).unwrap_or("");
    let duration = args.get("duration").and_then(|v| v.as_f64()).unwrap_or(60.0);
    let style = args.get("style").and_then(|v| v.as_str()).unwrap_or("educational");
    let tone = args.get("tone").and_then(|v| v.as_str()).unwrap_or("professional");

    if topic.is_empty() {
        return "❌ Error: topic is required".to_string();
    }

    // Get Gemini API key
    let api_key = match std::env::var("GEMINI_API_KEY").or_else(|_| std::env::var("GOOGLE_API_KEY")) {
        Ok(key) if !key.is_empty() => key,
        _ => return "❌ Error: GEMINI_API_KEY or GOOGLE_API_KEY environment variable not set".to_string(),
    };

    let gemini_client = crate::gemini_client::GeminiClient::new(api_key);

    match gemini_client.generate_video_script(
        style,
        topic,
        &format!("Create a {} video about {}", style, topic),
        duration as u32,
        Some(tone),
        Some(style),
    ).await {
        Ok(script) => {
            format!("📝 **Video Script Generated**\n\nTopic: {}\nDuration: {:.0}s\nStyle: {}\nTone: {}\n\n{}",
                topic, duration, style, tone, script)
        }
        Err(e) => format!("❌ Failed to generate video script: {}", e),
    }
}

fn execute_create_blank_video_gemini(args: &HashMap<String, Value>) -> String {
    let output = args.get("output_file").and_then(|v| v.as_str()).unwrap_or("");
    let duration = args.get("duration").and_then(|v| v.as_f64()).unwrap_or(10.0);
    let width = args.get("width").and_then(|v| v.as_u64()).unwrap_or(1920) as u32;
    let height = args.get("height").and_then(|v| v.as_u64()).unwrap_or(1080) as u32;
    let color = args.get("color").and_then(|v| v.as_str()).unwrap_or("black");
    crate::utils::create_blank_video(output, duration, width, height, color).unwrap_or_else(|e| e)
}

fn execute_submit_final_answer_gemini(args: &HashMap<String, Value>) -> String {
    let summary = args.get("summary").and_then(|v| v.as_str()).unwrap_or("Task completed");
    let output_files = args.get("output_files").and_then(|v| v.as_array())
        .map(|arr| arr.iter().filter_map(|v| v.as_str()).collect::<Vec<_>>())
        .unwrap_or_default();

    let mut response = format!("✅ {}\n\n", summary);

    if !output_files.is_empty() {
        response.push_str("📥 **Your edited videos are ready!**\n\n");
        for file_path in output_files {
            // Generate deterministic file ID from path (same as download endpoint uses)
            let file_id = generate_file_id_from_path(file_path);
            let file_name = std::path::Path::new(file_path)
                .file_name()
                .and_then(|n| n.to_str())
                .unwrap_or("video.mp4");

            // Create download, stream, and YouTube upload URLs (frontend will convert to buttons)
            response.push_str(&format!("**{}**\n", file_name));
            response.push_str(&format!("Download: `/api/outputs/download/{}`\n", file_id));
            response.push_str(&format!("Stream: `/api/outputs/stream/{}`\n", file_id));
            response.push_str(&format!("YouTube: `{}|{}`\n\n", file_path, file_name));
        }
    }

    response
}

// ============================================================================
// NEW TOOLS: IMAGE GENERATION & VIDEO ORCHESTRATION
// ============================================================================

/// Generate image using Nano Banana Pro (Claude version)
async fn execute_generate_image_claude(args: &Value) -> String {
    let prompt = args["prompt"].as_str().unwrap_or("");
    let output_file = args["output_file"].as_str().unwrap_or("");
    let aspect_ratio = args.get("aspect_ratio").and_then(|v| v.as_str());
    let image_size = args.get("image_size").and_then(|v| v.as_str());

    if prompt.is_empty() || output_file.is_empty() {
        return "❌ Error: prompt and output_file are required".to_string();
    }

    // Get Gemini API key from environment
    let api_key = std::env::var("GEMINI_API_KEY")
        .unwrap_or_else(|_| std::env::var("GOOGLE_API_KEY").unwrap_or_default());

    if api_key.is_empty() {
        return "❌ Error: GEMINI_API_KEY or GOOGLE_API_KEY environment variable not set".to_string();
    }

    // Create Gemini client for image generation
    let client = crate::gemini_client::GeminiClient::new(api_key);

    match client.generate_image(prompt, aspect_ratio, image_size).await {
        Ok(image_bytes) => {
            // Save image to file
            match tokio::fs::write(&output_file, &image_bytes).await {
                Ok(_) => format!("✅ Successfully generated image using Nano Banana Pro and saved to: {}", output_file),
                Err(e) => format!("❌ Failed to save generated image: {}", e),
            }
        }
        Err(e) => format!("❌ Failed to generate image: {}", e),
    }
}

/// Generate image using Nano Banana Pro (Gemini version)
async fn execute_generate_image_gemini(args: &HashMap<String, Value>) -> String {
    let prompt = args.get("prompt").and_then(|v| v.as_str()).unwrap_or("");
    let output_file_raw = args.get("output_file").and_then(|v| v.as_str()).unwrap_or("");
    let output_file = ensure_outputs_directory(output_file_raw);
    let aspect_ratio = args.get("aspect_ratio").and_then(|v| v.as_str());
    let image_size = args.get("image_size").and_then(|v| v.as_str());

    if prompt.is_empty() || output_file.is_empty() {
        return "❌ Error: prompt and output_file are required".to_string();
    }

    // Get Gemini API key from environment
    let api_key = std::env::var("GEMINI_API_KEY")
        .unwrap_or_else(|_| std::env::var("GOOGLE_API_KEY").unwrap_or_default());

    if api_key.is_empty() {
        return "❌ Error: GEMINI_API_KEY or GOOGLE_API_KEY environment variable not set".to_string();
    }

    // Create Gemini client for image generation
    let client = crate::gemini_client::GeminiClient::new(api_key);

    match client.generate_image(prompt, aspect_ratio, image_size).await {
        Ok(image_bytes) => {
            // Save image to file
            match tokio::fs::write(&output_file, &image_bytes).await {
                Ok(_) => format!("✅ Successfully generated image using Nano Banana Pro and saved to: {}", output_file),
                Err(e) => format!("❌ Failed to save generated image: {}", e),
            }
        }
        Err(e) => format!("❌ Failed to generate image: {}", e),
    }
}

/// Auto-generate video orchestration tool (Claude version)
async fn execute_auto_generate_video_claude(args: &Value) -> String {
    let topic = args["topic"].as_str().unwrap_or("");
    let output_filename = args["output_file"].as_str().unwrap_or("");
    // CRITICAL FIX: Save videos to outputs/ directory, not project root
    let output_file = format!("outputs/{}", output_filename);
    let duration = args.get("duration").and_then(|v| v.as_f64()).unwrap_or(30.0);
    let style = args.get("style").and_then(|v| v.as_str()).unwrap_or("cinematic");
    let include_text = args.get("include_text_overlays").and_then(|v| v.as_bool()).unwrap_or(true);
    let _include_music = args.get("include_music").and_then(|v| v.as_bool()).unwrap_or(false);
    let num_clips = args.get("num_clips").and_then(|v| v.as_u64()).unwrap_or(0) as usize;

    if topic.is_empty() || output_file.is_empty() {
        return "❌ Error: topic and output_file are required".to_string();
    }

    // Calculate number of clips based on duration if not specified
    let num_clips = if num_clips == 0 {
        ((duration / 10.0).ceil() as usize).max(3).min(8)
    } else {
        num_clips
    };

    let mut result = format!("🎬 **Auto-generating video about '{}'**\n\n", topic);
    result.push_str(&format!("Duration: {}s | Style: {} | Clips: {}\n\n", duration, style, num_clips));

    // Step 1: Generate search queries for Pexels
    result.push_str("📝 Step 1: Analyzing topic and generating search queries...\n");
    let search_queries = generate_search_queries_for_topic(topic, num_clips);

    // Step 2: Search and download clips from Pexels
    result.push_str("🔍 Step 2: Searching Pexels for relevant clips...\n");
    let mut downloaded_files = Vec::new();

    for (i, query) in search_queries.iter().enumerate().take(num_clips) {
        // Search Pexels
        let pexels_result = execute_pexels_search_claude(&serde_json::json!({
            "query": query,
            "media_type": "videos",
            "per_page": 1
        })).await;

        // Parse the result to extract video URL
        if let Ok(search_data) = serde_json::from_str::<Value>(&pexels_result) {
            if let Some(videos) = search_data["videos"].as_array() {
                if let Some(video) = videos.first() {
                    if let Some(files) = video["video_files"].as_array() {
                        if let Some(file) = files.first() {
                            if let Some(link) = file["link"].as_str() {
                                let clip_path = format!("outputs/clip_{}_{}.mp4", i, uuid::Uuid::new_v4().to_string().split('-').next().unwrap());

                                // Download the clip
                                let download_result = execute_pexels_download_video_claude(&serde_json::json!({
                                    "video_url": link,
                                    "output_file": &clip_path
                                })).await;

                                if download_result.contains("✅") {
                                    downloaded_files.push(clip_path.clone());
                                    result.push_str(&format!("  ✓ Downloaded clip {}: {}\n", i + 1, query));
                                }
                            }
                        }
                    }
                }
            }
        }
    }

    if downloaded_files.is_empty() {
        return format!("{}❌ Failed to download any video clips from Pexels", result);
    }

    result.push_str(&format!("\n✅ Downloaded {} clips\n\n", downloaded_files.len()));

    // Step 3: Merge clips
    result.push_str("🎞️  Step 3: Merging clips...\n");
    let merge_result = crate::core::merge_videos(&downloaded_files, &output_file).unwrap_or_else(|e| e);

    if merge_result.contains("❌") {
        return format!("{}❌ Failed to merge clips: {}", result, merge_result);
    }

    result.push_str("✅ Clips merged successfully\n\n");

    // Step 4: Add text overlays if requested
    if include_text {
        result.push_str("📝 Step 4: Adding text overlays...\n");
        let temp_output = format!("{}_with_text.mp4", output_file.trim_end_matches(".mp4"));

        let overlay_result = crate::visual::add_text_overlay(
            &output_file,
            &temp_output,
            &format!("{}", topic),
            "960",
            "100",
            "/usr/share/fonts/truetype/dejavu/DejaVuSans-Bold.ttf",
            64,
            "white",
            1.0,
            5.0
        ).unwrap_or_else(|e| e);

        if !overlay_result.contains("❌") {
            // Replace original with text version
            let _ = tokio::fs::rename(&temp_output, &output_file).await;
            result.push_str("✅ Text overlays added\n\n");
        }
    }

    // Cleanup temporary files
    for file in downloaded_files {
        let _ = tokio::fs::remove_file(&file).await;
    }

    result.push_str(&format!("🎉 **Video generation complete!**\n\n"));
    result.push_str(&format!("📥 Output: {}\n", output_file));

    result
}

/// Auto-generate video orchestration tool (Gemini version)
async fn execute_auto_generate_video_gemini(args: &HashMap<String, Value>) -> String {
    let topic = args.get("topic").and_then(|v| v.as_str()).unwrap_or("");
    let output_filename = args.get("output_file").and_then(|v| v.as_str()).unwrap_or("");
    // Ensure videos are saved to outputs/ directory
    let output_file = ensure_outputs_directory(output_filename);
    let duration = args.get("duration").and_then(|v| v.as_f64()).unwrap_or(30.0);
    let style = args.get("style").and_then(|v| v.as_str()).unwrap_or("cinematic");
    let include_text = args.get("include_text_overlays").and_then(|v| v.as_bool()).unwrap_or(true);
    let _include_music = args.get("include_music").and_then(|v| v.as_bool()).unwrap_or(false);
    let num_clips = args.get("num_clips").and_then(|v| v.as_u64()).unwrap_or(0) as usize;

    if topic.is_empty() || output_file.is_empty() {
        return "❌ Error: topic and output_file are required".to_string();
    }

    // Calculate number of clips based on duration if not specified
    let num_clips = if num_clips == 0 {
        ((duration / 10.0).ceil() as usize).max(3).min(8)
    } else {
        num_clips
    };

    let mut result = format!("🎬 **Auto-generating video about '{}'**\n\n", topic);
    result.push_str(&format!("Duration: {}s | Style: {} | Clips: {}\n\n", duration, style, num_clips));

    // Step 1: Generate search queries for Pexels
    result.push_str("📝 Step 1: Analyzing topic and generating search queries...\n");
    let search_queries = generate_search_queries_for_topic(topic, num_clips);

    // Step 2: Search and download clips from Pexels
    result.push_str("🔍 Step 2: Searching Pexels for relevant clips...\n");
    let mut downloaded_files = Vec::new();

    for (i, query) in search_queries.iter().enumerate().take(num_clips) {
        let mut search_args = HashMap::new();
        search_args.insert("query".to_string(), Value::String(query.clone()));
        search_args.insert("media_type".to_string(), Value::String("videos".to_string()));
        search_args.insert("per_page".to_string(), Value::Number(serde_json::Number::from(1)));

        // Search Pexels
        let pexels_result = execute_pexels_search_gemini(&search_args).await;

        // Parse the result to extract video URL
        if let Ok(search_data) = serde_json::from_str::<Value>(&pexels_result) {
            if let Some(videos) = search_data["videos"].as_array() {
                if let Some(video) = videos.first() {
                    if let Some(files) = video["video_files"].as_array() {
                        if let Some(file) = files.first() {
                            if let Some(link) = file["link"].as_str() {
                                let clip_path = format!("outputs/clip_{}_{}.mp4", i, uuid::Uuid::new_v4().to_string().split('-').next().unwrap());

                                let mut download_args = HashMap::new();
                                download_args.insert("video_url".to_string(), Value::String(link.to_string()));
                                download_args.insert("output_file".to_string(), Value::String(clip_path.clone()));

                                // Download the clip
                                let download_result = execute_pexels_download_video_gemini(&download_args).await;

                                if download_result.contains("✅") {
                                    downloaded_files.push(clip_path.clone());
                                    result.push_str(&format!("  ✓ Downloaded clip {}: {}\n", i + 1, query));
                                }
                            }
                        }
                    }
                }
            }
        }
    }

    if downloaded_files.is_empty() {
        return format!("{}❌ Failed to download any video clips from Pexels", result);
    }

    result.push_str(&format!("\n✅ Downloaded {} clips\n\n", downloaded_files.len()));

    // Step 3: Merge clips
    result.push_str("🎞️  Step 3: Merging clips...\n");
    let merge_result = crate::core::merge_videos(&downloaded_files, &output_file).unwrap_or_else(|e| e);

    if merge_result.contains("❌") {
        return format!("{}❌ Failed to merge clips: {}", result, merge_result);
    }

    result.push_str("✅ Clips merged successfully\n\n");

    // Step 4: Add text overlays if requested
    if include_text {
        result.push_str("📝 Step 4: Adding text overlays...\n");
        let temp_output = format!("{}_with_text.mp4", output_file.trim_end_matches(".mp4"));

        let overlay_result = crate::visual::add_text_overlay(
            &output_file,
            &temp_output,
            &format!("{}", topic),
            "960",
            "100",
            "/usr/share/fonts/truetype/dejavu/DejaVuSans-Bold.ttf",
            64,
            "white",
            1.0,
            5.0
        ).unwrap_or_else(|e| e);

        if !overlay_result.contains("❌") {
            // Replace original with text version
            let _ = tokio::fs::rename(&temp_output, &output_file).await;
            result.push_str("✅ Text overlays added\n\n");
        }
    }

    // Cleanup temporary files
    for file in downloaded_files {
        let _ = tokio::fs::remove_file(&file).await;
    }

    result.push_str(&format!("🎉 **Video generation complete!**\n\n"));
    result.push_str(&format!("📥 Output: {}\n", output_file));

    result
}

/// Helper function to generate search queries based on topic
fn generate_search_queries_for_topic(topic: &str, num_queries: usize) -> Vec<String> {
    // Simple keyword extraction and generation
    let base_keywords = vec![
        format!("{}", topic),
        format!("{} background", topic),
        format!("{} scenic", topic),
        format!("{} cinematic", topic),
        format!("{} atmosphere", topic),
        format!("{} landscape", topic),
        format!("{} aerial", topic),
        format!("{} closeup", topic),
    ];

    base_keywords.into_iter().take(num_queries).collect()
}

// ============================================================================
// VIDEO VIEWING & REVIEW TOOLS
// ============================================================================

/// View video by retrieving vectorized embeddings - WITH AppState (Claude version)
async fn execute_view_video_with_state_claude(args: &Value, ctx: &ToolExecutionContext) -> String {
    let video_path_input = args["video_path"].as_str().unwrap_or("");

    if video_path_input.is_empty() {
        return "❌ Error: video_path is required".to_string();
    }

    // Resolve file path - try as-is first, then try uploads/ directory
    let video_path = if tokio::fs::metadata(video_path_input).await.is_ok() {
        video_path_input.to_string()
    } else if tokio::fs::metadata(format!("uploads/{}", video_path_input)).await.is_ok() {
        format!("uploads/{}", video_path_input)
    } else {
        return format!("❌ Error: Video file not found: {}. Tried both '{}' and 'uploads/{}'", video_path_input, video_path_input, video_path_input);
    };

    // Retrieve video analysis from Qdrant
    match crate::services::VideoVectorizationService::retrieve_video_analysis(&video_path, &ctx.app_state).await {
        Ok(analysis) => {
            // Format the analysis for LLM consumption
            let summary = analysis.get("video_summary").and_then(|v| v.as_str()).unwrap_or("No summary");
            let duration = analysis.get("duration_seconds").and_then(|v| v.as_f64()).unwrap_or(0.0);
            let frame_count = analysis.get("frame_count").and_then(|v| v.as_u64()).unwrap_or(0);

            let mut result = format!("📹 **Video Analysis: {}**\n\n", video_path);
            result.push_str(&format!("**Duration:** {:.1}s\n", duration));
            result.push_str(&format!("**Frames Analyzed:** {}\n\n", frame_count));
            result.push_str(&format!("**Summary:**\n{}\n\n", summary));

            // Add frame details
            if let Some(frames) = analysis.get("frames").and_then(|v| v.as_array()) {
                result.push_str("**Frame-by-Frame Analysis:**\n");
                for (i, frame) in frames.iter().take(10).enumerate() {
                    let frame_num = frame.get("frame_number").and_then(|v| v.as_u64()).unwrap_or(i as u64);
                    let timestamp = frame.get("timestamp").and_then(|v| v.as_f64()).unwrap_or(0.0);
                    let desc = frame.get("description").and_then(|v| v.as_str()).unwrap_or("");

                    result.push_str(&format!("Frame {} ({:.1}s): {}\n", frame_num, timestamp, desc));
                }
                if frames.len() > 10 {
                    result.push_str(&format!("\n... and {} more frames\n", frames.len() - 10));
                }
            }

            result
        }
        Err(e) => {
            format!("❌ Failed to retrieve video analysis: {}. Note: Video may not be vectorized yet. Try re-analyzing or waiting for vectorization to complete.", e)
        }
    }
}

/// View video placeholder - calls context version
async fn execute_view_video_claude(args: &Value) -> String {
    format!("❌ Internal error: view_video must be called with context")
}

/// View video by retrieving vectorized embeddings - WITH AppState (Gemini version)
async fn execute_view_video_with_state_gemini(args: &HashMap<String, Value>, ctx: &ToolExecutionContext) -> String {
    let video_path_input = args.get("video_path").and_then(|v| v.as_str()).unwrap_or("");

    if video_path_input.is_empty() {
        return "❌ Error: video_path is required".to_string();
    }

    // Resolve file path - try as-is first, then try uploads/ directory
    let video_path = if tokio::fs::metadata(video_path_input).await.is_ok() {
        video_path_input.to_string()
    } else if tokio::fs::metadata(format!("uploads/{}", video_path_input)).await.is_ok() {
        format!("uploads/{}", video_path_input)
    } else {
        return format!("❌ Error: Video file not found: {}. Tried both '{}' and 'uploads/{}'", video_path_input, video_path_input, video_path_input);
    };

    // Retrieve video analysis from Qdrant
    match crate::services::VideoVectorizationService::retrieve_video_analysis(&video_path, &ctx.app_state).await {
        Ok(analysis) => {
            // Format the analysis for LLM consumption
            let summary = analysis.get("video_summary").and_then(|v| v.as_str()).unwrap_or("No summary");
            let duration = analysis.get("duration_seconds").and_then(|v| v.as_f64()).unwrap_or(0.0);
            let frame_count = analysis.get("frame_count").and_then(|v| v.as_u64()).unwrap_or(0);

            let mut result = format!("📹 **Video Analysis: {}**\n\n", video_path);
            result.push_str(&format!("**Duration:** {:.1}s\n", duration));
            result.push_str(&format!("**Frames Analyzed:** {}\n\n", frame_count));
            result.push_str(&format!("**Summary:**\n{}\n\n", summary));

            // Add frame details
            if let Some(frames) = analysis.get("frames").and_then(|v| v.as_array()) {
                result.push_str("**Frame-by-Frame Analysis:**\n");
                for (i, frame) in frames.iter().take(10).enumerate() {
                    let frame_num = frame.get("frame_number").and_then(|v| v.as_u64()).unwrap_or(i as u64);
                    let timestamp = frame.get("timestamp").and_then(|v| v.as_f64()).unwrap_or(0.0);
                    let desc = frame.get("description").and_then(|v| v.as_str()).unwrap_or("");

                    result.push_str(&format!("Frame {} ({:.1}s): {}\n", frame_num, timestamp, desc));
                }
                if frames.len() > 10 {
                    result.push_str(&format!("\n... and {} more frames\n", frames.len() - 10));
                }
            }

            result
        }
        Err(e) => {
            format!("❌ Failed to retrieve video analysis: {}. Note: Video may not be vectorized yet. Try re-analyzing or waiting for vectorization to complete.", e)
        }
    }
}

/// View video placeholder - calls context version
async fn execute_view_video_gemini(args: &HashMap<String, Value>) -> String {
    format!("❌ Internal error: view_video must be called with context")
}

/// Review video against original requirements - WITH AppState (Claude version)
async fn execute_review_video_with_state_claude(args: &Value, ctx: &ToolExecutionContext) -> String {
    let video_path_input = args["video_path"].as_str().unwrap_or("");
    let original_request = args["original_request"].as_str().unwrap_or("");
    let expected_features = args.get("expected_features").and_then(|v| v.as_array())
        .map(|arr| arr.iter().filter_map(|v| v.as_str()).collect::<Vec<_>>())
        .unwrap_or_default();

    if video_path_input.is_empty() || original_request.is_empty() {
        return "❌ Error: video_path and original_request are required".to_string();
    }

    // Resolve file path - try as-is first, then try uploads/, outputs/ directories
    let video_path = if tokio::fs::metadata(video_path_input).await.is_ok() {
        video_path_input.to_string()
    } else if tokio::fs::metadata(format!("uploads/{}", video_path_input)).await.is_ok() {
        format!("uploads/{}", video_path_input)
    } else if tokio::fs::metadata(format!("outputs/{}", video_path_input)).await.is_ok() {
        format!("outputs/{}", video_path_input)
    } else {
        return format!("❌ Error: Video file not found: {}. Tried 'uploads/', 'outputs/', and as-is", video_path_input);
    };

    // Check if file exists and is valid before attempting vectorization check
    if let Err(_) = tokio::fs::metadata(&video_path).await {
        return format!("❌ Error: Video file does not exist: {}", video_path);
    }

    // Retry logic for vectorization with exponential backoff
    let app_state = ctx.app_state.clone();
    let video_path_clone = video_path.clone();

    let analysis = retry_with_exponential_backoff(
        || {
            let path = video_path_clone.clone();
            let state = app_state.clone();
            async move {
                crate::services::VideoVectorizationService::retrieve_video_analysis(&path, &state).await
            }
        },
        5,  // Max 5 retries
        2000,  // Start with 2 second delay (2s, 4s, 8s, 16s, 32s)
    )
    .await;

    let analysis = match analysis {
        Ok(data) => data,
        Err(e) => {
            return format!(
                "❌ Failed to retrieve video analysis after multiple retries: {}.\n\n\
                 💡 Possible reasons:\n\
                 1. Video is still being vectorized (usually takes 5-15 seconds)\n\
                 2. Video file is corrupted or invalid\n\
                 3. Qdrant vector database is unavailable\n\n\
                 Try waiting a bit longer and calling review_video again.",
                e
            );
        }
    };

    // Build comprehensive review
    let mut review = format!("🔍 **Video Quality Review**\n\n");
    review.push_str(&format!("**Video:** {}\n", video_path));
    review.push_str(&format!("**Original Request:** {}\n\n", original_request));

    // Video summary
    let summary = analysis.get("video_summary").and_then(|v| v.as_str()).unwrap_or("No summary");
    review.push_str(&format!("**What's in the video:**\n{}\n\n", summary));

    // Check expected features
    let mut features_found = 0;
    let total_features = expected_features.len();

    if !expected_features.is_empty() {
        review.push_str("**Expected Features Check:**\n");
        for feature in &expected_features {
            // Check if feature is mentioned in summary or frame descriptions
            let feature_lower = feature.to_lowercase();
            let summary_lower = summary.to_lowercase();

            let found = summary_lower.contains(&feature_lower) ||
                analysis.get("frames").and_then(|v| v.as_array()).map(|frames| {
                    frames.iter().any(|f| {
                        f.get("description").and_then(|d| d.as_str())
                            .map(|desc| desc.to_lowercase().contains(&feature_lower))
                            .unwrap_or(false)
                    })
                }).unwrap_or(false);

            if found {
                features_found += 1;
            }

            let status = if found { "✅" } else { "⚠️" };
            review.push_str(&format!("  {} {}\n", status, feature));
        }
        review.push_str("\n");
    }

    // Technical verification
    let duration = analysis.get("duration_seconds").and_then(|v| v.as_f64()).unwrap_or(0.0);
    let frame_count = analysis.get("frame_count").and_then(|v| v.as_u64()).unwrap_or(0);

    review.push_str("**Technical Details:**\n");
    review.push_str(&format!("  • Duration: {:.1}s\n", duration));
    review.push_str(&format!("  • Frames analyzed: {}\n", frame_count));
    review.push_str(&format!("  • Vectorization: Complete ✅\n\n"));

    // Calculate pass/fail
    let all_features_found = expected_features.is_empty() || features_found == total_features;

    review.push_str("**Review Result:**\n");
    if all_features_found {
        review.push_str(&format!("✅ **PASS** - All requirements met ({}/{})\n", features_found, total_features));
        review.push_str("This video is ready to present to the user.\n");
    } else {
        review.push_str(&format!("⚠️ **FAIL** - Missing requirements ({}/{} found)\n", features_found, total_features));
        review.push_str("**Recommended Action:** Re-edit the video to include missing features or explain to user what cannot be achieved.\n");
    }

    review
}

/// Review video placeholder - calls context version
async fn execute_review_video_claude(args: &Value) -> String {
    format!("❌ Internal error: review_video must be called with context")
}

/// Review video against original requirements - WITH AppState (Gemini version)
async fn execute_review_video_with_state_gemini(args: &HashMap<String, Value>, ctx: &ToolExecutionContext) -> String {
    let video_path_input = args.get("video_path").and_then(|v| v.as_str()).unwrap_or("");
    let original_request = args.get("original_request").and_then(|v| v.as_str()).unwrap_or("");
    let expected_features = args.get("expected_features").and_then(|v| v.as_array())
        .map(|arr| arr.iter().filter_map(|v| v.as_str()).collect::<Vec<_>>())
        .unwrap_or_default();

    if video_path_input.is_empty() || original_request.is_empty() {
        return "❌ Error: video_path and original_request are required".to_string();
    }

    // Resolve file path - try as-is first, then try uploads/, outputs/ directories
    let video_path = if tokio::fs::metadata(video_path_input).await.is_ok() {
        video_path_input.to_string()
    } else if tokio::fs::metadata(format!("uploads/{}", video_path_input)).await.is_ok() {
        format!("uploads/{}", video_path_input)
    } else if tokio::fs::metadata(format!("outputs/{}", video_path_input)).await.is_ok() {
        format!("outputs/{}", video_path_input)
    } else {
        return format!("❌ Error: Video file not found: {}. Tried 'uploads/', 'outputs/', and as-is", video_path_input);
    };

    // Check if file exists and is valid
    if let Err(_) = tokio::fs::metadata(&video_path).await {
        return format!("❌ Error: Video file does not exist: {}", video_path);
    }

    // Retry logic with exponential backoff
    let app_state = ctx.app_state.clone();
    let video_path_clone = video_path.clone();

    let analysis = retry_with_exponential_backoff(
        || {
            let path = video_path_clone.clone();
            let state = app_state.clone();
            async move {
                crate::services::VideoVectorizationService::retrieve_video_analysis(&path, &state).await
            }
        },
        5,
        2000,
    )
    .await;

    let analysis = match analysis {
        Ok(data) => data,
        Err(e) => {
            return format!(
                "❌ Failed to retrieve video analysis after multiple retries: {}.\n\n\
                 💡 Possible reasons:\n\
                 1. Video is still being vectorized (usually takes 5-15 seconds)\n\
                 2. Video file is corrupted or invalid\n\
                 3. Qdrant vector database is unavailable\n\n\
                 Try waiting a bit longer and calling review_video again.",
                e
            );
        }
    };

    // Build comprehensive review
    let mut review = format!("🔍 **Video Quality Review**\n\n");
    review.push_str(&format!("**Video:** {}\n", video_path));
    review.push_str(&format!("**Original Request:** {}\n\n", original_request));

    // Video summary
    let summary = analysis.get("video_summary").and_then(|v| v.as_str()).unwrap_or("No summary");
    review.push_str(&format!("**What's in the video:**\n{}\n\n", summary));

    // Check expected features
    let mut features_found = 0;
    let total_features = expected_features.len();

    if !expected_features.is_empty() {
        review.push_str("**Expected Features Check:**\n");
        for feature in &expected_features {
            // Check if feature is mentioned in summary or frame descriptions
            let feature_lower = feature.to_lowercase();
            let summary_lower = summary.to_lowercase();

            let found = summary_lower.contains(&feature_lower) ||
                analysis.get("frames").and_then(|v| v.as_array()).map(|frames| {
                    frames.iter().any(|f| {
                        f.get("description").and_then(|d| d.as_str())
                            .map(|desc| desc.to_lowercase().contains(&feature_lower))
                            .unwrap_or(false)
                    })
                }).unwrap_or(false);

            if found {
                features_found += 1;
            }

            let status = if found { "✅" } else { "⚠️" };
            review.push_str(&format!("  {} {}\n", status, feature));
        }
        review.push_str("\n");
    }

    // Technical verification
    let duration = analysis.get("duration_seconds").and_then(|v| v.as_f64()).unwrap_or(0.0);
    let frame_count = analysis.get("frame_count").and_then(|v| v.as_u64()).unwrap_or(0);

    review.push_str("**Technical Details:**\n");
    review.push_str(&format!("  • Duration: {:.1}s\n", duration));
    review.push_str(&format!("  • Frames analyzed: {}\n", frame_count));
    review.push_str(&format!("  • Vectorization: Complete ✅\n\n"));

    // Calculate pass/fail
    let all_features_found = expected_features.is_empty() || features_found == total_features;

    review.push_str("**Review Result:**\n");
    if all_features_found {
        review.push_str(&format!("✅ **PASS** - All requirements met ({}/{})\n", features_found, total_features));
        review.push_str("This video is ready to present to the user.\n");
    } else {
        review.push_str(&format!("⚠️ **FAIL** - Missing requirements ({}/{} found)\n", features_found, total_features));
        review.push_str("**Recommended Action:** Re-edit the video to include missing features or explain to user what cannot be achieved.\n");
    }

    review
}

/// Review video placeholder - calls context version
async fn execute_review_video_gemini(args: &HashMap<String, Value>) -> String {
    format!("❌ Internal error: review_video must be called with context")
}

// ============================================================================
// IMAGE VIEWING TOOLS
// ============================================================================

/// View image placeholder - calls context version
async fn execute_view_image_claude(args: &Value) -> String {
    format!("❌ Internal error: view_image must be called with context")
}

/// View image placeholder - calls context version
async fn execute_view_image_gemini(args: &HashMap<String, Value>) -> String {
    format!("❌ Internal error: view_image must be called with context")
}

/// View/analyze an image using Gemini's vision capabilities - WITH AppState (Claude version)
async fn execute_view_image_with_state_claude(args: &Value, ctx: &ToolExecutionContext) -> String {
    let image_path_input = args["image_path"].as_str().unwrap_or("");

    if image_path_input.is_empty() {
        return "❌ Error: image_path is required".to_string();
    }

    // Resolve file path - try as-is first, then try outputs/ directory
    let image_path = if tokio::fs::metadata(image_path_input).await.is_ok() {
        image_path_input.to_string()
    } else if tokio::fs::metadata(format!("outputs/{}", image_path_input)).await.is_ok() {
        format!("outputs/{}", image_path_input)
    } else {
        return format!("❌ Error: Image file not found: {}. Tried both '{}' and 'outputs/{}'", image_path_input, image_path_input, image_path_input);
    };

    // Read image file
    let image_bytes = match tokio::fs::read(&image_path).await {
        Ok(bytes) => bytes,
        Err(e) => return format!("❌ Failed to read image file: {}", e),
    };

    // Use Gemini to analyze the image
    if let Some(ref gemini_client) = ctx.app_state.gemini_client {
        match gemini_client.analyze_image_bytes(&image_bytes, "Analyze this image in detail. Describe what you see, colors, composition, style, text if any, and whether it would work well as a video overlay or background.").await {
            Ok(analysis) => {
                format!("🖼️ **Image Analysis: {}**\n\n{}", image_path, analysis)
            }
            Err(e) => format!("❌ Failed to analyze image: {}", e),
        }
    } else {
        "❌ Gemini client not available for image analysis".to_string()
    }
}

/// View/analyze an image using Gemini's vision capabilities - WITH AppState (Gemini version)
async fn execute_view_image_with_state_gemini(args: &HashMap<String, Value>, ctx: &ToolExecutionContext) -> String {
    let image_path_input = args.get("image_path").and_then(|v| v.as_str()).unwrap_or("");

    if image_path_input.is_empty() {
        return "❌ Error: image_path is required".to_string();
    }

    // Resolve file path - try as-is first, then try outputs/ directory
    let image_path = if tokio::fs::metadata(image_path_input).await.is_ok() {
        image_path_input.to_string()
    } else if tokio::fs::metadata(format!("outputs/{}", image_path_input)).await.is_ok() {
        format!("outputs/{}", image_path_input)
    } else {
        return format!("❌ Error: Image file not found: {}. Tried both '{}' and 'outputs/{}'", image_path_input, image_path_input, image_path_input);
    };

    // Read image file
    let image_bytes = match tokio::fs::read(&image_path).await {
        Ok(bytes) => bytes,
        Err(e) => return format!("❌ Failed to read image file: {}", e),
    };

    // Use Gemini to analyze the image
    if let Some(ref gemini_client) = ctx.app_state.gemini_client {
        match gemini_client.analyze_image_bytes(&image_bytes, "Analyze this image in detail. Describe what you see, colors, composition, style, text if any, and whether it would work well as a video overlay or background.").await {
            Ok(analysis) => {
                format!("🖼️ **Image Analysis: {}**\n\n{}", image_path, analysis)
            }
            Err(e) => format!("❌ Failed to analyze image: {}", e),
        }
    } else {
        "❌ Gemini client not available for image analysis".to_string()
    }
}

// ============================================================================
// ELEVEN LABS AUDIO GENERATION TOOLS
// ============================================================================

/// Placeholder functions for tools that need context
async fn execute_generate_text_to_speech_placeholder_claude(_args: &Value) -> String {
    "❌ Internal error: generate_text_to_speech must be called with context".to_string()
}

async fn execute_generate_text_to_speech_placeholder_gemini(_args: &HashMap<String, Value>) -> String {
    "❌ Internal error: generate_text_to_speech must be called with context".to_string()
}

async fn execute_generate_sound_effect_placeholder_claude(_args: &Value) -> String {
    "❌ Internal error: generate_sound_effect must be called with context".to_string()
}

async fn execute_generate_sound_effect_placeholder_gemini(_args: &HashMap<String, Value>) -> String {
    "❌ Internal error: generate_sound_effect must be called with context".to_string()
}

async fn execute_generate_music_placeholder_claude(_args: &Value) -> String {
    "❌ Internal error: generate_music must be called with context".to_string()
}

async fn execute_generate_music_placeholder_gemini(_args: &HashMap<String, Value>) -> String {
    "❌ Internal error: generate_music must be called with context".to_string()
}

async fn execute_add_voiceover_placeholder_claude(_args: &Value) -> String {
    "❌ Internal error: add_voiceover_to_video must be called with context".to_string()
}

async fn execute_add_voiceover_placeholder_gemini(_args: &HashMap<String, Value>) -> String {
    "❌ Internal error: add_voiceover_to_video must be called with context".to_string()
}

/// Generate text-to-speech using Eleven Labs (Claude version)
async fn execute_generate_text_to_speech_with_state_claude(args: &Value, ctx: &ToolExecutionContext) -> String {
    let text = args["text"].as_str().unwrap_or("");
    let output_file = args["output_file"].as_str().unwrap_or("");
    let voice = args.get("voice").and_then(|v| v.as_str()).unwrap_or("Rachel");
    let model = args.get("model").and_then(|v| v.as_str());

    if text.is_empty() || output_file.is_empty() {
        return "❌ Error: text and output_file are required".to_string();
    }

    // Try Eleven Labs first if available
    if let Some(ref elevenlabs_client) = ctx.app_state.elevenlabs_client {
        let voice_id = crate::elevenlabs_client::DefaultVoices::get_voice_id_by_name(voice)
            .unwrap_or(crate::elevenlabs_client::DefaultVoices::RACHEL);

        let model_id = model.or(Some("eleven_flash_v2_5"));

        match elevenlabs_client.text_to_speech(text, voice_id, model_id, None, Some("mp3_44100_128")).await {
            Ok(audio_bytes) => {
                match tokio::fs::write(&output_file, &audio_bytes).await {
                    Ok(_) => return format!("✅ Generated speech using Eleven Labs ({}) and saved to: {}", voice, output_file),
                    Err(e) => return format!("❌ Failed to save audio file: {}", e),
                }
            }
            Err(e) => {
                tracing::warn!("Eleven Labs TTS failed, falling back to Gemini: {}", e);
            }
        }
    }

    // Fallback to Gemini TTS
    execute_generate_text_to_speech_claude(args).await
}

/// Generate text-to-speech using Eleven Labs (Gemini version)
async fn execute_generate_text_to_speech_with_state_gemini(args: &HashMap<String, Value>, ctx: &ToolExecutionContext) -> String {
    let text = args.get("text").and_then(|v| v.as_str()).unwrap_or("");
    let output_file_raw = args.get("output_file").and_then(|v| v.as_str()).unwrap_or("");
    let output_file = ensure_outputs_directory(output_file_raw);
    let voice = args.get("voice").and_then(|v| v.as_str()).unwrap_or("Rachel");
    let model = args.get("model").and_then(|v| v.as_str());

    if text.is_empty() || output_file.is_empty() {
        return "❌ Error: text and output_file are required".to_string();
    }

    // Try Eleven Labs first if available
    if let Some(ref elevenlabs_client) = ctx.app_state.elevenlabs_client {
        let voice_id = crate::elevenlabs_client::DefaultVoices::get_voice_id_by_name(voice)
            .unwrap_or(crate::elevenlabs_client::DefaultVoices::RACHEL);

        let model_id = model.or(Some("eleven_flash_v2_5"));

        match elevenlabs_client.text_to_speech(text, voice_id, model_id, None, Some("mp3_44100_128")).await {
            Ok(audio_bytes) => {
                match tokio::fs::write(&output_file, &audio_bytes).await {
                    Ok(_) => return format!("✅ Generated speech using Eleven Labs ({}) and saved to: {}", voice, output_file),
                    Err(e) => return format!("❌ Failed to save audio file: {}", e),
                }
            }
            Err(e) => {
                tracing::warn!("Eleven Labs TTS failed, falling back to Gemini: {}", e);
            }
        }
    }

    // Fallback to Gemini TTS
    execute_generate_text_to_speech_gemini(args).await
}

/// Generate sound effect using Eleven Labs (Claude version)
async fn execute_generate_sound_effect_with_state_claude(args: &Value, ctx: &ToolExecutionContext) -> String {
    let description = args["description"].as_str().unwrap_or("");
    let output_file_raw = args["output_file"].as_str().unwrap_or("");
    let output_file = ensure_outputs_directory(output_file_raw);
    let duration = args.get("duration_seconds").and_then(|v| v.as_f64());
    let prompt_influence = args.get("prompt_influence").and_then(|v| v.as_f64());

    if description.is_empty() || output_file.is_empty() {
        return "❌ Error: description and output_file are required".to_string();
    }

    if let Some(ref elevenlabs_client) = ctx.app_state.elevenlabs_client {
        match elevenlabs_client.generate_sound_effect(description, duration, prompt_influence).await {
            Ok(audio_bytes) => {
                match tokio::fs::write(&output_file, &audio_bytes).await {
                    Ok(_) => format!("✅ Generated sound effect using Eleven Labs and saved to: {}", output_file),
                    Err(e) => format!("❌ Failed to save sound effect: {}", e),
                }
            }
            Err(e) => format!("❌ Failed to generate sound effect: {}", e),
        }
    } else {
        "❌ Eleven Labs client not available. Set ELEVEN_LABS_API_KEY to enable sound effects.".to_string()
    }
}

/// Generate sound effect using Eleven Labs (Gemini version)
async fn execute_generate_sound_effect_with_state_gemini(args: &HashMap<String, Value>, ctx: &ToolExecutionContext) -> String {
    let description = args.get("description").and_then(|v| v.as_str()).unwrap_or("");
    let output_file_raw = args.get("output_file").and_then(|v| v.as_str()).unwrap_or("");
    let output_file = ensure_outputs_directory(output_file_raw);
    let duration = args.get("duration_seconds").and_then(|v| v.as_f64());
    let prompt_influence = args.get("prompt_influence").and_then(|v| v.as_f64());

    if description.is_empty() || output_file.is_empty() {
        return "❌ Error: description and output_file are required".to_string();
    }

    if let Some(ref elevenlabs_client) = ctx.app_state.elevenlabs_client {
        match elevenlabs_client.generate_sound_effect(description, duration, prompt_influence).await {
            Ok(audio_bytes) => {
                match tokio::fs::write(&output_file, &audio_bytes).await {
                    Ok(_) => format!("✅ Generated sound effect using Eleven Labs and saved to: {}", output_file),
                    Err(e) => format!("❌ Failed to save sound effect: {}", e),
                }
            }
            Err(e) => format!("❌ Failed to generate sound effect: {}", e),
        }
    } else {
        "❌ Eleven Labs client not available. Set ELEVEN_LABS_API_KEY to enable sound effects.".to_string()
    }
}

/// Generate music using Eleven Labs Eleven Music (Claude version)
async fn execute_generate_music_with_state_claude(args: &Value, ctx: &ToolExecutionContext) -> String {
    let prompt = args["prompt"].as_str().unwrap_or("");
    let output_file = args["output_file"].as_str().unwrap_or("");
    let duration_seconds = args.get("duration_seconds").and_then(|v| v.as_f64()).unwrap_or(30.0);

    if prompt.is_empty() || output_file.is_empty() {
        return "❌ Error: prompt and output_file are required".to_string();
    }

    let duration_ms = (duration_seconds * 1000.0) as u32;
    if duration_ms < 10000 || duration_ms > 300000 {
        return "❌ Error: duration_seconds must be between 10 and 300 seconds".to_string();
    }

    if let Some(ref elevenlabs_client) = ctx.app_state.elevenlabs_client {
        // Step 1: Create music generation task
        let generation_id = match elevenlabs_client.generate_music_task(prompt, duration_ms).await {
            Ok(id) => id,
            Err(e) => return format!("❌ Failed to start music generation: {}", e),
        };

        // Step 2: Poll for completion (wait up to 2 minutes)
        let max_attempts = 60; // 60 attempts x 2 seconds = 2 minutes
        for attempt in 0..max_attempts {
            tokio::time::sleep(tokio::time::Duration::from_secs(2)).await;

            match elevenlabs_client.get_music_status(&generation_id).await {
                Ok(status) => {
                    match status.status.as_str() {
                        "completed" => {
                            if let Some(audio_url) = status.audio_url {
                                // Download the audio
                                match elevenlabs_client.download_music(&audio_url).await {
                                    Ok(audio_bytes) => {
                                        match tokio::fs::write(&output_file, &audio_bytes).await {
                                            Ok(_) => return format!("✅ Generated music using Eleven Music and saved to: {} (took {}s)", output_file, attempt * 2),
                                            Err(e) => return format!("❌ Failed to save music file: {}", e),
                                        }
                                    }
                                    Err(e) => return format!("❌ Failed to download music: {}", e),
                                }
                            } else {
                                return "❌ Music generation completed but no audio URL provided".to_string();
                            }
                        }
                        "failed" => {
                            let error_msg = status.error.unwrap_or_else(|| "Unknown error".to_string());
                            return format!("❌ Music generation failed: {}", error_msg);
                        }
                        _ => {
                            // Still pending, continue polling
                            tracing::debug!("Music generation in progress... (attempt {}/{})", attempt + 1, max_attempts);
                        }
                    }
                }
                Err(e) => {
                    tracing::warn!("Failed to check music status: {}", e);
                }
            }
        }

        "❌ Music generation timed out after 2 minutes".to_string()
    } else {
        "❌ Eleven Labs client not available. Set ELEVEN_LABS_API_KEY to enable music generation.".to_string()
    }
}

/// Generate music using Eleven Labs Eleven Music (Gemini version)
async fn execute_generate_music_with_state_gemini(args: &HashMap<String, Value>, ctx: &ToolExecutionContext) -> String {
    let prompt = args.get("prompt").and_then(|v| v.as_str()).unwrap_or("");
    let output_file_raw = args.get("output_file").and_then(|v| v.as_str()).unwrap_or("");
    let output_file = ensure_outputs_directory(output_file_raw);
    let duration_seconds = args.get("duration_seconds").and_then(|v| v.as_f64()).unwrap_or(30.0);

    if prompt.is_empty() || output_file.is_empty() {
        return "❌ Error: prompt and output_file are required".to_string();
    }

    let duration_ms = (duration_seconds * 1000.0) as u32;
    if duration_ms < 10000 || duration_ms > 300000 {
        return "❌ Error: duration_seconds must be between 10 and 300 seconds".to_string();
    }

    if let Some(ref elevenlabs_client) = ctx.app_state.elevenlabs_client {
        // Step 1: Create music generation task
        let generation_id = match elevenlabs_client.generate_music_task(prompt, duration_ms).await {
            Ok(id) => id,
            Err(e) => return format!("❌ Failed to start music generation: {}", e),
        };

        // Step 2: Poll for completion
        let max_attempts = 60;
        for attempt in 0..max_attempts {
            tokio::time::sleep(tokio::time::Duration::from_secs(2)).await;

            match elevenlabs_client.get_music_status(&generation_id).await {
                Ok(status) => {
                    match status.status.as_str() {
                        "completed" => {
                            if let Some(audio_url) = status.audio_url {
                                match elevenlabs_client.download_music(&audio_url).await {
                                    Ok(audio_bytes) => {
                                        match tokio::fs::write(&output_file, &audio_bytes).await {
                                            Ok(_) => return format!("✅ Generated music using Eleven Music and saved to: {} (took {}s)", output_file, attempt * 2),
                                            Err(e) => return format!("❌ Failed to save music file: {}", e),
                                        }
                                    }
                                    Err(e) => return format!("❌ Failed to download music: {}", e),
                                }
                            } else {
                                return "❌ Music generation completed but no audio URL provided".to_string();
                            }
                        }
                        "failed" => {
                            let error_msg = status.error.unwrap_or_else(|| "Unknown error".to_string());
                            return format!("❌ Music generation failed: {}", error_msg);
                        }
                        _ => {
                            tracing::debug!("Music generation in progress... (attempt {}/{})", attempt + 1, max_attempts);
                        }
                    }
                }
                Err(e) => {
                    tracing::warn!("Failed to check music status: {}", e);
                }
            }
        }

        "❌ Music generation timed out after 2 minutes".to_string()
    } else {
        "❌ Eleven Labs client not available. Set ELEVEN_LABS_API_KEY to enable music generation.".to_string()
    }
}

/// Convenience tool: Add voiceover to video in one step (Claude version)
async fn execute_add_voiceover_to_video_with_state_claude(args: &Value, ctx: &ToolExecutionContext) -> String {
    let input_video = args["input_video"].as_str().unwrap_or("");
    let voiceover_text = args["voiceover_text"].as_str().unwrap_or("");
    let output_video = args["output_video"].as_str().unwrap_or("");
    let voice = args.get("voice").and_then(|v| v.as_str()).unwrap_or("Rachel");

    if input_video.is_empty() || voiceover_text.is_empty() || output_video.is_empty() {
        return "❌ Error: input_video, voiceover_text, and output_video are required".to_string();
    }

    // Step 1: Generate voiceover audio
    let temp_audio = format!("outputs/temp_voiceover_{}.mp3", uuid::Uuid::new_v4());

    let tts_args = serde_json::json!({
        "text": voiceover_text,
        "output_file": &temp_audio,
        "voice": voice,
    });

    let tts_result = execute_generate_text_to_speech_with_state_claude(&tts_args, ctx).await;
    if tts_result.starts_with("❌") {
        return format!("❌ Failed to generate voiceover: {}", tts_result);
    }

    // Step 2: Add audio to video using FFmpeg
    let add_audio_args = serde_json::json!({
        "input_file": input_video,
        "audio_file": &temp_audio,
        "output_file": output_video,
    });

    let result = execute_add_audio_claude(&add_audio_args);

    // Clean up temp audio file
    let _ = tokio::fs::remove_file(&temp_audio).await;

    if result.starts_with("❌") {
        format!("❌ Failed to add voiceover to video: {}", result)
    } else {
        format!("✅ Successfully added voiceover ({}) to video and saved to: {}", voice, output_video)
    }
}

/// Convenience tool: Add voiceover to video in one step (Gemini version)
async fn execute_add_voiceover_to_video_with_state_gemini(args: &HashMap<String, Value>, ctx: &ToolExecutionContext) -> String {
    let input_video = args.get("input_video").and_then(|v| v.as_str()).unwrap_or("");
    let voiceover_text = args.get("voiceover_text").and_then(|v| v.as_str()).unwrap_or("");
    let output_video = args.get("output_video").and_then(|v| v.as_str()).unwrap_or("");
    let voice = args.get("voice").and_then(|v| v.as_str()).unwrap_or("Rachel");

    if input_video.is_empty() || voiceover_text.is_empty() || output_video.is_empty() {
        return "❌ Error: input_video, voiceover_text, and output_video are required".to_string();
    }

    // Step 1: Generate voiceover audio
    let temp_audio = format!("outputs/temp_voiceover_{}.mp3", uuid::Uuid::new_v4());

    let mut tts_args = HashMap::new();
    tts_args.insert("text".to_string(), Value::String(voiceover_text.to_string()));
    tts_args.insert("output_file".to_string(), Value::String(temp_audio.clone()));
    tts_args.insert("voice".to_string(), Value::String(voice.to_string()));

    let tts_result = execute_generate_text_to_speech_with_state_gemini(&tts_args, ctx).await;
    if tts_result.starts_with("❌") {
        return format!("❌ Failed to generate voiceover: {}", tts_result);
    }

    // Step 2: Add audio to video using FFmpeg
    let mut add_audio_args = HashMap::new();
    add_audio_args.insert("input_file".to_string(), Value::String(input_video.to_string()));
    add_audio_args.insert("audio_file".to_string(), Value::String(temp_audio.clone()));
    add_audio_args.insert("output_file".to_string(), Value::String(output_video.to_string()));

    let result = execute_add_audio_gemini(&add_audio_args);

    // Clean up temp audio file
    let _ = tokio::fs::remove_file(&temp_audio).await;

    if result.starts_with("❌") {
        format!("❌ Failed to add voiceover to video: {}", result)
    } else {
        format!("✅ Successfully added voiceover ({}) to video and saved to: {}", voice, output_video)
    }
}

// ============================================================================
// CHAT TITLE MANAGEMENT TOOLS
// ============================================================================

/// Set a descriptive title for the current chat session (Claude version)
async fn execute_set_chat_title_with_state_claude(args: &Value, ctx: &ToolExecutionContext) -> String {
    let title = args["title"].as_str().unwrap_or("");

    if title.is_empty() {
        return "❌ Error: title is required".to_string();
    }

    if title.len() > 100 {
        return "❌ Error: title must be 100 characters or less".to_string();
    }

    // Update chat session title in database
    let session_id = &ctx.session_id;
    let pool = &ctx.app_state.db_pool;

    let result: Result<(), sqlx::Error> = sqlx::query(
        "UPDATE chat_sessions SET title = $1, updated_at = NOW() WHERE session_uuid = $2"
    )
    .bind(title)
    .bind(session_id)
    .execute(pool)
    .await
    .map(|_| ());

    match result {
        Ok(_) => {
            tracing::info!("✏️ Updated chat title to: {}", title);
            format!("✅ Chat title updated to: \"{}\"", title)
        }
        Err(e) => {
            tracing::error!("Failed to update chat title: {}", e);
            format!("❌ Failed to update chat title: {}", e)
        }
    }
}

/// Set a descriptive title for the current chat session (Gemini version)
async fn execute_set_chat_title_with_state_gemini(args: &HashMap<String, Value>, ctx: &ToolExecutionContext) -> String {
    let title = args.get("title").and_then(|v| v.as_str()).unwrap_or("");

    if title.is_empty() {
        return "❌ Error: title is required".to_string();
    }

    if title.len() > 100 {
        return "❌ Error: title must be 100 characters or less".to_string();
    }

    // Update chat session title in database
    let session_id = &ctx.session_id;
    let pool = &ctx.app_state.db_pool;

    let result: Result<(), sqlx::Error> = sqlx::query(
        "UPDATE chat_sessions SET title = $1, updated_at = NOW() WHERE session_uuid = $2"
    )
    .bind(title)
    .bind(session_id)
    .execute(pool)
    .await
    .map(|_| ());

    match result {
        Ok(_) => {
            tracing::info!("✏️ Updated chat title to: {}", title);
            format!("✅ Chat title updated to: \"{}\"", title)
        }
        Err(e) => {
            tracing::error!("Failed to update chat title: {}", e);
            format!("❌ Failed to update chat title: {}", e)
        }
    }
}

// ============================================================================
// YOUTUBE INTEGRATION TOOL EXECUTORS (READ-ONLY RESEARCH TOOLS - PHASE 1)
// ============================================================================

/// Optimize YouTube metadata using AI
async fn execute_optimize_youtube_metadata_with_state_claude(
    args: &Value,
    ctx: &ToolExecutionContext,
) -> String {
    let video_path = args["video_path"].as_str().unwrap_or("");
    let audience = args.get("target_audience").and_then(|v| v.as_str()).unwrap_or("general");
    let style = args.get("style").and_then(|v| v.as_str()).unwrap_or("professional");

    if video_path.is_empty() || !std::path::Path::new(video_path).exists() {
        return format!("❌ Video not found: {}", video_path);
    }

    tracing::info!("🎯 Optimizing YouTube metadata: {}", video_path);

    let info = match crate::core::analyze_video(video_path) {
        Ok(i) => i,
        Err(e) => return format!("❌ Analysis failed: {}", e),
    };

    let resolution = format!("{}x{}", info.width, info.height);
    let duration_min = (info.duration_seconds / 60.0) as i32;

    let prompt = format!(
        "Generate YouTube SEO metadata:\nDuration: {}s ({}min), Resolution: {}\nAudience: {}, Style: {}\n\nProvide: TITLE, DESCRIPTION, TAGS",
        info.duration_seconds as i32, duration_min, resolution, audience, style
    );

    let metadata = if let Some(claude) = ctx.app_state.claude_client.as_ref() {
        claude.generate_text(&prompt).await.unwrap_or_else(|_| "❌ AI generation failed".to_string())
    } else {
        // For Gemini, create a simple GenerateContentRequest
        if let Some(gemini) = ctx.app_state.gemini_client.as_ref() {
            let request = crate::gemini_client::GenerateContentRequest {
                contents: vec![crate::gemini_client::Content {
                    role: Some("user".to_string()),
                    parts: vec![crate::gemini_client::Part::Text { text: prompt.clone() }],
                }],
                tools: None,
                generation_config: None,
                tool_config: None,
            };

            match gemini.generate_content(request).await {
                Ok(response) => {
                    response.candidates.first()
                        .and_then(|c| c.content.as_ref())
                        .and_then(|content| content.parts.first())
                        .and_then(|p| match p {
                            crate::gemini_client::Part::Text { text } => Some(text.clone()),
                            _ => None,
                        })
                        .unwrap_or_else(|| "❌ AI generation failed".to_string())
                }
                Err(e) => format!("❌ Gemini failed: {}", e),
            }
        } else {
            return "❌ No AI client available".to_string();
        }
    };

    format!("✅ YouTube Metadata Optimization\n\n📹 Video: {}\n🎯 Audience: {}\n🎨 Style: {}\n\n{}", video_path, audience, style, metadata)
}

async fn execute_optimize_youtube_metadata_with_state_gemini(
    args: &HashMap<String, Value>,
    ctx: &ToolExecutionContext,
) -> String {
    execute_optimize_youtube_metadata_with_state_claude(&serde_json::to_value(args).unwrap_or_default(), ctx).await
}

/// Analyze YouTube performance
async fn execute_analyze_youtube_performance_with_state_claude(
    args: &Value,
    ctx: &ToolExecutionContext,
) -> String {
    let video_id = args["video_id"].as_str().unwrap_or("");
    let days = args.get("date_range_days").and_then(|v| v.as_i64()).unwrap_or(30).min(365) as i32;

    if video_id.is_empty() {
        return "❌ video_id required".to_string();
    }

    "🚧 Feature coming soon - analytics integration in progress".to_string()
}

async fn execute_analyze_youtube_performance_with_state_gemini(
    args: &HashMap<String, Value>,
    ctx: &ToolExecutionContext,
) -> String {
    execute_analyze_youtube_performance_with_state_claude(&serde_json::to_value(args).unwrap_or_default(), ctx).await
}

/// Suggest content ideas
async fn execute_suggest_content_ideas_with_state_claude(
    args: &Value,
    ctx: &ToolExecutionContext,
) -> String {
    "🚧 Feature coming soon - content strategy integration in progress".to_string()
}

async fn execute_suggest_content_ideas_with_state_gemini(
    args: &HashMap<String, Value>,
    ctx: &ToolExecutionContext,
) -> String {
    execute_suggest_content_ideas_with_state_claude(&serde_json::to_value(args).unwrap_or_default(), ctx).await
}

/// Search YouTube trends
async fn execute_search_youtube_trends_with_state_claude(
    args: &Value,
    ctx: &ToolExecutionContext,
) -> String {
    let query = args.get("query").and_then(|v| v.as_str());
    let region = args.get("region_code").and_then(|v| v.as_str()).unwrap_or("US");
    let max = args.get("max_results").and_then(|v| v.as_i64()).unwrap_or(10).min(50) as i32;

    let youtube = match ctx.app_state.youtube_client.as_ref() {
        Some(c) => c,
        None => return "❌ YouTube unavailable".to_string(),
    };

    let results = if let Some(q) = query {
        youtube.search_videos(None, q, max, Some("viewCount")).await
            .map(|r| r.items.iter().map(|v| format!("🎬 {}", v.snippet.title)).collect::<Vec<_>>().join("\n"))
            .unwrap_or_else(|e| format!("❌ {}", e))
    } else {
        youtube.get_trending_videos(Some(region), None, max).await
            .map(|r| r.items.iter().map(|v| format!("🔥 {} ({})", v.snippet.title, v.statistics.view_count)).collect::<Vec<_>>().join("\n"))
            .unwrap_or_else(|e| format!("❌ {}", e))
    };

    format!("✅ Trends ({})\n\n{}", region, results)
}

async fn execute_search_youtube_trends_with_state_gemini(
    args: &HashMap<String, Value>,
    ctx: &ToolExecutionContext,
) -> String {
    execute_search_youtube_trends_with_state_claude(&serde_json::to_value(args).unwrap_or_default(), ctx).await
}

/// Search for YouTube channels
async fn execute_search_youtube_channels_with_state_claude(
    args: &Value,
    ctx: &ToolExecutionContext,
) -> String {
    let query = args["query"].as_str().unwrap_or("");
    let max_results = args.get("max_results").and_then(|v| v.as_i64()).unwrap_or(10).min(50) as i32;
    let order = args.get("order").and_then(|v| v.as_str());

    if query.is_empty() {
        return "❌ Error: query is required".to_string();
    }

    tracing::info!("🔍 Searching YouTube channels: {}", query);

    let youtube = match ctx.app_state.youtube_client.as_ref() {
        Some(c) => c,
        None => return "❌ YouTube client not available".to_string(),
    };

    match youtube.search_channels(None, query, max_results, order).await {
        Ok(response) => {
            let channels: Vec<String> = response.items.iter().map(|item| {
                format!(
                    "📺 {}\n   Channel ID: {}\n   Description: {}\n   Created: {}",
                    item.snippet.title,
                    item.snippet.channel_id,
                    if item.snippet.description.len() > 100 {
                        format!("{}...", &item.snippet.description[..100])
                    } else {
                        item.snippet.description.clone()
                    },
                    item.snippet.published_at
                )
            }).collect();

            if channels.is_empty() {
                format!("No channels found for: {}", query)
            } else {
                format!(
                    "✅ YouTube Channel Search Results for '{}'\n\nFound {} channels:\n\n{}",
                    query,
                    channels.len(),
                    channels.join("\n\n")
                )
            }
        }
        Err(e) => format!("❌ Channel search failed: {}", e),
    }
}

async fn execute_search_youtube_channels_with_state_gemini(
    args: &HashMap<String, Value>,
    ctx: &ToolExecutionContext,
) -> String {
    execute_search_youtube_channels_with_state_claude(&serde_json::to_value(args).unwrap_or_default(), ctx).await
}
