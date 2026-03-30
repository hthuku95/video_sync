// Comprehensive tool executor for all 35+ video editing tools
// Maps tool names to actual video processing function calls

use serde_json::{json, Value};
use std::collections::HashMap;
use tokio::fs::File;
use tokio::io::AsyncWriteExt;
use std::sync::Arc;
use crate::AppState;
use base64::prelude::BASE64_STANDARD;
use base64::Engine;
use std::time::Duration;
use std::process::Command as StdCommand;

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
        if attempt > 0 {
            tracing::info!("🔄 Retry attempt {}/{} (waiting {}ms)", attempt + 1, max_retries, delay);
        }
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

    // auto_generate_video needs ctx for BlenderMCPClient (video_source param)
    if name == "auto_generate_video" {
        return execute_auto_generate_video_with_state_claude(args, ctx).await;
    }

    // BlenderMCPServer tools (all need AppState for the client)
    if name == "blender_generate_scene" {
        return execute_blender_generate_scene_claude(args, ctx).await;
    }
    if name == "blender_generate_thumbnail" {
        return execute_blender_generate_thumbnail_claude(args, ctx).await;
    }
    if name == "blender_generate_title_card" {
        return execute_blender_generate_title_card_claude(args, ctx).await;
    }
    if name == "blender_generate_data_viz" {
        return execute_blender_generate_data_viz_claude(args, ctx).await;
    }
    if name == "blender_generate_lower_third" {
        return execute_blender_generate_lower_third_claude(args, ctx).await;
    }
    if name == "blender_generate_latex" {
        return execute_blender_generate_latex_claude(args, ctx).await;
    }
    if name == "blender_generate_ui_mockup" {
        return execute_blender_generate_ui_mockup_claude(args, ctx).await;
    }
    if name == "blender_generate_animation" {
        return execute_blender_generate_animation_claude(args, ctx).await;
    }
    if name == "blender_generate_chart" {
        return execute_blender_generate_chart_claude(args, ctx).await;
    }
    if name == "blender_generate_flowchart" {
        return execute_blender_simple_manim_claude("blender_generate_flowchart", args, ctx).await;
    }
    if name == "blender_generate_3d_math" {
        return execute_blender_simple_manim_claude("blender_generate_3d_math", args, ctx).await;
    }
    if name == "blender_generate_code_animation" {
        return execute_blender_simple_manim_claude("blender_generate_code_animation", args, ctx).await;
    }
    if name == "blender_generate_timeline" {
        return execute_blender_simple_manim_claude("blender_generate_timeline", args, ctx).await;
    }
    if name == "blender_generate_network_graph" {
        return execute_blender_simple_manim_claude("blender_generate_network_graph", args, ctx).await;
    }
    if name == "blender_generate_logo_reveal" {
        return execute_blender_simple_manim_claude("blender_generate_logo_reveal", args, ctx).await;
    }
    if name == "blender_generate_abstract_bg" {
        return execute_blender_simple_manim_claude("blender_generate_abstract_bg", args, ctx).await;
    }
    if name == "blender_generate_countdown" {
        return execute_blender_simple_manim_claude("blender_generate_countdown", args, ctx).await;
    }
    if name == "blender_generate_text_animation" {
        return execute_blender_simple_manim_claude("blender_generate_text_animation", args, ctx).await;
    }
    if name == "blender_generate_vector_field" {
        return execute_blender_simple_manim_claude("blender_generate_vector_field", args, ctx).await;
    }
    if name == "blender_generate_matrix_transform" {
        return execute_blender_simple_manim_claude("blender_generate_matrix_transform", args, ctx).await;
    }
    if name == "blender_generate_polar_graph" {
        return execute_blender_simple_manim_claude("blender_generate_polar_graph", args, ctx).await;
    }
    if name == "blender_generate_geometry_proof" {
        return execute_blender_simple_manim_claude("blender_generate_geometry_proof", args, ctx).await;
    }

    // Execute the tool first
    let result = execute_tool_claude(name, args).await;

    // Auto-vectorize downloaded stock videos from Pexels
    if name == "pexels_download_video" && !result.starts_with("❌") {
        if let Some(_output_path) = extract_output_path_from_args(args) {
            // Background vectorization temporarily disabled due to lifetime constraints
            // Vectorization will happen on-demand when video is accessed
            tracing::debug!("Stock video download successful, vectorization deferred");
        }
    }

    // If tool succeeded and created an output file, save it to DB
    if !result.starts_with("❌") && !result.starts_with("Error") {
        if let Some(_output_path) = extract_output_path_from_args(args) {
            // Background vectorization temporarily disabled due to lifetime constraints
            // Output video saving and vectorization will happen on-demand
            tracing::debug!("Tool execution successful, background processing deferred");
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

    // Blender MCP tools — 3D rendering, Manim, thumbnails, data viz
    if name == "blender_generate_scene" {
        return execute_blender_generate_scene_gemini(args, ctx).await;
    }
    if name == "blender_generate_thumbnail" {
        return execute_blender_generate_thumbnail_gemini(args, ctx).await;
    }
    if name == "blender_generate_title_card" {
        return execute_blender_generate_title_card_gemini(args, ctx).await;
    }
    if name == "blender_generate_data_viz" {
        return execute_blender_generate_data_viz_gemini(args, ctx).await;
    }
    if name == "blender_generate_lower_third" {
        return execute_blender_generate_lower_third_gemini(args, ctx).await;
    }
    if name == "blender_generate_latex" {
        return execute_blender_generate_latex_gemini(args, ctx).await;
    }
    if name == "blender_generate_ui_mockup" {
        return execute_blender_generate_ui_mockup_gemini(args, ctx).await;
    }
    if name == "blender_generate_animation" {
        return execute_blender_generate_animation_gemini(args, ctx).await;
    }
    if name == "blender_generate_chart" {
        return execute_blender_generate_chart_gemini(args, ctx).await;
    }
    for tool_name in &[
        "blender_generate_flowchart",
        "blender_generate_3d_math",
        "blender_generate_code_animation",
        "blender_generate_timeline",
        "blender_generate_network_graph",
        "blender_generate_logo_reveal",
        "blender_generate_abstract_bg",
        "blender_generate_countdown",
        "blender_generate_text_animation",
        "blender_generate_vector_field",
        "blender_generate_matrix_transform",
        "blender_generate_polar_graph",
        "blender_generate_geometry_proof",
    ] {
        if name == *tool_name {
            return execute_blender_passthrough_gemini(name, args, ctx).await;
        }
    }

    // auto_generate_video needs ctx for BlenderMCPClient (video_source param)
    if name == "auto_generate_video" {
        return execute_auto_generate_video_with_state_gemini(args, ctx).await;
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
        if let Some(_output_path) = extract_output_path_from_gemini_args(args) {
            // Background vectorization temporarily disabled due to lifetime constraints
            // Vectorization will happen on-demand when video is accessed
            tracing::debug!("Stock video download successful, vectorization deferred");
        }
    }

    // If tool succeeded and created an output file, save it to DB
    if !result.starts_with("❌") && !result.starts_with("Error") {
        if let Some(_output_path) = extract_output_path_from_gemini_args(args) {
            // Background vectorization temporarily disabled due to lifetime constraints
            // Output video saving and vectorization will happen on-demand
            tracing::debug!("Tool execution successful, background processing deferred");
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
        "add_transition" => execute_add_transition_claude(args),
        "add_animated_text" => execute_add_animated_text_claude(args),
        "apply_filter_chain" => execute_apply_filter_chain_claude(args),
        "apply_audio_effect" => execute_apply_audio_effect_claude(args),
        "deinterlace_video" => execute_deinterlace_video_claude(args),
        "export_custom_quality" => execute_export_custom_quality_claude(args),

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
        "edit_image" => execute_edit_image_claude(args).await,
        "auto_generate_video" => execute_auto_generate_video_claude(args).await,
        "generate_video_queries" => execute_generate_video_queries_claude(args),
        "analyze_pexels_thumbnail" => execute_analyze_pexels_thumbnail_claude(args).await,
        "verify_clip_quality_tool" => execute_verify_clip_quality_tool_claude(args),
        "run_video_qa" => execute_run_video_qa_claude(args),
        "view_video" => execute_view_video_claude(args).await,
        "review_video" => execute_review_video_claude(args).await,
        "view_image" => execute_view_image_claude(args).await,

        // Control tools
        "submit_final_answer" => execute_submit_final_answer_claude(args),

        // New tools — Batch 1
        "create_thumbnail_hd" => execute_create_thumbnail_hd_claude(args),
        "get_video_duration" => execute_get_video_duration_claude(args),

        // New tools — Batch 2 Color Grading
        "adjust_hue" => execute_adjust_hue_claude(args),
        "color_balance" => execute_color_balance_claude(args),
        "normalize_video" => execute_normalize_video_claude(args),
        "apply_lut" => execute_apply_lut_claude(args),

        // New tools — Batch 3 Denoising & Sharpening
        "denoise_video" => execute_denoise_video_claude(args),
        "unsharp_mask" => execute_unsharp_mask_claude(args),
        "reduce_noise" => execute_reduce_noise_claude(args),

        // New tools — Batch 4 Audio Processing
        "compress_audio" => execute_compress_audio_claude(args),
        "normalize_audio" => execute_normalize_audio_claude(args),
        "equalize_audio" => execute_equalize_audio_claude(args),
        "gate_audio" => execute_gate_audio_claude(args),
        "denoise_audio" => execute_denoise_audio_claude(args),

        // New tools — Batch 5 Composition & Layout
        "pad_video" => execute_pad_video_claude(args),
        "blend_videos" => execute_blend_videos_claude(args),
        "stack_videos" => execute_stack_videos_claude(args),
        "add_vignette" => execute_add_vignette_claude(args),
        "draw_box" => execute_draw_box_claude(args),

        // New tools — Batch 6 Motion, Time & Frame
        "reverse_video" => execute_reverse_video_claude(args),
        "loop_video" => execute_loop_video_claude(args),
        "zoompan" => execute_zoompan_claude(args),
        "minterpolate" => execute_minterpolate_claude(args),

        // New tools — Batch 7 Media Analysis
        "detect_scene_changes" => execute_detect_scene_changes_claude(args),
        "measure_loudness" => execute_measure_loudness_claude(args),
        "detect_silence" => execute_detect_silence_claude(args),

        // New tools — Batch 8 Advanced Color Grading
        "adjust_curves" => execute_adjust_curves_claude(args),
        "adjust_levels" => execute_adjust_levels_claude(args),
        "split_tone" => execute_split_tone_claude(args),
        "convert_colorspace" => execute_convert_colorspace_claude(args),
        "apply_tonemap" => execute_apply_tonemap_claude(args),

        // New tools — Batch 9 Audio Tone Shaping
        "filter_highpass" => execute_filter_highpass_claude(args),
        "filter_lowpass" => execute_filter_lowpass_claude(args),
        "adjust_bass" => execute_adjust_bass_claude(args),
        "adjust_treble" => execute_adjust_treble_claude(args),
        "audio_compand" => execute_audio_compand_claude(args),
        "add_audio_delay" => execute_add_audio_delay_claude(args),
        "add_phaser" => execute_add_phaser_claude(args),

        // New tools — Batch 10 Audio Restoration
        "remove_clicks" => execute_remove_clicks_claude(args),
        "restore_clipping" => execute_restore_clipping_claude(args),
        "remove_silence" => execute_remove_silence_claude(args),

        // New tools — Batch 11 Quality Metrics
        "compare_ssim" => execute_compare_ssim_claude(args),
        "compare_psnr" => execute_compare_psnr_claude(args),
        "analyze_audio_stats" => execute_analyze_audio_stats_claude(args),
        "analyze_video_signal" => execute_analyze_video_signal_claude(args),

        // New tools — Batch 12 Geometric Transforms
        "correct_perspective" => execute_correct_perspective_claude(args),
        "correct_lens" => execute_correct_lens_claude(args),
        "apply_shear" => execute_apply_shear_claude(args),

        // New tools — Batch 13 Temporal Frame Effects
        "blend_frames" => execute_blend_frames_claude(args),
        "temporal_median" => execute_temporal_median_claude(args),
        "convert_framerate" => execute_convert_framerate_claude(args),
        "tile_frames" => execute_tile_frames_claude(args),

        // New tools — Batch 14 Spatial Audio
        "adjust_stereo_width" => execute_adjust_stereo_width_claude(args),
        "apply_stereo_widen" => execute_apply_stereo_widen_claude(args),
        "mix_audio_channels" => execute_mix_audio_channels_claude(args),

        // Phase D — Professional Finishing
        "adjust_color_temperature" => execute_adjust_color_temperature_claude(args),
        "adjust_vibrance" => execute_adjust_vibrance_claude(args),
        "remove_flicker" => execute_remove_flicker_claude(args),
        "denoise_video_bm3d" => execute_denoise_video_bm3d_claude(args),
        "deshake_video" => execute_deshake_video_claude(args),
        "measure_lufs" => execute_measure_lufs_claude(args),
        "parametric_eq" => execute_parametric_eq_claude(args),
        "audio_limiter" => execute_audio_limiter_claude(args),
        "reduce_sibilance" => execute_reduce_sibilance_claude(args),
        "denoise_speech_rnn" => execute_denoise_speech_rnn_claude(args),

        // Phase E — Vectorscope, Waveform, Grid, LumaKey, Binaural, Modulation
        "analyze_vectorscope" => execute_analyze_vectorscope_claude(args),
        "analyze_waveform" => execute_analyze_waveform_claude(args),
        "draw_grid" => execute_draw_grid_claude(args),
        "grid_stack_videos" => execute_grid_stack_videos_claude(args),
        "luma_key" => execute_luma_key_claude(args),
        "render_binaural" => execute_render_binaural_claude(args),
        "add_vibrato" => execute_add_vibrato_claude(args),
        "add_tremolo" => execute_add_tremolo_claude(args),
        "add_flanger" => execute_add_flanger_claude(args),
        "denoise_audio_nlm" => execute_denoise_audio_nlm_claude(args),

        // Phase F — Niche/Specialised
        "displace_video" => execute_displace_video_claude(args),
        "decimate_frames" => execute_decimate_frames_claude(args),
        "denoise_video_owden" => execute_denoise_video_owden_claude(args),
        "despill_video" => execute_despill_video_claude(args),
        "remap_pixels" => execute_remap_pixels_claude(args),
        "adjust_exposure" => execute_adjust_exposure_claude(args),
        "measure_vmaf" => execute_measure_vmaf_claude(args),
        "shift_audio_frequency" => execute_shift_audio_frequency_claude(args),
        "apply_audio_pulsator" => execute_apply_audio_pulsator_claude(args),
        "enhance_dialogue" => execute_enhance_dialogue_claude(args),
        "split_audio_channels" => execute_split_audio_channels_claude(args),
        "map_audio_channels" => execute_map_audio_channels_claude(args),
        "merge_audio_inputs" => execute_merge_audio_inputs_claude(args),
        "apply_crossfeed" => execute_apply_crossfeed_claude(args),
        "apply_extrastereo" => execute_apply_extrastereo_claude(args),
        "apply_firequalizer" => execute_apply_firequalizer_claude(args),
        "apply_biquad" => execute_apply_biquad_claude(args),
        "filter_bandpass" => execute_filter_bandpass_claude(args),
        "filter_bandreject" => execute_filter_bandreject_claude(args),
        "boost_sub_bass" => execute_boost_sub_bass_claude(args),
        "detect_objects_dnn" => execute_detect_objects_dnn_claude(args),
        "classify_frames_dnn" => execute_classify_frames_dnn_claude(args),
        "upscale_super_resolution" => execute_upscale_super_resolution_claude(args),
        "remove_rain_ai" => execute_remove_rain_ai_claude(args),
        "detect_frozen_frames" => execute_detect_frozen_frames_claude(args),
        "apply_edgedetect" => execute_apply_edgedetect_claude(args),
        "zoom_pan" => execute_zoom_pan_claude(args),
        "chromatic_aberration" => execute_chromatic_aberration_claude(args),
        "temporal_blend" => execute_temporal_blend_claude(args),
        "motion_interpolate" => execute_motion_interpolate_claude(args),
        "correct_lens_simple" => execute_correct_lens_simple_claude(args),
        "deinterlace_yadif" => execute_deinterlace_yadif_claude(args),
        "correct_perspective_linear" => execute_correct_perspective_linear_claude(args),
        "colorize_video" => execute_colorize_video_claude(args),
        "denoise_hqdn3d" => execute_denoise_hqdn3d_claude(args),
        "add_echo" => execute_add_echo_claude(args),
        "noise_gate" => execute_noise_gate_claude(args),
        "compress_dynamics" => execute_compress_dynamics_claude(args),
        "add_chorus" => execute_add_chorus_claude(args),
        "widen_stereo" => execute_widen_stereo_claude(args),
        "normalize_speech" => execute_normalize_speech_claude(args),
        "remove_silence_simple" => execute_remove_silence_simple_claude(args),
        "soft_clip_audio" => execute_soft_clip_audio_claude(args),
        "segment_video" => execute_segment_video_claude(args),
        "pad_video_time" => execute_pad_video_time_claude(args),
        "encode_vp9" => execute_encode_vp9_claude(args),
        "encode_av1" => execute_encode_av1_claude(args),
        "encode_hevc" => execute_encode_hevc_claude(args),
        "encode_opus" => execute_encode_opus_claude(args),
        "encode_hdr10" => execute_encode_hdr10_claude(args),
        "encode_nvenc" => execute_encode_nvenc_claude(args),
        "encode_vaapi" => execute_encode_vaapi_claude(args),
        "encode_qsv" => execute_encode_qsv_claude(args),
        "encode_prores" => execute_encode_prores_claude(args),
        "encode_dnxhd" => execute_encode_dnxhd_claude(args),
        "encode_gif" => execute_encode_gif_claude(args),
        "encode_webm" => execute_encode_webm_claude(args),
        "stabilize_video_2pass" => execute_stabilize_video_2pass_claude(args),
        "apply_lut_rgb" => execute_apply_lut_rgb_claude(args),
        "apply_hsvhold" => execute_apply_hsvhold_claude(args),
        "convert_pixel_format" => execute_convert_pixel_format_claude(args),
        "apply_setsar" => execute_apply_setsar_claude(args),
        "apply_random_frames" => execute_apply_random_frames_claude(args),
        "visualize_cqt" => execute_visualize_cqt_claude(args),
        "visualize_frequencies" => execute_visualize_frequencies_claude(args),
        "apply_audio_iir" => execute_apply_audio_iir_claude(args),
        "apply_audio_expression" => execute_apply_audio_expression_claude(args),
        "convert_audio_format" => execute_convert_audio_format_claude(args),
        "apply_cross_correlate" => execute_apply_cross_correlate_claude(args),
        "apply_audio_multiply" => execute_apply_audio_multiply_claude(args),
        "apply_audio_contrast" => execute_apply_audio_contrast_claude(args),
        "decode_hdcd" => execute_decode_hdcd_claude(args),
        "scale_to_reference" => execute_scale_to_reference_claude(args),
        "apply_fieldorder" => execute_apply_fieldorder_claude(args),
        "optimize_gif_palette" => execute_optimize_gif_palette_claude(args),
        "apply_hsv_key" => execute_apply_hsv_key_claude(args),
        "apply_lut_yuv" => execute_apply_lut_yuv_claude(args),
        "apply_freezeframes" => execute_apply_freezeframes_claude(args),
        "draw_signal_graph" => execute_draw_signal_graph_claude(args),
        "measure_video_entropy" => execute_measure_video_entropy_claude(args),
        "apply_compensation_delay" => execute_apply_compensation_delay_claude(args),
        "apply_earwax" => execute_apply_earwax_claude(args),
        "apply_allpass_filter" => execute_apply_allpass_filter_claude(args),
        "apply_highshelf" => execute_apply_highshelf_claude(args),
        "apply_lowshelf" => execute_apply_lowshelf_claude(args),
        "apply_surround_upmix" => execute_apply_surround_upmix_claude(args),
        "detect_volume_levels" => execute_detect_volume_levels_claude(args),
        "extract_alpha_channel" => execute_extract_alpha_channel_claude(args),
        "merge_alpha_channel" => execute_merge_alpha_channel_claude(args),
        "apply_framestep" => execute_apply_framestep_claude(args),
        "apply_swaprect" => execute_apply_swaprect_claude(args),
        "apply_fillborders" => execute_apply_fillborders_claude(args),
        "apply_chromanr" => execute_apply_chromanr_claude(args),
        "apply_weave" => execute_apply_weave_claude(args),
        "apply_interlace" => execute_apply_interlace_claude(args),
        "denoise_audio_fft" => execute_denoise_audio_fft_claude(args),
        "loop_audio" => execute_loop_audio_claude(args),
        "apply_dc_shift" => execute_apply_dc_shift_claude(args),
        "measure_dynamic_range" => execute_measure_dynamic_range_claude(args),
        "apply_single_eq_band" => execute_apply_single_eq_band_claude(args),
        "apply_stereotools" => execute_apply_stereotools_claude(args),
        "apply_asetrate" => execute_apply_asetrate_claude(args),
        "apply_xfade_transition" => execute_apply_xfade_transition_claude(args),
        "apply_color_key" => execute_apply_color_key_claude(args),
        "apply_monochrome" => execute_apply_monochrome_claude(args),
        "apply_maskedmerge" => execute_apply_maskedmerge_claude(args),
        "convert_360_video" => execute_convert_360_video_claude(args),
        "fix_banding" => execute_fix_banding_claude(args),
        "apply_greyedge" => execute_apply_greyedge_claude(args),
        "apply_fade_video" => execute_apply_fade_video_claude(args),
        "normalize_loudness" => execute_normalize_loudness_claude(args),
        "dynamic_audio_normalize" => execute_dynamic_audio_normalize_claude(args),
        "resample_audio" => execute_resample_audio_claude(args),
        "trim_audio" => execute_trim_audio_claude(args),
        "apply_crystalizer" => execute_apply_crystalizer_claude(args),
        "multiband_compress" => execute_multiband_compress_claude(args),
        "apply_super_equalizer" => execute_apply_super_equalizer_claude(args),
        "apply_colormatrix" => execute_apply_colormatrix_claude(args),
        "apply_chromashift" => execute_apply_chromashift_claude(args),
        "apply_cas" => execute_apply_cas_claude(args),
        "apply_nlmeans_video" => execute_apply_nlmeans_video_claude(args),
        "apply_spp" => execute_apply_spp_claude(args),
        "apply_pp" => execute_apply_pp_claude(args),
        "apply_mestimate" => execute_apply_mestimate_claude(args),
        "apply_midequalizer" => execute_apply_midequalizer_claude(args),
        "apply_median_spatial" => execute_apply_median_spatial_claude(args),
        "apply_acrusher" => execute_apply_acrusher_claude(args),
        "apply_atempo" => execute_apply_atempo_claude(args),
        "apply_asetnsamples" => execute_apply_asetnsamples_claude(args),
        "apply_apad" => execute_apply_apad_claude(args),
        "apply_asubcut" => execute_apply_asubcut_claude(args),
        "apply_asupercut" => execute_apply_asupercut_claude(args),
        "apply_threshold" => execute_apply_threshold_claude(args),
        "apply_maskedclamp" => execute_apply_maskedclamp_claude(args),
        "apply_roberts" => execute_apply_roberts_claude(args),
        "apply_sobel" => execute_apply_sobel_claude(args),
        "apply_prewitt" => execute_apply_prewitt_claude(args),
        "apply_kirsch" => execute_apply_kirsch_claude(args),
        "apply_video_limiter" => execute_apply_video_limiter_claude(args),
        "apply_bilateral" => execute_apply_bilateral_claude(args),
        "apply_unsharp_mask" => execute_apply_unsharp_mask_claude(args),
        "apply_lagfun" => execute_apply_lagfun_claude(args),
        "apply_tinterlace" => execute_apply_tinterlace_claude(args),
        "apply_datascope" => execute_apply_datascope_claude(args),
        "apply_fspp" => execute_apply_fspp_claude(args),
        "apply_haas" => execute_apply_haas_claude(args),
        "apply_aemphasis" => execute_apply_aemphasis_claude(args),
        "apply_negate" => execute_apply_negate_claude(args),
        "apply_pixelize" => execute_apply_pixelize_claude(args),
        "apply_colorlevels" => execute_apply_colorlevels_claude(args),
        "apply_pseudocolor" => execute_apply_pseudocolor_claude(args),
        "apply_colorhold" => execute_apply_colorhold_claude(args),
        "apply_shuffleplanes" => execute_apply_shuffleplanes_claude(args),
        "detect_black_frames" => execute_detect_black_frames_claude(args),
        "detect_interlace_type" => execute_detect_interlace_type_claude(args),
        "apply_vstack" => execute_apply_vstack_claude(args),
        "apply_hstack" => execute_apply_hstack_claude(args),
        "apply_setdar" => execute_apply_setdar_claude(args),
        "apply_stereo3d" => execute_apply_stereo3d_claude(args),
        "apply_telecine" => execute_apply_telecine_claude(args),
        "apply_pullup" => execute_apply_pullup_claude(args),
        "select_thumbnail_frame" => execute_select_thumbnail_frame_claude(args),
        "apply_gaussian_blur" => execute_apply_gaussian_blur_claude(args),
        "apply_box_blur" => execute_apply_box_blur_claude(args),
        "apply_smart_blur" => execute_apply_smart_blur_claude(args),
        "add_film_grain" => execute_add_film_grain_claude(args),
        "apply_rotate_angle" => execute_apply_rotate_angle_claude(args),
        "apply_geq" => execute_apply_geq_claude(args),
        "apply_colorchannelmixer" => execute_apply_colorchannelmixer_claude(args),
        "apply_atadenoise" => execute_apply_atadenoise_claude(args),
        "apply_vaguedenoiser" => execute_apply_vaguedenoiser_claude(args),
        "apply_fftdnoiz" => execute_apply_fftdnoiz_claude(args),
        "generate_waveform_video" => execute_generate_waveform_video_claude(args),
        "apply_lut3d" => execute_apply_lut3d_claude(args),
        "measure_siti" => execute_measure_siti_claude(args),
        "create_test_pattern" => execute_create_test_pattern_claude(args),
        "apply_amplify" => execute_apply_amplify_claude(args),
        "select_frames" => execute_select_frames_claude(args),
        "posterize_video" => execute_posterize_video_claude(args),
        "solarize_video" => execute_solarize_video_claude(args),
        "apply_dilation" => execute_apply_dilation_claude(args),
        "apply_erosion" => execute_apply_erosion_claude(args),
        "apply_median_filter" => execute_apply_median_filter_claude(args),
        "apply_histogram_eq" => execute_apply_histogram_eq_claude(args),
        "apply_clahe" => execute_apply_clahe_claude(args),
        "apply_deblock" => execute_apply_deblock_claude(args),
        "adjust_hue_saturation" => execute_adjust_hue_saturation_claude(args),
        "apply_convolution" => execute_apply_convolution_claude(args),
        "reverse_audio" => execute_reverse_audio_claude(args),
        "blend_audio_streams" => execute_blend_audio_streams_claude(args),
        "measure_silence" => execute_measure_silence_claude(args),
        "measure_audio_spectrum" => execute_measure_audio_spectrum_claude(args),

        // ── Workflow Recipes ──────────────────────────────────────────────
        "youtube_ready_export" => execute_youtube_ready_export_claude(args),
        "podcast_cleanup" => execute_podcast_cleanup_claude(args),
        "cinematic_grade" => execute_cinematic_grade_claude(args),
        "create_gif_workflow" => execute_create_gif_workflow_claude(args),
        "talking_head_cleanup" => execute_talking_head_cleanup_claude(args),

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
        "add_transition" => execute_add_transition_gemini(args),
        "add_animated_text" => execute_add_animated_text_gemini(args),
        "apply_filter_chain" => execute_apply_filter_chain_gemini(args),
        "apply_audio_effect" => execute_apply_audio_effect_gemini(args),
        "deinterlace_video" => execute_deinterlace_video_gemini(args),
        "export_custom_quality" => execute_export_custom_quality_gemini(args),

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
        "edit_image" => execute_edit_image_gemini(args).await,
        "auto_generate_video" => execute_auto_generate_video_gemini(args).await,
        "generate_video_queries" => execute_generate_video_queries_gemini(args),
        "analyze_pexels_thumbnail" => execute_analyze_pexels_thumbnail_gemini(args).await,
        "verify_clip_quality_tool" => execute_verify_clip_quality_tool_gemini(args),
        "run_video_qa" => execute_run_video_qa_gemini(args),
        "view_video" => execute_view_video_gemini(args).await,
        "review_video" => execute_review_video_gemini(args).await,
        "view_image" => execute_view_image_gemini(args).await,

        // Control tools
        "submit_final_answer" => execute_submit_final_answer_gemini(args),

        // New tools — Batch 1
        "create_thumbnail_hd" => execute_create_thumbnail_hd_gemini(args),
        "get_video_duration" => execute_get_video_duration_gemini(args),

        // New tools — Batch 2 Color Grading
        "adjust_hue" => execute_adjust_hue_gemini(args),
        "color_balance" => execute_color_balance_gemini(args),
        "normalize_video" => execute_normalize_video_gemini(args),
        "apply_lut" => execute_apply_lut_gemini(args),

        // New tools — Batch 3 Denoising & Sharpening
        "denoise_video" => execute_denoise_video_gemini(args),
        "unsharp_mask" => execute_unsharp_mask_gemini(args),
        "reduce_noise" => execute_reduce_noise_gemini(args),

        // New tools — Batch 4 Audio Processing
        "compress_audio" => execute_compress_audio_gemini(args),
        "normalize_audio" => execute_normalize_audio_gemini(args),
        "equalize_audio" => execute_equalize_audio_gemini(args),
        "gate_audio" => execute_gate_audio_gemini(args),
        "denoise_audio" => execute_denoise_audio_gemini(args),

        // New tools — Batch 5 Composition & Layout
        "pad_video" => execute_pad_video_gemini(args),
        "blend_videos" => execute_blend_videos_gemini(args),
        "stack_videos" => execute_stack_videos_gemini(args),
        "add_vignette" => execute_add_vignette_gemini(args),
        "draw_box" => execute_draw_box_gemini(args),

        // New tools — Batch 6 Motion, Time & Frame
        "reverse_video" => execute_reverse_video_gemini(args),
        "loop_video" => execute_loop_video_gemini(args),
        "zoompan" => execute_zoompan_gemini(args),
        "minterpolate" => execute_minterpolate_gemini(args),

        // New tools — Batch 7 Media Analysis
        "detect_scene_changes" => execute_detect_scene_changes_gemini(args),
        "measure_loudness" => execute_measure_loudness_gemini(args),
        "detect_silence" => execute_detect_silence_gemini(args),

        // New tools — Batch 8 Advanced Color Grading
        "adjust_curves" => execute_adjust_curves_gemini(args),
        "adjust_levels" => execute_adjust_levels_gemini(args),
        "split_tone" => execute_split_tone_gemini(args),
        "convert_colorspace" => execute_convert_colorspace_gemini(args),
        "apply_tonemap" => execute_apply_tonemap_gemini(args),

        // New tools — Batch 9 Audio Tone Shaping
        "filter_highpass" => execute_filter_highpass_gemini(args),
        "filter_lowpass" => execute_filter_lowpass_gemini(args),
        "adjust_bass" => execute_adjust_bass_gemini(args),
        "adjust_treble" => execute_adjust_treble_gemini(args),
        "audio_compand" => execute_audio_compand_gemini(args),
        "add_audio_delay" => execute_add_audio_delay_gemini(args),
        "add_phaser" => execute_add_phaser_gemini(args),

        // New tools — Batch 10 Audio Restoration
        "remove_clicks" => execute_remove_clicks_gemini(args),
        "restore_clipping" => execute_restore_clipping_gemini(args),
        "remove_silence" => execute_remove_silence_gemini(args),

        // New tools — Batch 11 Quality Metrics
        "compare_ssim" => execute_compare_ssim_gemini(args),
        "compare_psnr" => execute_compare_psnr_gemini(args),
        "analyze_audio_stats" => execute_analyze_audio_stats_gemini(args),
        "analyze_video_signal" => execute_analyze_video_signal_gemini(args),

        // New tools — Batch 12 Geometric Transforms
        "correct_perspective" => execute_correct_perspective_gemini(args),
        "correct_lens" => execute_correct_lens_gemini(args),
        "apply_shear" => execute_apply_shear_gemini(args),

        // New tools — Batch 13 Temporal Frame Effects
        "blend_frames" => execute_blend_frames_gemini(args),
        "temporal_median" => execute_temporal_median_gemini(args),
        "convert_framerate" => execute_convert_framerate_gemini(args),
        "tile_frames" => execute_tile_frames_gemini(args),

        // New tools — Batch 14 Spatial Audio
        "adjust_stereo_width" => execute_adjust_stereo_width_gemini(args),
        "apply_stereo_widen" => execute_apply_stereo_widen_gemini(args),
        "mix_audio_channels" => execute_mix_audio_channels_gemini(args),

        // Phase D — Professional Finishing
        "adjust_color_temperature" => execute_adjust_color_temperature_gemini(args),
        "adjust_vibrance" => execute_adjust_vibrance_gemini(args),
        "remove_flicker" => execute_remove_flicker_gemini(args),
        "denoise_video_bm3d" => execute_denoise_video_bm3d_gemini(args),
        "deshake_video" => execute_deshake_video_gemini(args),
        "measure_lufs" => execute_measure_lufs_gemini(args),
        "parametric_eq" => execute_parametric_eq_gemini(args),
        "audio_limiter" => execute_audio_limiter_gemini(args),
        "reduce_sibilance" => execute_reduce_sibilance_gemini(args),
        "denoise_speech_rnn" => execute_denoise_speech_rnn_gemini(args),

        // Phase E — Vectorscope, Waveform, Grid, LumaKey, Binaural, Modulation
        "analyze_vectorscope" => execute_analyze_vectorscope_gemini(args),
        "analyze_waveform" => execute_analyze_waveform_gemini(args),
        "draw_grid" => execute_draw_grid_gemini(args),
        "grid_stack_videos" => execute_grid_stack_videos_gemini(args),
        "luma_key" => execute_luma_key_gemini(args),
        "render_binaural" => execute_render_binaural_gemini(args),
        "add_vibrato" => execute_add_vibrato_gemini(args),
        "add_tremolo" => execute_add_tremolo_gemini(args),
        "add_flanger" => execute_add_flanger_gemini(args),
        "denoise_audio_nlm" => execute_denoise_audio_nlm_gemini(args),

        // Phase F — Niche/Specialised
        "displace_video" => execute_displace_video_gemini(args),
        "decimate_frames" => execute_decimate_frames_gemini(args),
        "denoise_video_owden" => execute_denoise_video_owden_gemini(args),
        "despill_video" => execute_despill_video_gemini(args),
        "remap_pixels" => execute_remap_pixels_gemini(args),
        "adjust_exposure" => execute_adjust_exposure_gemini(args),
        "measure_vmaf" => execute_measure_vmaf_gemini(args),
        "shift_audio_frequency" => execute_shift_audio_frequency_gemini(args),
        "apply_audio_pulsator" => execute_apply_audio_pulsator_gemini(args),
        "enhance_dialogue" => execute_enhance_dialogue_gemini(args),
        "split_audio_channels" => execute_split_audio_channels_gemini(args),
        "map_audio_channels" => execute_map_audio_channels_gemini(args),
        "merge_audio_inputs" => execute_merge_audio_inputs_gemini(args),
        "apply_crossfeed" => execute_apply_crossfeed_gemini(args),
        "apply_extrastereo" => execute_apply_extrastereo_gemini(args),
        "apply_firequalizer" => execute_apply_firequalizer_gemini(args),
        "apply_biquad" => execute_apply_biquad_gemini(args),
        "filter_bandpass" => execute_filter_bandpass_gemini(args),
        "filter_bandreject" => execute_filter_bandreject_gemini(args),
        "boost_sub_bass" => execute_boost_sub_bass_gemini(args),
        "detect_objects_dnn" => execute_detect_objects_dnn_gemini(args),
        "classify_frames_dnn" => execute_classify_frames_dnn_gemini(args),
        "upscale_super_resolution" => execute_upscale_super_resolution_gemini(args),
        "remove_rain_ai" => execute_remove_rain_ai_gemini(args),
        "detect_frozen_frames" => execute_detect_frozen_frames_gemini(args),
        "apply_edgedetect" => execute_apply_edgedetect_gemini(args),
        "zoom_pan" => execute_zoom_pan_gemini(args),
        "chromatic_aberration" => execute_chromatic_aberration_gemini(args),
        "temporal_blend" => execute_temporal_blend_gemini(args),
        "motion_interpolate" => execute_motion_interpolate_gemini(args),
        "correct_lens_simple" => execute_correct_lens_simple_gemini(args),
        "deinterlace_yadif" => execute_deinterlace_yadif_gemini(args),
        "correct_perspective_linear" => execute_correct_perspective_linear_gemini(args),
        "colorize_video" => execute_colorize_video_gemini(args),
        "denoise_hqdn3d" => execute_denoise_hqdn3d_gemini(args),
        "add_echo" => execute_add_echo_gemini(args),
        "noise_gate" => execute_noise_gate_gemini(args),
        "compress_dynamics" => execute_compress_dynamics_gemini(args),
        "add_chorus" => execute_add_chorus_gemini(args),
        "widen_stereo" => execute_widen_stereo_gemini(args),
        "normalize_speech" => execute_normalize_speech_gemini(args),
        "remove_silence_simple" => execute_remove_silence_simple_gemini(args),
        "soft_clip_audio" => execute_soft_clip_audio_gemini(args),
        "segment_video" => execute_segment_video_gemini(args),
        "pad_video_time" => execute_pad_video_time_gemini(args),
        "encode_vp9" => execute_encode_vp9_gemini(args),
        "encode_av1" => execute_encode_av1_gemini(args),
        "encode_hevc" => execute_encode_hevc_gemini(args),
        "encode_opus" => execute_encode_opus_gemini(args),
        "encode_hdr10" => execute_encode_hdr10_gemini(args),
        "encode_nvenc" => execute_encode_nvenc_gemini(args),
        "encode_vaapi" => execute_encode_vaapi_gemini(args),
        "encode_qsv" => execute_encode_qsv_gemini(args),
        "encode_prores" => execute_encode_prores_gemini(args),
        "encode_dnxhd" => execute_encode_dnxhd_gemini(args),
        "encode_gif" => execute_encode_gif_gemini(args),
        "encode_webm" => execute_encode_webm_gemini(args),
        "stabilize_video_2pass" => execute_stabilize_video_2pass_gemini(args),
        "apply_lut_rgb" => execute_apply_lut_rgb_gemini(args),
        "apply_hsvhold" => execute_apply_hsvhold_gemini(args),
        "convert_pixel_format" => execute_convert_pixel_format_gemini(args),
        "apply_setsar" => execute_apply_setsar_gemini(args),
        "apply_random_frames" => execute_apply_random_frames_gemini(args),
        "visualize_cqt" => execute_visualize_cqt_gemini(args),
        "visualize_frequencies" => execute_visualize_frequencies_gemini(args),
        "apply_audio_iir" => execute_apply_audio_iir_gemini(args),
        "apply_audio_expression" => execute_apply_audio_expression_gemini(args),
        "convert_audio_format" => execute_convert_audio_format_gemini(args),
        "apply_cross_correlate" => execute_apply_cross_correlate_gemini(args),
        "apply_audio_multiply" => execute_apply_audio_multiply_gemini(args),
        "apply_audio_contrast" => execute_apply_audio_contrast_gemini(args),
        "decode_hdcd" => execute_decode_hdcd_gemini(args),
        "scale_to_reference" => execute_scale_to_reference_gemini(args),
        "apply_fieldorder" => execute_apply_fieldorder_gemini(args),
        "optimize_gif_palette" => execute_optimize_gif_palette_gemini(args),
        "apply_hsv_key" => execute_apply_hsv_key_gemini(args),
        "apply_lut_yuv" => execute_apply_lut_yuv_gemini(args),
        "apply_freezeframes" => execute_apply_freezeframes_gemini(args),
        "draw_signal_graph" => execute_draw_signal_graph_gemini(args),
        "measure_video_entropy" => execute_measure_video_entropy_gemini(args),
        "apply_compensation_delay" => execute_apply_compensation_delay_gemini(args),
        "apply_earwax" => execute_apply_earwax_gemini(args),
        "apply_allpass_filter" => execute_apply_allpass_filter_gemini(args),
        "apply_highshelf" => execute_apply_highshelf_gemini(args),
        "apply_lowshelf" => execute_apply_lowshelf_gemini(args),
        "apply_surround_upmix" => execute_apply_surround_upmix_gemini(args),
        "detect_volume_levels" => execute_detect_volume_levels_gemini(args),
        "extract_alpha_channel" => execute_extract_alpha_channel_gemini(args),
        "merge_alpha_channel" => execute_merge_alpha_channel_gemini(args),
        "apply_framestep" => execute_apply_framestep_gemini(args),
        "apply_swaprect" => execute_apply_swaprect_gemini(args),
        "apply_fillborders" => execute_apply_fillborders_gemini(args),
        "apply_chromanr" => execute_apply_chromanr_gemini(args),
        "apply_weave" => execute_apply_weave_gemini(args),
        "apply_interlace" => execute_apply_interlace_gemini(args),
        "denoise_audio_fft" => execute_denoise_audio_fft_gemini(args),
        "loop_audio" => execute_loop_audio_gemini(args),
        "apply_dc_shift" => execute_apply_dc_shift_gemini(args),
        "measure_dynamic_range" => execute_measure_dynamic_range_gemini(args),
        "apply_single_eq_band" => execute_apply_single_eq_band_gemini(args),
        "apply_stereotools" => execute_apply_stereotools_gemini(args),
        "apply_asetrate" => execute_apply_asetrate_gemini(args),
        "apply_xfade_transition" => execute_apply_xfade_transition_gemini(args),
        "apply_color_key" => execute_apply_color_key_gemini(args),
        "apply_monochrome" => execute_apply_monochrome_gemini(args),
        "apply_maskedmerge" => execute_apply_maskedmerge_gemini(args),
        "convert_360_video" => execute_convert_360_video_gemini(args),
        "fix_banding" => execute_fix_banding_gemini(args),
        "apply_greyedge" => execute_apply_greyedge_gemini(args),
        "apply_fade_video" => execute_apply_fade_video_gemini(args),
        "normalize_loudness" => execute_normalize_loudness_gemini(args),
        "dynamic_audio_normalize" => execute_dynamic_audio_normalize_gemini(args),
        "resample_audio" => execute_resample_audio_gemini(args),
        "trim_audio" => execute_trim_audio_gemini(args),
        "apply_crystalizer" => execute_apply_crystalizer_gemini(args),
        "multiband_compress" => execute_multiband_compress_gemini(args),
        "apply_super_equalizer" => execute_apply_super_equalizer_gemini(args),
        "apply_colormatrix" => execute_apply_colormatrix_gemini(args),
        "apply_chromashift" => execute_apply_chromashift_gemini(args),
        "apply_cas" => execute_apply_cas_gemini(args),
        "apply_nlmeans_video" => execute_apply_nlmeans_video_gemini(args),
        "apply_spp" => execute_apply_spp_gemini(args),
        "apply_pp" => execute_apply_pp_gemini(args),
        "apply_mestimate" => execute_apply_mestimate_gemini(args),
        "apply_midequalizer" => execute_apply_midequalizer_gemini(args),
        "apply_median_spatial" => execute_apply_median_spatial_gemini(args),
        "apply_acrusher" => execute_apply_acrusher_gemini(args),
        "apply_atempo" => execute_apply_atempo_gemini(args),
        "apply_asetnsamples" => execute_apply_asetnsamples_gemini(args),
        "apply_apad" => execute_apply_apad_gemini(args),
        "apply_asubcut" => execute_apply_asubcut_gemini(args),
        "apply_asupercut" => execute_apply_asupercut_gemini(args),
        "apply_threshold" => execute_apply_threshold_gemini(args),
        "apply_maskedclamp" => execute_apply_maskedclamp_gemini(args),
        "apply_roberts" => execute_apply_roberts_gemini(args),
        "apply_sobel" => execute_apply_sobel_gemini(args),
        "apply_prewitt" => execute_apply_prewitt_gemini(args),
        "apply_kirsch" => execute_apply_kirsch_gemini(args),
        "apply_video_limiter" => execute_apply_video_limiter_gemini(args),
        "apply_bilateral" => execute_apply_bilateral_gemini(args),
        "apply_unsharp_mask" => execute_apply_unsharp_mask_gemini(args),
        "apply_lagfun" => execute_apply_lagfun_gemini(args),
        "apply_tinterlace" => execute_apply_tinterlace_gemini(args),
        "apply_datascope" => execute_apply_datascope_gemini(args),
        "apply_fspp" => execute_apply_fspp_gemini(args),
        "apply_haas" => execute_apply_haas_gemini(args),
        "apply_aemphasis" => execute_apply_aemphasis_gemini(args),
        "apply_negate" => execute_apply_negate_gemini(args),
        "apply_pixelize" => execute_apply_pixelize_gemini(args),
        "apply_colorlevels" => execute_apply_colorlevels_gemini(args),
        "apply_pseudocolor" => execute_apply_pseudocolor_gemini(args),
        "apply_colorhold" => execute_apply_colorhold_gemini(args),
        "apply_shuffleplanes" => execute_apply_shuffleplanes_gemini(args),
        "detect_black_frames" => execute_detect_black_frames_gemini(args),
        "detect_interlace_type" => execute_detect_interlace_type_gemini(args),
        "apply_vstack" => execute_apply_vstack_gemini(args),
        "apply_hstack" => execute_apply_hstack_gemini(args),
        "apply_setdar" => execute_apply_setdar_gemini(args),
        "apply_stereo3d" => execute_apply_stereo3d_gemini(args),
        "apply_telecine" => execute_apply_telecine_gemini(args),
        "apply_pullup" => execute_apply_pullup_gemini(args),
        "select_thumbnail_frame" => execute_select_thumbnail_frame_gemini(args),
        "apply_gaussian_blur" => execute_apply_gaussian_blur_gemini(args),
        "apply_box_blur" => execute_apply_box_blur_gemini(args),
        "apply_smart_blur" => execute_apply_smart_blur_gemini(args),
        "add_film_grain" => execute_add_film_grain_gemini(args),
        "apply_rotate_angle" => execute_apply_rotate_angle_gemini(args),
        "apply_geq" => execute_apply_geq_gemini(args),
        "apply_colorchannelmixer" => execute_apply_colorchannelmixer_gemini(args),
        "apply_atadenoise" => execute_apply_atadenoise_gemini(args),
        "apply_vaguedenoiser" => execute_apply_vaguedenoiser_gemini(args),
        "apply_fftdnoiz" => execute_apply_fftdnoiz_gemini(args),
        "generate_waveform_video" => execute_generate_waveform_video_gemini(args),
        "apply_lut3d" => execute_apply_lut3d_gemini(args),
        "measure_siti" => execute_measure_siti_gemini(args),
        "create_test_pattern" => execute_create_test_pattern_gemini(args),
        "apply_amplify" => execute_apply_amplify_gemini(args),
        "select_frames" => execute_select_frames_gemini(args),
        "posterize_video" => execute_posterize_video_gemini(args),
        "solarize_video" => execute_solarize_video_gemini(args),
        "apply_dilation" => execute_apply_dilation_gemini(args),
        "apply_erosion" => execute_apply_erosion_gemini(args),
        "apply_median_filter" => execute_apply_median_filter_gemini(args),
        "apply_histogram_eq" => execute_apply_histogram_eq_gemini(args),
        "apply_clahe" => execute_apply_clahe_gemini(args),
        "apply_deblock" => execute_apply_deblock_gemini(args),
        "adjust_hue_saturation" => execute_adjust_hue_saturation_gemini(args),
        "apply_convolution" => execute_apply_convolution_gemini(args),
        "reverse_audio" => execute_reverse_audio_gemini(args),
        "blend_audio_streams" => execute_blend_audio_streams_gemini(args),
        "measure_silence" => execute_measure_silence_gemini(args),
        "measure_audio_spectrum" => execute_measure_audio_spectrum_gemini(args),

        // ── Workflow Recipes ──────────────────────────────────────────────
        "youtube_ready_export" => execute_youtube_ready_export_gemini(args),
        "podcast_cleanup" => execute_podcast_cleanup_gemini(args),
        "cinematic_grade" => execute_cinematic_grade_gemini(args),
        "create_gif_workflow" => execute_create_gif_workflow_gemini(args),
        "talking_head_cleanup" => execute_talking_head_cleanup_gemini(args),

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

fn execute_add_transition_claude(args: &Value) -> String {
    let input1 = args["input_file1"].as_str().unwrap_or("");
    let input2 = args["input_file2"].as_str().unwrap_or("");
    let output_raw = args["output_file"].as_str().unwrap_or("");
    let output = ensure_outputs_directory(output_raw);
    let transition_type = args["transition_type"].as_str().unwrap_or("fade");
    let duration = args["duration_seconds"].as_f64().unwrap_or(1.0);
    let offset = args["offset_seconds"].as_f64().unwrap_or(0.0);
    crate::visual::add_transition(input1, input2, &output, transition_type, duration, offset).unwrap_or_else(|e| e)
}

fn execute_add_animated_text_claude(args: &Value) -> String {
    let input = args["input_file"].as_str().unwrap_or("");
    let output_raw = args["output_file"].as_str().unwrap_or("");
    let output = ensure_outputs_directory(output_raw);
    let text = args["text"].as_str().unwrap_or("");
    let animation_type = args["animation_type"].as_str().unwrap_or("fade_in");
    let start_time = args["start_time"].as_f64().unwrap_or(0.0);
    let duration = args["duration"].as_f64().unwrap_or(3.0);
    crate::visual::add_animated_text(input, &output, text, animation_type, start_time, duration).unwrap_or_else(|e| e)
}

fn execute_apply_filter_chain_claude(args: &Value) -> String {
    let input = args["input_file"].as_str().unwrap_or("");
    let output_raw = args["output_file"].as_str().unwrap_or("");
    let output = ensure_outputs_directory(output_raw);
    let filters: Vec<(String, serde_json::Value)> = args["filters"]
        .as_array()
        .unwrap_or(&vec![])
        .iter()
        .filter_map(|f| {
            let name = f["name"].as_str()?.to_string();
            let value = f["value"].clone();
            Some((name, value))
        })
        .collect();
    crate::visual::apply_filter_chain(input, &output, &filters).unwrap_or_else(|e| e)
}

fn execute_apply_audio_effect_claude(args: &Value) -> String {
    let input = args["input_file"].as_str().unwrap_or("");
    let output_raw = args["output_file"].as_str().unwrap_or("");
    let output = ensure_outputs_directory(output_raw);
    let effect = args["effect"].as_str().unwrap_or("echo");
    let intensity = args["intensity"].as_f64().unwrap_or(0.5);
    crate::audio::apply_audio_effect(input, &output, effect, intensity).unwrap_or_else(|e| e)
}

fn execute_deinterlace_video_claude(args: &Value) -> String {
    let input = args["input_file"].as_str().unwrap_or("");
    let output_raw = args["output_file"].as_str().unwrap_or("");
    let output = ensure_outputs_directory(output_raw);
    let mode = args["mode"].as_str().unwrap_or("0");
    crate::transform::deinterlace_video(input, &output, mode).unwrap_or_else(|e| e)
}

fn execute_export_custom_quality_claude(args: &Value) -> String {
    let input = args["input_file"].as_str().unwrap_or("");
    let output_raw = args["output_file"].as_str().unwrap_or("");
    let output = ensure_outputs_directory(output_raw);
    let quality = args["quality"].as_str().unwrap_or("medium");
    let resolution = match (args["width"].as_u64(), args["height"].as_u64()) {
        (Some(w), Some(h)) => Some((w as u32, h as u32)),
        _ => None,
    };
    let bitrate = args["bitrate_kbps"].as_u64().map(|b| b as u32);
    crate::export::export_custom_quality(input, &output, quality, resolution, bitrate).unwrap_or_else(|e| e)
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

    tracing::info!("📥 pexels_download_video: Starting download from {} to {}", video_url, output_file);
    match download_file_from_url(video_url, output_file).await {
        Ok(_) => {
            tracing::info!("✅ pexels_download_video: Download successful - {}", output_file);
            format!("✅ Successfully downloaded video from Pexels to: {}", output_file)
        }
        Err(e) => {
            tracing::error!("❌ pexels_download_video: Download failed - {}", e);
            format!("❌ Failed to download video: {}", e)
        }
    }
}

async fn execute_pexels_download_photo_claude(args: &Value) -> String {
    let photo_url = args["photo_url"].as_str().unwrap_or("");
    let output_file = args["output_file"].as_str().unwrap_or("");

    if photo_url.is_empty() || output_file.is_empty() {
        return "❌ Error: photo_url and output_file are required".to_string();
    }

    tracing::info!("📥 pexels_download_photo: Starting download from {} to {}", photo_url, output_file);
    match download_file_from_url(photo_url, output_file).await {
        Ok(_) => {
            tracing::info!("✅ pexels_download_photo: Download successful - {}", output_file);
            format!("✅ Successfully downloaded photo from Pexels to: {}", output_file)
        }
        Err(e) => {
            tracing::error!("❌ pexels_download_photo: Download failed - {}", e);
            format!("❌ Failed to download photo: {}", e)
        }
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

fn execute_add_transition_gemini(args: &HashMap<String, Value>) -> String {
    let input1 = args.get("input_file1").and_then(|v| v.as_str()).unwrap_or("");
    let input2 = args.get("input_file2").and_then(|v| v.as_str()).unwrap_or("");
    let output_raw = args.get("output_file").and_then(|v| v.as_str()).unwrap_or("");
    let output = ensure_outputs_directory(output_raw);
    let transition_type = args.get("transition_type").and_then(|v| v.as_str()).unwrap_or("fade");
    let duration = args.get("duration_seconds").and_then(|v| v.as_f64()).unwrap_or(1.0);
    let offset = args.get("offset_seconds").and_then(|v| v.as_f64()).unwrap_or(0.0);
    crate::visual::add_transition(input1, input2, &output, transition_type, duration, offset).unwrap_or_else(|e| e)
}

fn execute_add_animated_text_gemini(args: &HashMap<String, Value>) -> String {
    let input = args.get("input_file").and_then(|v| v.as_str()).unwrap_or("");
    let output_raw = args.get("output_file").and_then(|v| v.as_str()).unwrap_or("");
    let output = ensure_outputs_directory(output_raw);
    let text = args.get("text").and_then(|v| v.as_str()).unwrap_or("");
    let animation_type = args.get("animation_type").and_then(|v| v.as_str()).unwrap_or("fade_in");
    let start_time = args.get("start_time").and_then(|v| v.as_f64()).unwrap_or(0.0);
    let duration = args.get("duration").and_then(|v| v.as_f64()).unwrap_or(3.0);
    crate::visual::add_animated_text(input, &output, text, animation_type, start_time, duration).unwrap_or_else(|e| e)
}

fn execute_apply_filter_chain_gemini(args: &HashMap<String, Value>) -> String {
    let input = args.get("input_file").and_then(|v| v.as_str()).unwrap_or("");
    let output_raw = args.get("output_file").and_then(|v| v.as_str()).unwrap_or("");
    let output = ensure_outputs_directory(output_raw);
    let filters: Vec<(String, serde_json::Value)> = args
        .get("filters")
        .and_then(|v| v.as_array())
        .unwrap_or(&vec![])
        .iter()
        .filter_map(|f| {
            let name = f["name"].as_str()?.to_string();
            let value = f["value"].clone();
            Some((name, value))
        })
        .collect();
    crate::visual::apply_filter_chain(input, &output, &filters).unwrap_or_else(|e| e)
}

fn execute_apply_audio_effect_gemini(args: &HashMap<String, Value>) -> String {
    let input = args.get("input_file").and_then(|v| v.as_str()).unwrap_or("");
    let output_raw = args.get("output_file").and_then(|v| v.as_str()).unwrap_or("");
    let output = ensure_outputs_directory(output_raw);
    let effect = args.get("effect").and_then(|v| v.as_str()).unwrap_or("echo");
    let intensity = args.get("intensity").and_then(|v| v.as_f64()).unwrap_or(0.5);
    crate::audio::apply_audio_effect(input, &output, effect, intensity).unwrap_or_else(|e| e)
}

fn execute_deinterlace_video_gemini(args: &HashMap<String, Value>) -> String {
    let input = args.get("input_file").and_then(|v| v.as_str()).unwrap_or("");
    let output_raw = args.get("output_file").and_then(|v| v.as_str()).unwrap_or("");
    let output = ensure_outputs_directory(output_raw);
    let mode = args.get("mode").and_then(|v| v.as_str()).unwrap_or("0");
    crate::transform::deinterlace_video(input, &output, mode).unwrap_or_else(|e| e)
}

fn execute_export_custom_quality_gemini(args: &HashMap<String, Value>) -> String {
    let input = args.get("input_file").and_then(|v| v.as_str()).unwrap_or("");
    let output_raw = args.get("output_file").and_then(|v| v.as_str()).unwrap_or("");
    let output = ensure_outputs_directory(output_raw);
    let quality = args.get("quality").and_then(|v| v.as_str()).unwrap_or("medium");
    let resolution = match (
        args.get("width").and_then(|v| v.as_u64()),
        args.get("height").and_then(|v| v.as_u64()),
    ) {
        (Some(w), Some(h)) => Some((w as u32, h as u32)),
        _ => None,
    };
    let bitrate = args.get("bitrate_kbps").and_then(|v| v.as_u64()).map(|b| b as u32);
    crate::export::export_custom_quality(input, &output, quality, resolution, bitrate).unwrap_or_else(|e| e)
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
    let model = args.get("model").and_then(|v| v.as_str());

    if prompt.is_empty() || output_file.is_empty() {
        return "❌ Error: prompt and output_file are required".to_string();
    }

    let api_key = std::env::var("GEMINI_API_KEY")
        .unwrap_or_else(|_| std::env::var("GOOGLE_API_KEY").unwrap_or_default());

    if api_key.is_empty() {
        return "❌ Error: GEMINI_API_KEY or GOOGLE_API_KEY environment variable not set".to_string();
    }

    let client = crate::gemini_client::GeminiClient::new(api_key);

    match client.generate_image(prompt, aspect_ratio, image_size, model).await {
        Ok(image_bytes) => {
            match tokio::fs::write(&output_file, &image_bytes).await {
                Ok(_) => format!("✅ Successfully generated image and saved to: {}", output_file),
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
    let model = args.get("model").and_then(|v| v.as_str());

    if prompt.is_empty() || output_file.is_empty() {
        return "❌ Error: prompt and output_file are required".to_string();
    }

    let api_key = std::env::var("GEMINI_API_KEY")
        .unwrap_or_else(|_| std::env::var("GOOGLE_API_KEY").unwrap_or_default());

    if api_key.is_empty() {
        return "❌ Error: GEMINI_API_KEY or GOOGLE_API_KEY environment variable not set".to_string();
    }

    let client = crate::gemini_client::GeminiClient::new(api_key);

    match client.generate_image(prompt, aspect_ratio, image_size, model).await {
        Ok(image_bytes) => {
            match tokio::fs::write(&output_file, &image_bytes).await {
                Ok(_) => format!("✅ Successfully generated image and saved to: {}", output_file),
                Err(e) => format!("❌ Failed to save generated image: {}", e),
            }
        }
        Err(e) => format!("❌ Failed to generate image: {}", e),
    }
}

/// Edit an existing image using AI (Claude version)
async fn execute_edit_image_claude(args: &Value) -> String {
    let input_image = args["input_image"].as_str().unwrap_or("");
    let prompt = args["prompt"].as_str().unwrap_or("");
    let output_file = args["output_file"].as_str().unwrap_or("");
    let aspect_ratio = args.get("aspect_ratio").and_then(|v| v.as_str());
    let model = args.get("model").and_then(|v| v.as_str());

    if input_image.is_empty() || prompt.is_empty() || output_file.is_empty() {
        return "❌ Error: input_image, prompt, and output_file are required".to_string();
    }

    let image_bytes = match tokio::fs::read(input_image).await {
        Ok(b) => b,
        Err(e) => return format!("❌ Failed to read input image '{}': {}", input_image, e),
    };

    let api_key = std::env::var("GEMINI_API_KEY")
        .unwrap_or_else(|_| std::env::var("GOOGLE_API_KEY").unwrap_or_default());

    if api_key.is_empty() {
        return "❌ Error: GEMINI_API_KEY or GOOGLE_API_KEY environment variable not set".to_string();
    }

    let client = crate::gemini_client::GeminiClient::new(api_key);

    match client.edit_image(prompt, &image_bytes, aspect_ratio, model).await {
        Ok(result_bytes) => {
            match tokio::fs::write(&output_file, &result_bytes).await {
                Ok(_) => format!("✅ Image edited and saved to: {}", output_file),
                Err(e) => format!("❌ Failed to save edited image: {}", e),
            }
        }
        Err(e) => format!("❌ Failed to edit image: {}", e),
    }
}

/// Edit an existing image using AI (Gemini version)
async fn execute_edit_image_gemini(args: &HashMap<String, Value>) -> String {
    let input_image = args.get("input_image").and_then(|v| v.as_str()).unwrap_or("");
    let prompt = args.get("prompt").and_then(|v| v.as_str()).unwrap_or("");
    let output_file_raw = args.get("output_file").and_then(|v| v.as_str()).unwrap_or("");
    let output_file = ensure_outputs_directory(output_file_raw);
    let aspect_ratio = args.get("aspect_ratio").and_then(|v| v.as_str());
    let model = args.get("model").and_then(|v| v.as_str());

    if input_image.is_empty() || prompt.is_empty() || output_file.is_empty() {
        return "❌ Error: input_image, prompt, and output_file are required".to_string();
    }

    let image_bytes = match tokio::fs::read(input_image).await {
        Ok(b) => b,
        Err(e) => return format!("❌ Failed to read input image '{}': {}", input_image, e),
    };

    let api_key = std::env::var("GEMINI_API_KEY")
        .unwrap_or_else(|_| std::env::var("GOOGLE_API_KEY").unwrap_or_default());

    if api_key.is_empty() {
        return "❌ Error: GEMINI_API_KEY or GOOGLE_API_KEY environment variable not set".to_string();
    }

    let client = crate::gemini_client::GeminiClient::new(api_key);

    match client.edit_image(prompt, &image_bytes, aspect_ratio, model).await {
        Ok(result_bytes) => {
            match tokio::fs::write(&output_file, &result_bytes).await {
                Ok(_) => format!("✅ Image edited and saved to: {}", output_file),
                Err(e) => format!("❌ Failed to save edited image: {}", e),
            }
        }
        Err(e) => format!("❌ Failed to edit image: {}", e),
    }
}

// =============================================================================
// AUTO_GENERATE_VIDEO — video_source dispatch (Gemini + Claude)
// =============================================================================

/// Dispatcher: routes to pexels / blender / hybrid based on video_source param.
async fn execute_auto_generate_video_with_state_gemini(
    args: &HashMap<String, Value>,
    ctx: &ToolExecutionContext,
) -> String {
    let video_source = args.get("video_source").and_then(|v| v.as_str()).unwrap_or("pexels");
    let style = args.get("style").and_then(|v| v.as_str()).unwrap_or("cinematic");

    // Auto-route educational math to Blender LaTeX pipeline
    let effective_source = if style == "educational_math" && video_source == "pexels" {
        "blender"
    } else {
        video_source
    };

    match effective_source {
        "blender" => execute_auto_generate_video_blender_gemini(args, ctx).await,
        "hybrid"  => execute_auto_generate_video_hybrid_gemini(args, ctx).await,
        _         => execute_auto_generate_video_gemini(args).await,
    }
}

/// Dispatcher: Claude version.
async fn execute_auto_generate_video_with_state_claude(
    args: &Value,
    ctx: &ToolExecutionContext,
) -> String {
    let video_source = args.get("video_source").and_then(|v| v.as_str()).unwrap_or("pexels");
    let style = args.get("style").and_then(|v| v.as_str()).unwrap_or("cinematic");

    let effective_source = if style == "educational_math" && video_source == "pexels" {
        "blender"
    } else {
        video_source
    };

    match effective_source {
        "blender" => execute_auto_generate_video_blender_claude(args, ctx).await,
        "hybrid"  => execute_auto_generate_video_hybrid_claude(args, ctx).await,
        _         => execute_auto_generate_video_claude(args).await,
    }
}

// -----------------------------------------------------------------------------
// Blender-only clip acquisition (replaces Pexels search/download steps 1-2)
// Steps 3-6 (merge, text, music, QA) are identical to the Pexels path.
// -----------------------------------------------------------------------------

async fn execute_auto_generate_video_blender_gemini(
    args: &HashMap<String, Value>,
    ctx: &ToolExecutionContext,
) -> String {
    let blender = match ctx.app_state.blender_mcp_client.as_ref() {
        Some(c) => c,
        None => return "❌ BlenderMCPServer not configured (set BLENDER_MCP_URL). Cannot use video_source='blender'.".to_string(),
    };

    let topic        = args.get("topic").and_then(|v| v.as_str()).unwrap_or("");
    let output_filename = args.get("output_file").and_then(|v| v.as_str()).unwrap_or("");
    let output_file  = ensure_outputs_directory(output_filename);
    let duration     = args.get("duration").and_then(|v| v.as_f64()).unwrap_or(30.0);
    let style        = args.get("style").and_then(|v| v.as_str()).unwrap_or("cinematic");
    let include_text = args.get("include_text_overlays").and_then(|v| v.as_bool()).unwrap_or(true);
    let include_music = args.get("include_music").and_then(|v| v.as_bool()).unwrap_or(true);
    let num_clips    = args.get("num_clips").and_then(|v| v.as_u64()).unwrap_or(0) as usize;
    let aspect_ratio = args.get("aspect_ratio").and_then(|v| v.as_str()).unwrap_or("16:9").to_string();

    if topic.is_empty() || output_file.is_empty() {
        return "❌ Error: topic and output_file are required".to_string();
    }

    let num_clips = if num_clips == 0 { ((duration / 10.0).ceil() as usize).max(3).min(6) } else { num_clips };
    let clip_duration = (duration / num_clips as f64).max(5.0);

    let mut result = format!("🎨 **Auto-generating video (Blender 3D) about '{}'**\n\n", topic);
    result.push_str(&format!("Duration: {}s | Style: {} | Clips: {} | Source: Blender\n\n", duration, style, num_clips));
    result.push_str("🎬 Step 1-2: Rendering custom 3D clips via BlenderMCPServer...\n");

    let mut downloaded_files: Vec<String> = Vec::new();

    for i in 0..num_clips {
        let clip_path = format!("outputs/blender_clip_{}_{}.mp4", i, uuid::Uuid::new_v4().to_string().split('-').next().unwrap_or("x"));

        // Route educational_math style to LaTeX animation; everything else to scene
        let render_result = if style == "educational_math" {
            blender.generate_latex(topic, "step_by_step", clip_duration, "dark").await
        } else {
            blender.generate_scene(topic, clip_duration, style, None).await
        };

        match render_result {
            Ok(blender_path) => {
                // blender_mcp_client already downloads to outputs/; rename to our clip_path
                if blender_path != clip_path {
                    if let Err(e) = tokio::fs::rename(&blender_path, &clip_path).await {
                        // rename failed (cross-device) — try copy+delete
                        if tokio::fs::copy(&blender_path, &clip_path).await.is_ok() {
                            let _ = tokio::fs::remove_file(&blender_path).await;
                        } else {
                            result.push_str(&format!("  ⚠️ Clip {}: rename failed ({}), using original path\n", i + 1, e));
                            downloaded_files.push(blender_path);
                            continue;
                        }
                    }
                }
                result.push_str(&format!("  ✓ Clip {}: rendered ({:.1}s)\n", i + 1, clip_duration));
                downloaded_files.push(clip_path);
            }
            Err(e) => {
                result.push_str(&format!("  ✗ Clip {}: render failed — {}\n", i + 1, e));
            }
        }
    }

    if downloaded_files.is_empty() {
        return format!("{}❌ All Blender renders failed — check BlenderMCPServer logs", result);
    }

    result.push_str(&format!("\n✅ Rendered {} clips\n\n", downloaded_files.len()));
    finish_auto_generate_video(&downloaded_files, &output_file, topic, style, &aspect_ratio, include_text, include_music, duration, &mut result).await;
    result
}

async fn execute_auto_generate_video_blender_claude(
    args: &Value,
    ctx: &ToolExecutionContext,
) -> String {
    // Convert Value args to HashMap so we can reuse the Gemini blender path
    let map: HashMap<String, Value> = match args.as_object() {
        Some(obj) => obj.iter().map(|(k, v)| (k.clone(), v.clone())).collect(),
        None => return "❌ Internal error: args is not an object".to_string(),
    };
    execute_auto_generate_video_blender_gemini(&map, ctx).await
}

// -----------------------------------------------------------------------------
// Hybrid: Pexels primary, Blender fallback when thumbnail score < threshold
// -----------------------------------------------------------------------------

async fn execute_auto_generate_video_hybrid_gemini(
    args: &HashMap<String, Value>,
    ctx: &ToolExecutionContext,
) -> String {
    let topic    = args.get("topic").and_then(|v| v.as_str()).unwrap_or("");
    let duration = args.get("duration").and_then(|v| v.as_f64()).unwrap_or(30.0);
    let style    = args.get("style").and_then(|v| v.as_str()).unwrap_or("cinematic");
    let output_filename = args.get("output_file").and_then(|v| v.as_str()).unwrap_or("");
    let output_file = ensure_outputs_directory(output_filename);
    let include_text = args.get("include_text_overlays").and_then(|v| v.as_bool()).unwrap_or(true);
    let include_music = args.get("include_music").and_then(|v| v.as_bool()).unwrap_or(true);
    let num_clips = args.get("num_clips").and_then(|v| v.as_u64()).unwrap_or(0) as usize;
    let aspect_ratio = args.get("aspect_ratio").and_then(|v| v.as_str()).unwrap_or("16:9").to_string();

    let num_clips = if num_clips == 0 { ((duration / 10.0).ceil() as usize).max(3).min(8) } else { num_clips };
    let clip_duration = (duration / num_clips as f64).max(5.0);

    let mut result = format!("🔀 **Auto-generating video (Hybrid: Pexels + Blender) about '{}'**\n\n", topic);
    result.push_str(&format!("Duration: {}s | Style: {} | Clips: {}\n\n", duration, style, num_clips));

    let mut downloaded_files: Vec<String> = Vec::new();
    let mut used_ids: std::collections::HashSet<i64> = std::collections::HashSet::new();
    let search_queries = generate_search_queries_ai(topic, num_clips).await;

    for (i, query) in search_queries.iter().enumerate().take(num_clips) {
        result.push_str(&format!("  Clip {}: trying Pexels for '{}'\n", i + 1, query));

        // Try Pexels first
        let mut pexels_args = HashMap::new();
        pexels_args.insert("query".to_string(), Value::String(query.clone()));
        pexels_args.insert("media_type".to_string(), Value::String("videos".to_string()));
        pexels_args.insert("per_page".to_string(), Value::Number(serde_json::Number::from(5u64)));
        let pexels_result = execute_pexels_search_gemini(&pexels_args).await;

        let mut pexels_ok = false;
        if let Ok(search_data) = serde_json::from_str::<Value>(&pexels_result) {
            if let Some(videos) = search_data["videos"].as_array() {
                let candidates: Vec<Value> = videos.iter()
                    .filter_map(|v| v["id"].as_i64().filter(|id| !used_ids.contains(id)).map(|_| v.clone()))
                    .collect();

                let score_futures: Vec<_> = candidates.iter().map(|c| {
                    let thumb = c["video_pictures"][0]["picture"].as_str().unwrap_or("").to_string();
                    let t = topic.to_string();
                    async move { if thumb.is_empty() { 5i32 } else { screen_pexels_thumbnail(&thumb, &t).await } }
                }).collect();
                let scores: Vec<i32> = futures::future::join_all(score_futures).await;

                // Only accept Pexels clips that score >= 6 in hybrid mode (stricter than pure Pexels)
                if let Some((video, score)) = candidates.iter().zip(scores.iter()).find(|(_, &s)| s >= 6) {
                    if let Some(vid_id) = video["id"].as_i64() { used_ids.insert(vid_id); }
                    if let Some(link) = video["video_files"].as_array()
                        .and_then(|f| f.first())
                        .and_then(|f| f["link"].as_str())
                    {
                        let clip_path = format!("outputs/clip_{}_{}.mp4", i, uuid::Uuid::new_v4().to_string().split('-').next().unwrap());
                        let mut dl_args = HashMap::new();
                        dl_args.insert("video_url".to_string(), Value::String(link.to_string()));
                        dl_args.insert("output_file".to_string(), Value::String(clip_path.clone()));
                        let dl = execute_pexels_download_video_gemini(&dl_args).await;
                        if dl.contains("✅") {
                            if verify_clip_quality(&clip_path).is_ok() {
                                result.push_str(&format!("    ✓ Pexels clip (score {}/10)\n", score));
                                downloaded_files.push(clip_path);
                                pexels_ok = true;
                            } else {
                                let _ = std::fs::remove_file(&clip_path);
                            }
                        }
                    }
                }
            }
        }

        // Fallback to Blender if Pexels failed or scored too low
        if !pexels_ok {
            result.push_str(&format!("    ↳ Pexels miss — falling back to Blender render\n"));
            if let Some(blender) = ctx.app_state.blender_mcp_client.as_ref() {
                let clip_path = format!("outputs/blender_fallback_{}_{}.mp4", i, uuid::Uuid::new_v4().to_string().split('-').next().unwrap_or("x"));
                match blender.generate_scene(topic, clip_duration, style, None).await {
                    Ok(blender_path) => {
                        let _ = tokio::fs::rename(&blender_path, &clip_path).await;
                        result.push_str(&format!("    ✓ Blender fallback clip rendered\n"));
                        downloaded_files.push(clip_path);
                    }
                    Err(e) => result.push_str(&format!("    ✗ Blender fallback also failed: {}\n", e)),
                }
            } else {
                result.push_str("    ✗ BlenderMCPServer not configured — skipping clip\n");
            }
        }
    }

    if downloaded_files.is_empty() {
        return format!("{}❌ No clips acquired (Pexels and Blender both failed)", result);
    }

    result.push_str(&format!("\n✅ Acquired {} clips\n\n", downloaded_files.len()));
    finish_auto_generate_video(&downloaded_files, &output_file, topic, style, &aspect_ratio, include_text, include_music, duration, &mut result).await;
    result
}

async fn execute_auto_generate_video_hybrid_claude(
    args: &Value,
    ctx: &ToolExecutionContext,
) -> String {
    let map: HashMap<String, Value> = match args.as_object() {
        Some(obj) => obj.iter().map(|(k, v)| (k.clone(), v.clone())).collect(),
        None => return "❌ Internal error: args is not an object".to_string(),
    };
    execute_auto_generate_video_hybrid_gemini(&map, ctx).await
}

// -----------------------------------------------------------------------------
// Shared post-acquisition pipeline: merge → text → music → QA
// Both Blender and hybrid paths call this instead of duplicating Steps 3-6.
// -----------------------------------------------------------------------------

async fn finish_auto_generate_video(
    downloaded_files: &[String],
    output_file: &str,
    topic: &str,
    style: &str,
    aspect_ratio: &str,
    include_text: bool,
    include_music: bool,
    duration: f64,
    result: &mut String,
) {
    // Step 3: Merge with crossfade
    result.push_str("🎞️  Step 3: Merging clips with transitions...\n");
    match crate::core::merge_videos_with_transitions(downloaded_files, output_file, 0.5) {
        Ok(_) => {
            if !std::path::Path::new(output_file).exists()
                || std::fs::metadata(output_file).map(|m| m.len()).unwrap_or(0) < 1024
            {
                result.push_str("❌ Merged file missing or too small\n");
                return;
            }
            result.push_str("✅ Clips merged with crossfade transitions\n\n");
        }
        Err(e) => { result.push_str(&format!("❌ Failed to merge clips: {}\n", e)); return; }
    }

    // Aspect ratio crop
    if aspect_ratio != "16:9" {
        let crop_filter = match aspect_ratio {
            "9:16" => "scale=1080:1920:force_original_aspect_ratio=increase,crop=1080:1920",
            "1:1"  => "scale=1080:1080:force_original_aspect_ratio=increase,crop=1080:1080",
            "4:3"  => "scale=1440:1080:force_original_aspect_ratio=increase,crop=1440:1080",
            _      => "",
        };
        if !crop_filter.is_empty() {
            result.push_str(&format!("📐 Applying {} aspect ratio...\n", aspect_ratio));
            let cropped = format!("{}_crop.mp4", output_file.trim_end_matches(".mp4"));
            let mut cmd = StdCommand::new("ffmpeg");
            cmd.args(["-i", output_file, "-vf", crop_filter, "-c:a", "copy", "-y", &cropped]);
            if crate::utils::execute_ffmpeg_command_with_sync_timeout(cmd, Some(300)).is_ok()
                && std::path::Path::new(&cropped).exists()
            {
                let _ = std::fs::rename(&cropped, output_file);
                result.push_str(&format!("✅ Resized to {}\n\n", aspect_ratio));
            }
        }
    }

    // Step 4: Text overlay
    if include_text {
        result.push_str("📝 Step 4: Adding text overlay...\n");
        let temp_output = format!("{}_with_text.mp4", output_file.trim_end_matches(".mp4"));
        let safe_topic = topic.replace('\'', "\\'").replace(':', "\\:").replace(',', "\\,");
        let drawtext_filter = format!(
            "drawtext=text='{}':fontfile=/usr/share/fonts/truetype/dejavu/DejaVuSans-Bold.ttf:\
            fontsize=48:fontcolor=white:x=(w-text_w)/2:y=h*0.08:\
            box=1:boxcolor=black@0.5:boxborderw=10:\
            enable='between(t,0.5,4.5)':\
            alpha='if(lt(t,1.0),(t-0.5)/0.5,if(gt(t,4.0),1-(t-4.0)/0.5,1))'",
            safe_topic
        );
        let mut cmd = StdCommand::new("ffmpeg");
        cmd.args(["-i", output_file, "-vf", &drawtext_filter, "-c:a", "copy", "-y", &temp_output]);
        match crate::utils::execute_ffmpeg_command_with_sync_timeout(cmd, Some(300)) {
            Ok(_) if std::path::Path::new(&temp_output).exists() => {
                let _ = std::fs::rename(&temp_output, output_file);
                result.push_str("✅ Text overlay added\n\n");
            }
            Err(e) => result.push_str(&format!("⚠️ Text overlay failed: {} — continuing\n\n", e)),
            _ => {}
        }
    }

    // Step 5: Background music
    if include_music {
        result.push_str("🎵 Step 5: Generating background music...\n");
        let el_key = std::env::var("ELEVEN_LABS_API_KEY").unwrap_or_default();
        if !el_key.is_empty() {
            let el_client = crate::elevenlabs_client::ElevenLabsClient::new(el_key);
            let music_prompt = format!("{} {} background music, instrumental", style, topic);
            let music_path = format!("outputs/bgm_{}.mp3", uuid::Uuid::new_v4().to_string().split('-').next().unwrap_or("tmp"));
            let duration_ms = (duration * 1000.0) as u32;
            match el_client.generate_music_task(&music_prompt, duration_ms).await {
                Ok(task_id) => {
                    if let Some(audio_url) = poll_music_task(&el_client, &task_id, 45).await {
                        if let Ok(bytes) = el_client.download_music(&audio_url).await {
                            if tokio::fs::write(&music_path, &bytes).await.is_ok() {
                                let mixed_path = format!("{}_audio.mp4", output_file.trim_end_matches(".mp4"));
                                if crate::audio::add_audio(output_file, &music_path, &mixed_path).is_ok()
                                    && std::path::Path::new(&mixed_path).exists()
                                {
                                    let _ = std::fs::rename(&mixed_path, output_file);
                                    result.push_str("✅ Background music added\n\n");
                                }
                                let _ = tokio::fs::remove_file(&music_path).await;
                            }
                        }
                    } else {
                        result.push_str("⚠️ Music generation timed out\n\n");
                    }
                }
                Err(e) => result.push_str(&format!("⚠️ Music generation failed: {} — continuing\n\n", e)),
            }
        } else {
            result.push_str("⚠️ ELEVEN_LABS_API_KEY not set — video saved without music\n\n");
        }
    }

    // Cleanup
    for file in downloaded_files {
        let _ = tokio::fs::remove_file(file).await;
    }

    result.push_str("🎉 **Video generation complete!**\n\n");
    result.push_str(&format!("📥 Output: {}\n\n", output_file));

    let qa_report = run_final_qa(output_file);
    result.push_str(&qa_report);
    result.push_str("\n🔍 **AI Content Review Required:**\n");
    result.push_str("1. Call `view_video(path)` to visually confirm content\n");
    result.push_str("2. Call `review_video(path, original_request, expected_features)` for pass/fail verdict\n");
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
    let include_music = args.get("include_music").and_then(|v| v.as_bool()).unwrap_or(true); // ✅ Default TRUE - videos MUST have audio!
    let num_clips = args.get("num_clips").and_then(|v| v.as_u64()).unwrap_or(0) as usize;
    let aspect_ratio = args.get("aspect_ratio").and_then(|v| v.as_str()).unwrap_or("16:9").to_string();

    if topic.is_empty() || output_file.is_empty() {
        return "❌ Error: topic and output_file are required".to_string();
    }

    // Calculate number of clips based on duration if not specified
    let num_clips = if num_clips == 0 {
        ((duration / 10.0).ceil() as usize).max(3).min(8)
    } else {
        num_clips
    };

    tracing::info!("🎬 auto_generate_video (Claude): Starting generation for '{}' - Duration: {}s, Clips: {}, Music: {}", topic, duration, num_clips, include_music);

    let mut result = format!("🎬 **Auto-generating video about '{}'**\n\n", topic);
    result.push_str(&format!("Duration: {}s | Style: {} | Clips: {}\n\n", duration, style, num_clips));

    // Step 1: Generate search queries via AI (with heuristic fallback)
    result.push_str("📝 Step 1: Generating AI-powered search queries...\n");
    let search_queries = generate_search_queries_ai(topic, num_clips).await;
    tracing::info!("✅ Generated {} search queries: {:?}", search_queries.len(), search_queries.iter().take(3).collect::<Vec<_>>());
    result.push_str(&format!("  Queries: {}\n\n", search_queries.iter().take(3).map(|s| s.as_str()).collect::<Vec<_>>().join(", ")));

    // Load session-level used clip IDs to prevent cross-session duplicates (Fix 7)
    let session_ids_file = "outputs/.used_clip_ids.json";
    let mut used_ids: std::collections::HashSet<i64> = tokio::fs::read_to_string(session_ids_file)
        .await
        .ok()
        .and_then(|s| serde_json::from_str(&s).ok())
        .unwrap_or_default();

    // Step 2: Search and download clips — parallel thumbnail screening + duration accumulation
    result.push_str("🔍 Step 2: Searching Pexels for relevant clips...\n");
    let mut downloaded_files: Vec<String> = Vec::new();
    let mut downloaded_video_ids: Vec<i64> = Vec::new();
    let mut total_duration_secs = 0.0f64;
    let max_clips = num_clips.min(12);

    'query_loop_claude: for (i, query) in search_queries.iter().enumerate().take(max_clips) {
        if total_duration_secs >= duration * 0.9 {
            tracing::info!("Collected {:.1}s of footage (target: {:.1}s), stopping", total_duration_secs, duration);
            break;
        }

        let pexels_result = execute_pexels_search_claude(&serde_json::json!({
            "query": query,
            "media_type": "videos",
            "per_page": 5
        })).await;

        if let Ok(search_data) = serde_json::from_str::<Value>(&pexels_result) {
            if let Some(videos) = search_data["videos"].as_array() {
                // Filter session-level + within-call duplicates
                let candidates: Vec<Value> = videos.iter()
                    .filter_map(|v| v["id"].as_i64().and_then(|id| {
                        if !used_ids.contains(&id) && !downloaded_video_ids.contains(&id) {
                            Some(v.clone())
                        } else {
                            None
                        }
                    }))
                    .collect();

                if candidates.is_empty() {
                    result.push_str(&format!("  ⬜ Clip {}: all candidates already used\n", i + 1));
                    continue 'query_loop_claude;
                }

                // Screen all thumbnails in parallel (Fix 5)
                let score_futures: Vec<_> = candidates.iter().map(|c| {
                    let thumb = c["video_pictures"][0]["picture"].as_str().unwrap_or("").to_string();
                    let t = topic.to_string();
                    async move {
                        if thumb.is_empty() { 5i32 } else { screen_pexels_thumbnail(&thumb, &t).await }
                    }
                }).collect();
                let scores: Vec<i32> = futures::future::join_all(score_futures).await;

                let selected_video = candidates.iter().zip(scores.iter())
                    .find(|(_, &score)| score >= 5)
                    .map(|(v, score)| {
                        result.push_str(&format!("  🖼️ Clip {}: thumbnail {}/10 — downloading\n", i + 1, score));
                        v.clone()
                    })
                    .or_else(|| {
                        candidates.first().map(|v| {
                            result.push_str(&format!("  ⚠️ Clip {}: no high-score thumbnail for '{}', using best available\n", i + 1, query));
                            v.clone()
                        })
                    });

                if let Some(video) = selected_video {
                    if let Some(vid_id) = video["id"].as_i64() {
                        downloaded_video_ids.push(vid_id);
                        used_ids.insert(vid_id);
                    }
                    if let Some(files) = video["video_files"].as_array() {
                        if let Some(file) = files.first() {
                            if let Some(link) = file["link"].as_str() {
                                let clip_path = format!("outputs/clip_{}_{}.mp4", i, uuid::Uuid::new_v4().to_string().split('-').next().unwrap());

                                let download_result = execute_pexels_download_video_claude(&serde_json::json!({
                                    "video_url": link,
                                    "output_file": &clip_path
                                })).await;

                                if download_result.contains("✅") {
                                    match verify_clip_quality(&clip_path) {
                                        Ok(()) => {
                                            if let Ok(meta) = crate::core::analyze_video(&clip_path) {
                                                total_duration_secs += meta.duration_seconds;
                                            }
                                            downloaded_files.push(clip_path.clone());
                                            result.push_str(&format!("  ✓ Clip {}: {} (QA passed, {:.1}s total)\n", i + 1, query, total_duration_secs));
                                        }
                                        Err(reason) => {
                                            result.push_str(&format!("  ✗ Clip {}: rejected ({}), skipping\n", i + 1, reason));
                                            let _ = std::fs::remove_file(&clip_path);
                                        }
                                    }
                                }
                            }
                        }
                    }
                }
            }
        }
        let _ = tokio::fs::write(session_ids_file, serde_json::to_string(&used_ids).unwrap_or_default()).await;
    }

    if downloaded_files.is_empty() {
        tracing::error!("❌ auto_generate_video: Failed to download any clips from Pexels");
        return format!("{}❌ Failed to download any video clips from Pexels", result);
    }

    tracing::info!("✅ auto_generate_video: Successfully downloaded {} clips", downloaded_files.len());
    result.push_str(&format!("\n✅ Downloaded {} clips\n\n", downloaded_files.len()));

    // Step 3: Merge clips with crossfade transitions (Fix 6)
    result.push_str("🎞️  Step 3: Merging clips with transitions...\n");

    match crate::core::merge_videos_with_transitions(&downloaded_files, &output_file, 0.5) {
        Ok(_) => {
            if !std::path::Path::new(&output_file).exists()
                || std::fs::metadata(&output_file).map(|m| m.len()).unwrap_or(0) < 1024
            {
                return format!("{}❌ Merged file missing or too small", result);
            }
            result.push_str("✅ Clips merged with crossfade transitions\n\n");
        }
        Err(e) => return format!("{}❌ Failed to merge clips: {}", result, e),
    }

    // Aspect ratio crop (Fix 8)
    if aspect_ratio != "16:9" {
        let crop_filter = match aspect_ratio.as_str() {
            "9:16" => "scale=1080:1920:force_original_aspect_ratio=increase,crop=1080:1920",
            "1:1"  => "scale=1080:1080:force_original_aspect_ratio=increase,crop=1080:1080",
            "4:3"  => "scale=1440:1080:force_original_aspect_ratio=increase,crop=1440:1080",
            _      => "",
        };
        if !crop_filter.is_empty() {
            result.push_str(&format!("📐 Applying {} aspect ratio...\n", aspect_ratio));
            let cropped = format!("{}_crop.mp4", output_file.trim_end_matches(".mp4"));
            let mut cmd = StdCommand::new("ffmpeg");
            cmd.args(["-i", &output_file, "-vf", crop_filter, "-c:a", "copy", "-y", &cropped]);
            if crate::utils::execute_ffmpeg_command_with_sync_timeout(cmd, Some(300)).is_ok()
                && std::path::Path::new(&cropped).exists()
            {
                let _ = std::fs::rename(&cropped, &output_file);
                result.push_str(&format!("✅ Resized to {}\n\n", aspect_ratio));
            }
        }
    }

    // Step 4: Add text overlay — centered, background box, fade in/out (Fix 4)
    if include_text {
        result.push_str("📝 Step 4: Adding text overlay...\n");
        let temp_output = format!("{}_with_text.mp4", output_file.trim_end_matches(".mp4"));
        let safe_topic = topic.replace('\'', "\\'").replace(':', "\\:").replace(',', "\\,");
        let drawtext_filter = format!(
            "drawtext=text='{}':fontfile=/usr/share/fonts/truetype/dejavu/DejaVuSans-Bold.ttf:\
            fontsize=48:fontcolor=white:x=(w-text_w)/2:y=h*0.08:\
            box=1:boxcolor=black@0.5:boxborderw=10:\
            enable='between(t,0.5,4.5)':\
            alpha='if(lt(t,1.0),(t-0.5)/0.5,if(gt(t,4.0),1-(t-4.0)/0.5,1))'",
            safe_topic
        );
        let mut cmd = StdCommand::new("ffmpeg");
        cmd.args(["-i", &output_file, "-vf", &drawtext_filter, "-c:a", "copy", "-y", &temp_output]);
        match crate::utils::execute_ffmpeg_command_with_sync_timeout(cmd, Some(300)) {
            Ok(_) if std::path::Path::new(&temp_output).exists() => {
                let _ = tokio::fs::rename(&temp_output, &output_file).await;
                result.push_str("✅ Text overlay added (centered, fade in/out)\n\n");
            }
            Err(e) => result.push_str(&format!("⚠️ Text overlay failed: {} — continuing\n\n", e)),
            _ => {}
        }
    }

    // Step 5: Generate and mix background music via ElevenLabs (Fix 2)
    if include_music {
        result.push_str("🎵 Step 5: Generating background music...\n");
        let el_key = std::env::var("ELEVEN_LABS_API_KEY").unwrap_or_default();
        if !el_key.is_empty() {
            let el_client = crate::elevenlabs_client::ElevenLabsClient::new(el_key);
            let music_prompt = format!("{} {} background music, instrumental", style, topic);
            let music_path = format!("outputs/bgm_{}.mp3", uuid::Uuid::new_v4().to_string().split('-').next().unwrap_or("tmp"));
            let duration_ms = (duration * 1000.0) as u32;
            match el_client.generate_music_task(&music_prompt, duration_ms).await {
                Ok(task_id) => {
                    if let Some(audio_url) = poll_music_task(&el_client, &task_id, 45).await {
                        match el_client.download_music(&audio_url).await {
                            Ok(bytes) => {
                                if tokio::fs::write(&music_path, &bytes).await.is_ok() {
                                    let mixed_path = format!("{}_audio.mp4", output_file.trim_end_matches(".mp4"));
                                    if crate::audio::add_audio(&output_file, &music_path, &mixed_path).is_ok()
                                        && std::path::Path::new(&mixed_path).exists()
                                    {
                                        let _ = tokio::fs::rename(&mixed_path, &output_file).await;
                                        result.push_str("✅ Background music added\n\n");
                                    } else {
                                        result.push_str("⚠️ Music mix failed — video saved without music\n\n");
                                    }
                                    let _ = tokio::fs::remove_file(&music_path).await;
                                }
                            }
                            Err(e) => result.push_str(&format!("⚠️ Music download failed: {} — video saved without music\n\n", e)),
                        }
                    } else {
                        result.push_str("⚠️ Music generation timed out — video saved without music\n\n");
                    }
                }
                Err(e) => result.push_str(&format!("⚠️ Music generation failed: {} — video saved without music\n\n", e)),
            }
        } else {
            result.push_str("⚠️ ELEVEN_LABS_API_KEY not set — video saved without music\n\n");
        }
    }

    // Cleanup temporary clip files
    for file in &downloaded_files {
        let _ = tokio::fs::remove_file(file).await;
    }

    tracing::info!("🎉 auto_generate_video: Video generation COMPLETE - Output: {}", output_file);
    result.push_str("🎉 **Video generation complete!**\n\n");
    result.push_str(&format!("📥 Output: {}\n\n", output_file));

    // Run hardwired automated QA on the final merged video
    tracing::info!("🔬 auto_generate_video: Running automated QA on {}", output_file);
    let qa_report = run_final_qa(&output_file);
    result.push_str(&qa_report);
    result.push_str("\n");

    // Instruct agent on remaining AI review steps
    result.push_str("🔍 **AI Content Review Required:**\n");
    result.push_str("The automated QA above covers technical signal quality. You must ALSO:\n");
    result.push_str("1. Call `view_video(path)` to visually confirm content is relevant to the topic\n");
    result.push_str("2. Call `review_video(path, original_request, expected_features)` for pass/fail verdict\n");
    result.push_str("3. Optionally: `measure_loudness(path)` to verify audio levels, `analyze_video_signal(path)` for color/luma analysis\n");
    result.push_str("4. If the QA report shows warnings OR the review FAILS, call auto_generate_video again with different search queries\n\n");

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
    let include_music = args.get("include_music").and_then(|v| v.as_bool()).unwrap_or(true); // ✅ Default TRUE - videos MUST have audio!
    let num_clips = args.get("num_clips").and_then(|v| v.as_u64()).unwrap_or(0) as usize;
    let aspect_ratio = args.get("aspect_ratio").and_then(|v| v.as_str()).unwrap_or("16:9").to_string();

    if topic.is_empty() || output_file.is_empty() {
        return "❌ Error: topic and output_file are required".to_string();
    }

    // Calculate number of clips based on duration if not specified
    let num_clips = if num_clips == 0 {
        ((duration / 10.0).ceil() as usize).max(3).min(8)
    } else {
        num_clips
    };

    tracing::info!("🎬 auto_generate_video (Gemini): Starting generation for '{}' - Duration: {}s, Clips: {}, Music: {}", topic, duration, num_clips, include_music);

    let mut result = format!("🎬 **Auto-generating video about '{}'**\n\n", topic);
    result.push_str(&format!("Duration: {}s | Style: {} | Clips: {}\n\n", duration, style, num_clips));

    // Step 1: Generate search queries via AI (with heuristic fallback)
    result.push_str("📝 Step 1: Generating AI-powered search queries...\n");
    let search_queries = generate_search_queries_ai(topic, num_clips).await;
    tracing::info!("✅ Generated {} search queries: {:?}", search_queries.len(), search_queries.iter().take(3).collect::<Vec<_>>());
    result.push_str(&format!("  Queries: {}\n\n", search_queries.iter().take(3).map(|s| s.as_str()).collect::<Vec<_>>().join(", ")));

    // Load session-level used clip IDs to prevent cross-session duplicates (Fix 7)
    let session_ids_file = "outputs/.used_clip_ids.json";
    let mut used_ids: std::collections::HashSet<i64> = tokio::fs::read_to_string(session_ids_file)
        .await
        .ok()
        .and_then(|s| serde_json::from_str(&s).ok())
        .unwrap_or_default();

    // Step 2: Search and download clips — parallel screening + duration accumulation
    result.push_str("🔍 Step 2: Searching Pexels for relevant clips...\n");
    let mut downloaded_files: Vec<String> = Vec::new();
    let mut downloaded_video_ids: Vec<i64> = Vec::new();
    let mut total_duration_secs = 0.0f64;
    let max_clips = num_clips.min(12);

    'query_loop_gemini: for (i, query) in search_queries.iter().enumerate().take(max_clips) {
        if total_duration_secs >= duration * 0.9 {
            tracing::info!("Collected {:.1}s of footage (target: {:.1}s), stopping", total_duration_secs, duration);
            break;
        }

        let mut search_args = HashMap::new();
        search_args.insert("query".to_string(), Value::String(query.clone()));
        search_args.insert("media_type".to_string(), Value::String("videos".to_string()));
        search_args.insert("per_page".to_string(), Value::Number(serde_json::Number::from(5)));
        let pexels_result = execute_pexels_search_gemini(&search_args).await;

        if let Ok(search_data) = serde_json::from_str::<Value>(&pexels_result) {
            if let Some(videos) = search_data["videos"].as_array() {
                // Filter session-level + within-call duplicates
                let candidates: Vec<Value> = videos.iter()
                    .filter_map(|v| v["id"].as_i64().and_then(|id| {
                        if !used_ids.contains(&id) && !downloaded_video_ids.contains(&id) {
                            Some(v.clone())
                        } else {
                            None
                        }
                    }))
                    .collect();

                if candidates.is_empty() {
                    result.push_str(&format!("  ⬜ Clip {}: all candidates already used\n", i + 1));
                    continue 'query_loop_gemini;
                }

                // Screen all thumbnails in parallel (Fix 5)
                let score_futures: Vec<_> = candidates.iter().map(|c| {
                    let thumb = c["video_pictures"][0]["picture"].as_str().unwrap_or("").to_string();
                    let t = topic.to_string();
                    async move {
                        if thumb.is_empty() { 5i32 } else { screen_pexels_thumbnail(&thumb, &t).await }
                    }
                }).collect();
                let scores: Vec<i32> = futures::future::join_all(score_futures).await;

                let selected_video = candidates.iter().zip(scores.iter())
                    .find(|(_, &score)| score >= 5)
                    .map(|(v, score)| {
                        result.push_str(&format!("  🖼️ Clip {}: thumbnail {}/10 — downloading\n", i + 1, score));
                        v.clone()
                    })
                    .or_else(|| {
                        candidates.first().map(|v| {
                            result.push_str(&format!("  ⚠️ Clip {}: no high-score thumbnail for '{}', using best available\n", i + 1, query));
                            v.clone()
                        })
                    });

                if let Some(video) = selected_video {
                    if let Some(vid_id) = video["id"].as_i64() {
                        downloaded_video_ids.push(vid_id);
                        used_ids.insert(vid_id);
                    }
                    if let Some(files) = video["video_files"].as_array() {
                        if let Some(file) = files.first() {
                            if let Some(link) = file["link"].as_str() {
                                let clip_path = format!("outputs/clip_{}_{}.mp4", i, uuid::Uuid::new_v4().to_string().split('-').next().unwrap());

                                let mut download_args = HashMap::new();
                                download_args.insert("video_url".to_string(), Value::String(link.to_string()));
                                download_args.insert("output_file".to_string(), Value::String(clip_path.clone()));
                                let download_result = execute_pexels_download_video_gemini(&download_args).await;

                                if download_result.contains("✅") {
                                    match verify_clip_quality(&clip_path) {
                                        Ok(()) => {
                                            if let Ok(meta) = crate::core::analyze_video(&clip_path) {
                                                total_duration_secs += meta.duration_seconds;
                                            }
                                            downloaded_files.push(clip_path.clone());
                                            result.push_str(&format!("  ✓ Clip {}: {} (QA passed, {:.1}s total)\n", i + 1, query, total_duration_secs));
                                        }
                                        Err(reason) => {
                                            result.push_str(&format!("  ✗ Clip {}: rejected ({}), skipping\n", i + 1, reason));
                                            let _ = std::fs::remove_file(&clip_path);
                                        }
                                    }
                                }
                            }
                        }
                    }
                }
            }
        }
        let _ = tokio::fs::write(session_ids_file, serde_json::to_string(&used_ids).unwrap_or_default()).await;
    }

    if downloaded_files.is_empty() {
        tracing::error!("❌ auto_generate_video: Failed to download any clips from Pexels");
        return format!("{}❌ Failed to download any video clips from Pexels", result);
    }

    tracing::info!("✅ auto_generate_video: Successfully downloaded {} clips", downloaded_files.len());
    result.push_str(&format!("\n✅ Downloaded {} clips\n\n", downloaded_files.len()));

    // Step 3: Merge clips with crossfade transitions (Fix 6)
    result.push_str("🎞️  Step 3: Merging clips with transitions...\n");

    match crate::core::merge_videos_with_transitions(&downloaded_files, &output_file, 0.5) {
        Ok(_) => {
            if !std::path::Path::new(&output_file).exists()
                || std::fs::metadata(&output_file).map(|m| m.len()).unwrap_or(0) < 1024
            {
                return format!("{}❌ Merged file missing or too small", result);
            }
            result.push_str("✅ Clips merged with crossfade transitions\n\n");
        }
        Err(e) => return format!("{}❌ Failed to merge clips: {}", result, e),
    }

    // Aspect ratio crop (Fix 8)
    if aspect_ratio != "16:9" {
        let crop_filter = match aspect_ratio.as_str() {
            "9:16" => "scale=1080:1920:force_original_aspect_ratio=increase,crop=1080:1920",
            "1:1"  => "scale=1080:1080:force_original_aspect_ratio=increase,crop=1080:1080",
            "4:3"  => "scale=1440:1080:force_original_aspect_ratio=increase,crop=1440:1080",
            _      => "",
        };
        if !crop_filter.is_empty() {
            result.push_str(&format!("📐 Applying {} aspect ratio...\n", aspect_ratio));
            let cropped = format!("{}_crop.mp4", output_file.trim_end_matches(".mp4"));
            let mut cmd = StdCommand::new("ffmpeg");
            cmd.args(["-i", &output_file, "-vf", crop_filter, "-c:a", "copy", "-y", &cropped]);
            if crate::utils::execute_ffmpeg_command_with_sync_timeout(cmd, Some(300)).is_ok()
                && std::path::Path::new(&cropped).exists()
            {
                let _ = std::fs::rename(&cropped, &output_file);
                result.push_str(&format!("✅ Resized to {}\n\n", aspect_ratio));
            }
        }
    }

    // Step 4: Add text overlay — centered, background box, fade in/out (Fix 4)
    if include_text {
        result.push_str("📝 Step 4: Adding text overlay...\n");
        let temp_output = format!("{}_with_text.mp4", output_file.trim_end_matches(".mp4"));
        let safe_topic = topic.replace('\'', "\\'").replace(':', "\\:").replace(',', "\\,");
        let drawtext_filter = format!(
            "drawtext=text='{}':fontfile=/usr/share/fonts/truetype/dejavu/DejaVuSans-Bold.ttf:\
            fontsize=48:fontcolor=white:x=(w-text_w)/2:y=h*0.08:\
            box=1:boxcolor=black@0.5:boxborderw=10:\
            enable='between(t,0.5,4.5)':\
            alpha='if(lt(t,1.0),(t-0.5)/0.5,if(gt(t,4.0),1-(t-4.0)/0.5,1))'",
            safe_topic
        );
        let mut cmd = StdCommand::new("ffmpeg");
        cmd.args(["-i", &output_file, "-vf", &drawtext_filter, "-c:a", "copy", "-y", &temp_output]);
        match crate::utils::execute_ffmpeg_command_with_sync_timeout(cmd, Some(300)) {
            Ok(_) if std::path::Path::new(&temp_output).exists() => {
                let _ = tokio::fs::rename(&temp_output, &output_file).await;
                result.push_str("✅ Text overlay added (centered, fade in/out)\n\n");
            }
            Err(e) => result.push_str(&format!("⚠️ Text overlay failed: {} — continuing\n\n", e)),
            _ => {}
        }
    }

    // Step 5: Generate and mix background music via ElevenLabs (Fix 2)
    if include_music {
        result.push_str("🎵 Step 5: Generating background music...\n");
        let el_key = std::env::var("ELEVEN_LABS_API_KEY").unwrap_or_default();
        if !el_key.is_empty() {
            let el_client = crate::elevenlabs_client::ElevenLabsClient::new(el_key);
            let music_prompt = format!("{} {} background music, instrumental", style, topic);
            let music_path = format!("outputs/bgm_{}.mp3", uuid::Uuid::new_v4().to_string().split('-').next().unwrap_or("tmp"));
            let duration_ms = (duration * 1000.0) as u32;
            match el_client.generate_music_task(&music_prompt, duration_ms).await {
                Ok(task_id) => {
                    if let Some(audio_url) = poll_music_task(&el_client, &task_id, 45).await {
                        match el_client.download_music(&audio_url).await {
                            Ok(bytes) => {
                                if tokio::fs::write(&music_path, &bytes).await.is_ok() {
                                    let mixed_path = format!("{}_audio.mp4", output_file.trim_end_matches(".mp4"));
                                    if crate::audio::add_audio(&output_file, &music_path, &mixed_path).is_ok()
                                        && std::path::Path::new(&mixed_path).exists()
                                    {
                                        let _ = tokio::fs::rename(&mixed_path, &output_file).await;
                                        result.push_str("✅ Background music added\n\n");
                                    } else {
                                        result.push_str("⚠️ Music mix failed — video saved without music\n\n");
                                    }
                                    let _ = tokio::fs::remove_file(&music_path).await;
                                }
                            }
                            Err(e) => result.push_str(&format!("⚠️ Music download failed: {} — video saved without music\n\n", e)),
                        }
                    } else {
                        result.push_str("⚠️ Music generation timed out — video saved without music\n\n");
                    }
                }
                Err(e) => result.push_str(&format!("⚠️ Music generation failed: {} — video saved without music\n\n", e)),
            }
        } else {
            result.push_str("⚠️ ELEVEN_LABS_API_KEY not set — video saved without music\n\n");
        }
    }

    // Cleanup temporary clip files
    for file in &downloaded_files {
        let _ = tokio::fs::remove_file(file).await;
    }

    tracing::info!("🎉 auto_generate_video: Video generation COMPLETE - Output: {}", output_file);
    result.push_str("🎉 **Video generation complete!**\n\n");
    result.push_str(&format!("📥 Output: {}\n\n", output_file));

    // Run hardwired automated QA on the final merged video
    tracing::info!("🔬 auto_generate_video: Running automated QA on {}", output_file);
    let qa_report = run_final_qa(&output_file);
    result.push_str(&qa_report);
    result.push_str("\n");

    // Instruct agent on remaining AI review steps
    result.push_str("🔍 **AI Content Review Required:**\n");
    result.push_str("The automated QA above covers technical signal quality. You must ALSO:\n");
    result.push_str("1. Call `view_video(path)` to visually confirm content is relevant to the topic\n");
    result.push_str("2. Call `review_video(path, original_request, expected_features)` for pass/fail verdict\n");
    result.push_str("3. Optionally: `measure_loudness(path)` to verify audio levels, `analyze_video_signal(path)` for color/luma analysis\n");
    result.push_str("4. If the QA report shows warnings OR the review FAILS, call auto_generate_video again with different search queries\n\n");

    result
}

// ─────────────────────────────────────────────────────────────────────────────
// Option A pipeline tools — agentic video generation workflow
// ─────────────────────────────────────────────────────────────────────────────

/// Generate diverse Pexels search queries from a high-level video topic (Claude).
fn execute_generate_video_queries_claude(args: &Value) -> String {
    let topic = args["topic"].as_str().unwrap_or("");
    if topic.is_empty() {
        return "❌ Error: topic is required".to_string();
    }
    let num = args.get("num_queries").and_then(|v| v.as_u64()).unwrap_or(5) as usize;
    let queries = generate_search_queries_fallback(topic, num);
    match serde_json::to_string_pretty(&queries) {
        Ok(json) => format!("🔍 Generated {} search queries for topic '{}':\n{}", queries.len(), topic, json),
        Err(e) => format!("❌ Failed to serialize queries: {}", e),
    }
}

/// Generate diverse Pexels search queries from a high-level video topic (Gemini).
fn execute_generate_video_queries_gemini(args: &HashMap<String, Value>) -> String {
    let topic = args.get("topic").and_then(|v| v.as_str()).unwrap_or("");
    if topic.is_empty() {
        return "❌ Error: topic is required".to_string();
    }
    let num = args.get("num_queries").and_then(|v| v.as_u64()).unwrap_or(5) as usize;
    let queries = generate_search_queries_fallback(topic, num);
    match serde_json::to_string_pretty(&queries) {
        Ok(json) => format!("🔍 Generated {} search queries for topic '{}':\n{}", queries.len(), topic, json),
        Err(e) => format!("❌ Failed to serialize queries: {}", e),
    }
}

/// Download a Pexels thumbnail URL and analyze it with Gemini vision for topic relevance (Claude).
async fn execute_analyze_pexels_thumbnail_claude(args: &Value) -> String {
    let thumbnail_url = args["thumbnail_url"].as_str().unwrap_or("");
    let topic = args.get("topic").and_then(|v| v.as_str()).unwrap_or("video content");
    if thumbnail_url.is_empty() {
        return "❌ Error: thumbnail_url is required".to_string();
    }

    let api_key = std::env::var("GEMINI_API_KEY")
        .unwrap_or_else(|_| std::env::var("GOOGLE_API_KEY").unwrap_or_default());
    if api_key.is_empty() {
        return "❌ Error: GEMINI_API_KEY not set".to_string();
    }

    // Download thumbnail bytes
    let image_bytes = match reqwest::get(thumbnail_url).await {
        Ok(resp) => match resp.bytes().await {
            Ok(b) => b.to_vec(),
            Err(e) => return format!("❌ Failed to read thumbnail bytes: {}", e),
        },
        Err(e) => return format!("❌ Failed to fetch thumbnail: {}", e),
    };

    let client = crate::gemini_client::GeminiClient::new(api_key);
    let prompt = format!(
        "I am building a video about: \"{topic}\". \
        Look at this thumbnail image and answer: \
        1) What does it show? (1-2 sentences) \
        2) Relevance score 1-10 for the topic (10 = perfect match). \
        Format: DESCRIPTION: <text> | SCORE: <number>"
    );

    match client.analyze_image_bytes(&image_bytes, &prompt).await {
        Ok(analysis) => format!("🖼️ Thumbnail analysis for topic '{topic}':\n{analysis}"),
        Err(e) => format!("❌ Thumbnail analysis failed: {}", e),
    }
}

/// Download a Pexels thumbnail URL and analyze it with Gemini vision for topic relevance (Gemini).
async fn execute_analyze_pexels_thumbnail_gemini(args: &HashMap<String, Value>) -> String {
    let thumbnail_url = args.get("thumbnail_url").and_then(|v| v.as_str()).unwrap_or("");
    let topic = args.get("topic").and_then(|v| v.as_str()).unwrap_or("video content");
    if thumbnail_url.is_empty() {
        return "❌ Error: thumbnail_url is required".to_string();
    }

    let api_key = std::env::var("GEMINI_API_KEY")
        .unwrap_or_else(|_| std::env::var("GOOGLE_API_KEY").unwrap_or_default());
    if api_key.is_empty() {
        return "❌ Error: GEMINI_API_KEY not set".to_string();
    }

    let image_bytes = match reqwest::get(thumbnail_url).await {
        Ok(resp) => match resp.bytes().await {
            Ok(b) => b.to_vec(),
            Err(e) => return format!("❌ Failed to read thumbnail bytes: {}", e),
        },
        Err(e) => return format!("❌ Failed to fetch thumbnail: {}", e),
    };

    let client = crate::gemini_client::GeminiClient::new(api_key);
    let prompt = format!(
        "I am building a video about: \"{topic}\". \
        Look at this thumbnail image and answer: \
        1) What does it show? (1-2 sentences) \
        2) Relevance score 1-10 for the topic (10 = perfect match). \
        Format: DESCRIPTION: <text> | SCORE: <number>"
    );

    match client.analyze_image_bytes(&image_bytes, &prompt).await {
        Ok(analysis) => format!("🖼️ Thumbnail analysis for topic '{topic}':\n{analysis}"),
        Err(e) => format!("❌ Thumbnail analysis failed: {}", e),
    }
}

/// Expose verify_clip_quality as an agent-callable tool (Claude).
fn execute_verify_clip_quality_tool_claude(args: &Value) -> String {
    let file_path = args["file_path"].as_str().unwrap_or("");
    if file_path.is_empty() {
        return "❌ Error: file_path is required".to_string();
    }
    match verify_clip_quality(file_path) {
        Ok(()) => format!("✅ QA passed: '{}' is a valid, usable clip", file_path),
        Err(reason) => format!("❌ QA failed: {} — reason: {}", file_path, reason),
    }
}

/// Expose verify_clip_quality as an agent-callable tool (Gemini).
fn execute_verify_clip_quality_tool_gemini(args: &HashMap<String, Value>) -> String {
    let file_path = args.get("file_path").and_then(|v| v.as_str()).unwrap_or("");
    if file_path.is_empty() {
        return "❌ Error: file_path is required".to_string();
    }
    match verify_clip_quality(file_path) {
        Ok(()) => format!("✅ QA passed: '{}' is a valid, usable clip", file_path),
        Err(reason) => format!("❌ QA failed: {} — reason: {}", file_path, reason),
    }
}

/// Run the full automated QA suite on a video file and return the report (Claude).
fn execute_run_video_qa_claude(args: &Value) -> String {
    let file_path = args["file_path"].as_str().unwrap_or("");
    if file_path.is_empty() {
        return "❌ Error: file_path is required".to_string();
    }
    if !std::path::Path::new(file_path).exists() {
        return format!("❌ File not found: {}", file_path);
    }
    run_final_qa(file_path)
}

/// Run the full automated QA suite on a video file and return the report (Gemini).
fn execute_run_video_qa_gemini(args: &HashMap<String, Value>) -> String {
    let file_path = args.get("file_path").and_then(|v| v.as_str()).unwrap_or("");
    if file_path.is_empty() {
        return "❌ Error: file_path is required".to_string();
    }
    if !std::path::Path::new(file_path).exists() {
        return format!("❌ File not found: {}", file_path);
    }
    run_final_qa(file_path)
}

/// Option B: Download a Pexels video thumbnail and ask Gemini to score its relevance 1-10.
/// Returns the score, or 5 (neutral / proceed) if Gemini is unavailable or the call fails.
async fn screen_pexels_thumbnail(thumbnail_url: &str, topic: &str) -> i32 {
    let api_key = std::env::var("GEMINI_API_KEY")
        .unwrap_or_else(|_| std::env::var("GOOGLE_API_KEY").unwrap_or_default());
    if api_key.is_empty() {
        return 5; // no key → don't block downloads
    }

    let image_bytes = match reqwest::get(thumbnail_url).await {
        Ok(r) => match r.bytes().await {
            Ok(b) => b.to_vec(),
            Err(_) => return 5,
        },
        Err(_) => return 5,
    };

    let client = crate::gemini_client::GeminiClient::new(api_key);
    let prompt = format!(
        "Video topic: \"{topic}\". \
        Rate how relevant this thumbnail is for that topic, from 1 (completely unrelated) \
        to 10 (perfect match). Reply with ONLY a single integer, nothing else."
    );

    match client.analyze_image_bytes(&image_bytes, &prompt).await {
        Ok(response) => {
            // Extract the first 1-2 digit number from the response
            let digits: String = response.chars().filter(|c| c.is_ascii_digit()).take(2).collect();
            digits.parse::<i32>().unwrap_or(5).min(10).max(1)
        }
        Err(_) => 5, // fail open
    }
}

/// Lightweight FFmpeg-based quality check for a freshly downloaded Pexels clip.
/// Returns Ok(()) if the clip passes all checks, Err(reason) if it should be discarded.
/// Checks: (1) duration > 1.0s, (2) no significant frozen frames, (3) no significant black frames.
fn verify_clip_quality(clip_path: &str) -> Result<(), String> {
    // Check 1: Duration > 1 second (catches zero-length / corrupt downloads)
    match crate::core::analyze_video(clip_path) {
        Ok(meta) => {
            if meta.duration_seconds <= 1.0 {
                return Err(format!("clip too short ({:.2}s)", meta.duration_seconds));
            }
        }
        Err(e) => return Err(format!("could not read clip metadata: {}", e)),
    }

    // Check 2: Frozen frames — 1.5s window to avoid false positives on brief static shots
    match crate::visual::detect_frozen_frames(clip_path, -60.0, 1.5) {
        Ok(report) if !report.to_lowercase().contains("no frozen") && report.contains("freeze") => {
            return Err(format!("frozen frames detected: {}", &report[..report.len().min(120)]));
        }
        Err(e) => tracing::warn!("verify_clip_quality: frozen frame check skipped (non-fatal): {}", e),
        _ => {}
    }

    // Check 3: Black frames — 1.0s window
    match crate::visual::detect_black_frames(clip_path, 1.0, 0.98, 0.10) {
        Ok(report) if !report.to_lowercase().contains("no black") && report.contains("black_start") => {
            return Err(format!("black frames detected: {}", &report[..report.len().min(120)]));
        }
        Err(e) => tracing::warn!("verify_clip_quality: black frame check skipped (non-fatal): {}", e),
        _ => {}
    }

    Ok(())
}

/// Run a structured QA suite on the final merged video using FFmpeg analysis tools.
/// Returns a formatted multi-line report string to embed in the agent's response.
fn run_final_qa(output_file: &str) -> String {
    let mut qa = String::from("📊 **Automated QA Report:**\n");

    // QA 1: Video metadata (duration, resolution, fps, format, size)
    match crate::core::analyze_video(output_file) {
        Ok(meta) => {
            qa.push_str(&format!(
                "  • Duration: {:.1}s | Resolution: {}x{} | FPS: {:.1} | Format: {} | Size: {:.1}MB | Audio: {}\n",
                meta.duration_seconds,
                meta.width,
                meta.height,
                meta.fps,
                meta.format,
                meta.file_size_mb,
                if meta.has_audio { "✓" } else { "✗ (silent)" },
            ));
            if meta.duration_seconds < 2.0 {
                qa.push_str("  ⚠️ WARNING: Final video is very short (<2s) — merge may have failed silently\n");
            }
            if meta.width == 0 || meta.height == 0 {
                qa.push_str("  ⚠️ WARNING: Could not detect video resolution\n");
            }
        }
        Err(e) => qa.push_str(&format!("  ⚠️ Metadata check failed: {}\n", e)),
    }

    // QA 2: Frozen frames in final output (2.0s window — more lenient after merge transitions)
    match crate::visual::detect_frozen_frames(output_file, -60.0, 2.0) {
        Ok(report) if !report.to_lowercase().contains("no frozen") && report.contains("freeze") => {
            qa.push_str(&format!("  ⚠️ Frozen frames detected: {}\n", &report[..report.len().min(200)]));
        }
        Ok(_) => qa.push_str("  ✓ No frozen frames\n"),
        Err(e) => qa.push_str(&format!("  ⚠️ Frozen frame check error: {}\n", e)),
    }

    // QA 3: Black frames in final output
    match crate::visual::detect_black_frames(output_file, 2.0, 0.98, 0.10) {
        Ok(report) if !report.to_lowercase().contains("no black") && report.contains("black_start") => {
            qa.push_str(&format!("  ⚠️ Black frames detected: {}\n", &report[..report.len().min(200)]));
        }
        Ok(_) => qa.push_str("  ✓ No black frames\n"),
        Err(e) => qa.push_str(&format!("  ⚠️ Black frame check error: {}\n", e)),
    }

    // QA 4: Scene change count (verifies editing continuity)
    match crate::core::detect_scene_changes(output_file, 40.0) {
        Ok(report) => qa.push_str(&format!("  • Scene analysis: {}\n", &report[..report.len().min(300)])),
        Err(e) => qa.push_str(&format!("  ⚠️ Scene analysis error: {}\n", e)),
    }

    qa
}

/// AI-powered search query generation using Gemini for visual diversity
async fn generate_search_queries_ai(topic: &str, num_queries: usize) -> Vec<String> {
    let api_key = std::env::var("GEMINI_API_KEY")
        .or_else(|_| std::env::var("GOOGLE_API_KEY"))
        .unwrap_or_default();
    if api_key.is_empty() {
        return generate_search_queries_fallback(topic, num_queries);
    }
    let client = crate::gemini_client::GeminiClient::new(api_key);
    let prompt = format!(
        "Generate exactly {num_queries} diverse Pexels stock video search queries for this topic: \"{topic}\".\n\
        Rules:\n\
        - Each query is 2-4 words, specific and visual (something a camera would capture)\n\
        - Cover different visual aspects of the topic across queries for variety\n\
        - Avoid abstract concepts; focus on concrete visual scenes\n\
        - Output ONLY the queries, one per line, no numbering, no extra text."
    );
    match client.generate_text(&prompt).await {
        Ok(response) => {
            let queries: Vec<String> = response
                .lines()
                .map(|l| l.trim().to_string())
                .filter(|l| !l.is_empty() && l.len() > 3)
                .take(num_queries)
                .collect();
            if queries.len() >= num_queries.min(2) {
                tracing::info!("✅ AI generated {} search queries: {:?}", queries.len(), queries.iter().take(3).collect::<Vec<_>>());
                queries
            } else {
                tracing::warn!("⚠️ AI query generation returned too few results, using fallback");
                generate_search_queries_fallback(topic, num_queries)
            }
        }
        Err(e) => {
            tracing::warn!("⚠️ AI query generation failed ({}), using fallback", e);
            generate_search_queries_fallback(topic, num_queries)
        }
    }
}

/// Poll an ElevenLabs music generation task until completed or timeout
async fn poll_music_task(
    client: &crate::elevenlabs_client::ElevenLabsClient,
    task_id: &str,
    max_attempts: u32,
) -> Option<String> {
    for attempt in 0..max_attempts {
        tokio::time::sleep(tokio::time::Duration::from_secs(2)).await;
        match client.get_music_status(task_id).await {
            Ok(status) => match status.status.as_str() {
                "completed" => return status.audio_url,
                "failed" => {
                    tracing::warn!("🎵 Music generation failed at attempt {}", attempt);
                    return None;
                }
                _ => {
                    tracing::debug!("🎵 Music generation in progress... ({}/{})", attempt + 1, max_attempts);
                }
            },
            Err(e) => tracing::warn!("🎵 Music status poll error: {}", e),
        }
    }
    None
}

/// Helper function to generate search queries based on topic with intelligent keyword extraction (fallback)
fn generate_search_queries_fallback(topic: &str, num_queries: usize) -> Vec<String> {
    // INTELLIGENT APPROACH: Extract core concepts and generate DIVERSE queries
    // For "marketplace for social media accounts" → Extract: marketplace, business, digital, technology

    // Extract key concepts (simple heuristic-based for now)
    let keywords = extract_key_concepts(topic);

    // Generate diverse search queries using extracted keywords
    let mut queries = Vec::new();

    // Combine keywords creatively for diversity
    if keywords.iter().any(|k| k == "marketplace" || k == "buying" || k == "selling") {
        queries.push("business meeting professional handshake".to_string());
        queries.push("online shopping ecommerce technology".to_string());
        queries.push("digital marketplace office modern".to_string());
        queries.push("startup entrepreneur presentation".to_string());
        queries.push("business people networking office".to_string());
    } else if keywords.iter().any(|k| k == "technology" || k == "digital" || k == "tech") {
        queries.push("technology digital innovation".to_string());
        queries.push("modern office tech startup".to_string());
        queries.push("computer programming coding".to_string());
    } else if keywords.iter().any(|k| k == "book" || k == "reading" || k == "review") {
        queries.push("person reading book library".to_string());
        queries.push("bookshelf books cozy".to_string());
        queries.push("reading glasses notebook".to_string());
    } else {
        // Generic fallback - extract first few words
        let words: Vec<&str> = topic.split_whitespace().take(3).collect();
        let short_query = words.join(" ");
        queries.push(format!("{} professional", short_query));
        queries.push(format!("{} modern business", short_query));
        queries.push(format!("{} cinematic", short_query));
    }

    // Ensure we have enough queries
    while queries.len() < num_queries {
        queries.push("business professional modern".to_string());
    }

    queries.into_iter().take(num_queries).collect()
}

/// Extract key concepts from topic description
fn extract_key_concepts(topic: &str) -> Vec<String> {
    let topic_lower = topic.to_lowercase();
    let mut concepts = Vec::new();

    // Business keywords
    if topic_lower.contains("marketplace") { concepts.push("marketplace".to_string()); }
    if topic_lower.contains("buying") || topic_lower.contains("selling") { concepts.push("buying".to_string()); }
    if topic_lower.contains("business") { concepts.push("business".to_string()); }
    if topic_lower.contains("professional") { concepts.push("professional".to_string()); }

    // Tech keywords
    if topic_lower.contains("technology") || topic_lower.contains("tech") { concepts.push("technology".to_string()); }
    if topic_lower.contains("digital") { concepts.push("digital".to_string()); }
    if topic_lower.contains("online") { concepts.push("online".to_string()); }
    if topic_lower.contains("social media") { concepts.push("social_media".to_string()); }

    // Content keywords
    if topic_lower.contains("book") || topic_lower.contains("reading") { concepts.push("book".to_string()); }
    if topic_lower.contains("coffee") || topic_lower.contains("cafe") { concepts.push("coffee".to_string()); }
    if topic_lower.contains("fitness") || topic_lower.contains("gym") { concepts.push("fitness".to_string()); }

    concepts
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
async fn execute_view_video_claude(_args: &Value) -> String {
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
async fn execute_view_video_gemini(_args: &HashMap<String, Value>) -> String {
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
async fn execute_review_video_claude(_args: &Value) -> String {
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
async fn execute_review_video_gemini(_args: &HashMap<String, Value>) -> String {
    format!("❌ Internal error: review_video must be called with context")
}

// ============================================================================
// IMAGE VIEWING TOOLS
// ============================================================================

/// View image placeholder - calls context version
async fn execute_view_image_claude(_args: &Value) -> String {
    format!("❌ Internal error: view_image must be called with context")
}

/// View image placeholder - calls context version
async fn execute_view_image_gemini(_args: &HashMap<String, Value>) -> String {
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

    tracing::info!("🎙️ generate_text_to_speech: Starting TTS generation - Voice: {}, Text length: {} chars", voice, text.len());

    // Try Eleven Labs first if available
    if let Some(ref elevenlabs_client) = ctx.app_state.elevenlabs_client {
        tracing::info!("🎙️ generate_text_to_speech: Using ElevenLabs API");
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

    tracing::info!("🎙️ generate_text_to_speech: Starting TTS generation - Voice: {}, Text length: {} chars", voice, text.len());

    // Try Eleven Labs first if available
    if let Some(ref elevenlabs_client) = ctx.app_state.elevenlabs_client {
        tracing::info!("🎙️ generate_text_to_speech: Using ElevenLabs API");
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

    tracing::info!("🎵 generate_music: Starting music generation - Prompt: '{}', Duration: {}s", prompt, duration_seconds);

    if let Some(ref elevenlabs_client) = ctx.app_state.elevenlabs_client {
        // Step 1: Create music generation task
        tracing::info!("🎵 generate_music: Creating ElevenLabs music generation task...");
        let generation_id = match elevenlabs_client.generate_music_task(prompt, duration_ms).await {
            Ok(id) => {
                tracing::info!("✅ Music generation task created successfully: {}", id);
                id
            },
            Err(e) => {
                tracing::error!("❌ ElevenLabs music generation task creation FAILED: {}", e);
                return format!("❌ Failed to start music generation (ElevenLabs API error): {}\n\n💡 Possible causes:\n- API quota exceeded\n- Rate limiting\n- Service temporarily unavailable\n\nYou can try again later or proceed without music for now.", e);
            },
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
                            tracing::info!("🎵 generate_music: Generation in progress... (attempt {}/{}, {}s elapsed)", attempt + 1, max_attempts, attempt * 2);
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

    tracing::info!("🎵 generate_music: Starting music generation - Prompt: '{}', Duration: {}s", prompt, duration_seconds);

    if let Some(ref elevenlabs_client) = ctx.app_state.elevenlabs_client {
        // Step 1: Create music generation task
        tracing::info!("🎵 generate_music: Creating ElevenLabs music generation task...");
        let generation_id = match elevenlabs_client.generate_music_task(prompt, duration_ms).await {
            Ok(id) => {
                tracing::info!("✅ Music generation task created successfully: {}", id);
                id
            },
            Err(e) => {
                tracing::error!("❌ ElevenLabs music generation task creation FAILED: {}", e);
                return format!("❌ Failed to start music generation (ElevenLabs API error): {}\n\n💡 Possible causes:\n- API quota exceeded\n- Rate limiting\n- Service temporarily unavailable\n\nYou can try again later or proceed without music for now.", e);
            },
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

    tracing::info!("🎙️ add_voiceover_to_video: Starting voiceover addition - Voice: {}, Text length: {} chars", voice, voiceover_text.len());

    // Step 1: Generate voiceover audio
    tracing::info!("🎙️ add_voiceover_to_video: Step 1/2 - Generating TTS audio");
    let temp_audio = format!("outputs/temp_voiceover_{}.mp3", uuid::Uuid::new_v4());

    let tts_args = serde_json::json!({
        "text": voiceover_text,
        "output_file": &temp_audio,
        "voice": voice,
    });

    let tts_result = execute_generate_text_to_speech_with_state_claude(&tts_args, ctx).await;
    if tts_result.starts_with("❌") {
        tracing::error!("❌ add_voiceover_to_video: TTS generation failed - {}", tts_result);
        return format!("❌ Failed to generate voiceover: {}", tts_result);
    }
    tracing::info!("✅ add_voiceover_to_video: TTS audio generated successfully");

    // Step 2: Add audio to video using FFmpeg
    tracing::info!("🎙️ add_voiceover_to_video: Step 2/2 - Adding audio track to video");
    let add_audio_args = serde_json::json!({
        "input_file": input_video,
        "audio_file": &temp_audio,
        "output_file": output_video,
    });

    let result = execute_add_audio_claude(&add_audio_args);

    // Clean up temp audio file
    tracing::info!("🧹 add_voiceover_to_video: Cleaning up temporary audio file");
    let _ = tokio::fs::remove_file(&temp_audio).await;

    if result.starts_with("❌") {
        tracing::error!("❌ add_voiceover_to_video: Failed to add audio - {}", result);
        format!("❌ Failed to add voiceover to video: {}", result)
    } else {
        tracing::info!("✅ add_voiceover_to_video: Successfully completed - Output: {}", output_video);
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

    tracing::info!("🎙️ add_voiceover_to_video: Starting voiceover addition - Voice: {}, Text length: {} chars", voice, voiceover_text.len());

    // Step 1: Generate voiceover audio
    tracing::info!("🎙️ add_voiceover_to_video: Step 1/2 - Generating TTS audio");
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
    tracing::info!("🧹 add_voiceover_to_video: Cleaning up temporary audio file");
    let _ = tokio::fs::remove_file(&temp_audio).await;

    if result.starts_with("❌") {
        tracing::error!("❌ add_voiceover_to_video: Failed to add audio - {}", result);
        format!("❌ Failed to add voiceover to video: {}", result)
    } else {
        tracing::info!("✅ add_voiceover_to_video: Successfully completed - Output: {}", output_video);
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
                system_instruction: None,
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
    _ctx: &ToolExecutionContext,
) -> String {
    let video_id = args["video_id"].as_str().unwrap_or("");
    let _days = args.get("date_range_days").and_then(|v| v.as_i64()).unwrap_or(30).min(365) as i32;

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
    _args: &Value,
    _ctx: &ToolExecutionContext,
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

// ============================================================================
// NEW TOOL EXECUTORS — BATCH 1 (Wire existing Rust functions)
// ============================================================================

fn execute_create_thumbnail_hd_claude(args: &Value) -> String {
    let input = args["input_file"].as_str().unwrap_or("");
    let output_raw = args["output_file"].as_str().unwrap_or("");
    let output = ensure_outputs_directory(output_raw);
    let timestamp = args.get("timestamp").and_then(|v| v.as_f64()).unwrap_or(0.0);
    let width = args.get("width").and_then(|v| v.as_u64()).unwrap_or(1280) as u32;
    let height = args.get("height").and_then(|v| v.as_u64()).unwrap_or(720) as u32;
    crate::transform::create_thumbnail_scaled(input, &output, timestamp, width, height).unwrap_or_else(|e| e)
}

fn execute_create_thumbnail_hd_gemini(args: &HashMap<String, Value>) -> String {
    let input = args.get("input_file").and_then(|v| v.as_str()).unwrap_or("");
    let output_raw = args.get("output_file").and_then(|v| v.as_str()).unwrap_or("");
    let output = ensure_outputs_directory(output_raw);
    let timestamp = args.get("timestamp").and_then(|v| v.as_f64()).unwrap_or(0.0);
    let width = args.get("width").and_then(|v| v.as_u64()).unwrap_or(1280) as u32;
    let height = args.get("height").and_then(|v| v.as_u64()).unwrap_or(720) as u32;
    crate::transform::create_thumbnail_scaled(input, &output, timestamp, width, height).unwrap_or_else(|e| e)
}

fn execute_get_video_duration_claude(args: &Value) -> String {
    let input = args["input_file"].as_str().unwrap_or("");
    match crate::core::get_video_duration(input) {
        Ok(dur) => format!("Duration: {} seconds", dur),
        Err(e) => e,
    }
}

fn execute_get_video_duration_gemini(args: &HashMap<String, Value>) -> String {
    let input = args.get("input_file").and_then(|v| v.as_str()).unwrap_or("");
    match crate::core::get_video_duration(input) {
        Ok(dur) => format!("Duration: {} seconds", dur),
        Err(e) => e,
    }
}

// ============================================================================
// NEW TOOL EXECUTORS — BATCH 2 (Color Grading)
// ============================================================================

fn execute_adjust_hue_claude(args: &Value) -> String {
    let input = args["input_file"].as_str().unwrap_or("");
    let output_raw = args["output_file"].as_str().unwrap_or("");
    let output = ensure_outputs_directory(output_raw);
    let hue_degrees = args.get("hue_degrees").and_then(|v| v.as_f64()).unwrap_or(0.0);
    let saturation_factor = args.get("saturation_factor").and_then(|v| v.as_f64()).unwrap_or(1.0);
    crate::visual::adjust_hue(input, &output, hue_degrees, saturation_factor).unwrap_or_else(|e| e)
}

fn execute_adjust_hue_gemini(args: &HashMap<String, Value>) -> String {
    let input = args.get("input_file").and_then(|v| v.as_str()).unwrap_or("");
    let output_raw = args.get("output_file").and_then(|v| v.as_str()).unwrap_or("");
    let output = ensure_outputs_directory(output_raw);
    let hue_degrees = args.get("hue_degrees").and_then(|v| v.as_f64()).unwrap_or(0.0);
    let saturation_factor = args.get("saturation_factor").and_then(|v| v.as_f64()).unwrap_or(1.0);
    crate::visual::adjust_hue(input, &output, hue_degrees, saturation_factor).unwrap_or_else(|e| e)
}

fn execute_color_balance_claude(args: &Value) -> String {
    let input = args["input_file"].as_str().unwrap_or("");
    let output_raw = args["output_file"].as_str().unwrap_or("");
    let output = ensure_outputs_directory(output_raw);
    let shadows = (
        args.get("shadows_r").and_then(|v| v.as_f64()).unwrap_or(0.0),
        args.get("shadows_g").and_then(|v| v.as_f64()).unwrap_or(0.0),
        args.get("shadows_b").and_then(|v| v.as_f64()).unwrap_or(0.0),
    );
    let midtones = (
        args.get("midtones_r").and_then(|v| v.as_f64()).unwrap_or(0.0),
        args.get("midtones_g").and_then(|v| v.as_f64()).unwrap_or(0.0),
        args.get("midtones_b").and_then(|v| v.as_f64()).unwrap_or(0.0),
    );
    let highlights = (
        args.get("highlights_r").and_then(|v| v.as_f64()).unwrap_or(0.0),
        args.get("highlights_g").and_then(|v| v.as_f64()).unwrap_or(0.0),
        args.get("highlights_b").and_then(|v| v.as_f64()).unwrap_or(0.0),
    );
    crate::visual::color_balance(input, &output, shadows, midtones, highlights).unwrap_or_else(|e| e)
}

fn execute_color_balance_gemini(args: &HashMap<String, Value>) -> String {
    let input = args.get("input_file").and_then(|v| v.as_str()).unwrap_or("");
    let output_raw = args.get("output_file").and_then(|v| v.as_str()).unwrap_or("");
    let output = ensure_outputs_directory(output_raw);
    let shadows = (
        args.get("shadows_r").and_then(|v| v.as_f64()).unwrap_or(0.0),
        args.get("shadows_g").and_then(|v| v.as_f64()).unwrap_or(0.0),
        args.get("shadows_b").and_then(|v| v.as_f64()).unwrap_or(0.0),
    );
    let midtones = (
        args.get("midtones_r").and_then(|v| v.as_f64()).unwrap_or(0.0),
        args.get("midtones_g").and_then(|v| v.as_f64()).unwrap_or(0.0),
        args.get("midtones_b").and_then(|v| v.as_f64()).unwrap_or(0.0),
    );
    let highlights = (
        args.get("highlights_r").and_then(|v| v.as_f64()).unwrap_or(0.0),
        args.get("highlights_g").and_then(|v| v.as_f64()).unwrap_or(0.0),
        args.get("highlights_b").and_then(|v| v.as_f64()).unwrap_or(0.0),
    );
    crate::visual::color_balance(input, &output, shadows, midtones, highlights).unwrap_or_else(|e| e)
}

fn execute_normalize_video_claude(args: &Value) -> String {
    let input = args["input_file"].as_str().unwrap_or("");
    let output_raw = args["output_file"].as_str().unwrap_or("");
    let output = ensure_outputs_directory(output_raw);
    let smoothing = args.get("smoothing").and_then(|v| v.as_u64()).unwrap_or(0) as u32;
    crate::visual::normalize_video(input, &output, smoothing).unwrap_or_else(|e| e)
}

fn execute_normalize_video_gemini(args: &HashMap<String, Value>) -> String {
    let input = args.get("input_file").and_then(|v| v.as_str()).unwrap_or("");
    let output_raw = args.get("output_file").and_then(|v| v.as_str()).unwrap_or("");
    let output = ensure_outputs_directory(output_raw);
    let smoothing = args.get("smoothing").and_then(|v| v.as_u64()).unwrap_or(0) as u32;
    crate::visual::normalize_video(input, &output, smoothing).unwrap_or_else(|e| e)
}

fn execute_apply_lut_claude(args: &Value) -> String {
    let input = args["input_file"].as_str().unwrap_or("");
    let output_raw = args["output_file"].as_str().unwrap_or("");
    let output = ensure_outputs_directory(output_raw);
    let lut_file = args.get("lut_file").and_then(|v| v.as_str()).unwrap_or("");
    let interp = args.get("interp").and_then(|v| v.as_str()).unwrap_or("tetrahedral");
    crate::visual::apply_lut(input, &output, lut_file, interp).unwrap_or_else(|e| e)
}

fn execute_apply_lut_gemini(args: &HashMap<String, Value>) -> String {
    let input = args.get("input_file").and_then(|v| v.as_str()).unwrap_or("");
    let output_raw = args.get("output_file").and_then(|v| v.as_str()).unwrap_or("");
    let output = ensure_outputs_directory(output_raw);
    let lut_file = args.get("lut_file").and_then(|v| v.as_str()).unwrap_or("");
    let interp = args.get("interp").and_then(|v| v.as_str()).unwrap_or("tetrahedral");
    crate::visual::apply_lut(input, &output, lut_file, interp).unwrap_or_else(|e| e)
}

// ============================================================================
// NEW TOOL EXECUTORS — BATCH 3 (Denoising & Sharpening)
// ============================================================================

fn execute_denoise_video_claude(args: &Value) -> String {
    let input = args["input_file"].as_str().unwrap_or("");
    let output_raw = args["output_file"].as_str().unwrap_or("");
    let output = ensure_outputs_directory(output_raw);
    let luma_spatial = args.get("luma_spatial").and_then(|v| v.as_f64()).unwrap_or(4.0);
    let luma_temporal = args.get("luma_temporal").and_then(|v| v.as_f64()).unwrap_or(6.0);
    let chroma_spatial = args.get("chroma_spatial").and_then(|v| v.as_f64()).unwrap_or(3.0);
    let chroma_temporal = args.get("chroma_temporal").and_then(|v| v.as_f64()).unwrap_or(4.5);
    crate::visual::denoise_video(input, &output, luma_spatial, luma_temporal, chroma_spatial, chroma_temporal).unwrap_or_else(|e| e)
}

fn execute_denoise_video_gemini(args: &HashMap<String, Value>) -> String {
    let input = args.get("input_file").and_then(|v| v.as_str()).unwrap_or("");
    let output_raw = args.get("output_file").and_then(|v| v.as_str()).unwrap_or("");
    let output = ensure_outputs_directory(output_raw);
    let luma_spatial = args.get("luma_spatial").and_then(|v| v.as_f64()).unwrap_or(4.0);
    let luma_temporal = args.get("luma_temporal").and_then(|v| v.as_f64()).unwrap_or(6.0);
    let chroma_spatial = args.get("chroma_spatial").and_then(|v| v.as_f64()).unwrap_or(3.0);
    let chroma_temporal = args.get("chroma_temporal").and_then(|v| v.as_f64()).unwrap_or(4.5);
    crate::visual::denoise_video(input, &output, luma_spatial, luma_temporal, chroma_spatial, chroma_temporal).unwrap_or_else(|e| e)
}

fn execute_unsharp_mask_claude(args: &Value) -> String {
    let input = args["input_file"].as_str().unwrap_or("");
    let output_raw = args["output_file"].as_str().unwrap_or("");
    let output = ensure_outputs_directory(output_raw);
    let luma_msize_x = args.get("luma_msize_x").and_then(|v| v.as_u64()).unwrap_or(5) as u32;
    let luma_msize_y = args.get("luma_msize_y").and_then(|v| v.as_u64()).unwrap_or(5) as u32;
    let luma_amount = args.get("luma_amount").and_then(|v| v.as_f64()).unwrap_or(1.0);
    crate::visual::unsharp_mask(input, &output, luma_msize_x, luma_msize_y, luma_amount).unwrap_or_else(|e| e)
}

fn execute_unsharp_mask_gemini(args: &HashMap<String, Value>) -> String {
    let input = args.get("input_file").and_then(|v| v.as_str()).unwrap_or("");
    let output_raw = args.get("output_file").and_then(|v| v.as_str()).unwrap_or("");
    let output = ensure_outputs_directory(output_raw);
    let luma_msize_x = args.get("luma_msize_x").and_then(|v| v.as_u64()).unwrap_or(5) as u32;
    let luma_msize_y = args.get("luma_msize_y").and_then(|v| v.as_u64()).unwrap_or(5) as u32;
    let luma_amount = args.get("luma_amount").and_then(|v| v.as_f64()).unwrap_or(1.0);
    crate::visual::unsharp_mask(input, &output, luma_msize_x, luma_msize_y, luma_amount).unwrap_or_else(|e| e)
}

fn execute_reduce_noise_claude(args: &Value) -> String {
    let input = args["input_file"].as_str().unwrap_or("");
    let output_raw = args["output_file"].as_str().unwrap_or("");
    let output = ensure_outputs_directory(output_raw);
    let strength = args.get("strength").and_then(|v| v.as_f64()).unwrap_or(8.0);
    let research_size = args.get("research_size").and_then(|v| v.as_u64()).unwrap_or(15) as u32;
    let patch_size = args.get("patch_size").and_then(|v| v.as_u64()).unwrap_or(7) as u32;
    crate::visual::reduce_noise(input, &output, strength, research_size, patch_size).unwrap_or_else(|e| e)
}

fn execute_reduce_noise_gemini(args: &HashMap<String, Value>) -> String {
    let input = args.get("input_file").and_then(|v| v.as_str()).unwrap_or("");
    let output_raw = args.get("output_file").and_then(|v| v.as_str()).unwrap_or("");
    let output = ensure_outputs_directory(output_raw);
    let strength = args.get("strength").and_then(|v| v.as_f64()).unwrap_or(8.0);
    let research_size = args.get("research_size").and_then(|v| v.as_u64()).unwrap_or(15) as u32;
    let patch_size = args.get("patch_size").and_then(|v| v.as_u64()).unwrap_or(7) as u32;
    crate::visual::reduce_noise(input, &output, strength, research_size, patch_size).unwrap_or_else(|e| e)
}

// ============================================================================
// NEW TOOL EXECUTORS — BATCH 4 (Audio Processing)
// ============================================================================

fn execute_compress_audio_claude(args: &Value) -> String {
    let input = args["input_file"].as_str().unwrap_or("");
    let output_raw = args["output_file"].as_str().unwrap_or("");
    let output = ensure_outputs_directory(output_raw);
    let threshold_db = args.get("threshold_db").and_then(|v| v.as_f64()).unwrap_or(-20.0);
    let ratio = args.get("ratio").and_then(|v| v.as_f64()).unwrap_or(4.0);
    let attack_ms = args.get("attack_ms").and_then(|v| v.as_f64()).unwrap_or(20.0);
    let release_ms = args.get("release_ms").and_then(|v| v.as_f64()).unwrap_or(250.0);
    let makeup_gain_db = args.get("makeup_gain_db").and_then(|v| v.as_f64()).unwrap_or(0.0);
    crate::audio::compress_audio(input, &output, threshold_db, ratio, attack_ms, release_ms, makeup_gain_db).unwrap_or_else(|e| e)
}

fn execute_compress_audio_gemini(args: &HashMap<String, Value>) -> String {
    let input = args.get("input_file").and_then(|v| v.as_str()).unwrap_or("");
    let output_raw = args.get("output_file").and_then(|v| v.as_str()).unwrap_or("");
    let output = ensure_outputs_directory(output_raw);
    let threshold_db = args.get("threshold_db").and_then(|v| v.as_f64()).unwrap_or(-20.0);
    let ratio = args.get("ratio").and_then(|v| v.as_f64()).unwrap_or(4.0);
    let attack_ms = args.get("attack_ms").and_then(|v| v.as_f64()).unwrap_or(20.0);
    let release_ms = args.get("release_ms").and_then(|v| v.as_f64()).unwrap_or(250.0);
    let makeup_gain_db = args.get("makeup_gain_db").and_then(|v| v.as_f64()).unwrap_or(0.0);
    crate::audio::compress_audio(input, &output, threshold_db, ratio, attack_ms, release_ms, makeup_gain_db).unwrap_or_else(|e| e)
}

fn execute_normalize_audio_claude(args: &Value) -> String {
    let input = args["input_file"].as_str().unwrap_or("");
    let output_raw = args["output_file"].as_str().unwrap_or("");
    let output = ensure_outputs_directory(output_raw);
    let target_lufs = args.get("target_lufs").and_then(|v| v.as_f64()).unwrap_or(-16.0);
    let loudness_range_target = args.get("loudness_range_target").and_then(|v| v.as_f64()).unwrap_or(11.0);
    let true_peak_dbtp = args.get("true_peak_dbtp").and_then(|v| v.as_f64()).unwrap_or(-1.0);
    crate::audio::normalize_audio(input, &output, target_lufs, loudness_range_target, true_peak_dbtp).unwrap_or_else(|e| e)
}

fn execute_normalize_audio_gemini(args: &HashMap<String, Value>) -> String {
    let input = args.get("input_file").and_then(|v| v.as_str()).unwrap_or("");
    let output_raw = args.get("output_file").and_then(|v| v.as_str()).unwrap_or("");
    let output = ensure_outputs_directory(output_raw);
    let target_lufs = args.get("target_lufs").and_then(|v| v.as_f64()).unwrap_or(-16.0);
    let loudness_range_target = args.get("loudness_range_target").and_then(|v| v.as_f64()).unwrap_or(11.0);
    let true_peak_dbtp = args.get("true_peak_dbtp").and_then(|v| v.as_f64()).unwrap_or(-1.0);
    crate::audio::normalize_audio(input, &output, target_lufs, loudness_range_target, true_peak_dbtp).unwrap_or_else(|e| e)
}

fn execute_equalize_audio_claude(args: &Value) -> String {
    let input = args["input_file"].as_str().unwrap_or("");
    let output_raw = args["output_file"].as_str().unwrap_or("");
    let output = ensure_outputs_directory(output_raw);
    let frequency_hz = args.get("frequency_hz").and_then(|v| v.as_f64()).unwrap_or(1000.0);
    let gain_db = args.get("gain_db").and_then(|v| v.as_f64()).unwrap_or(0.0);
    let bandwidth_hz = args.get("bandwidth_hz").and_then(|v| v.as_f64()).unwrap_or(200.0);
    let eq_type = args.get("eq_type").and_then(|v| v.as_str()).unwrap_or("peak");
    crate::audio::equalize_audio(input, &output, frequency_hz, gain_db, bandwidth_hz, eq_type).unwrap_or_else(|e| e)
}

fn execute_equalize_audio_gemini(args: &HashMap<String, Value>) -> String {
    let input = args.get("input_file").and_then(|v| v.as_str()).unwrap_or("");
    let output_raw = args.get("output_file").and_then(|v| v.as_str()).unwrap_or("");
    let output = ensure_outputs_directory(output_raw);
    let frequency_hz = args.get("frequency_hz").and_then(|v| v.as_f64()).unwrap_or(1000.0);
    let gain_db = args.get("gain_db").and_then(|v| v.as_f64()).unwrap_or(0.0);
    let bandwidth_hz = args.get("bandwidth_hz").and_then(|v| v.as_f64()).unwrap_or(200.0);
    let eq_type = args.get("eq_type").and_then(|v| v.as_str()).unwrap_or("peak");
    crate::audio::equalize_audio(input, &output, frequency_hz, gain_db, bandwidth_hz, eq_type).unwrap_or_else(|e| e)
}

fn execute_gate_audio_claude(args: &Value) -> String {
    let input = args["input_file"].as_str().unwrap_or("");
    let output_raw = args["output_file"].as_str().unwrap_or("");
    let output = ensure_outputs_directory(output_raw);
    let threshold_db = args.get("threshold_db").and_then(|v| v.as_f64()).unwrap_or(-40.0);
    let ratio = args.get("ratio").and_then(|v| v.as_f64()).unwrap_or(2.0);
    let attack_ms = args.get("attack_ms").and_then(|v| v.as_f64()).unwrap_or(20.0);
    let release_ms = args.get("release_ms").and_then(|v| v.as_f64()).unwrap_or(250.0);
    crate::audio::gate_audio(input, &output, threshold_db, ratio, attack_ms, release_ms).unwrap_or_else(|e| e)
}

fn execute_gate_audio_gemini(args: &HashMap<String, Value>) -> String {
    let input = args.get("input_file").and_then(|v| v.as_str()).unwrap_or("");
    let output_raw = args.get("output_file").and_then(|v| v.as_str()).unwrap_or("");
    let output = ensure_outputs_directory(output_raw);
    let threshold_db = args.get("threshold_db").and_then(|v| v.as_f64()).unwrap_or(-40.0);
    let ratio = args.get("ratio").and_then(|v| v.as_f64()).unwrap_or(2.0);
    let attack_ms = args.get("attack_ms").and_then(|v| v.as_f64()).unwrap_or(20.0);
    let release_ms = args.get("release_ms").and_then(|v| v.as_f64()).unwrap_or(250.0);
    crate::audio::gate_audio(input, &output, threshold_db, ratio, attack_ms, release_ms).unwrap_or_else(|e| e)
}

fn execute_denoise_audio_claude(args: &Value) -> String {
    let input = args["input_file"].as_str().unwrap_or("");
    let output_raw = args["output_file"].as_str().unwrap_or("");
    let output = ensure_outputs_directory(output_raw);
    let noise_floor_db = args.get("noise_floor_db").and_then(|v| v.as_f64()).unwrap_or(-40.0);
    let noise_reduction_db = args.get("noise_reduction_db").and_then(|v| v.as_f64()).unwrap_or(12.0);
    let track_noise = args.get("track_noise").and_then(|v| v.as_bool()).unwrap_or(false);
    crate::audio::denoise_audio(input, &output, noise_floor_db, noise_reduction_db, track_noise).unwrap_or_else(|e| e)
}

fn execute_denoise_audio_gemini(args: &HashMap<String, Value>) -> String {
    let input = args.get("input_file").and_then(|v| v.as_str()).unwrap_or("");
    let output_raw = args.get("output_file").and_then(|v| v.as_str()).unwrap_or("");
    let output = ensure_outputs_directory(output_raw);
    let noise_floor_db = args.get("noise_floor_db").and_then(|v| v.as_f64()).unwrap_or(-40.0);
    let noise_reduction_db = args.get("noise_reduction_db").and_then(|v| v.as_f64()).unwrap_or(12.0);
    let track_noise = args.get("track_noise").and_then(|v| v.as_bool()).unwrap_or(false);
    crate::audio::denoise_audio(input, &output, noise_floor_db, noise_reduction_db, track_noise).unwrap_or_else(|e| e)
}

// ============================================================================
// NEW TOOL EXECUTORS — BATCH 5 (Video Composition & Layout)
// ============================================================================

fn execute_pad_video_claude(args: &Value) -> String {
    let input = args["input_file"].as_str().unwrap_or("");
    let output_raw = args["output_file"].as_str().unwrap_or("");
    let output = ensure_outputs_directory(output_raw);
    let width = args.get("width").and_then(|v| v.as_u64()).unwrap_or(1920) as u32;
    let height = args.get("height").and_then(|v| v.as_u64()).unwrap_or(1080) as u32;
    let x_offset = args.get("x_offset").and_then(|v| v.as_u64()).unwrap_or(0) as u32;
    let y_offset = args.get("y_offset").and_then(|v| v.as_u64()).unwrap_or(0) as u32;
    let color = args.get("color").and_then(|v| v.as_str()).unwrap_or("black");
    crate::visual::pad_video(input, &output, width, height, x_offset, y_offset, color).unwrap_or_else(|e| e)
}

fn execute_pad_video_gemini(args: &HashMap<String, Value>) -> String {
    let input = args.get("input_file").and_then(|v| v.as_str()).unwrap_or("");
    let output_raw = args.get("output_file").and_then(|v| v.as_str()).unwrap_or("");
    let output = ensure_outputs_directory(output_raw);
    let width = args.get("width").and_then(|v| v.as_u64()).unwrap_or(1920) as u32;
    let height = args.get("height").and_then(|v| v.as_u64()).unwrap_or(1080) as u32;
    let x_offset = args.get("x_offset").and_then(|v| v.as_u64()).unwrap_or(0) as u32;
    let y_offset = args.get("y_offset").and_then(|v| v.as_u64()).unwrap_or(0) as u32;
    let color = args.get("color").and_then(|v| v.as_str()).unwrap_or("black");
    crate::visual::pad_video(input, &output, width, height, x_offset, y_offset, color).unwrap_or_else(|e| e)
}

fn execute_blend_videos_claude(args: &Value) -> String {
    let input1 = args.get("input_file1").and_then(|v| v.as_str()).unwrap_or("");
    let input2 = args.get("input_file2").and_then(|v| v.as_str()).unwrap_or("");
    let output_raw = args["output_file"].as_str().unwrap_or("");
    let output = ensure_outputs_directory(output_raw);
    let blend_mode = args.get("blend_mode").and_then(|v| v.as_str()).unwrap_or("overlay");
    let opacity = args.get("opacity").and_then(|v| v.as_f64()).unwrap_or(1.0);
    crate::visual::blend_videos(input1, input2, &output, blend_mode, opacity).unwrap_or_else(|e| e)
}

fn execute_blend_videos_gemini(args: &HashMap<String, Value>) -> String {
    let input1 = args.get("input_file1").and_then(|v| v.as_str()).unwrap_or("");
    let input2 = args.get("input_file2").and_then(|v| v.as_str()).unwrap_or("");
    let output_raw = args.get("output_file").and_then(|v| v.as_str()).unwrap_or("");
    let output = ensure_outputs_directory(output_raw);
    let blend_mode = args.get("blend_mode").and_then(|v| v.as_str()).unwrap_or("overlay");
    let opacity = args.get("opacity").and_then(|v| v.as_f64()).unwrap_or(1.0);
    crate::visual::blend_videos(input1, input2, &output, blend_mode, opacity).unwrap_or_else(|e| e)
}

fn execute_stack_videos_claude(args: &Value) -> String {
    let input1 = args.get("input_file1").and_then(|v| v.as_str()).unwrap_or("");
    let input2 = args.get("input_file2").and_then(|v| v.as_str()).unwrap_or("");
    let output_raw = args["output_file"].as_str().unwrap_or("");
    let output = ensure_outputs_directory(output_raw);
    let direction = args.get("direction").and_then(|v| v.as_str()).unwrap_or("horizontal");
    crate::visual::stack_videos(input1, input2, &output, direction).unwrap_or_else(|e| e)
}

fn execute_stack_videos_gemini(args: &HashMap<String, Value>) -> String {
    let input1 = args.get("input_file1").and_then(|v| v.as_str()).unwrap_or("");
    let input2 = args.get("input_file2").and_then(|v| v.as_str()).unwrap_or("");
    let output_raw = args.get("output_file").and_then(|v| v.as_str()).unwrap_or("");
    let output = ensure_outputs_directory(output_raw);
    let direction = args.get("direction").and_then(|v| v.as_str()).unwrap_or("horizontal");
    crate::visual::stack_videos(input1, input2, &output, direction).unwrap_or_else(|e| e)
}

fn execute_add_vignette_claude(args: &Value) -> String {
    let input = args["input_file"].as_str().unwrap_or("");
    let output_raw = args["output_file"].as_str().unwrap_or("");
    let output = ensure_outputs_directory(output_raw);
    let angle = args.get("angle").and_then(|v| v.as_f64()).unwrap_or(std::f64::consts::PI / 4.0);
    let mode = args.get("mode").and_then(|v| v.as_str()).unwrap_or("forward");
    crate::visual::add_vignette(input, &output, angle, mode).unwrap_or_else(|e| e)
}

fn execute_add_vignette_gemini(args: &HashMap<String, Value>) -> String {
    let input = args.get("input_file").and_then(|v| v.as_str()).unwrap_or("");
    let output_raw = args.get("output_file").and_then(|v| v.as_str()).unwrap_or("");
    let output = ensure_outputs_directory(output_raw);
    let angle = args.get("angle").and_then(|v| v.as_f64()).unwrap_or(std::f64::consts::PI / 4.0);
    let mode = args.get("mode").and_then(|v| v.as_str()).unwrap_or("forward");
    crate::visual::add_vignette(input, &output, angle, mode).unwrap_or_else(|e| e)
}

fn execute_draw_box_claude(args: &Value) -> String {
    let input = args["input_file"].as_str().unwrap_or("");
    let output_raw = args["output_file"].as_str().unwrap_or("");
    let output = ensure_outputs_directory(output_raw);
    let x = args.get("x").and_then(|v| v.as_u64()).unwrap_or(0) as u32;
    let y = args.get("y").and_then(|v| v.as_u64()).unwrap_or(0) as u32;
    let width = args.get("width").and_then(|v| v.as_u64()).unwrap_or(100) as u32;
    let height = args.get("height").and_then(|v| v.as_u64()).unwrap_or(100) as u32;
    let color = args.get("color").and_then(|v| v.as_str()).unwrap_or("white");
    let thickness = args.get("thickness").and_then(|v| v.as_i64()).unwrap_or(2) as i32;
    crate::visual::draw_box(input, &output, x, y, width, height, color, thickness).unwrap_or_else(|e| e)
}

fn execute_draw_box_gemini(args: &HashMap<String, Value>) -> String {
    let input = args.get("input_file").and_then(|v| v.as_str()).unwrap_or("");
    let output_raw = args.get("output_file").and_then(|v| v.as_str()).unwrap_or("");
    let output = ensure_outputs_directory(output_raw);
    let x = args.get("x").and_then(|v| v.as_u64()).unwrap_or(0) as u32;
    let y = args.get("y").and_then(|v| v.as_u64()).unwrap_or(0) as u32;
    let width = args.get("width").and_then(|v| v.as_u64()).unwrap_or(100) as u32;
    let height = args.get("height").and_then(|v| v.as_u64()).unwrap_or(100) as u32;
    let color = args.get("color").and_then(|v| v.as_str()).unwrap_or("white");
    let thickness = args.get("thickness").and_then(|v| v.as_i64()).unwrap_or(2) as i32;
    crate::visual::draw_box(input, &output, x, y, width, height, color, thickness).unwrap_or_else(|e| e)
}

// ============================================================================
// NEW TOOL EXECUTORS — BATCH 6 (Motion, Time & Frame Effects)
// ============================================================================

fn execute_reverse_video_claude(args: &Value) -> String {
    let input = args["input_file"].as_str().unwrap_or("");
    let output_raw = args["output_file"].as_str().unwrap_or("");
    let output = ensure_outputs_directory(output_raw);
    crate::transform::reverse_video(input, &output).unwrap_or_else(|e| e)
}

fn execute_reverse_video_gemini(args: &HashMap<String, Value>) -> String {
    let input = args.get("input_file").and_then(|v| v.as_str()).unwrap_or("");
    let output_raw = args.get("output_file").and_then(|v| v.as_str()).unwrap_or("");
    let output = ensure_outputs_directory(output_raw);
    crate::transform::reverse_video(input, &output).unwrap_or_else(|e| e)
}

fn execute_loop_video_claude(args: &Value) -> String {
    let input = args["input_file"].as_str().unwrap_or("");
    let output_raw = args["output_file"].as_str().unwrap_or("");
    let output = ensure_outputs_directory(output_raw);
    let loop_count = args.get("loop_count").and_then(|v| v.as_i64()).unwrap_or(2) as i32;
    let loop_duration_sec = args.get("loop_duration_sec").and_then(|v| v.as_f64()).unwrap_or(30.0);
    crate::transform::loop_video(input, &output, loop_count, loop_duration_sec).unwrap_or_else(|e| e)
}

fn execute_loop_video_gemini(args: &HashMap<String, Value>) -> String {
    let input = args.get("input_file").and_then(|v| v.as_str()).unwrap_or("");
    let output_raw = args.get("output_file").and_then(|v| v.as_str()).unwrap_or("");
    let output = ensure_outputs_directory(output_raw);
    let loop_count = args.get("loop_count").and_then(|v| v.as_i64()).unwrap_or(2) as i32;
    let loop_duration_sec = args.get("loop_duration_sec").and_then(|v| v.as_f64()).unwrap_or(30.0);
    crate::transform::loop_video(input, &output, loop_count, loop_duration_sec).unwrap_or_else(|e| e)
}

fn execute_zoompan_claude(args: &Value) -> String {
    let input = args["input_file"].as_str().unwrap_or("");
    let output_raw = args["output_file"].as_str().unwrap_or("");
    let output = ensure_outputs_directory(output_raw);
    let zoom_factor = args.get("zoom_factor").and_then(|v| v.as_f64()).unwrap_or(1.5);
    let x_expr = args.get("x_expr").and_then(|v| v.as_str()).unwrap_or("iw/2-(iw/zoom/2)");
    let y_expr = args.get("y_expr").and_then(|v| v.as_str()).unwrap_or("ih/2-(ih/zoom/2)");
    let duration_frames = args.get("duration_frames").and_then(|v| v.as_u64()).unwrap_or(125) as u32;
    let fps = args.get("fps").and_then(|v| v.as_u64()).unwrap_or(25) as u32;
    crate::visual::zoompan(input, &output, zoom_factor, x_expr, y_expr, duration_frames, fps).unwrap_or_else(|e| e)
}

fn execute_zoompan_gemini(args: &HashMap<String, Value>) -> String {
    let input = args.get("input_file").and_then(|v| v.as_str()).unwrap_or("");
    let output_raw = args.get("output_file").and_then(|v| v.as_str()).unwrap_or("");
    let output = ensure_outputs_directory(output_raw);
    let zoom_factor = args.get("zoom_factor").and_then(|v| v.as_f64()).unwrap_or(1.5);
    let x_expr = args.get("x_expr").and_then(|v| v.as_str()).unwrap_or("iw/2-(iw/zoom/2)");
    let y_expr = args.get("y_expr").and_then(|v| v.as_str()).unwrap_or("ih/2-(ih/zoom/2)");
    let duration_frames = args.get("duration_frames").and_then(|v| v.as_u64()).unwrap_or(125) as u32;
    let fps = args.get("fps").and_then(|v| v.as_u64()).unwrap_or(25) as u32;
    crate::visual::zoompan(input, &output, zoom_factor, x_expr, y_expr, duration_frames, fps).unwrap_or_else(|e| e)
}

fn execute_minterpolate_claude(args: &Value) -> String {
    let input = args["input_file"].as_str().unwrap_or("");
    let output_raw = args["output_file"].as_str().unwrap_or("");
    let output = ensure_outputs_directory(output_raw);
    let fps_target = args.get("fps_target").and_then(|v| v.as_u64()).unwrap_or(60) as u32;
    let mi_mode = args.get("mi_mode").and_then(|v| v.as_str()).unwrap_or("mci");
    crate::visual::minterpolate(input, &output, fps_target, mi_mode).unwrap_or_else(|e| e)
}

fn execute_minterpolate_gemini(args: &HashMap<String, Value>) -> String {
    let input = args.get("input_file").and_then(|v| v.as_str()).unwrap_or("");
    let output_raw = args.get("output_file").and_then(|v| v.as_str()).unwrap_or("");
    let output = ensure_outputs_directory(output_raw);
    let fps_target = args.get("fps_target").and_then(|v| v.as_u64()).unwrap_or(60) as u32;
    let mi_mode = args.get("mi_mode").and_then(|v| v.as_str()).unwrap_or("mci");
    crate::visual::minterpolate(input, &output, fps_target, mi_mode).unwrap_or_else(|e| e)
}

// ============================================================================
// NEW TOOL EXECUTORS — BATCH 7 (Media Analysis Tools)
// ============================================================================

fn execute_detect_scene_changes_claude(args: &Value) -> String {
    let input = args["input_file"].as_str().unwrap_or("");
    let threshold = args.get("threshold").and_then(|v| v.as_f64()).unwrap_or(40.0);
    crate::core::detect_scene_changes(input, threshold).unwrap_or_else(|e| e)
}

fn execute_detect_scene_changes_gemini(args: &HashMap<String, Value>) -> String {
    let input = args.get("input_file").and_then(|v| v.as_str()).unwrap_or("");
    let threshold = args.get("threshold").and_then(|v| v.as_f64()).unwrap_or(40.0);
    crate::core::detect_scene_changes(input, threshold).unwrap_or_else(|e| e)
}

fn execute_measure_loudness_claude(args: &Value) -> String {
    let input = args["input_file"].as_str().unwrap_or("");
    crate::core::measure_loudness(input).unwrap_or_else(|e| e)
}

fn execute_measure_loudness_gemini(args: &HashMap<String, Value>) -> String {
    let input = args.get("input_file").and_then(|v| v.as_str()).unwrap_or("");
    crate::core::measure_loudness(input).unwrap_or_else(|e| e)
}

fn execute_detect_silence_claude(args: &Value) -> String {
    let input = args["input_file"].as_str().unwrap_or("");
    let noise_tolerance_db = args.get("noise_tolerance_db").and_then(|v| v.as_f64()).unwrap_or(-60.0);
    let min_duration_sec = args.get("min_duration_sec").and_then(|v| v.as_f64()).unwrap_or(0.1);
    crate::core::detect_silence(input, noise_tolerance_db, min_duration_sec).unwrap_or_else(|e| e)
}

fn execute_detect_silence_gemini(args: &HashMap<String, Value>) -> String {
    let input = args.get("input_file").and_then(|v| v.as_str()).unwrap_or("");
    let noise_tolerance_db = args.get("noise_tolerance_db").and_then(|v| v.as_f64()).unwrap_or(-60.0);
    let min_duration_sec = args.get("min_duration_sec").and_then(|v| v.as_f64()).unwrap_or(0.1);
    crate::core::detect_silence(input, noise_tolerance_db, min_duration_sec).unwrap_or_else(|e| e)
}

// ============================================================================
// BATCH 8 — Advanced Color Grading executor functions
// ============================================================================

fn execute_adjust_curves_claude(args: &Value) -> String {
    let input = args["input_file"].as_str().unwrap_or("");
    let output_raw = args["output_file"].as_str().unwrap_or("");
    let output = ensure_outputs_directory(output_raw);
    let preset = args.get("preset").and_then(|v| v.as_str()).unwrap_or("");
    let master = args.get("master").and_then(|v| v.as_str()).unwrap_or("");
    let red = args.get("red_channel").and_then(|v| v.as_str()).unwrap_or("");
    let green = args.get("green_channel").and_then(|v| v.as_str()).unwrap_or("");
    let blue = args.get("blue_channel").and_then(|v| v.as_str()).unwrap_or("");
    crate::visual::adjust_curves(input, &output, preset, master, red, green, blue).unwrap_or_else(|e| e)
}

fn execute_adjust_curves_gemini(args: &HashMap<String, Value>) -> String {
    let input = args.get("input_file").and_then(|v| v.as_str()).unwrap_or("");
    let output = args.get("output_file").and_then(|v| v.as_str()).unwrap_or("");
    let preset = args.get("preset").and_then(|v| v.as_str()).unwrap_or("");
    let master = args.get("master").and_then(|v| v.as_str()).unwrap_or("");
    let red = args.get("red_channel").and_then(|v| v.as_str()).unwrap_or("");
    let green = args.get("green_channel").and_then(|v| v.as_str()).unwrap_or("");
    let blue = args.get("blue_channel").and_then(|v| v.as_str()).unwrap_or("");
    crate::visual::adjust_curves(input, output, preset, master, red, green, blue).unwrap_or_else(|e| e)
}

fn execute_adjust_levels_claude(args: &Value) -> String {
    let input = args["input_file"].as_str().unwrap_or("");
    let output_raw = args["output_file"].as_str().unwrap_or("");
    let output = ensure_outputs_directory(output_raw);
    let rimin = args.get("rimin").and_then(|v| v.as_f64()).unwrap_or(0.0);
    let rimax = args.get("rimax").and_then(|v| v.as_f64()).unwrap_or(1.0);
    let gimin = args.get("gimin").and_then(|v| v.as_f64()).unwrap_or(0.0);
    let gimax = args.get("gimax").and_then(|v| v.as_f64()).unwrap_or(1.0);
    let bimin = args.get("bimin").and_then(|v| v.as_f64()).unwrap_or(0.0);
    let bimax = args.get("bimax").and_then(|v| v.as_f64()).unwrap_or(1.0);
    let romin = args.get("romin").and_then(|v| v.as_f64()).unwrap_or(0.0);
    let romax = args.get("romax").and_then(|v| v.as_f64()).unwrap_or(1.0);
    let gomin = args.get("gomin").and_then(|v| v.as_f64()).unwrap_or(0.0);
    let gomax = args.get("gomax").and_then(|v| v.as_f64()).unwrap_or(1.0);
    let bomin = args.get("bomin").and_then(|v| v.as_f64()).unwrap_or(0.0);
    let bomax = args.get("bomax").and_then(|v| v.as_f64()).unwrap_or(1.0);
    crate::visual::adjust_levels(input, &output, rimin, rimax, gimin, gimax, bimin, bimax, romin, romax, gomin, gomax, bomin, bomax).unwrap_or_else(|e| e)
}

fn execute_adjust_levels_gemini(args: &HashMap<String, Value>) -> String {
    let input = args.get("input_file").and_then(|v| v.as_str()).unwrap_or("");
    let output = args.get("output_file").and_then(|v| v.as_str()).unwrap_or("");
    let rimin = args.get("rimin").and_then(|v| v.as_f64()).unwrap_or(0.0);
    let rimax = args.get("rimax").and_then(|v| v.as_f64()).unwrap_or(1.0);
    let gimin = args.get("gimin").and_then(|v| v.as_f64()).unwrap_or(0.0);
    let gimax = args.get("gimax").and_then(|v| v.as_f64()).unwrap_or(1.0);
    let bimin = args.get("bimin").and_then(|v| v.as_f64()).unwrap_or(0.0);
    let bimax = args.get("bimax").and_then(|v| v.as_f64()).unwrap_or(1.0);
    let romin = args.get("romin").and_then(|v| v.as_f64()).unwrap_or(0.0);
    let romax = args.get("romax").and_then(|v| v.as_f64()).unwrap_or(1.0);
    let gomin = args.get("gomin").and_then(|v| v.as_f64()).unwrap_or(0.0);
    let gomax = args.get("gomax").and_then(|v| v.as_f64()).unwrap_or(1.0);
    let bomin = args.get("bomin").and_then(|v| v.as_f64()).unwrap_or(0.0);
    let bomax = args.get("bomax").and_then(|v| v.as_f64()).unwrap_or(1.0);
    crate::visual::adjust_levels(input, output, rimin, rimax, gimin, gimax, bimin, bimax, romin, romax, gomin, gomax, bomin, bomax).unwrap_or_else(|e| e)
}

fn execute_split_tone_claude(args: &Value) -> String {
    let input = args["input_file"].as_str().unwrap_or("");
    let output_raw = args["output_file"].as_str().unwrap_or("");
    let output = ensure_outputs_directory(output_raw);
    let shadow_hue = args.get("shadow_hue").and_then(|v| v.as_f64()).unwrap_or(0.0);
    let shadow_sat = args.get("shadow_saturation").and_then(|v| v.as_f64()).unwrap_or(0.0);
    let highlight_hue = args.get("highlight_hue").and_then(|v| v.as_f64()).unwrap_or(0.0);
    let highlight_sat = args.get("highlight_saturation").and_then(|v| v.as_f64()).unwrap_or(0.0);
    let balance = args.get("balance").and_then(|v| v.as_f64()).unwrap_or(0.0);
    crate::visual::split_tone(input, &output, shadow_hue, shadow_sat, highlight_hue, highlight_sat, balance).unwrap_or_else(|e| e)
}

fn execute_split_tone_gemini(args: &HashMap<String, Value>) -> String {
    let input = args.get("input_file").and_then(|v| v.as_str()).unwrap_or("");
    let output = args.get("output_file").and_then(|v| v.as_str()).unwrap_or("");
    let shadow_hue = args.get("shadow_hue").and_then(|v| v.as_f64()).unwrap_or(0.0);
    let shadow_sat = args.get("shadow_saturation").and_then(|v| v.as_f64()).unwrap_or(0.0);
    let highlight_hue = args.get("highlight_hue").and_then(|v| v.as_f64()).unwrap_or(0.0);
    let highlight_sat = args.get("highlight_saturation").and_then(|v| v.as_f64()).unwrap_or(0.0);
    let balance = args.get("balance").and_then(|v| v.as_f64()).unwrap_or(0.0);
    crate::visual::split_tone(input, output, shadow_hue, shadow_sat, highlight_hue, highlight_sat, balance).unwrap_or_else(|e| e)
}

fn execute_convert_colorspace_claude(args: &Value) -> String {
    let input = args["input_file"].as_str().unwrap_or("");
    let output_raw = args["output_file"].as_str().unwrap_or("");
    let output = ensure_outputs_directory(output_raw);
    let colorspace = args["colorspace"].as_str().unwrap_or("bt709");
    let trc = args.get("transfer_characteristics").and_then(|v| v.as_str()).unwrap_or("");
    let primaries = args.get("color_primaries").and_then(|v| v.as_str()).unwrap_or("");
    crate::visual::convert_colorspace(input, &output, colorspace, trc, primaries).unwrap_or_else(|e| e)
}

fn execute_convert_colorspace_gemini(args: &HashMap<String, Value>) -> String {
    let input = args.get("input_file").and_then(|v| v.as_str()).unwrap_or("");
    let output = args.get("output_file").and_then(|v| v.as_str()).unwrap_or("");
    let colorspace = args.get("colorspace").and_then(|v| v.as_str()).unwrap_or("bt709");
    let trc = args.get("transfer_characteristics").and_then(|v| v.as_str()).unwrap_or("");
    let primaries = args.get("color_primaries").and_then(|v| v.as_str()).unwrap_or("");
    crate::visual::convert_colorspace(input, output, colorspace, trc, primaries).unwrap_or_else(|e| e)
}

fn execute_apply_tonemap_claude(args: &Value) -> String {
    let input = args["input_file"].as_str().unwrap_or("");
    let output_raw = args["output_file"].as_str().unwrap_or("");
    let output = ensure_outputs_directory(output_raw);
    let algorithm = args["algorithm"].as_str().unwrap_or("reinhard");
    let peak = args.get("peak").and_then(|v| v.as_f64()).unwrap_or(0.0);
    let desat = args.get("desat").and_then(|v| v.as_f64()).unwrap_or(0.5);
    crate::visual::apply_tonemap(input, &output, algorithm, peak, desat).unwrap_or_else(|e| e)
}

fn execute_apply_tonemap_gemini(args: &HashMap<String, Value>) -> String {
    let input = args.get("input_file").and_then(|v| v.as_str()).unwrap_or("");
    let output = args.get("output_file").and_then(|v| v.as_str()).unwrap_or("");
    let algorithm = args.get("algorithm").and_then(|v| v.as_str()).unwrap_or("reinhard");
    let peak = args.get("peak").and_then(|v| v.as_f64()).unwrap_or(0.0);
    let desat = args.get("desat").and_then(|v| v.as_f64()).unwrap_or(0.5);
    crate::visual::apply_tonemap(input, output, algorithm, peak, desat).unwrap_or_else(|e| e)
}

// ============================================================================
// BATCH 9 — Audio Tone Shaping executor functions
// ============================================================================

fn execute_filter_highpass_claude(args: &Value) -> String {
    let input = args["input_file"].as_str().unwrap_or("");
    let output_raw = args["output_file"].as_str().unwrap_or("");
    let output = ensure_outputs_directory(output_raw);
    let freq = args["frequency_hz"].as_f64().unwrap_or(80.0);
    let poles = args.get("poles").and_then(|v| v.as_u64()).unwrap_or(2) as u32;
    let width = args.get("width_hz").and_then(|v| v.as_f64()).unwrap_or(0.707);
    crate::audio::filter_highpass(input, &output, freq, poles, width).unwrap_or_else(|e| e)
}

fn execute_filter_highpass_gemini(args: &HashMap<String, Value>) -> String {
    let input = args.get("input_file").and_then(|v| v.as_str()).unwrap_or("");
    let output = args.get("output_file").and_then(|v| v.as_str()).unwrap_or("");
    let freq = args.get("frequency_hz").and_then(|v| v.as_f64()).unwrap_or(80.0);
    let poles = args.get("poles").and_then(|v| v.as_u64()).unwrap_or(2) as u32;
    let width = args.get("width_hz").and_then(|v| v.as_f64()).unwrap_or(0.707);
    crate::audio::filter_highpass(input, output, freq, poles, width).unwrap_or_else(|e| e)
}

fn execute_filter_lowpass_claude(args: &Value) -> String {
    let input = args["input_file"].as_str().unwrap_or("");
    let output_raw = args["output_file"].as_str().unwrap_or("");
    let output = ensure_outputs_directory(output_raw);
    let freq = args["frequency_hz"].as_f64().unwrap_or(8000.0);
    let poles = args.get("poles").and_then(|v| v.as_u64()).unwrap_or(2) as u32;
    let width = args.get("width_hz").and_then(|v| v.as_f64()).unwrap_or(0.707);
    crate::audio::filter_lowpass(input, &output, freq, poles, width).unwrap_or_else(|e| e)
}

fn execute_filter_lowpass_gemini(args: &HashMap<String, Value>) -> String {
    let input = args.get("input_file").and_then(|v| v.as_str()).unwrap_or("");
    let output = args.get("output_file").and_then(|v| v.as_str()).unwrap_or("");
    let freq = args.get("frequency_hz").and_then(|v| v.as_f64()).unwrap_or(8000.0);
    let poles = args.get("poles").and_then(|v| v.as_u64()).unwrap_or(2) as u32;
    let width = args.get("width_hz").and_then(|v| v.as_f64()).unwrap_or(0.707);
    crate::audio::filter_lowpass(input, output, freq, poles, width).unwrap_or_else(|e| e)
}

fn execute_adjust_bass_claude(args: &Value) -> String {
    let input = args["input_file"].as_str().unwrap_or("");
    let output_raw = args["output_file"].as_str().unwrap_or("");
    let output = ensure_outputs_directory(output_raw);
    let gain_db = args["gain_db"].as_f64().unwrap_or(0.0);
    let freq = args.get("frequency_hz").and_then(|v| v.as_f64()).unwrap_or(100.0);
    let width = args.get("width_hz").and_then(|v| v.as_f64()).unwrap_or(0.5);
    crate::audio::adjust_bass(input, &output, gain_db, freq, width).unwrap_or_else(|e| e)
}

fn execute_adjust_bass_gemini(args: &HashMap<String, Value>) -> String {
    let input = args.get("input_file").and_then(|v| v.as_str()).unwrap_or("");
    let output = args.get("output_file").and_then(|v| v.as_str()).unwrap_or("");
    let gain_db = args.get("gain_db").and_then(|v| v.as_f64()).unwrap_or(0.0);
    let freq = args.get("frequency_hz").and_then(|v| v.as_f64()).unwrap_or(100.0);
    let width = args.get("width_hz").and_then(|v| v.as_f64()).unwrap_or(0.5);
    crate::audio::adjust_bass(input, output, gain_db, freq, width).unwrap_or_else(|e| e)
}

fn execute_adjust_treble_claude(args: &Value) -> String {
    let input = args["input_file"].as_str().unwrap_or("");
    let output_raw = args["output_file"].as_str().unwrap_or("");
    let output = ensure_outputs_directory(output_raw);
    let gain_db = args["gain_db"].as_f64().unwrap_or(0.0);
    let freq = args.get("frequency_hz").and_then(|v| v.as_f64()).unwrap_or(3000.0);
    let width = args.get("width_hz").and_then(|v| v.as_f64()).unwrap_or(0.5);
    crate::audio::adjust_treble(input, &output, gain_db, freq, width).unwrap_or_else(|e| e)
}

fn execute_adjust_treble_gemini(args: &HashMap<String, Value>) -> String {
    let input = args.get("input_file").and_then(|v| v.as_str()).unwrap_or("");
    let output = args.get("output_file").and_then(|v| v.as_str()).unwrap_or("");
    let gain_db = args.get("gain_db").and_then(|v| v.as_f64()).unwrap_or(0.0);
    let freq = args.get("frequency_hz").and_then(|v| v.as_f64()).unwrap_or(3000.0);
    let width = args.get("width_hz").and_then(|v| v.as_f64()).unwrap_or(0.5);
    crate::audio::adjust_treble(input, output, gain_db, freq, width).unwrap_or_else(|e| e)
}

fn execute_audio_compand_claude(args: &Value) -> String {
    let input = args["input_file"].as_str().unwrap_or("");
    let output_raw = args["output_file"].as_str().unwrap_or("");
    let output = ensure_outputs_directory(output_raw);
    let attacks = args.get("attacks").and_then(|v| v.as_str()).unwrap_or("0.3");
    let decays = args.get("decays").and_then(|v| v.as_str()).unwrap_or("0.8");
    let points = args.get("points").and_then(|v| v.as_str()).unwrap_or("-70/-70 -60/-20 1/0");
    let soft_knee = args.get("soft_knee_db").and_then(|v| v.as_f64()).unwrap_or(0.01);
    let gain = args.get("gain_db").and_then(|v| v.as_f64()).unwrap_or(0.0);
    crate::audio::audio_compand(input, &output, attacks, decays, points, soft_knee, gain).unwrap_or_else(|e| e)
}

fn execute_audio_compand_gemini(args: &HashMap<String, Value>) -> String {
    let input = args.get("input_file").and_then(|v| v.as_str()).unwrap_or("");
    let output = args.get("output_file").and_then(|v| v.as_str()).unwrap_or("");
    let attacks = args.get("attacks").and_then(|v| v.as_str()).unwrap_or("0.3");
    let decays = args.get("decays").and_then(|v| v.as_str()).unwrap_or("0.8");
    let points = args.get("points").and_then(|v| v.as_str()).unwrap_or("-70/-70 -60/-20 1/0");
    let soft_knee = args.get("soft_knee_db").and_then(|v| v.as_f64()).unwrap_or(0.01);
    let gain = args.get("gain_db").and_then(|v| v.as_f64()).unwrap_or(0.0);
    crate::audio::audio_compand(input, output, attacks, decays, points, soft_knee, gain).unwrap_or_else(|e| e)
}

fn execute_add_audio_delay_claude(args: &Value) -> String {
    let input = args["input_file"].as_str().unwrap_or("");
    let output_raw = args["output_file"].as_str().unwrap_or("");
    let output = ensure_outputs_directory(output_raw);
    let delays_ms = args["delays_ms"].as_str().unwrap_or("500");
    let all_channels = args.get("all_channels").and_then(|v| v.as_bool()).unwrap_or(true);
    crate::audio::add_audio_delay(input, &output, delays_ms, all_channels).unwrap_or_else(|e| e)
}

fn execute_add_audio_delay_gemini(args: &HashMap<String, Value>) -> String {
    let input = args.get("input_file").and_then(|v| v.as_str()).unwrap_or("");
    let output = args.get("output_file").and_then(|v| v.as_str()).unwrap_or("");
    let delays_ms = args.get("delays_ms").and_then(|v| v.as_str()).unwrap_or("500");
    let all_channels = args.get("all_channels").and_then(|v| v.as_bool()).unwrap_or(true);
    crate::audio::add_audio_delay(input, output, delays_ms, all_channels).unwrap_or_else(|e| e)
}

fn execute_add_phaser_claude(args: &Value) -> String {
    let input = args["input_file"].as_str().unwrap_or("");
    let output_raw = args["output_file"].as_str().unwrap_or("");
    let output = ensure_outputs_directory(output_raw);
    let in_gain = args.get("in_gain").and_then(|v| v.as_f64()).unwrap_or(0.4);
    let out_gain = args.get("out_gain").and_then(|v| v.as_f64()).unwrap_or(0.74);
    let delay = args.get("delay_ms").and_then(|v| v.as_f64()).unwrap_or(3.0);
    let decay = args.get("decay").and_then(|v| v.as_f64()).unwrap_or(0.4);
    let speed = args.get("speed_hz").and_then(|v| v.as_f64()).unwrap_or(0.5);
    let phaser_type = args.get("type").and_then(|v| v.as_str()).unwrap_or("triangular");
    crate::audio::add_phaser(input, &output, in_gain, out_gain, delay, decay, speed, phaser_type).unwrap_or_else(|e| e)
}

fn execute_add_phaser_gemini(args: &HashMap<String, Value>) -> String {
    let input = args.get("input_file").and_then(|v| v.as_str()).unwrap_or("");
    let output = args.get("output_file").and_then(|v| v.as_str()).unwrap_or("");
    let in_gain = args.get("in_gain").and_then(|v| v.as_f64()).unwrap_or(0.4);
    let out_gain = args.get("out_gain").and_then(|v| v.as_f64()).unwrap_or(0.74);
    let delay = args.get("delay_ms").and_then(|v| v.as_f64()).unwrap_or(3.0);
    let decay = args.get("decay").and_then(|v| v.as_f64()).unwrap_or(0.4);
    let speed = args.get("speed_hz").and_then(|v| v.as_f64()).unwrap_or(0.5);
    let phaser_type = args.get("type").and_then(|v| v.as_str()).unwrap_or("triangular");
    crate::audio::add_phaser(input, output, in_gain, out_gain, delay, decay, speed, phaser_type).unwrap_or_else(|e| e)
}

// ============================================================================
// BATCH 10 — Audio Restoration executor functions
// ============================================================================

fn execute_remove_clicks_claude(args: &Value) -> String {
    let input = args["input_file"].as_str().unwrap_or("");
    let output_raw = args["output_file"].as_str().unwrap_or("");
    let output = ensure_outputs_directory(output_raw);
    let window = args.get("window_ms").and_then(|v| v.as_f64()).unwrap_or(55.0);
    let overlap = args.get("overlap_pct").and_then(|v| v.as_f64()).unwrap_or(75.0);
    let arorder = args.get("ar_order").and_then(|v| v.as_u64()).unwrap_or(2) as u32;
    let threshold = args.get("threshold").and_then(|v| v.as_f64()).unwrap_or(2.0);
    crate::audio::remove_clicks(input, &output, window, overlap, arorder, threshold).unwrap_or_else(|e| e)
}

fn execute_remove_clicks_gemini(args: &HashMap<String, Value>) -> String {
    let input = args.get("input_file").and_then(|v| v.as_str()).unwrap_or("");
    let output = args.get("output_file").and_then(|v| v.as_str()).unwrap_or("");
    let window = args.get("window_ms").and_then(|v| v.as_f64()).unwrap_or(55.0);
    let overlap = args.get("overlap_pct").and_then(|v| v.as_f64()).unwrap_or(75.0);
    let arorder = args.get("ar_order").and_then(|v| v.as_u64()).unwrap_or(2) as u32;
    let threshold = args.get("threshold").and_then(|v| v.as_f64()).unwrap_or(2.0);
    crate::audio::remove_clicks(input, output, window, overlap, arorder, threshold).unwrap_or_else(|e| e)
}

fn execute_restore_clipping_claude(args: &Value) -> String {
    let input = args["input_file"].as_str().unwrap_or("");
    let output_raw = args["output_file"].as_str().unwrap_or("");
    let output = ensure_outputs_directory(output_raw);
    let window = args.get("window_ms").and_then(|v| v.as_f64()).unwrap_or(55.0);
    let overlap = args.get("overlap_pct").and_then(|v| v.as_f64()).unwrap_or(75.0);
    let arorder = args.get("ar_order").and_then(|v| v.as_u64()).unwrap_or(8) as u32;
    let threshold = args.get("threshold").and_then(|v| v.as_f64()).unwrap_or(10.0);
    crate::audio::restore_clipping(input, &output, window, overlap, arorder, threshold).unwrap_or_else(|e| e)
}

fn execute_restore_clipping_gemini(args: &HashMap<String, Value>) -> String {
    let input = args.get("input_file").and_then(|v| v.as_str()).unwrap_or("");
    let output = args.get("output_file").and_then(|v| v.as_str()).unwrap_or("");
    let window = args.get("window_ms").and_then(|v| v.as_f64()).unwrap_or(55.0);
    let overlap = args.get("overlap_pct").and_then(|v| v.as_f64()).unwrap_or(75.0);
    let arorder = args.get("ar_order").and_then(|v| v.as_u64()).unwrap_or(8) as u32;
    let threshold = args.get("threshold").and_then(|v| v.as_f64()).unwrap_or(10.0);
    crate::audio::restore_clipping(input, output, window, overlap, arorder, threshold).unwrap_or_else(|e| e)
}

fn execute_remove_silence_claude(args: &Value) -> String {
    let input = args["input_file"].as_str().unwrap_or("");
    let output_raw = args["output_file"].as_str().unwrap_or("");
    let output = ensure_outputs_directory(output_raw);
    let start_periods = args.get("start_periods").and_then(|v| v.as_u64()).unwrap_or(1) as u32;
    let start_threshold_db = args.get("start_threshold_db").and_then(|v| v.as_f64()).unwrap_or(-50.0);
    let stop_periods = args.get("stop_periods").and_then(|v| v.as_i64()).unwrap_or(-1) as i32;
    let stop_threshold_db = args.get("stop_threshold_db").and_then(|v| v.as_f64()).unwrap_or(-50.0);
    let stop_duration = args.get("stop_duration_sec").and_then(|v| v.as_f64()).unwrap_or(0.1);
    crate::audio::remove_silence(input, &output, start_periods, start_threshold_db, stop_periods, stop_threshold_db, stop_duration).unwrap_or_else(|e| e)
}

fn execute_remove_silence_gemini(args: &HashMap<String, Value>) -> String {
    let input = args.get("input_file").and_then(|v| v.as_str()).unwrap_or("");
    let output = args.get("output_file").and_then(|v| v.as_str()).unwrap_or("");
    let start_periods = args.get("start_periods").and_then(|v| v.as_u64()).unwrap_or(1) as u32;
    let start_threshold_db = args.get("start_threshold_db").and_then(|v| v.as_f64()).unwrap_or(-50.0);
    let stop_periods = args.get("stop_periods").and_then(|v| v.as_i64()).unwrap_or(-1) as i32;
    let stop_threshold_db = args.get("stop_threshold_db").and_then(|v| v.as_f64()).unwrap_or(-50.0);
    let stop_duration = args.get("stop_duration_sec").and_then(|v| v.as_f64()).unwrap_or(0.1);
    crate::audio::remove_silence(input, output, start_periods, start_threshold_db, stop_periods, stop_threshold_db, stop_duration).unwrap_or_else(|e| e)
}

// ============================================================================
// BATCH 11 — Quality Metrics executor functions
// ============================================================================

fn execute_compare_ssim_claude(args: &Value) -> String {
    let reference = args["reference_file"].as_str().unwrap_or("");
    let distorted = args["distorted_file"].as_str().unwrap_or("");
    crate::core::compare_ssim(reference, distorted).unwrap_or_else(|e| e)
}

fn execute_compare_ssim_gemini(args: &HashMap<String, Value>) -> String {
    let reference = args.get("reference_file").and_then(|v| v.as_str()).unwrap_or("");
    let distorted = args.get("distorted_file").and_then(|v| v.as_str()).unwrap_or("");
    crate::core::compare_ssim(reference, distorted).unwrap_or_else(|e| e)
}

fn execute_compare_psnr_claude(args: &Value) -> String {
    let reference = args["reference_file"].as_str().unwrap_or("");
    let distorted = args["distorted_file"].as_str().unwrap_or("");
    crate::core::compare_psnr(reference, distorted).unwrap_or_else(|e| e)
}

fn execute_compare_psnr_gemini(args: &HashMap<String, Value>) -> String {
    let reference = args.get("reference_file").and_then(|v| v.as_str()).unwrap_or("");
    let distorted = args.get("distorted_file").and_then(|v| v.as_str()).unwrap_or("");
    crate::core::compare_psnr(reference, distorted).unwrap_or_else(|e| e)
}

fn execute_analyze_audio_stats_claude(args: &Value) -> String {
    let input = args["input_file"].as_str().unwrap_or("");
    let reset = args.get("reset_interval").and_then(|v| v.as_u64()).unwrap_or(0) as u32;
    crate::core::analyze_audio_stats(input, reset).unwrap_or_else(|e| e)
}

fn execute_analyze_audio_stats_gemini(args: &HashMap<String, Value>) -> String {
    let input = args.get("input_file").and_then(|v| v.as_str()).unwrap_or("");
    let reset = args.get("reset_interval").and_then(|v| v.as_u64()).unwrap_or(0) as u32;
    crate::core::analyze_audio_stats(input, reset).unwrap_or_else(|e| e)
}

fn execute_analyze_video_signal_claude(args: &Value) -> String {
    let input = args["input_file"].as_str().unwrap_or("");
    crate::core::analyze_video_signal(input).unwrap_or_else(|e| e)
}

fn execute_analyze_video_signal_gemini(args: &HashMap<String, Value>) -> String {
    let input = args.get("input_file").and_then(|v| v.as_str()).unwrap_or("");
    crate::core::analyze_video_signal(input).unwrap_or_else(|e| e)
}

// ============================================================================
// BATCH 12 — Geometric Transforms executor functions
// ============================================================================

fn execute_correct_perspective_claude(args: &Value) -> String {
    let input = args["input_file"].as_str().unwrap_or("");
    let output_raw = args["output_file"].as_str().unwrap_or("");
    let output = ensure_outputs_directory(output_raw);
    let x0 = args.get("x0").and_then(|v| v.as_f64()).unwrap_or(0.0);
    let y0 = args.get("y0").and_then(|v| v.as_f64()).unwrap_or(0.0);
    let x1 = args.get("x1").and_then(|v| v.as_f64()).unwrap_or(0.0);
    let y1 = args.get("y1").and_then(|v| v.as_f64()).unwrap_or(0.0);
    let x2 = args.get("x2").and_then(|v| v.as_f64()).unwrap_or(0.0);
    let y2 = args.get("y2").and_then(|v| v.as_f64()).unwrap_or(0.0);
    let x3 = args.get("x3").and_then(|v| v.as_f64()).unwrap_or(0.0);
    let y3 = args.get("y3").and_then(|v| v.as_f64()).unwrap_or(0.0);
    let interp = args.get("interpolation").and_then(|v| v.as_str()).unwrap_or("linear");
    crate::visual::correct_perspective(input, &output, x0, y0, x1, y1, x2, y2, x3, y3, interp).unwrap_or_else(|e| e)
}

fn execute_correct_perspective_gemini(args: &HashMap<String, Value>) -> String {
    let input = args.get("input_file").and_then(|v| v.as_str()).unwrap_or("");
    let output = args.get("output_file").and_then(|v| v.as_str()).unwrap_or("");
    let x0 = args.get("x0").and_then(|v| v.as_f64()).unwrap_or(0.0);
    let y0 = args.get("y0").and_then(|v| v.as_f64()).unwrap_or(0.0);
    let x1 = args.get("x1").and_then(|v| v.as_f64()).unwrap_or(0.0);
    let y1 = args.get("y1").and_then(|v| v.as_f64()).unwrap_or(0.0);
    let x2 = args.get("x2").and_then(|v| v.as_f64()).unwrap_or(0.0);
    let y2 = args.get("y2").and_then(|v| v.as_f64()).unwrap_or(0.0);
    let x3 = args.get("x3").and_then(|v| v.as_f64()).unwrap_or(0.0);
    let y3 = args.get("y3").and_then(|v| v.as_f64()).unwrap_or(0.0);
    let interp = args.get("interpolation").and_then(|v| v.as_str()).unwrap_or("linear");
    crate::visual::correct_perspective(input, output, x0, y0, x1, y1, x2, y2, x3, y3, interp).unwrap_or_else(|e| e)
}

fn execute_correct_lens_claude(args: &Value) -> String {
    let input = args["input_file"].as_str().unwrap_or("");
    let output_raw = args["output_file"].as_str().unwrap_or("");
    let output = ensure_outputs_directory(output_raw);
    let k1 = args["k1"].as_f64().unwrap_or(0.0);
    let k2 = args.get("k2").and_then(|v| v.as_f64()).unwrap_or(0.0);
    let cx = args.get("center_x").and_then(|v| v.as_f64()).unwrap_or(0.5);
    let cy = args.get("center_y").and_then(|v| v.as_f64()).unwrap_or(0.5);
    let interp = args.get("interpolation").and_then(|v| v.as_str()).unwrap_or("bilinear");
    crate::visual::correct_lens(input, &output, k1, k2, cx, cy, interp).unwrap_or_else(|e| e)
}

fn execute_correct_lens_gemini(args: &HashMap<String, Value>) -> String {
    let input = args.get("input_file").and_then(|v| v.as_str()).unwrap_or("");
    let output = args.get("output_file").and_then(|v| v.as_str()).unwrap_or("");
    let k1 = args.get("k1").and_then(|v| v.as_f64()).unwrap_or(0.0);
    let k2 = args.get("k2").and_then(|v| v.as_f64()).unwrap_or(0.0);
    let cx = args.get("center_x").and_then(|v| v.as_f64()).unwrap_or(0.5);
    let cy = args.get("center_y").and_then(|v| v.as_f64()).unwrap_or(0.5);
    let interp = args.get("interpolation").and_then(|v| v.as_str()).unwrap_or("bilinear");
    crate::visual::correct_lens(input, output, k1, k2, cx, cy, interp).unwrap_or_else(|e| e)
}

fn execute_apply_shear_claude(args: &Value) -> String {
    let input = args["input_file"].as_str().unwrap_or("");
    let output_raw = args["output_file"].as_str().unwrap_or("");
    let output = ensure_outputs_directory(output_raw);
    let shx = args.get("shear_x").and_then(|v| v.as_f64()).unwrap_or(0.0);
    let shy = args.get("shear_y").and_then(|v| v.as_f64()).unwrap_or(0.0);
    let fillcolor = args.get("fill_color").and_then(|v| v.as_str()).unwrap_or("black");
    let interp = args.get("interpolation").and_then(|v| v.as_str()).unwrap_or("bilinear");
    crate::visual::apply_shear(input, &output, shx, shy, fillcolor, interp).unwrap_or_else(|e| e)
}

fn execute_apply_shear_gemini(args: &HashMap<String, Value>) -> String {
    let input = args.get("input_file").and_then(|v| v.as_str()).unwrap_or("");
    let output = args.get("output_file").and_then(|v| v.as_str()).unwrap_or("");
    let shx = args.get("shear_x").and_then(|v| v.as_f64()).unwrap_or(0.0);
    let shy = args.get("shear_y").and_then(|v| v.as_f64()).unwrap_or(0.0);
    let fillcolor = args.get("fill_color").and_then(|v| v.as_str()).unwrap_or("black");
    let interp = args.get("interpolation").and_then(|v| v.as_str()).unwrap_or("bilinear");
    crate::visual::apply_shear(input, output, shx, shy, fillcolor, interp).unwrap_or_else(|e| e)
}

// ============================================================================
// BATCH 13 — Temporal Frame Effects executor functions
// ============================================================================

fn execute_blend_frames_claude(args: &Value) -> String {
    let input = args["input_file"].as_str().unwrap_or("");
    let output_raw = args["output_file"].as_str().unwrap_or("");
    let output = ensure_outputs_directory(output_raw);
    let blend_mode = args.get("blend_mode").and_then(|v| v.as_str()).unwrap_or("average");
    let opacity = args.get("opacity").and_then(|v| v.as_f64()).unwrap_or(1.0);
    crate::visual::blend_frames(input, &output, blend_mode, opacity).unwrap_or_else(|e| e)
}

fn execute_blend_frames_gemini(args: &HashMap<String, Value>) -> String {
    let input = args.get("input_file").and_then(|v| v.as_str()).unwrap_or("");
    let output = args.get("output_file").and_then(|v| v.as_str()).unwrap_or("");
    let blend_mode = args.get("blend_mode").and_then(|v| v.as_str()).unwrap_or("average");
    let opacity = args.get("opacity").and_then(|v| v.as_f64()).unwrap_or(1.0);
    crate::visual::blend_frames(input, output, blend_mode, opacity).unwrap_or_else(|e| e)
}

fn execute_temporal_median_claude(args: &Value) -> String {
    let input = args["input_file"].as_str().unwrap_or("");
    let output_raw = args["output_file"].as_str().unwrap_or("");
    let output = ensure_outputs_directory(output_raw);
    let radius = args.get("radius").and_then(|v| v.as_u64()).unwrap_or(1) as u32;
    crate::visual::temporal_median(input, &output, radius).unwrap_or_else(|e| e)
}

fn execute_temporal_median_gemini(args: &HashMap<String, Value>) -> String {
    let input = args.get("input_file").and_then(|v| v.as_str()).unwrap_or("");
    let output = args.get("output_file").and_then(|v| v.as_str()).unwrap_or("");
    let radius = args.get("radius").and_then(|v| v.as_u64()).unwrap_or(1) as u32;
    crate::visual::temporal_median(input, output, radius).unwrap_or_else(|e| e)
}

fn execute_convert_framerate_claude(args: &Value) -> String {
    let input = args["input_file"].as_str().unwrap_or("");
    let output_raw = args["output_file"].as_str().unwrap_or("");
    let output = ensure_outputs_directory(output_raw);
    let target_fps = args["target_fps"].as_f64().unwrap_or(30.0);
    let round = args.get("round_mode").and_then(|v| v.as_str()).unwrap_or("near");
    crate::visual::convert_framerate(input, &output, target_fps, round).unwrap_or_else(|e| e)
}

fn execute_convert_framerate_gemini(args: &HashMap<String, Value>) -> String {
    let input = args.get("input_file").and_then(|v| v.as_str()).unwrap_or("");
    let output = args.get("output_file").and_then(|v| v.as_str()).unwrap_or("");
    let target_fps = args.get("target_fps").and_then(|v| v.as_f64()).unwrap_or(30.0);
    let round = args.get("round_mode").and_then(|v| v.as_str()).unwrap_or("near");
    crate::visual::convert_framerate(input, output, target_fps, round).unwrap_or_else(|e| e)
}

fn execute_tile_frames_claude(args: &Value) -> String {
    let input = args["input_file"].as_str().unwrap_or("");
    let output_raw = args["output_file"].as_str().unwrap_or("");
    let output = ensure_outputs_directory(output_raw);
    let columns = args["columns"].as_u64().unwrap_or(4) as u32;
    let rows = args["rows"].as_u64().unwrap_or(3) as u32;
    let frame_gap = args.get("frame_gap").and_then(|v| v.as_u64()).unwrap_or(2) as u32;
    crate::visual::tile_frames(input, &output, columns, rows, frame_gap).unwrap_or_else(|e| e)
}

fn execute_tile_frames_gemini(args: &HashMap<String, Value>) -> String {
    let input = args.get("input_file").and_then(|v| v.as_str()).unwrap_or("");
    let output = args.get("output_file").and_then(|v| v.as_str()).unwrap_or("");
    let columns = args.get("columns").and_then(|v| v.as_u64()).unwrap_or(4) as u32;
    let rows = args.get("rows").and_then(|v| v.as_u64()).unwrap_or(3) as u32;
    let frame_gap = args.get("frame_gap").and_then(|v| v.as_u64()).unwrap_or(2) as u32;
    crate::visual::tile_frames(input, output, columns, rows, frame_gap).unwrap_or_else(|e| e)
}

// ============================================================================
// BATCH 14 — Spatial Audio executor functions
// ============================================================================

fn execute_adjust_stereo_width_claude(args: &Value) -> String {
    let input = args["input_file"].as_str().unwrap_or("");
    let output_raw = args["output_file"].as_str().unwrap_or("");
    let output = ensure_outputs_directory(output_raw);
    let width = args.get("width").and_then(|v| v.as_f64()).unwrap_or(1.0);
    let balance = args.get("balance").and_then(|v| v.as_f64()).unwrap_or(0.0);
    let mode = args.get("mode").and_then(|v| v.as_str()).unwrap_or("lr>lr");
    crate::audio::adjust_stereo_width(input, &output, width, balance, mode).unwrap_or_else(|e| e)
}

fn execute_adjust_stereo_width_gemini(args: &HashMap<String, Value>) -> String {
    let input = args.get("input_file").and_then(|v| v.as_str()).unwrap_or("");
    let output = args.get("output_file").and_then(|v| v.as_str()).unwrap_or("");
    let width = args.get("width").and_then(|v| v.as_f64()).unwrap_or(1.0);
    let balance = args.get("balance").and_then(|v| v.as_f64()).unwrap_or(0.0);
    let mode = args.get("mode").and_then(|v| v.as_str()).unwrap_or("lr>lr");
    crate::audio::adjust_stereo_width(input, output, width, balance, mode).unwrap_or_else(|e| e)
}

fn execute_apply_stereo_widen_claude(args: &Value) -> String {
    let input = args["input_file"].as_str().unwrap_or("");
    let output_raw = args["output_file"].as_str().unwrap_or("");
    let output = ensure_outputs_directory(output_raw);
    let delay_ms = args.get("delay_ms").and_then(|v| v.as_f64()).unwrap_or(20.0);
    let feedback = args.get("feedback").and_then(|v| v.as_f64()).unwrap_or(0.3);
    let crossfeed = args.get("crossfeed").and_then(|v| v.as_f64()).unwrap_or(0.3);
    let drymix = args.get("drymix").and_then(|v| v.as_f64()).unwrap_or(0.8);
    crate::audio::apply_stereo_widen(input, &output, delay_ms, feedback, crossfeed, drymix).unwrap_or_else(|e| e)
}

fn execute_apply_stereo_widen_gemini(args: &HashMap<String, Value>) -> String {
    let input = args.get("input_file").and_then(|v| v.as_str()).unwrap_or("");
    let output = args.get("output_file").and_then(|v| v.as_str()).unwrap_or("");
    let delay_ms = args.get("delay_ms").and_then(|v| v.as_f64()).unwrap_or(20.0);
    let feedback = args.get("feedback").and_then(|v| v.as_f64()).unwrap_or(0.3);
    let crossfeed = args.get("crossfeed").and_then(|v| v.as_f64()).unwrap_or(0.3);
    let drymix = args.get("drymix").and_then(|v| v.as_f64()).unwrap_or(0.8);
    crate::audio::apply_stereo_widen(input, output, delay_ms, feedback, crossfeed, drymix).unwrap_or_else(|e| e)
}

fn execute_mix_audio_channels_claude(args: &Value) -> String {
    let input = args["input_file"].as_str().unwrap_or("");
    let output_raw = args["output_file"].as_str().unwrap_or("");
    let output = ensure_outputs_directory(output_raw);
    let layout = args["channel_layout"].as_str().unwrap_or("stereo");
    let mix_str = args["channel_mix"].as_str().unwrap_or("c0=c0|c1=c1");
    let exprs: Vec<String> = mix_str.split('|').map(|s| s.to_string()).collect();
    crate::audio::mix_audio_channels(input, &output, layout, &exprs).unwrap_or_else(|e| e)
}

fn execute_mix_audio_channels_gemini(args: &HashMap<String, Value>) -> String {
    let input = args.get("input_file").and_then(|v| v.as_str()).unwrap_or("");
    let output = args.get("output_file").and_then(|v| v.as_str()).unwrap_or("");
    let layout = args.get("channel_layout").and_then(|v| v.as_str()).unwrap_or("stereo");
    let mix_str = args.get("channel_mix").and_then(|v| v.as_str()).unwrap_or("c0=c0|c1=c1");
    let exprs: Vec<String> = mix_str.split('|').map(|s| s.to_string()).collect();
    crate::audio::mix_audio_channels(input, output, layout, &exprs).unwrap_or_else(|e| e)
}

// ============================================================================
// PHASE D — Professional Finishing executor functions
// ============================================================================

fn execute_adjust_color_temperature_claude(args: &Value) -> String {
    let input = args["input_file"].as_str().unwrap_or("");
    let output_raw = args["output_file"].as_str().unwrap_or("");
    let output = ensure_outputs_directory(output_raw);
    let temperature = args.get("temperature").and_then(|v| v.as_f64()).unwrap_or(6500.0);
    let mix = args.get("mix").and_then(|v| v.as_f64()).unwrap_or(1.0);
    crate::visual::adjust_color_temperature(input, &output, temperature, mix).unwrap_or_else(|e| e)
}

fn execute_adjust_color_temperature_gemini(args: &HashMap<String, Value>) -> String {
    let input = args.get("input_file").and_then(|v| v.as_str()).unwrap_or("");
    let output_raw = args.get("output_file").and_then(|v| v.as_str()).unwrap_or("");
    let output = ensure_outputs_directory(output_raw);
    let temperature = args.get("temperature").and_then(|v| v.as_f64()).unwrap_or(6500.0);
    let mix = args.get("mix").and_then(|v| v.as_f64()).unwrap_or(1.0);
    crate::visual::adjust_color_temperature(input, &output, temperature, mix).unwrap_or_else(|e| e)
}

fn execute_adjust_vibrance_claude(args: &Value) -> String {
    let input = args["input_file"].as_str().unwrap_or("");
    let output_raw = args["output_file"].as_str().unwrap_or("");
    let output = ensure_outputs_directory(output_raw);
    let intensity = args.get("intensity").and_then(|v| v.as_f64()).unwrap_or(0.0);
    let rbal = args.get("red_balance").and_then(|v| v.as_f64()).unwrap_or(1.0);
    let gbal = args.get("green_balance").and_then(|v| v.as_f64()).unwrap_or(1.0);
    let bbal = args.get("blue_balance").and_then(|v| v.as_f64()).unwrap_or(1.0);
    crate::visual::adjust_vibrance(input, &output, intensity, rbal, gbal, bbal).unwrap_or_else(|e| e)
}

fn execute_adjust_vibrance_gemini(args: &HashMap<String, Value>) -> String {
    let input = args.get("input_file").and_then(|v| v.as_str()).unwrap_or("");
    let output_raw = args.get("output_file").and_then(|v| v.as_str()).unwrap_or("");
    let output = ensure_outputs_directory(output_raw);
    let intensity = args.get("intensity").and_then(|v| v.as_f64()).unwrap_or(0.0);
    let rbal = args.get("red_balance").and_then(|v| v.as_f64()).unwrap_or(1.0);
    let gbal = args.get("green_balance").and_then(|v| v.as_f64()).unwrap_or(1.0);
    let bbal = args.get("blue_balance").and_then(|v| v.as_f64()).unwrap_or(1.0);
    crate::visual::adjust_vibrance(input, &output, intensity, rbal, gbal, bbal).unwrap_or_else(|e| e)
}

fn execute_remove_flicker_claude(args: &Value) -> String {
    let input = args["input_file"].as_str().unwrap_or("");
    let output_raw = args["output_file"].as_str().unwrap_or("");
    let output = ensure_outputs_directory(output_raw);
    let size = args.get("size").and_then(|v| v.as_u64()).unwrap_or(5) as u32;
    let mode = args.get("mode").and_then(|v| v.as_str()).unwrap_or("am");
    crate::visual::remove_flicker(input, &output, size, mode).unwrap_or_else(|e| e)
}

fn execute_remove_flicker_gemini(args: &HashMap<String, Value>) -> String {
    let input = args.get("input_file").and_then(|v| v.as_str()).unwrap_or("");
    let output_raw = args.get("output_file").and_then(|v| v.as_str()).unwrap_or("");
    let output = ensure_outputs_directory(output_raw);
    let size = args.get("size").and_then(|v| v.as_u64()).unwrap_or(5) as u32;
    let mode = args.get("mode").and_then(|v| v.as_str()).unwrap_or("am");
    crate::visual::remove_flicker(input, &output, size, mode).unwrap_or_else(|e| e)
}

fn execute_denoise_video_bm3d_claude(args: &Value) -> String {
    let input = args["input_file"].as_str().unwrap_or("");
    let output_raw = args["output_file"].as_str().unwrap_or("");
    let output = ensure_outputs_directory(output_raw);
    let sigma = args.get("sigma").and_then(|v| v.as_f64()).unwrap_or(1.0);
    let block_size = args.get("block_size").and_then(|v| v.as_u64()).unwrap_or(16) as u32;
    let mode = args.get("mode").and_then(|v| v.as_str()).unwrap_or("basic");
    crate::visual::denoise_video_bm3d(input, &output, sigma, block_size, mode).unwrap_or_else(|e| e)
}

fn execute_denoise_video_bm3d_gemini(args: &HashMap<String, Value>) -> String {
    let input = args.get("input_file").and_then(|v| v.as_str()).unwrap_or("");
    let output_raw = args.get("output_file").and_then(|v| v.as_str()).unwrap_or("");
    let output = ensure_outputs_directory(output_raw);
    let sigma = args.get("sigma").and_then(|v| v.as_f64()).unwrap_or(1.0);
    let block_size = args.get("block_size").and_then(|v| v.as_u64()).unwrap_or(16) as u32;
    let mode = args.get("mode").and_then(|v| v.as_str()).unwrap_or("basic");
    crate::visual::denoise_video_bm3d(input, &output, sigma, block_size, mode).unwrap_or_else(|e| e)
}

fn execute_deshake_video_claude(args: &Value) -> String {
    let input = args["input_file"].as_str().unwrap_or("");
    let output_raw = args["output_file"].as_str().unwrap_or("");
    let output = ensure_outputs_directory(output_raw);
    let x = args.get("x").and_then(|v| v.as_i64()).unwrap_or(-1) as i32;
    let y = args.get("y").and_then(|v| v.as_i64()).unwrap_or(-1) as i32;
    let w = args.get("w").and_then(|v| v.as_i64()).unwrap_or(-1) as i32;
    let h = args.get("h").and_then(|v| v.as_i64()).unwrap_or(-1) as i32;
    let rx = args.get("rx").and_then(|v| v.as_u64()).unwrap_or(16) as u32;
    let ry = args.get("ry").and_then(|v| v.as_u64()).unwrap_or(16) as u32;
    crate::transform::deshake_video(input, &output, x, y, w, h, rx, ry).unwrap_or_else(|e| e)
}

fn execute_deshake_video_gemini(args: &HashMap<String, Value>) -> String {
    let input = args.get("input_file").and_then(|v| v.as_str()).unwrap_or("");
    let output_raw = args.get("output_file").and_then(|v| v.as_str()).unwrap_or("");
    let output = ensure_outputs_directory(output_raw);
    let x = args.get("x").and_then(|v| v.as_i64()).unwrap_or(-1) as i32;
    let y = args.get("y").and_then(|v| v.as_i64()).unwrap_or(-1) as i32;
    let w = args.get("w").and_then(|v| v.as_i64()).unwrap_or(-1) as i32;
    let h = args.get("h").and_then(|v| v.as_i64()).unwrap_or(-1) as i32;
    let rx = args.get("rx").and_then(|v| v.as_u64()).unwrap_or(16) as u32;
    let ry = args.get("ry").and_then(|v| v.as_u64()).unwrap_or(16) as u32;
    crate::transform::deshake_video(input, &output, x, y, w, h, rx, ry).unwrap_or_else(|e| e)
}

fn execute_measure_lufs_claude(args: &Value) -> String {
    let input = args["input_file"].as_str().unwrap_or("");
    let target = args.get("target_lufs").and_then(|v| v.as_f64()).unwrap_or(-23.0);
    crate::audio::measure_lufs(input, target).unwrap_or_else(|e| e)
}

fn execute_measure_lufs_gemini(args: &HashMap<String, Value>) -> String {
    let input = args.get("input_file").and_then(|v| v.as_str()).unwrap_or("");
    let target = args.get("target_lufs").and_then(|v| v.as_f64()).unwrap_or(-23.0);
    crate::audio::measure_lufs(input, target).unwrap_or_else(|e| e)
}

fn execute_parametric_eq_claude(args: &Value) -> String {
    let input = args["input_file"].as_str().unwrap_or("");
    let output_raw = args["output_file"].as_str().unwrap_or("");
    let output = ensure_outputs_directory(output_raw);
    let params = args["eq_params"].as_str().unwrap_or("");
    crate::audio::parametric_eq(input, &output, params).unwrap_or_else(|e| e)
}

fn execute_parametric_eq_gemini(args: &HashMap<String, Value>) -> String {
    let input = args.get("input_file").and_then(|v| v.as_str()).unwrap_or("");
    let output_raw = args.get("output_file").and_then(|v| v.as_str()).unwrap_or("");
    let output = ensure_outputs_directory(output_raw);
    let params = args.get("eq_params").and_then(|v| v.as_str()).unwrap_or("");
    crate::audio::parametric_eq(input, &output, params).unwrap_or_else(|e| e)
}

fn execute_audio_limiter_claude(args: &Value) -> String {
    let input = args["input_file"].as_str().unwrap_or("");
    let output_raw = args["output_file"].as_str().unwrap_or("");
    let output = ensure_outputs_directory(output_raw);
    let limit = args.get("limit_db").and_then(|v| v.as_f64()).unwrap_or(-1.0);
    let attack = args.get("attack_ms").and_then(|v| v.as_f64()).unwrap_or(5.0);
    let release = args.get("release_ms").and_then(|v| v.as_f64()).unwrap_or(50.0);
    let asc = args.get("auto_sc").and_then(|v| v.as_bool()).unwrap_or(false);
    crate::audio::audio_limiter(input, &output, limit, attack, release, asc).unwrap_or_else(|e| e)
}

fn execute_audio_limiter_gemini(args: &HashMap<String, Value>) -> String {
    let input = args.get("input_file").and_then(|v| v.as_str()).unwrap_or("");
    let output_raw = args.get("output_file").and_then(|v| v.as_str()).unwrap_or("");
    let output = ensure_outputs_directory(output_raw);
    let limit = args.get("limit_db").and_then(|v| v.as_f64()).unwrap_or(-1.0);
    let attack = args.get("attack_ms").and_then(|v| v.as_f64()).unwrap_or(5.0);
    let release = args.get("release_ms").and_then(|v| v.as_f64()).unwrap_or(50.0);
    let asc = args.get("auto_sc").and_then(|v| v.as_bool()).unwrap_or(false);
    crate::audio::audio_limiter(input, &output, limit, attack, release, asc).unwrap_or_else(|e| e)
}

fn execute_reduce_sibilance_claude(args: &Value) -> String {
    let input = args["input_file"].as_str().unwrap_or("");
    let output_raw = args["output_file"].as_str().unwrap_or("");
    let output = ensure_outputs_directory(output_raw);
    let split = args.get("split_hz").and_then(|v| v.as_f64()).unwrap_or(8500.0);
    let threshold = args.get("threshold").and_then(|v| v.as_f64()).unwrap_or(0.1);
    let mode = args.get("mode").and_then(|v| v.as_str()).unwrap_or("split");
    crate::audio::reduce_sibilance(input, &output, split, threshold, mode).unwrap_or_else(|e| e)
}

fn execute_reduce_sibilance_gemini(args: &HashMap<String, Value>) -> String {
    let input = args.get("input_file").and_then(|v| v.as_str()).unwrap_or("");
    let output_raw = args.get("output_file").and_then(|v| v.as_str()).unwrap_or("");
    let output = ensure_outputs_directory(output_raw);
    let split = args.get("split_hz").and_then(|v| v.as_f64()).unwrap_or(8500.0);
    let threshold = args.get("threshold").and_then(|v| v.as_f64()).unwrap_or(0.1);
    let mode = args.get("mode").and_then(|v| v.as_str()).unwrap_or("split");
    crate::audio::reduce_sibilance(input, &output, split, threshold, mode).unwrap_or_else(|e| e)
}

fn execute_denoise_speech_rnn_claude(args: &Value) -> String {
    let input = args["input_file"].as_str().unwrap_or("");
    let output_raw = args["output_file"].as_str().unwrap_or("");
    let output = ensure_outputs_directory(output_raw);
    let model = args.get("model_file").and_then(|v| v.as_str()).unwrap_or("");
    let mix = args.get("mix").and_then(|v| v.as_f64()).unwrap_or(1.0);
    crate::audio::denoise_speech_rnn(input, &output, model, mix).unwrap_or_else(|e| e)
}

fn execute_denoise_speech_rnn_gemini(args: &HashMap<String, Value>) -> String {
    let input = args.get("input_file").and_then(|v| v.as_str()).unwrap_or("");
    let output_raw = args.get("output_file").and_then(|v| v.as_str()).unwrap_or("");
    let output = ensure_outputs_directory(output_raw);
    let model = args.get("model_file").and_then(|v| v.as_str()).unwrap_or("");
    let mix = args.get("mix").and_then(|v| v.as_f64()).unwrap_or(1.0);
    crate::audio::denoise_speech_rnn(input, &output, model, mix).unwrap_or_else(|e| e)
}

// ============================================================================
// PHASE E executor functions
// ============================================================================

fn execute_analyze_vectorscope_claude(args: &Value) -> String {
    let input = args["input_file"].as_str().unwrap_or("");
    let output_raw = args["output_file"].as_str().unwrap_or("");
    let output = ensure_outputs_directory(output_raw);
    let mode = args.get("mode").and_then(|v| v.as_str()).unwrap_or("color");
    crate::visual::analyze_vectorscope(input, &output, mode).unwrap_or_else(|e| e)
}

fn execute_analyze_vectorscope_gemini(args: &HashMap<String, Value>) -> String {
    let input = args.get("input_file").and_then(|v| v.as_str()).unwrap_or("");
    let output_raw = args.get("output_file").and_then(|v| v.as_str()).unwrap_or("");
    let output = ensure_outputs_directory(output_raw);
    let mode = args.get("mode").and_then(|v| v.as_str()).unwrap_or("color");
    crate::visual::analyze_vectorscope(input, &output, mode).unwrap_or_else(|e| e)
}

fn execute_analyze_waveform_claude(args: &Value) -> String {
    let input = args["input_file"].as_str().unwrap_or("");
    let output_raw = args["output_file"].as_str().unwrap_or("");
    let output = ensure_outputs_directory(output_raw);
    let mode = args.get("mode").and_then(|v| v.as_str()).unwrap_or("row");
    let filter_type = args.get("filter_type").and_then(|v| v.as_str()).unwrap_or("lowpass");
    crate::visual::analyze_waveform(input, &output, mode, filter_type).unwrap_or_else(|e| e)
}

fn execute_analyze_waveform_gemini(args: &HashMap<String, Value>) -> String {
    let input = args.get("input_file").and_then(|v| v.as_str()).unwrap_or("");
    let output_raw = args.get("output_file").and_then(|v| v.as_str()).unwrap_or("");
    let output = ensure_outputs_directory(output_raw);
    let mode = args.get("mode").and_then(|v| v.as_str()).unwrap_or("row");
    let filter_type = args.get("filter_type").and_then(|v| v.as_str()).unwrap_or("lowpass");
    crate::visual::analyze_waveform(input, &output, mode, filter_type).unwrap_or_else(|e| e)
}

fn execute_draw_grid_claude(args: &Value) -> String {
    let input = args["input_file"].as_str().unwrap_or("");
    let output_raw = args["output_file"].as_str().unwrap_or("");
    let output = ensure_outputs_directory(output_raw);
    let width = args.get("width").and_then(|v| v.as_u64()).unwrap_or(100) as u32;
    let height = args.get("height").and_then(|v| v.as_u64()).unwrap_or(100) as u32;
    let thickness = args.get("thickness").and_then(|v| v.as_u64()).unwrap_or(1) as u32;
    let color = args.get("color").and_then(|v| v.as_str()).unwrap_or("white@0.5");
    crate::visual::draw_grid(input, &output, width, height, thickness, color).unwrap_or_else(|e| e)
}

fn execute_draw_grid_gemini(args: &HashMap<String, Value>) -> String {
    let input = args.get("input_file").and_then(|v| v.as_str()).unwrap_or("");
    let output_raw = args.get("output_file").and_then(|v| v.as_str()).unwrap_or("");
    let output = ensure_outputs_directory(output_raw);
    let width = args.get("width").and_then(|v| v.as_u64()).unwrap_or(100) as u32;
    let height = args.get("height").and_then(|v| v.as_u64()).unwrap_or(100) as u32;
    let thickness = args.get("thickness").and_then(|v| v.as_u64()).unwrap_or(1) as u32;
    let color = args.get("color").and_then(|v| v.as_str()).unwrap_or("white@0.5");
    crate::visual::draw_grid(input, &output, width, height, thickness, color).unwrap_or_else(|e| e)
}

fn execute_grid_stack_videos_claude(args: &Value) -> String {
    let output_raw = args["output_file"].as_str().unwrap_or("");
    let output = ensure_outputs_directory(output_raw);
    let layout = args.get("layout").and_then(|v| v.as_str()).unwrap_or("");
    let files: Vec<String> = if let Some(arr) = args.get("input_files").and_then(|v| v.as_array()) {
        arr.iter().filter_map(|v| v.as_str()).map(|s| s.to_string()).collect()
    } else if let Some(s) = args.get("input_files").and_then(|v| v.as_str()) {
        s.split(',').map(|s| s.trim().to_string()).collect()
    } else {
        return "❌ No input files provided for grid_stack_videos".to_string();
    };
    crate::visual::grid_stack_videos(&files, &output, layout).unwrap_or_else(|e| e)
}

fn execute_grid_stack_videos_gemini(args: &HashMap<String, Value>) -> String {
    let output_raw = args.get("output_file").and_then(|v| v.as_str()).unwrap_or("");
    let output = ensure_outputs_directory(output_raw);
    let layout = args.get("layout").and_then(|v| v.as_str()).unwrap_or("");
    let files: Vec<String> = if let Some(s) = args.get("input_files").and_then(|v| v.as_str()) {
        s.split(',').map(|s| s.trim().to_string()).collect()
    } else {
        return "❌ No input files provided for grid_stack_videos".to_string();
    };
    crate::visual::grid_stack_videos(&files, &output, layout).unwrap_or_else(|e| e)
}

fn execute_luma_key_claude(args: &Value) -> String {
    let input = args["input_file"].as_str().unwrap_or("");
    let output_raw = args["output_file"].as_str().unwrap_or("");
    let output = ensure_outputs_directory(output_raw);
    let threshold = args.get("threshold").and_then(|v| v.as_f64()).unwrap_or(0.1);
    let tolerance = args.get("tolerance").and_then(|v| v.as_f64()).unwrap_or(0.1);
    let softness = args.get("softness").and_then(|v| v.as_f64()).unwrap_or(0.0);
    crate::visual::luma_key(input, &output, threshold, tolerance, softness).unwrap_or_else(|e| e)
}

fn execute_luma_key_gemini(args: &HashMap<String, Value>) -> String {
    let input = args.get("input_file").and_then(|v| v.as_str()).unwrap_or("");
    let output_raw = args.get("output_file").and_then(|v| v.as_str()).unwrap_or("");
    let output = ensure_outputs_directory(output_raw);
    let threshold = args.get("threshold").and_then(|v| v.as_f64()).unwrap_or(0.1);
    let tolerance = args.get("tolerance").and_then(|v| v.as_f64()).unwrap_or(0.1);
    let softness = args.get("softness").and_then(|v| v.as_f64()).unwrap_or(0.0);
    crate::visual::luma_key(input, &output, threshold, tolerance, softness).unwrap_or_else(|e| e)
}

fn execute_render_binaural_claude(args: &Value) -> String {
    let input = args["input_file"].as_str().unwrap_or("");
    let output_raw = args["output_file"].as_str().unwrap_or("");
    let output = ensure_outputs_directory(output_raw);
    let hrir_type = args.get("hrir_type").and_then(|v| v.as_str()).unwrap_or("stereo");
    crate::audio::render_binaural(input, &output, hrir_type).unwrap_or_else(|e| e)
}

fn execute_render_binaural_gemini(args: &HashMap<String, Value>) -> String {
    let input = args.get("input_file").and_then(|v| v.as_str()).unwrap_or("");
    let output_raw = args.get("output_file").and_then(|v| v.as_str()).unwrap_or("");
    let output = ensure_outputs_directory(output_raw);
    let hrir_type = args.get("hrir_type").and_then(|v| v.as_str()).unwrap_or("stereo");
    crate::audio::render_binaural(input, &output, hrir_type).unwrap_or_else(|e| e)
}

fn execute_add_vibrato_claude(args: &Value) -> String {
    let input = args["input_file"].as_str().unwrap_or("");
    let output_raw = args["output_file"].as_str().unwrap_or("");
    let output = ensure_outputs_directory(output_raw);
    let frequency = args.get("frequency").and_then(|v| v.as_f64()).unwrap_or(5.0);
    let depth = args.get("depth").and_then(|v| v.as_f64()).unwrap_or(0.5);
    crate::audio::add_vibrato(input, &output, frequency, depth).unwrap_or_else(|e| e)
}

fn execute_add_vibrato_gemini(args: &HashMap<String, Value>) -> String {
    let input = args.get("input_file").and_then(|v| v.as_str()).unwrap_or("");
    let output_raw = args.get("output_file").and_then(|v| v.as_str()).unwrap_or("");
    let output = ensure_outputs_directory(output_raw);
    let frequency = args.get("frequency").and_then(|v| v.as_f64()).unwrap_or(5.0);
    let depth = args.get("depth").and_then(|v| v.as_f64()).unwrap_or(0.5);
    crate::audio::add_vibrato(input, &output, frequency, depth).unwrap_or_else(|e| e)
}

fn execute_add_tremolo_claude(args: &Value) -> String {
    let input = args["input_file"].as_str().unwrap_or("");
    let output_raw = args["output_file"].as_str().unwrap_or("");
    let output = ensure_outputs_directory(output_raw);
    let frequency = args.get("frequency").and_then(|v| v.as_f64()).unwrap_or(5.0);
    let depth = args.get("depth").and_then(|v| v.as_f64()).unwrap_or(0.5);
    crate::audio::add_tremolo(input, &output, frequency, depth).unwrap_or_else(|e| e)
}

fn execute_add_tremolo_gemini(args: &HashMap<String, Value>) -> String {
    let input = args.get("input_file").and_then(|v| v.as_str()).unwrap_or("");
    let output_raw = args.get("output_file").and_then(|v| v.as_str()).unwrap_or("");
    let output = ensure_outputs_directory(output_raw);
    let frequency = args.get("frequency").and_then(|v| v.as_f64()).unwrap_or(5.0);
    let depth = args.get("depth").and_then(|v| v.as_f64()).unwrap_or(0.5);
    crate::audio::add_tremolo(input, &output, frequency, depth).unwrap_or_else(|e| e)
}

fn execute_add_flanger_claude(args: &Value) -> String {
    let input = args["input_file"].as_str().unwrap_or("");
    let output_raw = args["output_file"].as_str().unwrap_or("");
    let output = ensure_outputs_directory(output_raw);
    let delay = args.get("delay").and_then(|v| v.as_f64()).unwrap_or(0.0);
    let depth = args.get("depth").and_then(|v| v.as_f64()).unwrap_or(2.0);
    let speed = args.get("speed").and_then(|v| v.as_f64()).unwrap_or(0.5);
    let shape = args.get("shape").and_then(|v| v.as_str()).unwrap_or("sinusoidal");
    crate::audio::add_flanger(input, &output, delay, depth, speed, shape).unwrap_or_else(|e| e)
}

fn execute_add_flanger_gemini(args: &HashMap<String, Value>) -> String {
    let input = args.get("input_file").and_then(|v| v.as_str()).unwrap_or("");
    let output_raw = args.get("output_file").and_then(|v| v.as_str()).unwrap_or("");
    let output = ensure_outputs_directory(output_raw);
    let delay = args.get("delay").and_then(|v| v.as_f64()).unwrap_or(0.0);
    let depth = args.get("depth").and_then(|v| v.as_f64()).unwrap_or(2.0);
    let speed = args.get("speed").and_then(|v| v.as_f64()).unwrap_or(0.5);
    let shape = args.get("shape").and_then(|v| v.as_str()).unwrap_or("sinusoidal");
    crate::audio::add_flanger(input, &output, delay, depth, speed, shape).unwrap_or_else(|e| e)
}

fn execute_denoise_audio_nlm_claude(args: &Value) -> String {
    let input = args["input_file"].as_str().unwrap_or("");
    let output_raw = args["output_file"].as_str().unwrap_or("");
    let output = ensure_outputs_directory(output_raw);
    let strength = args.get("strength").and_then(|v| v.as_f64()).unwrap_or(0.0001);
    let patch_size = args.get("patch_size").and_then(|v| v.as_f64()).unwrap_or(0.002);
    let research_size = args.get("research_size").and_then(|v| v.as_f64()).unwrap_or(0.002);
    crate::audio::denoise_audio_nlm(input, &output, strength, patch_size, research_size).unwrap_or_else(|e| e)
}

fn execute_denoise_audio_nlm_gemini(args: &HashMap<String, Value>) -> String {
    let input = args.get("input_file").and_then(|v| v.as_str()).unwrap_or("");
    let output_raw = args.get("output_file").and_then(|v| v.as_str()).unwrap_or("");
    let output = ensure_outputs_directory(output_raw);
    let strength = args.get("strength").and_then(|v| v.as_f64()).unwrap_or(0.0001);
    let patch_size = args.get("patch_size").and_then(|v| v.as_f64()).unwrap_or(0.002);
    let research_size = args.get("research_size").and_then(|v| v.as_f64()).unwrap_or(0.002);
    crate::audio::denoise_audio_nlm(input, &output, strength, patch_size, research_size).unwrap_or_else(|e| e)
}

// ============================================================================
// PHASE F executor functions
// ============================================================================

fn execute_displace_video_claude(args: &Value) -> String {
    let input = args["input_file"].as_str().unwrap_or("");
    let xmap = args["xmap_file"].as_str().unwrap_or("");
    let ymap = args["ymap_file"].as_str().unwrap_or("");
    let output_raw = args["output_file"].as_str().unwrap_or("");
    let output = ensure_outputs_directory(output_raw);
    let edge = args.get("edge").and_then(|v| v.as_str()).unwrap_or("smear");
    crate::visual::displace_video(input, xmap, ymap, &output, edge).unwrap_or_else(|e| e)
}

fn execute_displace_video_gemini(args: &HashMap<String, Value>) -> String {
    let input = args.get("input_file").and_then(|v| v.as_str()).unwrap_or("");
    let xmap = args.get("xmap_file").and_then(|v| v.as_str()).unwrap_or("");
    let ymap = args.get("ymap_file").and_then(|v| v.as_str()).unwrap_or("");
    let output_raw = args.get("output_file").and_then(|v| v.as_str()).unwrap_or("");
    let output = ensure_outputs_directory(output_raw);
    let edge = args.get("edge").and_then(|v| v.as_str()).unwrap_or("smear");
    crate::visual::displace_video(input, xmap, ymap, &output, edge).unwrap_or_else(|e| e)
}

fn execute_decimate_frames_claude(args: &Value) -> String {
    let input = args["input_file"].as_str().unwrap_or("");
    let output_raw = args["output_file"].as_str().unwrap_or("");
    let output = ensure_outputs_directory(output_raw);
    let cycle = args.get("cycle").and_then(|v| v.as_u64()).unwrap_or(5) as u32;
    let dupthresh = args.get("dupthresh").and_then(|v| v.as_f64()).unwrap_or(1.1);
    let scthresh = args.get("scthresh").and_then(|v| v.as_f64()).unwrap_or(15.0);
    crate::visual::decimate_frames(input, &output, cycle, dupthresh, scthresh).unwrap_or_else(|e| e)
}

fn execute_decimate_frames_gemini(args: &HashMap<String, Value>) -> String {
    let input = args.get("input_file").and_then(|v| v.as_str()).unwrap_or("");
    let output_raw = args.get("output_file").and_then(|v| v.as_str()).unwrap_or("");
    let output = ensure_outputs_directory(output_raw);
    let cycle = args.get("cycle").and_then(|v| v.as_u64()).unwrap_or(5) as u32;
    let dupthresh = args.get("dupthresh").and_then(|v| v.as_f64()).unwrap_or(1.1);
    let scthresh = args.get("scthresh").and_then(|v| v.as_f64()).unwrap_or(15.0);
    crate::visual::decimate_frames(input, &output, cycle, dupthresh, scthresh).unwrap_or_else(|e| e)
}

fn execute_denoise_video_owden_claude(args: &Value) -> String {
    let input = args["input_file"].as_str().unwrap_or("");
    let output_raw = args["output_file"].as_str().unwrap_or("");
    let output = ensure_outputs_directory(output_raw);
    let luma = args.get("luma_strength").and_then(|v| v.as_f64()).unwrap_or(10.0);
    let chroma = args.get("chroma_strength").and_then(|v| v.as_f64()).unwrap_or(10.0);
    crate::visual::denoise_video_owden(input, &output, luma, chroma).unwrap_or_else(|e| e)
}

fn execute_denoise_video_owden_gemini(args: &HashMap<String, Value>) -> String {
    let input = args.get("input_file").and_then(|v| v.as_str()).unwrap_or("");
    let output_raw = args.get("output_file").and_then(|v| v.as_str()).unwrap_or("");
    let output = ensure_outputs_directory(output_raw);
    let luma = args.get("luma_strength").and_then(|v| v.as_f64()).unwrap_or(10.0);
    let chroma = args.get("chroma_strength").and_then(|v| v.as_f64()).unwrap_or(10.0);
    crate::visual::denoise_video_owden(input, &output, luma, chroma).unwrap_or_else(|e| e)
}

fn execute_despill_video_claude(args: &Value) -> String {
    let input = args["input_file"].as_str().unwrap_or("");
    let output_raw = args["output_file"].as_str().unwrap_or("");
    let output = ensure_outputs_directory(output_raw);
    let spill_type = args.get("spill_type").and_then(|v| v.as_str()).unwrap_or("green");
    let mix = args.get("mix").and_then(|v| v.as_f64()).unwrap_or(0.5);
    let expand = args.get("expand").and_then(|v| v.as_f64()).unwrap_or(0.0);
    crate::visual::despill_video(input, &output, spill_type, mix, expand).unwrap_or_else(|e| e)
}

fn execute_despill_video_gemini(args: &HashMap<String, Value>) -> String {
    let input = args.get("input_file").and_then(|v| v.as_str()).unwrap_or("");
    let output_raw = args.get("output_file").and_then(|v| v.as_str()).unwrap_or("");
    let output = ensure_outputs_directory(output_raw);
    let spill_type = args.get("spill_type").and_then(|v| v.as_str()).unwrap_or("green");
    let mix = args.get("mix").and_then(|v| v.as_f64()).unwrap_or(0.5);
    let expand = args.get("expand").and_then(|v| v.as_f64()).unwrap_or(0.0);
    crate::visual::despill_video(input, &output, spill_type, mix, expand).unwrap_or_else(|e| e)
}

fn execute_remap_pixels_claude(args: &Value) -> String {
    let input = args["input_file"].as_str().unwrap_or("");
    let xmap = args["xmap_file"].as_str().unwrap_or("");
    let ymap = args["ymap_file"].as_str().unwrap_or("");
    let output_raw = args["output_file"].as_str().unwrap_or("");
    let output = ensure_outputs_directory(output_raw);
    let fill = args.get("fill").and_then(|v| v.as_str()).unwrap_or("black");
    crate::visual::remap_pixels(input, xmap, ymap, &output, fill).unwrap_or_else(|e| e)
}

fn execute_remap_pixels_gemini(args: &HashMap<String, Value>) -> String {
    let input = args.get("input_file").and_then(|v| v.as_str()).unwrap_or("");
    let xmap = args.get("xmap_file").and_then(|v| v.as_str()).unwrap_or("");
    let ymap = args.get("ymap_file").and_then(|v| v.as_str()).unwrap_or("");
    let output_raw = args.get("output_file").and_then(|v| v.as_str()).unwrap_or("");
    let output = ensure_outputs_directory(output_raw);
    let fill = args.get("fill").and_then(|v| v.as_str()).unwrap_or("black");
    crate::visual::remap_pixels(input, xmap, ymap, &output, fill).unwrap_or_else(|e| e)
}

fn execute_adjust_exposure_claude(args: &Value) -> String {
    let input = args["input_file"].as_str().unwrap_or("");
    let output_raw = args["output_file"].as_str().unwrap_or("");
    let output = ensure_outputs_directory(output_raw);
    let exposure = args.get("exposure").and_then(|v| v.as_f64()).unwrap_or(0.0);
    let black = args.get("black").and_then(|v| v.as_f64()).unwrap_or(0.0);
    crate::visual::adjust_exposure(input, &output, exposure, black).unwrap_or_else(|e| e)
}

fn execute_adjust_exposure_gemini(args: &HashMap<String, Value>) -> String {
    let input = args.get("input_file").and_then(|v| v.as_str()).unwrap_or("");
    let output_raw = args.get("output_file").and_then(|v| v.as_str()).unwrap_or("");
    let output = ensure_outputs_directory(output_raw);
    let exposure = args.get("exposure").and_then(|v| v.as_f64()).unwrap_or(0.0);
    let black = args.get("black").and_then(|v| v.as_f64()).unwrap_or(0.0);
    crate::visual::adjust_exposure(input, &output, exposure, black).unwrap_or_else(|e| e)
}

fn execute_measure_vmaf_claude(args: &Value) -> String {
    let distorted = args["distorted_file"].as_str().unwrap_or("");
    let reference = args["reference_file"].as_str().unwrap_or("");
    let model = args.get("model_path").and_then(|v| v.as_str()).unwrap_or("");
    crate::visual::measure_vmaf(distorted, reference, model).unwrap_or_else(|e| e)
}

fn execute_measure_vmaf_gemini(args: &HashMap<String, Value>) -> String {
    let distorted = args.get("distorted_file").and_then(|v| v.as_str()).unwrap_or("");
    let reference = args.get("reference_file").and_then(|v| v.as_str()).unwrap_or("");
    let model = args.get("model_path").and_then(|v| v.as_str()).unwrap_or("");
    crate::visual::measure_vmaf(distorted, reference, model).unwrap_or_else(|e| e)
}

fn execute_shift_audio_frequency_claude(args: &Value) -> String {
    let input = args["input_file"].as_str().unwrap_or("");
    let output_raw = args["output_file"].as_str().unwrap_or("");
    let output = ensure_outputs_directory(output_raw);
    let shift = args["shift"].as_f64().unwrap_or(0.0);
    crate::audio::shift_audio_frequency(input, &output, shift).unwrap_or_else(|e| e)
}

fn execute_shift_audio_frequency_gemini(args: &HashMap<String, Value>) -> String {
    let input = args.get("input_file").and_then(|v| v.as_str()).unwrap_or("");
    let output_raw = args.get("output_file").and_then(|v| v.as_str()).unwrap_or("");
    let output = ensure_outputs_directory(output_raw);
    let shift = args.get("shift").and_then(|v| v.as_f64()).unwrap_or(0.0);
    crate::audio::shift_audio_frequency(input, &output, shift).unwrap_or_else(|e| e)
}

fn execute_apply_audio_pulsator_claude(args: &Value) -> String {
    let input = args["input_file"].as_str().unwrap_or("");
    let output_raw = args["output_file"].as_str().unwrap_or("");
    let output = ensure_outputs_directory(output_raw);
    let hz = args.get("hz").and_then(|v| v.as_f64()).unwrap_or(2.0);
    let amount = args.get("amount").and_then(|v| v.as_f64()).unwrap_or(1.0);
    let offset_l = args.get("offset_l").and_then(|v| v.as_f64()).unwrap_or(0.0);
    let offset_r = args.get("offset_r").and_then(|v| v.as_f64()).unwrap_or(0.5);
    let mode = args.get("mode").and_then(|v| v.as_str()).unwrap_or("sine");
    crate::audio::apply_audio_pulsator(input, &output, hz, amount, offset_l, offset_r, mode).unwrap_or_else(|e| e)
}

fn execute_apply_audio_pulsator_gemini(args: &HashMap<String, Value>) -> String {
    let input = args.get("input_file").and_then(|v| v.as_str()).unwrap_or("");
    let output_raw = args.get("output_file").and_then(|v| v.as_str()).unwrap_or("");
    let output = ensure_outputs_directory(output_raw);
    let hz = args.get("hz").and_then(|v| v.as_f64()).unwrap_or(2.0);
    let amount = args.get("amount").and_then(|v| v.as_f64()).unwrap_or(1.0);
    let offset_l = args.get("offset_l").and_then(|v| v.as_f64()).unwrap_or(0.0);
    let offset_r = args.get("offset_r").and_then(|v| v.as_f64()).unwrap_or(0.5);
    let mode = args.get("mode").and_then(|v| v.as_str()).unwrap_or("sine");
    crate::audio::apply_audio_pulsator(input, &output, hz, amount, offset_l, offset_r, mode).unwrap_or_else(|e| e)
}

fn execute_enhance_dialogue_claude(args: &Value) -> String {
    let input = args["input_file"].as_str().unwrap_or("");
    let output_raw = args["output_file"].as_str().unwrap_or("");
    let output = ensure_outputs_directory(output_raw);
    let original = args.get("original").and_then(|v| v.as_f64()).unwrap_or(0.5);
    let expand = args.get("expand").and_then(|v| v.as_f64()).unwrap_or(2.0);
    crate::audio::enhance_dialogue(input, &output, original, expand).unwrap_or_else(|e| e)
}

fn execute_enhance_dialogue_gemini(args: &HashMap<String, Value>) -> String {
    let input = args.get("input_file").and_then(|v| v.as_str()).unwrap_or("");
    let output_raw = args.get("output_file").and_then(|v| v.as_str()).unwrap_or("");
    let output = ensure_outputs_directory(output_raw);
    let original = args.get("original").and_then(|v| v.as_f64()).unwrap_or(0.5);
    let expand = args.get("expand").and_then(|v| v.as_f64()).unwrap_or(2.0);
    crate::audio::enhance_dialogue(input, &output, original, expand).unwrap_or_else(|e| e)
}

fn execute_split_audio_channels_claude(args: &Value) -> String {
    let input = args["input_file"].as_str().unwrap_or("");
    let output_raw = args["output_file"].as_str().unwrap_or("");
    let output = ensure_outputs_directory(output_raw);
    let layout = args.get("channel_layout").and_then(|v| v.as_str()).unwrap_or("stereo");
    let channel = args.get("channel").and_then(|v| v.as_str()).unwrap_or("FL");
    crate::audio::split_audio_channels(input, &output, layout, channel).unwrap_or_else(|e| e)
}

fn execute_split_audio_channels_gemini(args: &HashMap<String, Value>) -> String {
    let input = args.get("input_file").and_then(|v| v.as_str()).unwrap_or("");
    let output_raw = args.get("output_file").and_then(|v| v.as_str()).unwrap_or("");
    let output = ensure_outputs_directory(output_raw);
    let layout = args.get("channel_layout").and_then(|v| v.as_str()).unwrap_or("stereo");
    let channel = args.get("channel").and_then(|v| v.as_str()).unwrap_or("FL");
    crate::audio::split_audio_channels(input, &output, layout, channel).unwrap_or_else(|e| e)
}

fn execute_map_audio_channels_claude(args: &Value) -> String {
    let input = args["input_file"].as_str().unwrap_or("");
    let output_raw = args["output_file"].as_str().unwrap_or("");
    let output = ensure_outputs_directory(output_raw);
    let map = args.get("channel_map").and_then(|v| v.as_str()).unwrap_or("FL-FL|FR-FR");
    let layout = args.get("channel_layout").and_then(|v| v.as_str()).unwrap_or("stereo");
    crate::audio::map_audio_channels(input, &output, map, layout).unwrap_or_else(|e| e)
}

fn execute_map_audio_channels_gemini(args: &HashMap<String, Value>) -> String {
    let input = args.get("input_file").and_then(|v| v.as_str()).unwrap_or("");
    let output_raw = args.get("output_file").and_then(|v| v.as_str()).unwrap_or("");
    let output = ensure_outputs_directory(output_raw);
    let map = args.get("channel_map").and_then(|v| v.as_str()).unwrap_or("FL-FL|FR-FR");
    let layout = args.get("channel_layout").and_then(|v| v.as_str()).unwrap_or("stereo");
    crate::audio::map_audio_channels(input, &output, map, layout).unwrap_or_else(|e| e)
}

fn execute_merge_audio_inputs_claude(args: &Value) -> String {
    let output_raw = args["output_file"].as_str().unwrap_or("");
    let output = ensure_outputs_directory(output_raw);
    let files: Vec<String> = if let Some(arr) = args.get("input_files").and_then(|v| v.as_array()) {
        arr.iter().filter_map(|v| v.as_str()).map(|s| s.to_string()).collect()
    } else if let Some(s) = args.get("input_files").and_then(|v| v.as_str()) {
        s.split(',').map(|s| s.trim().to_string()).collect()
    } else {
        return "❌ No input files provided for merge_audio_inputs".to_string();
    };
    crate::audio::merge_audio_inputs(&files, &output).unwrap_or_else(|e| e)
}

fn execute_merge_audio_inputs_gemini(args: &HashMap<String, Value>) -> String {
    let output_raw = args.get("output_file").and_then(|v| v.as_str()).unwrap_or("");
    let output = ensure_outputs_directory(output_raw);
    let files: Vec<String> = if let Some(s) = args.get("input_files").and_then(|v| v.as_str()) {
        s.split(',').map(|s| s.trim().to_string()).collect()
    } else {
        return "❌ No input files provided for merge_audio_inputs".to_string();
    };
    crate::audio::merge_audio_inputs(&files, &output).unwrap_or_else(|e| e)
}

fn execute_apply_crossfeed_claude(args: &Value) -> String {
    let input = args["input_file"].as_str().unwrap_or("");
    let output_raw = args["output_file"].as_str().unwrap_or("");
    let output = ensure_outputs_directory(output_raw);
    let strength = args.get("strength").and_then(|v| v.as_f64()).unwrap_or(0.5);
    let slope = args.get("slope").and_then(|v| v.as_f64()).unwrap_or(0.5);
    let level_in = args.get("level_in").and_then(|v| v.as_f64()).unwrap_or(1.0);
    let level_out = args.get("level_out").and_then(|v| v.as_f64()).unwrap_or(1.0);
    crate::audio::apply_crossfeed(input, &output, strength, slope, level_in, level_out).unwrap_or_else(|e| e)
}

fn execute_apply_crossfeed_gemini(args: &HashMap<String, Value>) -> String {
    let input = args.get("input_file").and_then(|v| v.as_str()).unwrap_or("");
    let output_raw = args.get("output_file").and_then(|v| v.as_str()).unwrap_or("");
    let output = ensure_outputs_directory(output_raw);
    let strength = args.get("strength").and_then(|v| v.as_f64()).unwrap_or(0.5);
    let slope = args.get("slope").and_then(|v| v.as_f64()).unwrap_or(0.5);
    let level_in = args.get("level_in").and_then(|v| v.as_f64()).unwrap_or(1.0);
    let level_out = args.get("level_out").and_then(|v| v.as_f64()).unwrap_or(1.0);
    crate::audio::apply_crossfeed(input, &output, strength, slope, level_in, level_out).unwrap_or_else(|e| e)
}

fn execute_apply_extrastereo_claude(args: &Value) -> String {
    let input = args["input_file"].as_str().unwrap_or("");
    let output_raw = args["output_file"].as_str().unwrap_or("");
    let output = ensure_outputs_directory(output_raw);
    let multiplier = args.get("multiplier").and_then(|v| v.as_f64()).unwrap_or(2.5);
    let clipping = args.get("clipping").and_then(|v| v.as_bool()).unwrap_or(false);
    crate::audio::apply_extrastereo(input, &output, multiplier, clipping).unwrap_or_else(|e| e)
}

fn execute_apply_extrastereo_gemini(args: &HashMap<String, Value>) -> String {
    let input = args.get("input_file").and_then(|v| v.as_str()).unwrap_or("");
    let output_raw = args.get("output_file").and_then(|v| v.as_str()).unwrap_or("");
    let output = ensure_outputs_directory(output_raw);
    let multiplier = args.get("multiplier").and_then(|v| v.as_f64()).unwrap_or(2.5);
    let clipping = args.get("clipping").and_then(|v| v.as_bool()).unwrap_or(false);
    crate::audio::apply_extrastereo(input, &output, multiplier, clipping).unwrap_or_else(|e| e)
}

fn execute_apply_firequalizer_claude(args: &Value) -> String {
    let input = args["input_file"].as_str().unwrap_or("");
    let output_raw = args["output_file"].as_str().unwrap_or("");
    let output = ensure_outputs_directory(output_raw);
    let gain_entry = args["gain_entry"].as_str().unwrap_or("entry(0,0);entry(22050,0)");
    crate::audio::apply_firequalizer(input, &output, gain_entry).unwrap_or_else(|e| e)
}

fn execute_apply_firequalizer_gemini(args: &HashMap<String, Value>) -> String {
    let input = args.get("input_file").and_then(|v| v.as_str()).unwrap_or("");
    let output_raw = args.get("output_file").and_then(|v| v.as_str()).unwrap_or("");
    let output = ensure_outputs_directory(output_raw);
    let gain_entry = args.get("gain_entry").and_then(|v| v.as_str()).unwrap_or("entry(0,0);entry(22050,0)");
    crate::audio::apply_firequalizer(input, &output, gain_entry).unwrap_or_else(|e| e)
}

fn execute_apply_biquad_claude(args: &Value) -> String {
    let input = args["input_file"].as_str().unwrap_or("");
    let output_raw = args["output_file"].as_str().unwrap_or("");
    let output = ensure_outputs_directory(output_raw);
    let b0 = args.get("b0").and_then(|v| v.as_f64()).unwrap_or(1.0);
    let b1 = args.get("b1").and_then(|v| v.as_f64()).unwrap_or(0.0);
    let b2 = args.get("b2").and_then(|v| v.as_f64()).unwrap_or(0.0);
    let a0 = args.get("a0").and_then(|v| v.as_f64()).unwrap_or(1.0);
    let a1 = args.get("a1").and_then(|v| v.as_f64()).unwrap_or(0.0);
    let a2 = args.get("a2").and_then(|v| v.as_f64()).unwrap_or(0.0);
    crate::audio::apply_biquad(input, &output, b0, b1, b2, a0, a1, a2).unwrap_or_else(|e| e)
}

fn execute_apply_biquad_gemini(args: &HashMap<String, Value>) -> String {
    let input = args.get("input_file").and_then(|v| v.as_str()).unwrap_or("");
    let output_raw = args.get("output_file").and_then(|v| v.as_str()).unwrap_or("");
    let output = ensure_outputs_directory(output_raw);
    let b0 = args.get("b0").and_then(|v| v.as_f64()).unwrap_or(1.0);
    let b1 = args.get("b1").and_then(|v| v.as_f64()).unwrap_or(0.0);
    let b2 = args.get("b2").and_then(|v| v.as_f64()).unwrap_or(0.0);
    let a0 = args.get("a0").and_then(|v| v.as_f64()).unwrap_or(1.0);
    let a1 = args.get("a1").and_then(|v| v.as_f64()).unwrap_or(0.0);
    let a2 = args.get("a2").and_then(|v| v.as_f64()).unwrap_or(0.0);
    crate::audio::apply_biquad(input, &output, b0, b1, b2, a0, a1, a2).unwrap_or_else(|e| e)
}

fn execute_filter_bandpass_claude(args: &Value) -> String {
    let input = args["input_file"].as_str().unwrap_or("");
    let output_raw = args["output_file"].as_str().unwrap_or("");
    let output = ensure_outputs_directory(output_raw);
    let frequency = args.get("frequency").and_then(|v| v.as_f64()).unwrap_or(3000.0);
    let width = args.get("width").and_then(|v| v.as_f64()).unwrap_or(200.0);
    let width_type = args.get("width_type").and_then(|v| v.as_str()).unwrap_or("h");
    crate::audio::filter_bandpass(input, &output, frequency, width, width_type).unwrap_or_else(|e| e)
}

fn execute_filter_bandpass_gemini(args: &HashMap<String, Value>) -> String {
    let input = args.get("input_file").and_then(|v| v.as_str()).unwrap_or("");
    let output_raw = args.get("output_file").and_then(|v| v.as_str()).unwrap_or("");
    let output = ensure_outputs_directory(output_raw);
    let frequency = args.get("frequency").and_then(|v| v.as_f64()).unwrap_or(3000.0);
    let width = args.get("width").and_then(|v| v.as_f64()).unwrap_or(200.0);
    let width_type = args.get("width_type").and_then(|v| v.as_str()).unwrap_or("h");
    crate::audio::filter_bandpass(input, &output, frequency, width, width_type).unwrap_or_else(|e| e)
}

fn execute_filter_bandreject_claude(args: &Value) -> String {
    let input = args["input_file"].as_str().unwrap_or("");
    let output_raw = args["output_file"].as_str().unwrap_or("");
    let output = ensure_outputs_directory(output_raw);
    let frequency = args.get("frequency").and_then(|v| v.as_f64()).unwrap_or(3000.0);
    let width = args.get("width").and_then(|v| v.as_f64()).unwrap_or(200.0);
    let width_type = args.get("width_type").and_then(|v| v.as_str()).unwrap_or("h");
    crate::audio::filter_bandreject(input, &output, frequency, width, width_type).unwrap_or_else(|e| e)
}

fn execute_filter_bandreject_gemini(args: &HashMap<String, Value>) -> String {
    let input = args.get("input_file").and_then(|v| v.as_str()).unwrap_or("");
    let output_raw = args.get("output_file").and_then(|v| v.as_str()).unwrap_or("");
    let output = ensure_outputs_directory(output_raw);
    let frequency = args.get("frequency").and_then(|v| v.as_f64()).unwrap_or(3000.0);
    let width = args.get("width").and_then(|v| v.as_f64()).unwrap_or(200.0);
    let width_type = args.get("width_type").and_then(|v| v.as_str()).unwrap_or("h");
    crate::audio::filter_bandreject(input, &output, frequency, width, width_type).unwrap_or_else(|e| e)
}

fn execute_boost_sub_bass_claude(args: &Value) -> String {
    let input = args["input_file"].as_str().unwrap_or("");
    let output_raw = args["output_file"].as_str().unwrap_or("");
    let output = ensure_outputs_directory(output_raw);
    let dry = args.get("dry").and_then(|v| v.as_f64()).unwrap_or(1.0);
    let wet = args.get("wet").and_then(|v| v.as_f64()).unwrap_or(1.0);
    let freq = args.get("freq").and_then(|v| v.as_f64()).unwrap_or(20.0);
    let decay = args.get("decay").and_then(|v| v.as_f64()).unwrap_or(0.7);
    crate::audio::boost_sub_bass(input, &output, dry, wet, freq, decay).unwrap_or_else(|e| e)
}

fn execute_boost_sub_bass_gemini(args: &HashMap<String, Value>) -> String {
    let input = args.get("input_file").and_then(|v| v.as_str()).unwrap_or("");
    let output_raw = args.get("output_file").and_then(|v| v.as_str()).unwrap_or("");
    let output = ensure_outputs_directory(output_raw);
    let dry = args.get("dry").and_then(|v| v.as_f64()).unwrap_or(1.0);
    let wet = args.get("wet").and_then(|v| v.as_f64()).unwrap_or(1.0);
    let freq = args.get("freq").and_then(|v| v.as_f64()).unwrap_or(20.0);
    let decay = args.get("decay").and_then(|v| v.as_f64()).unwrap_or(0.7);
    crate::audio::boost_sub_bass(input, &output, dry, wet, freq, decay).unwrap_or_else(|e| e)
}

// ================================================================
// PHASE G — AI/ML Filters
// ================================================================

fn execute_detect_objects_dnn_claude(args: &Value) -> String {
    let input = args["input_file"].as_str().unwrap_or("");
    let output_raw = args["output_file"].as_str().unwrap_or("");
    let output = ensure_outputs_directory(output_raw);
    let model = args["model"].as_str().unwrap_or("");
    let backend = args.get("backend").and_then(|v| v.as_str()).unwrap_or("native");
    let confidence = args.get("confidence").and_then(|v| v.as_f64()).unwrap_or(0.5);
    let labels = args.get("labels").and_then(|v| v.as_str()).unwrap_or("");
    crate::visual::detect_objects_dnn(input, &output, model, backend, confidence, labels).unwrap_or_else(|e| e)
}

fn execute_detect_objects_dnn_gemini(args: &HashMap<String, Value>) -> String {
    let input = args.get("input_file").and_then(|v| v.as_str()).unwrap_or("");
    let output_raw = args.get("output_file").and_then(|v| v.as_str()).unwrap_or("");
    let output = ensure_outputs_directory(output_raw);
    let model = args.get("model").and_then(|v| v.as_str()).unwrap_or("");
    let backend = args.get("backend").and_then(|v| v.as_str()).unwrap_or("native");
    let confidence = args.get("confidence").and_then(|v| v.as_f64()).unwrap_or(0.5);
    let labels = args.get("labels").and_then(|v| v.as_str()).unwrap_or("");
    crate::visual::detect_objects_dnn(input, &output, model, backend, confidence, labels).unwrap_or_else(|e| e)
}

fn execute_classify_frames_dnn_claude(args: &Value) -> String {
    let input = args["input_file"].as_str().unwrap_or("");
    let output_raw = args["output_file"].as_str().unwrap_or("");
    let output = ensure_outputs_directory(output_raw);
    let model = args["model"].as_str().unwrap_or("");
    let backend = args.get("backend").and_then(|v| v.as_str()).unwrap_or("native");
    let labels = args.get("labels").and_then(|v| v.as_str()).unwrap_or("");
    crate::visual::classify_frames_dnn(input, &output, model, backend, labels).unwrap_or_else(|e| e)
}

fn execute_classify_frames_dnn_gemini(args: &HashMap<String, Value>) -> String {
    let input = args.get("input_file").and_then(|v| v.as_str()).unwrap_or("");
    let output_raw = args.get("output_file").and_then(|v| v.as_str()).unwrap_or("");
    let output = ensure_outputs_directory(output_raw);
    let model = args.get("model").and_then(|v| v.as_str()).unwrap_or("");
    let backend = args.get("backend").and_then(|v| v.as_str()).unwrap_or("native");
    let labels = args.get("labels").and_then(|v| v.as_str()).unwrap_or("");
    crate::visual::classify_frames_dnn(input, &output, model, backend, labels).unwrap_or_else(|e| e)
}

fn execute_upscale_super_resolution_claude(args: &Value) -> String {
    let input = args["input_file"].as_str().unwrap_or("");
    let output_raw = args["output_file"].as_str().unwrap_or("");
    let output = ensure_outputs_directory(output_raw);
    let scale_factor = args.get("scale_factor").and_then(|v| v.as_u64()).unwrap_or(2) as u32;
    let model = args.get("model").and_then(|v| v.as_str()).unwrap_or("");
    let backend = args.get("backend").and_then(|v| v.as_str()).unwrap_or("native");
    crate::visual::upscale_super_resolution(input, &output, scale_factor, model, backend).unwrap_or_else(|e| e)
}

fn execute_upscale_super_resolution_gemini(args: &HashMap<String, Value>) -> String {
    let input = args.get("input_file").and_then(|v| v.as_str()).unwrap_or("");
    let output_raw = args.get("output_file").and_then(|v| v.as_str()).unwrap_or("");
    let output = ensure_outputs_directory(output_raw);
    let scale_factor = args.get("scale_factor").and_then(|v| v.as_u64()).unwrap_or(2) as u32;
    let model = args.get("model").and_then(|v| v.as_str()).unwrap_or("");
    let backend = args.get("backend").and_then(|v| v.as_str()).unwrap_or("native");
    crate::visual::upscale_super_resolution(input, &output, scale_factor, model, backend).unwrap_or_else(|e| e)
}

fn execute_remove_rain_ai_claude(args: &Value) -> String {
    let input = args["input_file"].as_str().unwrap_or("");
    let output_raw = args["output_file"].as_str().unwrap_or("");
    let output = ensure_outputs_directory(output_raw);
    let model = args["model"].as_str().unwrap_or("");
    let backend = args.get("backend").and_then(|v| v.as_str()).unwrap_or("native");
    crate::visual::remove_rain_ai(input, &output, model, backend).unwrap_or_else(|e| e)
}

fn execute_remove_rain_ai_gemini(args: &HashMap<String, Value>) -> String {
    let input = args.get("input_file").and_then(|v| v.as_str()).unwrap_or("");
    let output_raw = args.get("output_file").and_then(|v| v.as_str()).unwrap_or("");
    let output = ensure_outputs_directory(output_raw);
    let model = args.get("model").and_then(|v| v.as_str()).unwrap_or("");
    let backend = args.get("backend").and_then(|v| v.as_str()).unwrap_or("native");
    crate::visual::remove_rain_ai(input, &output, model, backend).unwrap_or_else(|e| e)
}

fn execute_detect_frozen_frames_claude(args: &Value) -> String {
    let input = args["input_file"].as_str().unwrap_or("");
    let noise_db = args.get("noise_db").and_then(|v| v.as_f64()).unwrap_or(-60.0);
    let duration = args.get("duration").and_then(|v| v.as_f64()).unwrap_or(2.0);
    crate::visual::detect_frozen_frames(input, noise_db, duration).unwrap_or_else(|e| e)
}

fn execute_detect_frozen_frames_gemini(args: &HashMap<String, Value>) -> String {
    let input = args.get("input_file").and_then(|v| v.as_str()).unwrap_or("");
    let noise_db = args.get("noise_db").and_then(|v| v.as_f64()).unwrap_or(-60.0);
    let duration = args.get("duration").and_then(|v| v.as_f64()).unwrap_or(2.0);
    crate::visual::detect_frozen_frames(input, noise_db, duration).unwrap_or_else(|e| e)
}

fn execute_apply_edgedetect_claude(args: &Value) -> String {
    let input = args["input_file"].as_str().unwrap_or("");
    let output_raw = args["output_file"].as_str().unwrap_or("");
    let output = ensure_outputs_directory(output_raw);
    let low = args.get("low").and_then(|v| v.as_f64()).unwrap_or(0.0625);
    let high = args.get("high").and_then(|v| v.as_f64()).unwrap_or(0.1875);
    let mode = args.get("mode").and_then(|v| v.as_str()).unwrap_or("wires");
    crate::visual::apply_edgedetect(input, &output, low, high, mode).unwrap_or_else(|e| e)
}

fn execute_apply_edgedetect_gemini(args: &HashMap<String, Value>) -> String {
    let input = args.get("input_file").and_then(|v| v.as_str()).unwrap_or("");
    let output_raw = args.get("output_file").and_then(|v| v.as_str()).unwrap_or("");
    let output = ensure_outputs_directory(output_raw);
    let low = args.get("low").and_then(|v| v.as_f64()).unwrap_or(0.0625);
    let high = args.get("high").and_then(|v| v.as_f64()).unwrap_or(0.1875);
    let mode = args.get("mode").and_then(|v| v.as_str()).unwrap_or("wires");
    crate::visual::apply_edgedetect(input, &output, low, high, mode).unwrap_or_else(|e| e)
}

// ================================================================
// PHASE H — Codec / Format Depth
// ================================================================

fn execute_encode_vp9_claude(args: &Value) -> String {
    let input = args["input_file"].as_str().unwrap_or("");
    let output_raw = args["output_file"].as_str().unwrap_or("");
    let output = ensure_outputs_directory(output_raw);
    let crf = args.get("crf").and_then(|v| v.as_u64()).unwrap_or(31) as u32;
    let bitrate = args.get("bitrate").and_then(|v| v.as_str()).unwrap_or("");
    let speed = args.get("speed").and_then(|v| v.as_u64()).unwrap_or(2) as u32;
    let threads = args.get("threads").and_then(|v| v.as_u64()).unwrap_or(4) as u32;
    crate::core::encode_vp9(input, &output, crf, bitrate, speed, threads).unwrap_or_else(|e| e)
}

fn execute_encode_vp9_gemini(args: &HashMap<String, Value>) -> String {
    let input = args.get("input_file").and_then(|v| v.as_str()).unwrap_or("");
    let output_raw = args.get("output_file").and_then(|v| v.as_str()).unwrap_or("");
    let output = ensure_outputs_directory(output_raw);
    let crf = args.get("crf").and_then(|v| v.as_u64()).unwrap_or(31) as u32;
    let bitrate = args.get("bitrate").and_then(|v| v.as_str()).unwrap_or("");
    let speed = args.get("speed").and_then(|v| v.as_u64()).unwrap_or(2) as u32;
    let threads = args.get("threads").and_then(|v| v.as_u64()).unwrap_or(4) as u32;
    crate::core::encode_vp9(input, &output, crf, bitrate, speed, threads).unwrap_or_else(|e| e)
}

fn execute_encode_av1_claude(args: &Value) -> String {
    let input = args["input_file"].as_str().unwrap_or("");
    let output_raw = args["output_file"].as_str().unwrap_or("");
    let output = ensure_outputs_directory(output_raw);
    let crf = args.get("crf").and_then(|v| v.as_u64()).unwrap_or(30) as u32;
    let speed = args.get("speed").and_then(|v| v.as_u64()).unwrap_or(4) as u32;
    let threads = args.get("threads").and_then(|v| v.as_u64()).unwrap_or(4) as u32;
    let encoder = args.get("encoder").and_then(|v| v.as_str()).unwrap_or("");
    crate::core::encode_av1(input, &output, crf, speed, threads, encoder).unwrap_or_else(|e| e)
}

fn execute_encode_av1_gemini(args: &HashMap<String, Value>) -> String {
    let input = args.get("input_file").and_then(|v| v.as_str()).unwrap_or("");
    let output_raw = args.get("output_file").and_then(|v| v.as_str()).unwrap_or("");
    let output = ensure_outputs_directory(output_raw);
    let crf = args.get("crf").and_then(|v| v.as_u64()).unwrap_or(30) as u32;
    let speed = args.get("speed").and_then(|v| v.as_u64()).unwrap_or(4) as u32;
    let threads = args.get("threads").and_then(|v| v.as_u64()).unwrap_or(4) as u32;
    let encoder = args.get("encoder").and_then(|v| v.as_str()).unwrap_or("");
    crate::core::encode_av1(input, &output, crf, speed, threads, encoder).unwrap_or_else(|e| e)
}

fn execute_encode_hevc_claude(args: &Value) -> String {
    let input = args["input_file"].as_str().unwrap_or("");
    let output_raw = args["output_file"].as_str().unwrap_or("");
    let output = ensure_outputs_directory(output_raw);
    let crf = args.get("crf").and_then(|v| v.as_u64()).unwrap_or(28) as u32;
    let preset = args.get("preset").and_then(|v| v.as_str()).unwrap_or("");
    let tune = args.get("tune").and_then(|v| v.as_str()).unwrap_or("");
    crate::core::encode_hevc(input, &output, crf, preset, tune).unwrap_or_else(|e| e)
}

fn execute_encode_hevc_gemini(args: &HashMap<String, Value>) -> String {
    let input = args.get("input_file").and_then(|v| v.as_str()).unwrap_or("");
    let output_raw = args.get("output_file").and_then(|v| v.as_str()).unwrap_or("");
    let output = ensure_outputs_directory(output_raw);
    let crf = args.get("crf").and_then(|v| v.as_u64()).unwrap_or(28) as u32;
    let preset = args.get("preset").and_then(|v| v.as_str()).unwrap_or("");
    let tune = args.get("tune").and_then(|v| v.as_str()).unwrap_or("");
    crate::core::encode_hevc(input, &output, crf, preset, tune).unwrap_or_else(|e| e)
}

fn execute_encode_opus_claude(args: &Value) -> String {
    let input = args["input_file"].as_str().unwrap_or("");
    let output_raw = args["output_file"].as_str().unwrap_or("");
    let output = ensure_outputs_directory(output_raw);
    let bitrate_kbps = args.get("bitrate_kbps").and_then(|v| v.as_u64()).unwrap_or(128) as u32;
    let vbr_str = args.get("vbr").and_then(|v| v.as_str()).unwrap_or("true");
    let vbr = vbr_str != "false";
    let compression = args.get("compression").and_then(|v| v.as_u64()).unwrap_or(10) as u32;
    crate::core::encode_opus(input, &output, bitrate_kbps, vbr, compression).unwrap_or_else(|e| e)
}

fn execute_encode_opus_gemini(args: &HashMap<String, Value>) -> String {
    let input = args.get("input_file").and_then(|v| v.as_str()).unwrap_or("");
    let output_raw = args.get("output_file").and_then(|v| v.as_str()).unwrap_or("");
    let output = ensure_outputs_directory(output_raw);
    let bitrate_kbps = args.get("bitrate_kbps").and_then(|v| v.as_u64()).unwrap_or(128) as u32;
    let vbr_str = args.get("vbr").and_then(|v| v.as_str()).unwrap_or("true");
    let vbr = vbr_str != "false";
    let compression = args.get("compression").and_then(|v| v.as_u64()).unwrap_or(10) as u32;
    crate::core::encode_opus(input, &output, bitrate_kbps, vbr, compression).unwrap_or_else(|e| e)
}

fn execute_encode_hdr10_claude(args: &Value) -> String {
    let input = args["input_file"].as_str().unwrap_or("");
    let output_raw = args["output_file"].as_str().unwrap_or("");
    let output = ensure_outputs_directory(output_raw);
    let crf = args.get("crf").and_then(|v| v.as_u64()).unwrap_or(22) as u32;
    let preset = args.get("preset").and_then(|v| v.as_str()).unwrap_or("");
    let master_display = args.get("master_display").and_then(|v| v.as_str()).unwrap_or("");
    let max_cll = args.get("max_cll").and_then(|v| v.as_str()).unwrap_or("");
    crate::core::encode_hdr10(input, &output, crf, preset, master_display, max_cll).unwrap_or_else(|e| e)
}

fn execute_encode_hdr10_gemini(args: &HashMap<String, Value>) -> String {
    let input = args.get("input_file").and_then(|v| v.as_str()).unwrap_or("");
    let output_raw = args.get("output_file").and_then(|v| v.as_str()).unwrap_or("");
    let output = ensure_outputs_directory(output_raw);
    let crf = args.get("crf").and_then(|v| v.as_u64()).unwrap_or(22) as u32;
    let preset = args.get("preset").and_then(|v| v.as_str()).unwrap_or("");
    let master_display = args.get("master_display").and_then(|v| v.as_str()).unwrap_or("");
    let max_cll = args.get("max_cll").and_then(|v| v.as_str()).unwrap_or("");
    crate::core::encode_hdr10(input, &output, crf, preset, master_display, max_cll).unwrap_or_else(|e| e)
}

fn execute_encode_nvenc_claude(args: &Value) -> String {
    let input = args["input_file"].as_str().unwrap_or("");
    let output_raw = args["output_file"].as_str().unwrap_or("");
    let output = ensure_outputs_directory(output_raw);
    let codec = args.get("codec").and_then(|v| v.as_str()).unwrap_or("h264");
    let preset = args.get("preset").and_then(|v| v.as_str()).unwrap_or("");
    let bitrate = args.get("bitrate").and_then(|v| v.as_str()).unwrap_or("");
    let cq = args.get("cq").and_then(|v| v.as_u64()).unwrap_or(23) as u32;
    crate::core::encode_nvenc(input, &output, codec, preset, bitrate, cq).unwrap_or_else(|e| e)
}

fn execute_encode_nvenc_gemini(args: &HashMap<String, Value>) -> String {
    let input = args.get("input_file").and_then(|v| v.as_str()).unwrap_or("");
    let output_raw = args.get("output_file").and_then(|v| v.as_str()).unwrap_or("");
    let output = ensure_outputs_directory(output_raw);
    let codec = args.get("codec").and_then(|v| v.as_str()).unwrap_or("h264");
    let preset = args.get("preset").and_then(|v| v.as_str()).unwrap_or("");
    let bitrate = args.get("bitrate").and_then(|v| v.as_str()).unwrap_or("");
    let cq = args.get("cq").and_then(|v| v.as_u64()).unwrap_or(23) as u32;
    crate::core::encode_nvenc(input, &output, codec, preset, bitrate, cq).unwrap_or_else(|e| e)
}

fn execute_encode_vaapi_claude(args: &Value) -> String {
    let input = args["input_file"].as_str().unwrap_or("");
    let output_raw = args["output_file"].as_str().unwrap_or("");
    let output = ensure_outputs_directory(output_raw);
    let codec = args.get("codec").and_then(|v| v.as_str()).unwrap_or("h264");
    let quality = args.get("quality").and_then(|v| v.as_u64()).unwrap_or(23) as u32;
    let profile = args.get("profile").and_then(|v| v.as_str()).unwrap_or("");
    crate::core::encode_vaapi(input, &output, codec, quality, profile).unwrap_or_else(|e| e)
}

fn execute_encode_vaapi_gemini(args: &HashMap<String, Value>) -> String {
    let input = args.get("input_file").and_then(|v| v.as_str()).unwrap_or("");
    let output_raw = args.get("output_file").and_then(|v| v.as_str()).unwrap_or("");
    let output = ensure_outputs_directory(output_raw);
    let codec = args.get("codec").and_then(|v| v.as_str()).unwrap_or("h264");
    let quality = args.get("quality").and_then(|v| v.as_u64()).unwrap_or(23) as u32;
    let profile = args.get("profile").and_then(|v| v.as_str()).unwrap_or("");
    crate::core::encode_vaapi(input, &output, codec, quality, profile).unwrap_or_else(|e| e)
}

fn execute_encode_qsv_claude(args: &Value) -> String {
    let input = args["input_file"].as_str().unwrap_or("");
    let output_raw = args["output_file"].as_str().unwrap_or("");
    let output = ensure_outputs_directory(output_raw);
    let codec = args.get("codec").and_then(|v| v.as_str()).unwrap_or("h264");
    let preset = args.get("preset").and_then(|v| v.as_str()).unwrap_or("");
    let bitrate = args.get("bitrate").and_then(|v| v.as_str()).unwrap_or("");
    crate::core::encode_qsv(input, &output, codec, preset, bitrate).unwrap_or_else(|e| e)
}

fn execute_encode_qsv_gemini(args: &HashMap<String, Value>) -> String {
    let input = args.get("input_file").and_then(|v| v.as_str()).unwrap_or("");
    let output_raw = args.get("output_file").and_then(|v| v.as_str()).unwrap_or("");
    let output = ensure_outputs_directory(output_raw);
    let codec = args.get("codec").and_then(|v| v.as_str()).unwrap_or("h264");
    let preset = args.get("preset").and_then(|v| v.as_str()).unwrap_or("");
    let bitrate = args.get("bitrate").and_then(|v| v.as_str()).unwrap_or("");
    crate::core::encode_qsv(input, &output, codec, preset, bitrate).unwrap_or_else(|e| e)
}

fn execute_encode_prores_claude(args: &Value) -> String {
    let input = args["input_file"].as_str().unwrap_or("");
    let output_raw = args["output_file"].as_str().unwrap_or("");
    let output = ensure_outputs_directory(output_raw);
    let profile = args.get("profile").and_then(|v| v.as_u64()).unwrap_or(3) as u32;
    let vendor = args.get("vendor").and_then(|v| v.as_str()).unwrap_or("");
    crate::core::encode_prores(input, &output, profile, vendor).unwrap_or_else(|e| e)
}

fn execute_encode_prores_gemini(args: &HashMap<String, Value>) -> String {
    let input = args.get("input_file").and_then(|v| v.as_str()).unwrap_or("");
    let output_raw = args.get("output_file").and_then(|v| v.as_str()).unwrap_or("");
    let output = ensure_outputs_directory(output_raw);
    let profile = args.get("profile").and_then(|v| v.as_u64()).unwrap_or(3) as u32;
    let vendor = args.get("vendor").and_then(|v| v.as_str()).unwrap_or("");
    crate::core::encode_prores(input, &output, profile, vendor).unwrap_or_else(|e| e)
}

fn execute_encode_dnxhd_claude(args: &Value) -> String {
    let input = args["input_file"].as_str().unwrap_or("");
    let output_raw = args["output_file"].as_str().unwrap_or("");
    let output = ensure_outputs_directory(output_raw);
    let profile = args.get("profile").and_then(|v| v.as_str()).unwrap_or("");
    crate::core::encode_dnxhd(input, &output, profile).unwrap_or_else(|e| e)
}

fn execute_encode_dnxhd_gemini(args: &HashMap<String, Value>) -> String {
    let input = args.get("input_file").and_then(|v| v.as_str()).unwrap_or("");
    let output_raw = args.get("output_file").and_then(|v| v.as_str()).unwrap_or("");
    let output = ensure_outputs_directory(output_raw);
    let profile = args.get("profile").and_then(|v| v.as_str()).unwrap_or("");
    crate::core::encode_dnxhd(input, &output, profile).unwrap_or_else(|e| e)
}

fn execute_encode_gif_claude(args: &Value) -> String {
    let input = args["input_file"].as_str().unwrap_or("");
    let output_raw = args["output_file"].as_str().unwrap_or("");
    let output = ensure_outputs_directory(output_raw);
    let fps = args.get("fps").and_then(|v| v.as_f64()).unwrap_or(15.0);
    let scale = args.get("scale").and_then(|v| v.as_u64()).unwrap_or(480) as u32;
    let loop_count = args.get("loop_count").and_then(|v| v.as_i64()).unwrap_or(0) as i32;
    crate::core::encode_gif(input, &output, fps, scale, loop_count).unwrap_or_else(|e| e)
}

fn execute_encode_gif_gemini(args: &HashMap<String, Value>) -> String {
    let input = args.get("input_file").and_then(|v| v.as_str()).unwrap_or("");
    let output_raw = args.get("output_file").and_then(|v| v.as_str()).unwrap_or("");
    let output = ensure_outputs_directory(output_raw);
    let fps = args.get("fps").and_then(|v| v.as_f64()).unwrap_or(15.0);
    let scale = args.get("scale").and_then(|v| v.as_u64()).unwrap_or(480) as u32;
    let loop_count = args.get("loop_count").and_then(|v| v.as_i64()).unwrap_or(0) as i32;
    crate::core::encode_gif(input, &output, fps, scale, loop_count).unwrap_or_else(|e| e)
}

fn execute_encode_webm_claude(args: &Value) -> String {
    let input = args["input_file"].as_str().unwrap_or("");
    let output_raw = args["output_file"].as_str().unwrap_or("");
    let output = ensure_outputs_directory(output_raw);
    let video_codec = args.get("video_codec").and_then(|v| v.as_str()).unwrap_or("vp8");
    let audio_codec = args.get("audio_codec").and_then(|v| v.as_str()).unwrap_or("vorbis");
    let crf = args.get("crf").and_then(|v| v.as_u64()).unwrap_or(10) as u32;
    let bitrate = args.get("bitrate").and_then(|v| v.as_str()).unwrap_or("");
    crate::core::encode_webm(input, &output, video_codec, audio_codec, crf, bitrate).unwrap_or_else(|e| e)
}

fn execute_encode_webm_gemini(args: &HashMap<String, Value>) -> String {
    let input = args.get("input_file").and_then(|v| v.as_str()).unwrap_or("");
    let output_raw = args.get("output_file").and_then(|v| v.as_str()).unwrap_or("");
    let output = ensure_outputs_directory(output_raw);
    let video_codec = args.get("video_codec").and_then(|v| v.as_str()).unwrap_or("vp8");
    let audio_codec = args.get("audio_codec").and_then(|v| v.as_str()).unwrap_or("vorbis");
    let crf = args.get("crf").and_then(|v| v.as_u64()).unwrap_or(10) as u32;
    let bitrate = args.get("bitrate").and_then(|v| v.as_str()).unwrap_or("");
    crate::core::encode_webm(input, &output, video_codec, audio_codec, crf, bitrate).unwrap_or_else(|e| e)
}

// ================================================================
// PHASE I — Long-tail sweep, Batch 1
// ================================================================

fn execute_zoom_pan_claude(args: &Value) -> String {
    let input = args["input_file"].as_str().unwrap_or("");
    let output_raw = args["output_file"].as_str().unwrap_or("");
    let output = ensure_outputs_directory(output_raw);
    let zoom = args.get("zoom").and_then(|v| v.as_f64()).unwrap_or(1.5);
    let x_expr = args.get("x_expr").and_then(|v| v.as_str()).unwrap_or("");
    let y_expr = args.get("y_expr").and_then(|v| v.as_str()).unwrap_or("");
    let duration_frames = args.get("duration_frames").and_then(|v| v.as_u64()).unwrap_or(125) as u32;
    let fps = args.get("fps").and_then(|v| v.as_f64()).unwrap_or(25.0);
    crate::visual::zoom_pan(input, &output, zoom, x_expr, y_expr, duration_frames, fps).unwrap_or_else(|e| e)
}

fn execute_zoom_pan_gemini(args: &HashMap<String, Value>) -> String {
    let input = args.get("input_file").and_then(|v| v.as_str()).unwrap_or("");
    let output_raw = args.get("output_file").and_then(|v| v.as_str()).unwrap_or("");
    let output = ensure_outputs_directory(output_raw);
    let zoom = args.get("zoom").and_then(|v| v.as_f64()).unwrap_or(1.5);
    let x_expr = args.get("x_expr").and_then(|v| v.as_str()).unwrap_or("");
    let y_expr = args.get("y_expr").and_then(|v| v.as_str()).unwrap_or("");
    let duration_frames = args.get("duration_frames").and_then(|v| v.as_u64()).unwrap_or(125) as u32;
    let fps = args.get("fps").and_then(|v| v.as_f64()).unwrap_or(25.0);
    crate::visual::zoom_pan(input, &output, zoom, x_expr, y_expr, duration_frames, fps).unwrap_or_else(|e| e)
}

fn execute_chromatic_aberration_claude(args: &Value) -> String {
    let input = args["input_file"].as_str().unwrap_or("");
    let output_raw = args["output_file"].as_str().unwrap_or("");
    let output = ensure_outputs_directory(output_raw);
    let rh = args.get("rh").and_then(|v| v.as_i64()).unwrap_or(5) as i32;
    let rv = args.get("rv").and_then(|v| v.as_i64()).unwrap_or(0) as i32;
    let bh = args.get("bh").and_then(|v| v.as_i64()).unwrap_or(-5) as i32;
    let bv = args.get("bv").and_then(|v| v.as_i64()).unwrap_or(0) as i32;
    crate::visual::chromatic_aberration(input, &output, rh, rv, bh, bv).unwrap_or_else(|e| e)
}

fn execute_chromatic_aberration_gemini(args: &HashMap<String, Value>) -> String {
    let input = args.get("input_file").and_then(|v| v.as_str()).unwrap_or("");
    let output_raw = args.get("output_file").and_then(|v| v.as_str()).unwrap_or("");
    let output = ensure_outputs_directory(output_raw);
    let rh = args.get("rh").and_then(|v| v.as_i64()).unwrap_or(5) as i32;
    let rv = args.get("rv").and_then(|v| v.as_i64()).unwrap_or(0) as i32;
    let bh = args.get("bh").and_then(|v| v.as_i64()).unwrap_or(-5) as i32;
    let bv = args.get("bv").and_then(|v| v.as_i64()).unwrap_or(0) as i32;
    crate::visual::chromatic_aberration(input, &output, rh, rv, bh, bv).unwrap_or_else(|e| e)
}

fn execute_temporal_blend_claude(args: &Value) -> String {
    let input = args["input_file"].as_str().unwrap_or("");
    let output_raw = args["output_file"].as_str().unwrap_or("");
    let output = ensure_outputs_directory(output_raw);
    let mode = args.get("mode").and_then(|v| v.as_str()).unwrap_or("average");
    let opacity = args.get("opacity").and_then(|v| v.as_f64()).unwrap_or(1.0);
    crate::visual::temporal_blend(input, &output, mode, opacity).unwrap_or_else(|e| e)
}

fn execute_temporal_blend_gemini(args: &HashMap<String, Value>) -> String {
    let input = args.get("input_file").and_then(|v| v.as_str()).unwrap_or("");
    let output_raw = args.get("output_file").and_then(|v| v.as_str()).unwrap_or("");
    let output = ensure_outputs_directory(output_raw);
    let mode = args.get("mode").and_then(|v| v.as_str()).unwrap_or("average");
    let opacity = args.get("opacity").and_then(|v| v.as_f64()).unwrap_or(1.0);
    crate::visual::temporal_blend(input, &output, mode, opacity).unwrap_or_else(|e| e)
}

fn execute_motion_interpolate_claude(args: &Value) -> String {
    let input = args["input_file"].as_str().unwrap_or("");
    let output_raw = args["output_file"].as_str().unwrap_or("");
    let output = ensure_outputs_directory(output_raw);
    let target_fps = args.get("target_fps").and_then(|v| v.as_f64()).unwrap_or(60.0);
    let mi_mode = args.get("mi_mode").and_then(|v| v.as_str()).unwrap_or("mci");
    crate::visual::motion_interpolate(input, &output, target_fps, mi_mode).unwrap_or_else(|e| e)
}

fn execute_motion_interpolate_gemini(args: &HashMap<String, Value>) -> String {
    let input = args.get("input_file").and_then(|v| v.as_str()).unwrap_or("");
    let output_raw = args.get("output_file").and_then(|v| v.as_str()).unwrap_or("");
    let output = ensure_outputs_directory(output_raw);
    let target_fps = args.get("target_fps").and_then(|v| v.as_f64()).unwrap_or(60.0);
    let mi_mode = args.get("mi_mode").and_then(|v| v.as_str()).unwrap_or("mci");
    crate::visual::motion_interpolate(input, &output, target_fps, mi_mode).unwrap_or_else(|e| e)
}

fn execute_correct_lens_simple_claude(args: &Value) -> String {
    let input = args["input_file"].as_str().unwrap_or("");
    let output_raw = args["output_file"].as_str().unwrap_or("");
    let output = ensure_outputs_directory(output_raw);
    let k1 = args.get("k1").and_then(|v| v.as_f64()).unwrap_or(-0.1);
    let k2 = args.get("k2").and_then(|v| v.as_f64()).unwrap_or(0.0);
    crate::visual::correct_lens_simple(input, &output, k1, k2).unwrap_or_else(|e| e)
}

fn execute_correct_lens_simple_gemini(args: &HashMap<String, Value>) -> String {
    let input = args.get("input_file").and_then(|v| v.as_str()).unwrap_or("");
    let output_raw = args.get("output_file").and_then(|v| v.as_str()).unwrap_or("");
    let output = ensure_outputs_directory(output_raw);
    let k1 = args.get("k1").and_then(|v| v.as_f64()).unwrap_or(-0.1);
    let k2 = args.get("k2").and_then(|v| v.as_f64()).unwrap_or(0.0);
    crate::visual::correct_lens_simple(input, &output, k1, k2).unwrap_or_else(|e| e)
}

fn execute_deinterlace_yadif_claude(args: &Value) -> String {
    let input = args["input_file"].as_str().unwrap_or("");
    let output_raw = args["output_file"].as_str().unwrap_or("");
    let output = ensure_outputs_directory(output_raw);
    let mode = args.get("mode").and_then(|v| v.as_u64()).unwrap_or(0) as u32;
    let parity = args.get("parity").and_then(|v| v.as_i64()).unwrap_or(-1) as i32;
    crate::visual::deinterlace_yadif(input, &output, mode, parity).unwrap_or_else(|e| e)
}

fn execute_deinterlace_yadif_gemini(args: &HashMap<String, Value>) -> String {
    let input = args.get("input_file").and_then(|v| v.as_str()).unwrap_or("");
    let output_raw = args.get("output_file").and_then(|v| v.as_str()).unwrap_or("");
    let output = ensure_outputs_directory(output_raw);
    let mode = args.get("mode").and_then(|v| v.as_u64()).unwrap_or(0) as u32;
    let parity = args.get("parity").and_then(|v| v.as_i64()).unwrap_or(-1) as i32;
    crate::visual::deinterlace_yadif(input, &output, mode, parity).unwrap_or_else(|e| e)
}

fn execute_correct_perspective_linear_claude(args: &Value) -> String {
    let input = args["input_file"].as_str().unwrap_or("");
    let output_raw = args["output_file"].as_str().unwrap_or("");
    let output = ensure_outputs_directory(output_raw);
    let x0 = args.get("x0").and_then(|v| v.as_f64()).unwrap_or(0.0);
    let y0 = args.get("y0").and_then(|v| v.as_f64()).unwrap_or(0.0);
    let x1 = args.get("x1").and_then(|v| v.as_f64()).unwrap_or(1920.0);
    let y1 = args.get("y1").and_then(|v| v.as_f64()).unwrap_or(0.0);
    let x2 = args.get("x2").and_then(|v| v.as_f64()).unwrap_or(0.0);
    let y2 = args.get("y2").and_then(|v| v.as_f64()).unwrap_or(1080.0);
    let x3 = args.get("x3").and_then(|v| v.as_f64()).unwrap_or(1920.0);
    let y3 = args.get("y3").and_then(|v| v.as_f64()).unwrap_or(1080.0);
    crate::visual::correct_perspective_linear(input, &output, x0, y0, x1, y1, x2, y2, x3, y3).unwrap_or_else(|e| e)
}

fn execute_correct_perspective_linear_gemini(args: &HashMap<String, Value>) -> String {
    let input = args.get("input_file").and_then(|v| v.as_str()).unwrap_or("");
    let output_raw = args.get("output_file").and_then(|v| v.as_str()).unwrap_or("");
    let output = ensure_outputs_directory(output_raw);
    let x0 = args.get("x0").and_then(|v| v.as_f64()).unwrap_or(0.0);
    let y0 = args.get("y0").and_then(|v| v.as_f64()).unwrap_or(0.0);
    let x1 = args.get("x1").and_then(|v| v.as_f64()).unwrap_or(1920.0);
    let y1 = args.get("y1").and_then(|v| v.as_f64()).unwrap_or(0.0);
    let x2 = args.get("x2").and_then(|v| v.as_f64()).unwrap_or(0.0);
    let y2 = args.get("y2").and_then(|v| v.as_f64()).unwrap_or(1080.0);
    let x3 = args.get("x3").and_then(|v| v.as_f64()).unwrap_or(1920.0);
    let y3 = args.get("y3").and_then(|v| v.as_f64()).unwrap_or(1080.0);
    crate::visual::correct_perspective_linear(input, &output, x0, y0, x1, y1, x2, y2, x3, y3).unwrap_or_else(|e| e)
}

fn execute_colorize_video_claude(args: &Value) -> String {
    let input = args["input_file"].as_str().unwrap_or("");
    let output_raw = args["output_file"].as_str().unwrap_or("");
    let output = ensure_outputs_directory(output_raw);
    let hue = args.get("hue").and_then(|v| v.as_f64()).unwrap_or(210.0);
    let saturation = args.get("saturation").and_then(|v| v.as_f64()).unwrap_or(0.5);
    let lightness = args.get("lightness").and_then(|v| v.as_f64()).unwrap_or(0.0);
    crate::visual::colorize_video(input, &output, hue, saturation, lightness).unwrap_or_else(|e| e)
}

fn execute_colorize_video_gemini(args: &HashMap<String, Value>) -> String {
    let input = args.get("input_file").and_then(|v| v.as_str()).unwrap_or("");
    let output_raw = args.get("output_file").and_then(|v| v.as_str()).unwrap_or("");
    let output = ensure_outputs_directory(output_raw);
    let hue = args.get("hue").and_then(|v| v.as_f64()).unwrap_or(210.0);
    let saturation = args.get("saturation").and_then(|v| v.as_f64()).unwrap_or(0.5);
    let lightness = args.get("lightness").and_then(|v| v.as_f64()).unwrap_or(0.0);
    crate::visual::colorize_video(input, &output, hue, saturation, lightness).unwrap_or_else(|e| e)
}

fn execute_denoise_hqdn3d_claude(args: &Value) -> String {
    let input = args["input_file"].as_str().unwrap_or("");
    let output_raw = args["output_file"].as_str().unwrap_or("");
    let output = ensure_outputs_directory(output_raw);
    let ls = args.get("luma_spatial").and_then(|v| v.as_f64()).unwrap_or(0.0);
    let cs = args.get("chroma_spatial").and_then(|v| v.as_f64()).unwrap_or(0.0);
    let lt = args.get("luma_tmp").and_then(|v| v.as_f64()).unwrap_or(0.0);
    let ct = args.get("chroma_tmp").and_then(|v| v.as_f64()).unwrap_or(0.0);
    crate::visual::denoise_hqdn3d(input, &output, ls, cs, lt, ct).unwrap_or_else(|e| e)
}

fn execute_denoise_hqdn3d_gemini(args: &HashMap<String, Value>) -> String {
    let input = args.get("input_file").and_then(|v| v.as_str()).unwrap_or("");
    let output_raw = args.get("output_file").and_then(|v| v.as_str()).unwrap_or("");
    let output = ensure_outputs_directory(output_raw);
    let ls = args.get("luma_spatial").and_then(|v| v.as_f64()).unwrap_or(0.0);
    let cs = args.get("chroma_spatial").and_then(|v| v.as_f64()).unwrap_or(0.0);
    let lt = args.get("luma_tmp").and_then(|v| v.as_f64()).unwrap_or(0.0);
    let ct = args.get("chroma_tmp").and_then(|v| v.as_f64()).unwrap_or(0.0);
    crate::visual::denoise_hqdn3d(input, &output, ls, cs, lt, ct).unwrap_or_else(|e| e)
}

fn execute_add_echo_claude(args: &Value) -> String {
    let input = args["input_file"].as_str().unwrap_or("");
    let output_raw = args["output_file"].as_str().unwrap_or("");
    let output = ensure_outputs_directory(output_raw);
    let in_gain = args.get("in_gain").and_then(|v| v.as_f64()).unwrap_or(0.0);
    let out_gain = args.get("out_gain").and_then(|v| v.as_f64()).unwrap_or(0.0);
    let delays = args.get("delays").and_then(|v| v.as_str()).unwrap_or("");
    let decays = args.get("decays").and_then(|v| v.as_str()).unwrap_or("");
    crate::audio::add_echo(input, &output, in_gain, out_gain, delays, decays).unwrap_or_else(|e| e)
}

fn execute_add_echo_gemini(args: &HashMap<String, Value>) -> String {
    let input = args.get("input_file").and_then(|v| v.as_str()).unwrap_or("");
    let output_raw = args.get("output_file").and_then(|v| v.as_str()).unwrap_or("");
    let output = ensure_outputs_directory(output_raw);
    let in_gain = args.get("in_gain").and_then(|v| v.as_f64()).unwrap_or(0.0);
    let out_gain = args.get("out_gain").and_then(|v| v.as_f64()).unwrap_or(0.0);
    let delays = args.get("delays").and_then(|v| v.as_str()).unwrap_or("");
    let decays = args.get("decays").and_then(|v| v.as_str()).unwrap_or("");
    crate::audio::add_echo(input, &output, in_gain, out_gain, delays, decays).unwrap_or_else(|e| e)
}

fn execute_noise_gate_claude(args: &Value) -> String {
    let input = args["input_file"].as_str().unwrap_or("");
    let output_raw = args["output_file"].as_str().unwrap_or("");
    let output = ensure_outputs_directory(output_raw);
    let threshold = args.get("threshold").and_then(|v| v.as_f64()).unwrap_or(0.0);
    let range = args.get("range").and_then(|v| v.as_f64()).unwrap_or(0.0);
    let attack = args.get("attack").and_then(|v| v.as_f64()).unwrap_or(0.0);
    let release = args.get("release").and_then(|v| v.as_f64()).unwrap_or(0.0);
    crate::audio::noise_gate(input, &output, threshold, range, attack, release).unwrap_or_else(|e| e)
}

fn execute_noise_gate_gemini(args: &HashMap<String, Value>) -> String {
    let input = args.get("input_file").and_then(|v| v.as_str()).unwrap_or("");
    let output_raw = args.get("output_file").and_then(|v| v.as_str()).unwrap_or("");
    let output = ensure_outputs_directory(output_raw);
    let threshold = args.get("threshold").and_then(|v| v.as_f64()).unwrap_or(0.0);
    let range = args.get("range").and_then(|v| v.as_f64()).unwrap_or(0.0);
    let attack = args.get("attack").and_then(|v| v.as_f64()).unwrap_or(0.0);
    let release = args.get("release").and_then(|v| v.as_f64()).unwrap_or(0.0);
    crate::audio::noise_gate(input, &output, threshold, range, attack, release).unwrap_or_else(|e| e)
}

fn execute_compress_dynamics_claude(args: &Value) -> String {
    let input = args["input_file"].as_str().unwrap_or("");
    let output_raw = args["output_file"].as_str().unwrap_or("");
    let output = ensure_outputs_directory(output_raw);
    let threshold = args.get("threshold").and_then(|v| v.as_f64()).unwrap_or(0.0);
    let ratio = args.get("ratio").and_then(|v| v.as_f64()).unwrap_or(0.0);
    let attack = args.get("attack").and_then(|v| v.as_f64()).unwrap_or(0.0);
    let release = args.get("release").and_then(|v| v.as_f64()).unwrap_or(0.0);
    let makeup = args.get("makeup").and_then(|v| v.as_f64()).unwrap_or(0.0);
    crate::audio::compress_dynamics(input, &output, threshold, ratio, attack, release, makeup).unwrap_or_else(|e| e)
}

fn execute_compress_dynamics_gemini(args: &HashMap<String, Value>) -> String {
    let input = args.get("input_file").and_then(|v| v.as_str()).unwrap_or("");
    let output_raw = args.get("output_file").and_then(|v| v.as_str()).unwrap_or("");
    let output = ensure_outputs_directory(output_raw);
    let threshold = args.get("threshold").and_then(|v| v.as_f64()).unwrap_or(0.0);
    let ratio = args.get("ratio").and_then(|v| v.as_f64()).unwrap_or(0.0);
    let attack = args.get("attack").and_then(|v| v.as_f64()).unwrap_or(0.0);
    let release = args.get("release").and_then(|v| v.as_f64()).unwrap_or(0.0);
    let makeup = args.get("makeup").and_then(|v| v.as_f64()).unwrap_or(0.0);
    crate::audio::compress_dynamics(input, &output, threshold, ratio, attack, release, makeup).unwrap_or_else(|e| e)
}

fn execute_add_chorus_claude(args: &Value) -> String {
    let input = args["input_file"].as_str().unwrap_or("");
    let output_raw = args["output_file"].as_str().unwrap_or("");
    let output = ensure_outputs_directory(output_raw);
    let in_gain = args.get("in_gain").and_then(|v| v.as_f64()).unwrap_or(0.0);
    let out_gain = args.get("out_gain").and_then(|v| v.as_f64()).unwrap_or(0.0);
    let delays = args.get("delays").and_then(|v| v.as_str()).unwrap_or("");
    let decays = args.get("decays").and_then(|v| v.as_str()).unwrap_or("");
    let speeds = args.get("speeds").and_then(|v| v.as_str()).unwrap_or("");
    let depths = args.get("depths").and_then(|v| v.as_str()).unwrap_or("");
    crate::audio::add_chorus(input, &output, in_gain, out_gain, delays, decays, speeds, depths).unwrap_or_else(|e| e)
}

fn execute_add_chorus_gemini(args: &HashMap<String, Value>) -> String {
    let input = args.get("input_file").and_then(|v| v.as_str()).unwrap_or("");
    let output_raw = args.get("output_file").and_then(|v| v.as_str()).unwrap_or("");
    let output = ensure_outputs_directory(output_raw);
    let in_gain = args.get("in_gain").and_then(|v| v.as_f64()).unwrap_or(0.0);
    let out_gain = args.get("out_gain").and_then(|v| v.as_f64()).unwrap_or(0.0);
    let delays = args.get("delays").and_then(|v| v.as_str()).unwrap_or("");
    let decays = args.get("decays").and_then(|v| v.as_str()).unwrap_or("");
    let speeds = args.get("speeds").and_then(|v| v.as_str()).unwrap_or("");
    let depths = args.get("depths").and_then(|v| v.as_str()).unwrap_or("");
    crate::audio::add_chorus(input, &output, in_gain, out_gain, delays, decays, speeds, depths).unwrap_or_else(|e| e)
}

fn execute_widen_stereo_claude(args: &Value) -> String {
    let input = args["input_file"].as_str().unwrap_or("");
    let output_raw = args["output_file"].as_str().unwrap_or("");
    let output = ensure_outputs_directory(output_raw);
    let delay = args.get("delay").and_then(|v| v.as_f64()).unwrap_or(0.0);
    let feedback = args.get("feedback").and_then(|v| v.as_f64()).unwrap_or(0.0);
    let crossfeed = args.get("crossfeed").and_then(|v| v.as_f64()).unwrap_or(0.0);
    let drymix = args.get("drymix").and_then(|v| v.as_f64()).unwrap_or(0.0);
    crate::audio::widen_stereo(input, &output, delay, feedback, crossfeed, drymix).unwrap_or_else(|e| e)
}

fn execute_widen_stereo_gemini(args: &HashMap<String, Value>) -> String {
    let input = args.get("input_file").and_then(|v| v.as_str()).unwrap_or("");
    let output_raw = args.get("output_file").and_then(|v| v.as_str()).unwrap_or("");
    let output = ensure_outputs_directory(output_raw);
    let delay = args.get("delay").and_then(|v| v.as_f64()).unwrap_or(0.0);
    let feedback = args.get("feedback").and_then(|v| v.as_f64()).unwrap_or(0.0);
    let crossfeed = args.get("crossfeed").and_then(|v| v.as_f64()).unwrap_or(0.0);
    let drymix = args.get("drymix").and_then(|v| v.as_f64()).unwrap_or(0.0);
    crate::audio::widen_stereo(input, &output, delay, feedback, crossfeed, drymix).unwrap_or_else(|e| e)
}

fn execute_normalize_speech_claude(args: &Value) -> String {
    let input = args["input_file"].as_str().unwrap_or("");
    let output_raw = args["output_file"].as_str().unwrap_or("");
    let output = ensure_outputs_directory(output_raw);
    let peak = args.get("peak").and_then(|v| v.as_f64()).unwrap_or(0.0);
    let strength = args.get("strength").and_then(|v| v.as_f64()).unwrap_or(0.0);
    crate::audio::normalize_speech(input, &output, peak, strength).unwrap_or_else(|e| e)
}

fn execute_normalize_speech_gemini(args: &HashMap<String, Value>) -> String {
    let input = args.get("input_file").and_then(|v| v.as_str()).unwrap_or("");
    let output_raw = args.get("output_file").and_then(|v| v.as_str()).unwrap_or("");
    let output = ensure_outputs_directory(output_raw);
    let peak = args.get("peak").and_then(|v| v.as_f64()).unwrap_or(0.0);
    let strength = args.get("strength").and_then(|v| v.as_f64()).unwrap_or(0.0);
    crate::audio::normalize_speech(input, &output, peak, strength).unwrap_or_else(|e| e)
}

fn execute_remove_silence_simple_claude(args: &Value) -> String {
    let input = args["input_file"].as_str().unwrap_or("");
    let output_raw = args["output_file"].as_str().unwrap_or("");
    let output = ensure_outputs_directory(output_raw);
    let threshold = args.get("threshold").and_then(|v| v.as_f64()).unwrap_or(0.0);
    let duration = args.get("duration").and_then(|v| v.as_f64()).unwrap_or(0.0);
    let periods = args.get("periods").and_then(|v| v.as_i64()).unwrap_or(0) as i32;
    crate::audio::remove_silence_simple(input, &output, threshold, duration, periods).unwrap_or_else(|e| e)
}

fn execute_remove_silence_simple_gemini(args: &HashMap<String, Value>) -> String {
    let input = args.get("input_file").and_then(|v| v.as_str()).unwrap_or("");
    let output_raw = args.get("output_file").and_then(|v| v.as_str()).unwrap_or("");
    let output = ensure_outputs_directory(output_raw);
    let threshold = args.get("threshold").and_then(|v| v.as_f64()).unwrap_or(0.0);
    let duration = args.get("duration").and_then(|v| v.as_f64()).unwrap_or(0.0);
    let periods = args.get("periods").and_then(|v| v.as_i64()).unwrap_or(0) as i32;
    crate::audio::remove_silence_simple(input, &output, threshold, duration, periods).unwrap_or_else(|e| e)
}

fn execute_soft_clip_audio_claude(args: &Value) -> String {
    let input = args["input_file"].as_str().unwrap_or("");
    let output_raw = args["output_file"].as_str().unwrap_or("");
    let output = ensure_outputs_directory(output_raw);
    let clip_type = args.get("clip_type").and_then(|v| v.as_str()).unwrap_or("tanh");
    let param = args.get("param").and_then(|v| v.as_f64()).unwrap_or(0.0);
    crate::audio::soft_clip_audio(input, &output, clip_type, param).unwrap_or_else(|e| e)
}

fn execute_soft_clip_audio_gemini(args: &HashMap<String, Value>) -> String {
    let input = args.get("input_file").and_then(|v| v.as_str()).unwrap_or("");
    let output_raw = args.get("output_file").and_then(|v| v.as_str()).unwrap_or("");
    let output = ensure_outputs_directory(output_raw);
    let clip_type = args.get("clip_type").and_then(|v| v.as_str()).unwrap_or("tanh");
    let param = args.get("param").and_then(|v| v.as_f64()).unwrap_or(0.0);
    crate::audio::soft_clip_audio(input, &output, clip_type, param).unwrap_or_else(|e| e)
}

fn execute_segment_video_claude(args: &Value) -> String {
    let input = args["input_file"].as_str().unwrap_or("");
    let output_pattern = args["output_pattern"].as_str().unwrap_or("");
    let output_pattern_out = ensure_outputs_directory(output_pattern);
    let segment_time = args.get("segment_time").and_then(|v| v.as_f64()).unwrap_or(60.0);
    let reset_str = args.get("reset_timestamps").and_then(|v| v.as_str()).unwrap_or("true");
    let reset = reset_str != "false";
    crate::core::segment_video(input, &output_pattern_out, segment_time, reset).unwrap_or_else(|e| e)
}

fn execute_segment_video_gemini(args: &HashMap<String, Value>) -> String {
    let input = args.get("input_file").and_then(|v| v.as_str()).unwrap_or("");
    let output_pattern = args.get("output_pattern").and_then(|v| v.as_str()).unwrap_or("");
    let output_pattern_out = ensure_outputs_directory(output_pattern);
    let segment_time = args.get("segment_time").and_then(|v| v.as_f64()).unwrap_or(60.0);
    let reset_str = args.get("reset_timestamps").and_then(|v| v.as_str()).unwrap_or("true");
    let reset = reset_str != "false";
    crate::core::segment_video(input, &output_pattern_out, segment_time, reset).unwrap_or_else(|e| e)
}

fn execute_pad_video_time_claude(args: &Value) -> String {
    let input = args["input_file"].as_str().unwrap_or("");
    let output_raw = args["output_file"].as_str().unwrap_or("");
    let output = ensure_outputs_directory(output_raw);
    let start_duration = args.get("start_duration").and_then(|v| v.as_f64()).unwrap_or(0.0);
    let stop_duration = args.get("stop_duration").and_then(|v| v.as_f64()).unwrap_or(0.0);
    let color = args.get("color").and_then(|v| v.as_str()).unwrap_or("black");
    crate::core::pad_video_time(input, &output, start_duration, stop_duration, color).unwrap_or_else(|e| e)
}

fn execute_pad_video_time_gemini(args: &HashMap<String, Value>) -> String {
    let input = args.get("input_file").and_then(|v| v.as_str()).unwrap_or("");
    let output_raw = args.get("output_file").and_then(|v| v.as_str()).unwrap_or("");
    let output = ensure_outputs_directory(output_raw);
    let start_duration = args.get("start_duration").and_then(|v| v.as_f64()).unwrap_or(0.0);
    let stop_duration = args.get("stop_duration").and_then(|v| v.as_f64()).unwrap_or(0.0);
    let color = args.get("color").and_then(|v| v.as_str()).unwrap_or("black");
    crate::core::pad_video_time(input, &output, start_duration, stop_duration, color).unwrap_or_else(|e| e)
}

// ============================================================================
// PHASE I BATCH 2 — Executor Functions
// ============================================================================

fn execute_select_frames_claude(args: &Value) -> String {
    let input = args["input_file"].as_str().unwrap_or("");
    let output_raw = args["output_file"].as_str().unwrap_or("");
    let output = ensure_outputs_directory(output_raw);
    let expr = args.get("expr").and_then(|v| v.as_str()).unwrap_or("");
    let fps = args.get("fps").and_then(|v| v.as_f64()).unwrap_or(0.0);
    crate::visual::select_frames(input, &output, expr, fps).unwrap_or_else(|e| e)
}

fn execute_select_frames_gemini(args: &HashMap<String, Value>) -> String {
    let input = args.get("input_file").and_then(|v| v.as_str()).unwrap_or("");
    let output_raw = args.get("output_file").and_then(|v| v.as_str()).unwrap_or("");
    let output = ensure_outputs_directory(output_raw);
    let expr = args.get("expr").and_then(|v| v.as_str()).unwrap_or("");
    let fps = args.get("fps").and_then(|v| v.as_f64()).unwrap_or(0.0);
    crate::visual::select_frames(input, &output, expr, fps).unwrap_or_else(|e| e)
}

fn execute_posterize_video_claude(args: &Value) -> String {
    let input = args["input_file"].as_str().unwrap_or("");
    let output_raw = args["output_file"].as_str().unwrap_or("");
    let output = ensure_outputs_directory(output_raw);
    let levels = args.get("levels").and_then(|v| v.as_u64()).unwrap_or(5) as u32;
    crate::visual::posterize_video(input, &output, levels).unwrap_or_else(|e| e)
}

fn execute_posterize_video_gemini(args: &HashMap<String, Value>) -> String {
    let input = args.get("input_file").and_then(|v| v.as_str()).unwrap_or("");
    let output_raw = args.get("output_file").and_then(|v| v.as_str()).unwrap_or("");
    let output = ensure_outputs_directory(output_raw);
    let levels = args.get("levels").and_then(|v| v.as_u64()).unwrap_or(5) as u32;
    crate::visual::posterize_video(input, &output, levels).unwrap_or_else(|e| e)
}

fn execute_solarize_video_claude(args: &Value) -> String {
    let input = args["input_file"].as_str().unwrap_or("");
    let output_raw = args["output_file"].as_str().unwrap_or("");
    let output = ensure_outputs_directory(output_raw);
    let threshold = args.get("threshold").and_then(|v| v.as_u64()).unwrap_or(128) as u32;
    crate::visual::solarize_video(input, &output, threshold).unwrap_or_else(|e| e)
}

fn execute_solarize_video_gemini(args: &HashMap<String, Value>) -> String {
    let input = args.get("input_file").and_then(|v| v.as_str()).unwrap_or("");
    let output_raw = args.get("output_file").and_then(|v| v.as_str()).unwrap_or("");
    let output = ensure_outputs_directory(output_raw);
    let threshold = args.get("threshold").and_then(|v| v.as_u64()).unwrap_or(128) as u32;
    crate::visual::solarize_video(input, &output, threshold).unwrap_or_else(|e| e)
}

fn execute_apply_dilation_claude(args: &Value) -> String {
    let input = args["input_file"].as_str().unwrap_or("");
    let output_raw = args["output_file"].as_str().unwrap_or("");
    let output = ensure_outputs_directory(output_raw);
    let t0 = args.get("threshold0").and_then(|v| v.as_u64()).unwrap_or(65535) as u32;
    let t1 = args.get("threshold1").and_then(|v| v.as_u64()).unwrap_or(65535) as u32;
    let t2 = args.get("threshold2").and_then(|v| v.as_u64()).unwrap_or(65535) as u32;
    let t3 = args.get("threshold3").and_then(|v| v.as_u64()).unwrap_or(65535) as u32;
    let coords = args.get("coordinates").and_then(|v| v.as_u64()).unwrap_or(255) as u32;
    crate::visual::apply_dilation(input, &output, t0, t1, t2, t3, coords).unwrap_or_else(|e| e)
}

fn execute_apply_dilation_gemini(args: &HashMap<String, Value>) -> String {
    let input = args.get("input_file").and_then(|v| v.as_str()).unwrap_or("");
    let output_raw = args.get("output_file").and_then(|v| v.as_str()).unwrap_or("");
    let output = ensure_outputs_directory(output_raw);
    let t0 = args.get("threshold0").and_then(|v| v.as_u64()).unwrap_or(65535) as u32;
    let t1 = args.get("threshold1").and_then(|v| v.as_u64()).unwrap_or(65535) as u32;
    let t2 = args.get("threshold2").and_then(|v| v.as_u64()).unwrap_or(65535) as u32;
    let t3 = args.get("threshold3").and_then(|v| v.as_u64()).unwrap_or(65535) as u32;
    let coords = args.get("coordinates").and_then(|v| v.as_u64()).unwrap_or(255) as u32;
    crate::visual::apply_dilation(input, &output, t0, t1, t2, t3, coords).unwrap_or_else(|e| e)
}

fn execute_apply_erosion_claude(args: &Value) -> String {
    let input = args["input_file"].as_str().unwrap_or("");
    let output_raw = args["output_file"].as_str().unwrap_or("");
    let output = ensure_outputs_directory(output_raw);
    let t0 = args.get("threshold0").and_then(|v| v.as_u64()).unwrap_or(65535) as u32;
    let t1 = args.get("threshold1").and_then(|v| v.as_u64()).unwrap_or(65535) as u32;
    let t2 = args.get("threshold2").and_then(|v| v.as_u64()).unwrap_or(65535) as u32;
    let t3 = args.get("threshold3").and_then(|v| v.as_u64()).unwrap_or(65535) as u32;
    let coords = args.get("coordinates").and_then(|v| v.as_u64()).unwrap_or(255) as u32;
    crate::visual::apply_erosion(input, &output, t0, t1, t2, t3, coords).unwrap_or_else(|e| e)
}

fn execute_apply_erosion_gemini(args: &HashMap<String, Value>) -> String {
    let input = args.get("input_file").and_then(|v| v.as_str()).unwrap_or("");
    let output_raw = args.get("output_file").and_then(|v| v.as_str()).unwrap_or("");
    let output = ensure_outputs_directory(output_raw);
    let t0 = args.get("threshold0").and_then(|v| v.as_u64()).unwrap_or(65535) as u32;
    let t1 = args.get("threshold1").and_then(|v| v.as_u64()).unwrap_or(65535) as u32;
    let t2 = args.get("threshold2").and_then(|v| v.as_u64()).unwrap_or(65535) as u32;
    let t3 = args.get("threshold3").and_then(|v| v.as_u64()).unwrap_or(65535) as u32;
    let coords = args.get("coordinates").and_then(|v| v.as_u64()).unwrap_or(255) as u32;
    crate::visual::apply_erosion(input, &output, t0, t1, t2, t3, coords).unwrap_or_else(|e| e)
}

fn execute_apply_median_filter_claude(args: &Value) -> String {
    let input = args["input_file"].as_str().unwrap_or("");
    let output_raw = args["output_file"].as_str().unwrap_or("");
    let output = ensure_outputs_directory(output_raw);
    let radius = args.get("radius").and_then(|v| v.as_u64()).unwrap_or(1) as u32;
    let planes = args.get("planes").and_then(|v| v.as_u64()).unwrap_or(15) as u32;
    crate::visual::apply_median_filter(input, &output, radius, planes).unwrap_or_else(|e| e)
}

fn execute_apply_median_filter_gemini(args: &HashMap<String, Value>) -> String {
    let input = args.get("input_file").and_then(|v| v.as_str()).unwrap_or("");
    let output_raw = args.get("output_file").and_then(|v| v.as_str()).unwrap_or("");
    let output = ensure_outputs_directory(output_raw);
    let radius = args.get("radius").and_then(|v| v.as_u64()).unwrap_or(1) as u32;
    let planes = args.get("planes").and_then(|v| v.as_u64()).unwrap_or(15) as u32;
    crate::visual::apply_median_filter(input, &output, radius, planes).unwrap_or_else(|e| e)
}

fn execute_apply_histogram_eq_claude(args: &Value) -> String {
    let input = args["input_file"].as_str().unwrap_or("");
    let output_raw = args["output_file"].as_str().unwrap_or("");
    let output = ensure_outputs_directory(output_raw);
    let strength = args.get("strength").and_then(|v| v.as_f64()).unwrap_or(0.2);
    let intensity = args.get("intensity").and_then(|v| v.as_f64()).unwrap_or(0.21);
    let antibanding = args.get("antibanding").and_then(|v| v.as_str()).unwrap_or("none");
    crate::visual::apply_histogram_eq(input, &output, strength, intensity, antibanding).unwrap_or_else(|e| e)
}

fn execute_apply_histogram_eq_gemini(args: &HashMap<String, Value>) -> String {
    let input = args.get("input_file").and_then(|v| v.as_str()).unwrap_or("");
    let output_raw = args.get("output_file").and_then(|v| v.as_str()).unwrap_or("");
    let output = ensure_outputs_directory(output_raw);
    let strength = args.get("strength").and_then(|v| v.as_f64()).unwrap_or(0.2);
    let intensity = args.get("intensity").and_then(|v| v.as_f64()).unwrap_or(0.21);
    let antibanding = args.get("antibanding").and_then(|v| v.as_str()).unwrap_or("none");
    crate::visual::apply_histogram_eq(input, &output, strength, intensity, antibanding).unwrap_or_else(|e| e)
}

fn execute_apply_clahe_claude(args: &Value) -> String {
    let input = args["input_file"].as_str().unwrap_or("");
    let output_raw = args["output_file"].as_str().unwrap_or("");
    let output = ensure_outputs_directory(output_raw);
    let clip_limit = args.get("clip_limit").and_then(|v| v.as_f64()).unwrap_or(25.0);
    let nb_tiles_x = args.get("nb_tiles_x").and_then(|v| v.as_u64()).unwrap_or(8) as u32;
    let nb_tiles_y = args.get("nb_tiles_y").and_then(|v| v.as_u64()).unwrap_or(8) as u32;
    crate::visual::apply_clahe(input, &output, clip_limit, nb_tiles_x, nb_tiles_y).unwrap_or_else(|e| e)
}

fn execute_apply_clahe_gemini(args: &HashMap<String, Value>) -> String {
    let input = args.get("input_file").and_then(|v| v.as_str()).unwrap_or("");
    let output_raw = args.get("output_file").and_then(|v| v.as_str()).unwrap_or("");
    let output = ensure_outputs_directory(output_raw);
    let clip_limit = args.get("clip_limit").and_then(|v| v.as_f64()).unwrap_or(25.0);
    let nb_tiles_x = args.get("nb_tiles_x").and_then(|v| v.as_u64()).unwrap_or(8) as u32;
    let nb_tiles_y = args.get("nb_tiles_y").and_then(|v| v.as_u64()).unwrap_or(8) as u32;
    crate::visual::apply_clahe(input, &output, clip_limit, nb_tiles_x, nb_tiles_y).unwrap_or_else(|e| e)
}

fn execute_apply_deblock_claude(args: &Value) -> String {
    let input = args["input_file"].as_str().unwrap_or("");
    let output_raw = args["output_file"].as_str().unwrap_or("");
    let output = ensure_outputs_directory(output_raw);
    let filter_type = args.get("filter_type").and_then(|v| v.as_u64()).unwrap_or(4) as u32;
    let block_size = args.get("block_size").and_then(|v| v.as_u64()).unwrap_or(8) as u32;
    let strength = args.get("strength").and_then(|v| v.as_f64()).unwrap_or(0.5);
    let planes = args.get("planes").and_then(|v| v.as_u64()).unwrap_or(15) as u32;
    crate::visual::apply_deblock(input, &output, filter_type, block_size, strength, planes).unwrap_or_else(|e| e)
}

fn execute_apply_deblock_gemini(args: &HashMap<String, Value>) -> String {
    let input = args.get("input_file").and_then(|v| v.as_str()).unwrap_or("");
    let output_raw = args.get("output_file").and_then(|v| v.as_str()).unwrap_or("");
    let output = ensure_outputs_directory(output_raw);
    let filter_type = args.get("filter_type").and_then(|v| v.as_u64()).unwrap_or(4) as u32;
    let block_size = args.get("block_size").and_then(|v| v.as_u64()).unwrap_or(8) as u32;
    let strength = args.get("strength").and_then(|v| v.as_f64()).unwrap_or(0.5);
    let planes = args.get("planes").and_then(|v| v.as_u64()).unwrap_or(15) as u32;
    crate::visual::apply_deblock(input, &output, filter_type, block_size, strength, planes).unwrap_or_else(|e| e)
}

fn execute_adjust_hue_saturation_claude(args: &Value) -> String {
    let input = args["input_file"].as_str().unwrap_or("");
    let output_raw = args["output_file"].as_str().unwrap_or("");
    let output = ensure_outputs_directory(output_raw);
    let hue = args.get("hue").and_then(|v| v.as_f64()).unwrap_or(0.0);
    let saturation = args.get("saturation").and_then(|v| v.as_f64()).unwrap_or(0.0);
    let intensity = args.get("intensity").and_then(|v| v.as_f64()).unwrap_or(0.0);
    let lightness = args.get("lightness").and_then(|v| v.as_f64()).unwrap_or(0.0);
    crate::visual::adjust_hue_saturation(input, &output, hue, saturation, intensity, lightness).unwrap_or_else(|e| e)
}

fn execute_adjust_hue_saturation_gemini(args: &HashMap<String, Value>) -> String {
    let input = args.get("input_file").and_then(|v| v.as_str()).unwrap_or("");
    let output_raw = args.get("output_file").and_then(|v| v.as_str()).unwrap_or("");
    let output = ensure_outputs_directory(output_raw);
    let hue = args.get("hue").and_then(|v| v.as_f64()).unwrap_or(0.0);
    let saturation = args.get("saturation").and_then(|v| v.as_f64()).unwrap_or(0.0);
    let intensity = args.get("intensity").and_then(|v| v.as_f64()).unwrap_or(0.0);
    let lightness = args.get("lightness").and_then(|v| v.as_f64()).unwrap_or(0.0);
    crate::visual::adjust_hue_saturation(input, &output, hue, saturation, intensity, lightness).unwrap_or_else(|e| e)
}

fn execute_apply_convolution_claude(args: &Value) -> String {
    let input = args["input_file"].as_str().unwrap_or("");
    let output_raw = args["output_file"].as_str().unwrap_or("");
    let output = ensure_outputs_directory(output_raw);
    let matrix = args.get("matrix").and_then(|v| v.as_str()).unwrap_or("");
    let rdiv = args.get("rdiv").and_then(|v| v.as_f64()).unwrap_or(1.0);
    let bias = args.get("bias").and_then(|v| v.as_f64()).unwrap_or(0.0);
    let planes = args.get("planes").and_then(|v| v.as_u64()).unwrap_or(15) as u32;
    crate::visual::apply_convolution(input, &output, matrix, rdiv, bias, planes).unwrap_or_else(|e| e)
}

fn execute_apply_convolution_gemini(args: &HashMap<String, Value>) -> String {
    let input = args.get("input_file").and_then(|v| v.as_str()).unwrap_or("");
    let output_raw = args.get("output_file").and_then(|v| v.as_str()).unwrap_or("");
    let output = ensure_outputs_directory(output_raw);
    let matrix = args.get("matrix").and_then(|v| v.as_str()).unwrap_or("");
    let rdiv = args.get("rdiv").and_then(|v| v.as_f64()).unwrap_or(1.0);
    let bias = args.get("bias").and_then(|v| v.as_f64()).unwrap_or(0.0);
    let planes = args.get("planes").and_then(|v| v.as_u64()).unwrap_or(15) as u32;
    crate::visual::apply_convolution(input, &output, matrix, rdiv, bias, planes).unwrap_or_else(|e| e)
}

fn execute_reverse_audio_claude(args: &Value) -> String {
    let input = args["input_file"].as_str().unwrap_or("");
    let output_raw = args["output_file"].as_str().unwrap_or("");
    let output = ensure_outputs_directory(output_raw);
    crate::audio::reverse_audio(input, &output).unwrap_or_else(|e| e)
}

fn execute_reverse_audio_gemini(args: &HashMap<String, Value>) -> String {
    let input = args.get("input_file").and_then(|v| v.as_str()).unwrap_or("");
    let output_raw = args.get("output_file").and_then(|v| v.as_str()).unwrap_or("");
    let output = ensure_outputs_directory(output_raw);
    crate::audio::reverse_audio(input, &output).unwrap_or_else(|e| e)
}

fn execute_blend_audio_streams_claude(args: &Value) -> String {
    let input = args["input_file"].as_str().unwrap_or("");
    let secondary = args["secondary_file"].as_str().unwrap_or("");
    let output_raw = args["output_file"].as_str().unwrap_or("");
    let output = ensure_outputs_directory(output_raw);
    let duration = args.get("duration").and_then(|v| v.as_str()).unwrap_or("longest");
    let dropout = args.get("dropout_transition").and_then(|v| v.as_f64()).unwrap_or(2.0);
    crate::audio::blend_audio_streams(input, secondary, &output, duration, dropout).unwrap_or_else(|e| e)
}

fn execute_blend_audio_streams_gemini(args: &HashMap<String, Value>) -> String {
    let input = args.get("input_file").and_then(|v| v.as_str()).unwrap_or("");
    let secondary = args.get("secondary_file").and_then(|v| v.as_str()).unwrap_or("");
    let output_raw = args.get("output_file").and_then(|v| v.as_str()).unwrap_or("");
    let output = ensure_outputs_directory(output_raw);
    let duration = args.get("duration").and_then(|v| v.as_str()).unwrap_or("longest");
    let dropout = args.get("dropout_transition").and_then(|v| v.as_f64()).unwrap_or(2.0);
    crate::audio::blend_audio_streams(input, secondary, &output, duration, dropout).unwrap_or_else(|e| e)
}

fn execute_measure_silence_claude(args: &Value) -> String {
    let input = args["input_file"].as_str().unwrap_or("");
    let noise_db = args.get("noise_db").and_then(|v| v.as_f64()).unwrap_or(-30.0);
    let duration_s = args.get("duration_s").and_then(|v| v.as_f64()).unwrap_or(0.5);
    crate::audio::measure_silence(input, noise_db, duration_s).unwrap_or_else(|e| e)
}

fn execute_measure_silence_gemini(args: &HashMap<String, Value>) -> String {
    let input = args.get("input_file").and_then(|v| v.as_str()).unwrap_or("");
    let noise_db = args.get("noise_db").and_then(|v| v.as_f64()).unwrap_or(-30.0);
    let duration_s = args.get("duration_s").and_then(|v| v.as_f64()).unwrap_or(0.5);
    crate::audio::measure_silence(input, noise_db, duration_s).unwrap_or_else(|e| e)
}

fn execute_measure_audio_spectrum_claude(args: &Value) -> String {
    let input = args["input_file"].as_str().unwrap_or("");
    let output_raw = args["output_file"].as_str().unwrap_or("");
    let output = ensure_outputs_directory(output_raw);
    let width = args.get("width").and_then(|v| v.as_u64()).unwrap_or(1024) as u32;
    let height = args.get("height").and_then(|v| v.as_u64()).unwrap_or(512) as u32;
    let mode = args.get("mode").and_then(|v| v.as_str()).unwrap_or("combined");
    let color = args.get("color").and_then(|v| v.as_str()).unwrap_or("intensity");
    crate::audio::measure_audio_spectrum(input, &output, width, height, mode, color).unwrap_or_else(|e| e)
}

fn execute_measure_audio_spectrum_gemini(args: &HashMap<String, Value>) -> String {
    let input = args.get("input_file").and_then(|v| v.as_str()).unwrap_or("");
    let output_raw = args.get("output_file").and_then(|v| v.as_str()).unwrap_or("");
    let output = ensure_outputs_directory(output_raw);
    let width = args.get("width").and_then(|v| v.as_u64()).unwrap_or(1024) as u32;
    let height = args.get("height").and_then(|v| v.as_u64()).unwrap_or(512) as u32;
    let mode = args.get("mode").and_then(|v| v.as_str()).unwrap_or("combined");
    let color = args.get("color").and_then(|v| v.as_str()).unwrap_or("intensity");
    crate::audio::measure_audio_spectrum(input, &output, width, height, mode, color).unwrap_or_else(|e| e)
}

// ============================================================================
// PHASE I BATCH 4 — Executor Functions
// ============================================================================

fn execute_apply_negate_claude(args: &Value) -> String {
    let input = args["input_file"].as_str().unwrap_or("");
    let output_raw = args["output_file"].as_str().unwrap_or("");
    let output = ensure_outputs_directory(output_raw);
    let components = args.get("components").and_then(|v| v.as_u64()).unwrap_or(7) as u32;
    crate::visual::apply_negate(input, &output, components).unwrap_or_else(|e| e)
}
fn execute_apply_negate_gemini(args: &HashMap<String, Value>) -> String {
    let input = args.get("input_file").and_then(|v| v.as_str()).unwrap_or("");
    let output_raw = args.get("output_file").and_then(|v| v.as_str()).unwrap_or("");
    let output = ensure_outputs_directory(output_raw);
    let components = args.get("components").and_then(|v| v.as_u64()).unwrap_or(7) as u32;
    crate::visual::apply_negate(input, &output, components).unwrap_or_else(|e| e)
}

fn execute_apply_pixelize_claude(args: &Value) -> String {
    let input = args["input_file"].as_str().unwrap_or("");
    let output_raw = args["output_file"].as_str().unwrap_or("");
    let output = ensure_outputs_directory(output_raw);
    let width = args.get("width").and_then(|v| v.as_u64()).unwrap_or(16) as u32;
    let height = args.get("height").and_then(|v| v.as_u64()).unwrap_or(0) as u32;
    let mode = args.get("mode").and_then(|v| v.as_u64()).unwrap_or(0) as u32;
    crate::visual::apply_pixelize(input, &output, width, height, mode).unwrap_or_else(|e| e)
}
fn execute_apply_pixelize_gemini(args: &HashMap<String, Value>) -> String {
    let input = args.get("input_file").and_then(|v| v.as_str()).unwrap_or("");
    let output_raw = args.get("output_file").and_then(|v| v.as_str()).unwrap_or("");
    let output = ensure_outputs_directory(output_raw);
    let width = args.get("width").and_then(|v| v.as_u64()).unwrap_or(16) as u32;
    let height = args.get("height").and_then(|v| v.as_u64()).unwrap_or(0) as u32;
    let mode = args.get("mode").and_then(|v| v.as_u64()).unwrap_or(0) as u32;
    crate::visual::apply_pixelize(input, &output, width, height, mode).unwrap_or_else(|e| e)
}

fn execute_apply_colorlevels_claude(args: &Value) -> String {
    let input = args["input_file"].as_str().unwrap_or("");
    let output_raw = args["output_file"].as_str().unwrap_or("");
    let output = ensure_outputs_directory(output_raw);
    let rimin = args.get("rimin").and_then(|v| v.as_f64()).unwrap_or(0.0);
    let rimax = args.get("rimax").and_then(|v| v.as_f64()).unwrap_or(1.0);
    let gimin = args.get("gimin").and_then(|v| v.as_f64()).unwrap_or(0.0);
    let gimax = args.get("gimax").and_then(|v| v.as_f64()).unwrap_or(1.0);
    let bimin = args.get("bimin").and_then(|v| v.as_f64()).unwrap_or(0.0);
    let bimax = args.get("bimax").and_then(|v| v.as_f64()).unwrap_or(1.0);
    let romin = args.get("romin").and_then(|v| v.as_f64()).unwrap_or(0.0);
    let romax = args.get("romax").and_then(|v| v.as_f64()).unwrap_or(1.0);
    crate::visual::apply_colorlevels(input, &output, rimin, rimax, gimin, gimax, bimin, bimax, romin, romax).unwrap_or_else(|e| e)
}
fn execute_apply_colorlevels_gemini(args: &HashMap<String, Value>) -> String {
    let input = args.get("input_file").and_then(|v| v.as_str()).unwrap_or("");
    let output_raw = args.get("output_file").and_then(|v| v.as_str()).unwrap_or("");
    let output = ensure_outputs_directory(output_raw);
    let rimin = args.get("rimin").and_then(|v| v.as_f64()).unwrap_or(0.0);
    let rimax = args.get("rimax").and_then(|v| v.as_f64()).unwrap_or(1.0);
    let gimin = args.get("gimin").and_then(|v| v.as_f64()).unwrap_or(0.0);
    let gimax = args.get("gimax").and_then(|v| v.as_f64()).unwrap_or(1.0);
    let bimin = args.get("bimin").and_then(|v| v.as_f64()).unwrap_or(0.0);
    let bimax = args.get("bimax").and_then(|v| v.as_f64()).unwrap_or(1.0);
    let romin = args.get("romin").and_then(|v| v.as_f64()).unwrap_or(0.0);
    let romax = args.get("romax").and_then(|v| v.as_f64()).unwrap_or(1.0);
    crate::visual::apply_colorlevels(input, &output, rimin, rimax, gimin, gimax, bimin, bimax, romin, romax).unwrap_or_else(|e| e)
}

fn execute_apply_pseudocolor_claude(args: &Value) -> String {
    let input = args["input_file"].as_str().unwrap_or("");
    let output_raw = args["output_file"].as_str().unwrap_or("");
    let output = ensure_outputs_directory(output_raw);
    let preset = args.get("preset").and_then(|v| v.as_i64()).unwrap_or(0) as i32;
    let opacity = args.get("opacity").and_then(|v| v.as_f64()).unwrap_or(1.0);
    crate::visual::apply_pseudocolor(input, &output, preset, opacity).unwrap_or_else(|e| e)
}
fn execute_apply_pseudocolor_gemini(args: &HashMap<String, Value>) -> String {
    let input = args.get("input_file").and_then(|v| v.as_str()).unwrap_or("");
    let output_raw = args.get("output_file").and_then(|v| v.as_str()).unwrap_or("");
    let output = ensure_outputs_directory(output_raw);
    let preset = args.get("preset").and_then(|v| v.as_i64()).unwrap_or(0) as i32;
    let opacity = args.get("opacity").and_then(|v| v.as_f64()).unwrap_or(1.0);
    crate::visual::apply_pseudocolor(input, &output, preset, opacity).unwrap_or_else(|e| e)
}

fn execute_apply_colorhold_claude(args: &Value) -> String {
    let input = args["input_file"].as_str().unwrap_or("");
    let output_raw = args["output_file"].as_str().unwrap_or("");
    let output = ensure_outputs_directory(output_raw);
    let color = args.get("color").and_then(|v| v.as_str()).unwrap_or("red");
    let similarity = args.get("similarity").and_then(|v| v.as_f64()).unwrap_or(0.1);
    let blend = args.get("blend").and_then(|v| v.as_f64()).unwrap_or(0.0);
    crate::visual::apply_colorhold(input, &output, color, similarity, blend).unwrap_or_else(|e| e)
}
fn execute_apply_colorhold_gemini(args: &HashMap<String, Value>) -> String {
    let input = args.get("input_file").and_then(|v| v.as_str()).unwrap_or("");
    let output_raw = args.get("output_file").and_then(|v| v.as_str()).unwrap_or("");
    let output = ensure_outputs_directory(output_raw);
    let color = args.get("color").and_then(|v| v.as_str()).unwrap_or("red");
    let similarity = args.get("similarity").and_then(|v| v.as_f64()).unwrap_or(0.1);
    let blend = args.get("blend").and_then(|v| v.as_f64()).unwrap_or(0.0);
    crate::visual::apply_colorhold(input, &output, color, similarity, blend).unwrap_or_else(|e| e)
}

fn execute_apply_shuffleplanes_claude(args: &Value) -> String {
    let input = args["input_file"].as_str().unwrap_or("");
    let output_raw = args["output_file"].as_str().unwrap_or("");
    let output = ensure_outputs_directory(output_raw);
    let map0 = args.get("map0").and_then(|v| v.as_u64()).unwrap_or(0) as u32;
    let map1 = args.get("map1").and_then(|v| v.as_u64()).unwrap_or(1) as u32;
    let map2 = args.get("map2").and_then(|v| v.as_u64()).unwrap_or(2) as u32;
    let map3 = args.get("map3").and_then(|v| v.as_u64()).unwrap_or(3) as u32;
    crate::visual::apply_shuffleplanes(input, &output, map0, map1, map2, map3).unwrap_or_else(|e| e)
}
fn execute_apply_shuffleplanes_gemini(args: &HashMap<String, Value>) -> String {
    let input = args.get("input_file").and_then(|v| v.as_str()).unwrap_or("");
    let output_raw = args.get("output_file").and_then(|v| v.as_str()).unwrap_or("");
    let output = ensure_outputs_directory(output_raw);
    let map0 = args.get("map0").and_then(|v| v.as_u64()).unwrap_or(0) as u32;
    let map1 = args.get("map1").and_then(|v| v.as_u64()).unwrap_or(1) as u32;
    let map2 = args.get("map2").and_then(|v| v.as_u64()).unwrap_or(2) as u32;
    let map3 = args.get("map3").and_then(|v| v.as_u64()).unwrap_or(3) as u32;
    crate::visual::apply_shuffleplanes(input, &output, map0, map1, map2, map3).unwrap_or_else(|e| e)
}

fn execute_detect_black_frames_claude(args: &Value) -> String {
    let input = args["input_file"].as_str().unwrap_or("");
    let d = args.get("black_min_duration").and_then(|v| v.as_f64()).unwrap_or(2.0);
    let pbr = args.get("picture_black_ratio_th").and_then(|v| v.as_f64()).unwrap_or(0.98);
    let pbt = args.get("pixel_black_th").and_then(|v| v.as_f64()).unwrap_or(0.10);
    crate::visual::detect_black_frames(input, d, pbr, pbt).unwrap_or_else(|e| e)
}
fn execute_detect_black_frames_gemini(args: &HashMap<String, Value>) -> String {
    let input = args.get("input_file").and_then(|v| v.as_str()).unwrap_or("");
    let d = args.get("black_min_duration").and_then(|v| v.as_f64()).unwrap_or(2.0);
    let pbr = args.get("picture_black_ratio_th").and_then(|v| v.as_f64()).unwrap_or(0.98);
    let pbt = args.get("pixel_black_th").and_then(|v| v.as_f64()).unwrap_or(0.10);
    crate::visual::detect_black_frames(input, d, pbr, pbt).unwrap_or_else(|e| e)
}

fn execute_detect_interlace_type_claude(args: &Value) -> String {
    let input = args["input_file"].as_str().unwrap_or("");
    crate::visual::detect_interlace_type(input).unwrap_or_else(|e| e)
}
fn execute_detect_interlace_type_gemini(args: &HashMap<String, Value>) -> String {
    let input = args.get("input_file").and_then(|v| v.as_str()).unwrap_or("");
    crate::visual::detect_interlace_type(input).unwrap_or_else(|e| e)
}

fn execute_apply_vstack_claude(args: &Value) -> String {
    let input = args["input_file"].as_str().unwrap_or("");
    let secondary = args["secondary_file"].as_str().unwrap_or("");
    let output_raw = args["output_file"].as_str().unwrap_or("");
    let output = ensure_outputs_directory(output_raw);
    let shortest = args.get("shortest").and_then(|v| v.as_bool()).unwrap_or(false);
    crate::visual::apply_vstack(input, secondary, &output, shortest).unwrap_or_else(|e| e)
}
fn execute_apply_vstack_gemini(args: &HashMap<String, Value>) -> String {
    let input = args.get("input_file").and_then(|v| v.as_str()).unwrap_or("");
    let secondary = args.get("secondary_file").and_then(|v| v.as_str()).unwrap_or("");
    let output_raw = args.get("output_file").and_then(|v| v.as_str()).unwrap_or("");
    let output = ensure_outputs_directory(output_raw);
    let shortest = args.get("shortest").and_then(|v| v.as_bool()).unwrap_or(false);
    crate::visual::apply_vstack(input, secondary, &output, shortest).unwrap_or_else(|e| e)
}

fn execute_apply_hstack_claude(args: &Value) -> String {
    let input = args["input_file"].as_str().unwrap_or("");
    let secondary = args["secondary_file"].as_str().unwrap_or("");
    let output_raw = args["output_file"].as_str().unwrap_or("");
    let output = ensure_outputs_directory(output_raw);
    let shortest = args.get("shortest").and_then(|v| v.as_bool()).unwrap_or(false);
    crate::visual::apply_hstack(input, secondary, &output, shortest).unwrap_or_else(|e| e)
}
fn execute_apply_hstack_gemini(args: &HashMap<String, Value>) -> String {
    let input = args.get("input_file").and_then(|v| v.as_str()).unwrap_or("");
    let secondary = args.get("secondary_file").and_then(|v| v.as_str()).unwrap_or("");
    let output_raw = args.get("output_file").and_then(|v| v.as_str()).unwrap_or("");
    let output = ensure_outputs_directory(output_raw);
    let shortest = args.get("shortest").and_then(|v| v.as_bool()).unwrap_or(false);
    crate::visual::apply_hstack(input, secondary, &output, shortest).unwrap_or_else(|e| e)
}

fn execute_apply_setdar_claude(args: &Value) -> String {
    let input = args["input_file"].as_str().unwrap_or("");
    let output_raw = args["output_file"].as_str().unwrap_or("");
    let output = ensure_outputs_directory(output_raw);
    let dar = args.get("dar").and_then(|v| v.as_str()).unwrap_or("16/9");
    crate::visual::apply_setdar(input, &output, dar).unwrap_or_else(|e| e)
}
fn execute_apply_setdar_gemini(args: &HashMap<String, Value>) -> String {
    let input = args.get("input_file").and_then(|v| v.as_str()).unwrap_or("");
    let output_raw = args.get("output_file").and_then(|v| v.as_str()).unwrap_or("");
    let output = ensure_outputs_directory(output_raw);
    let dar = args.get("dar").and_then(|v| v.as_str()).unwrap_or("16/9");
    crate::visual::apply_setdar(input, &output, dar).unwrap_or_else(|e| e)
}

fn execute_apply_stereo3d_claude(args: &Value) -> String {
    let input = args["input_file"].as_str().unwrap_or("");
    let output_raw = args["output_file"].as_str().unwrap_or("");
    let output = ensure_outputs_directory(output_raw);
    let inf = args.get("input_format").and_then(|v| v.as_str()).unwrap_or("sbsl");
    let outf = args.get("output_format").and_then(|v| v.as_str()).unwrap_or("arcd");
    crate::visual::apply_stereo3d(input, &output, inf, outf).unwrap_or_else(|e| e)
}
fn execute_apply_stereo3d_gemini(args: &HashMap<String, Value>) -> String {
    let input = args.get("input_file").and_then(|v| v.as_str()).unwrap_or("");
    let output_raw = args.get("output_file").and_then(|v| v.as_str()).unwrap_or("");
    let output = ensure_outputs_directory(output_raw);
    let inf = args.get("input_format").and_then(|v| v.as_str()).unwrap_or("sbsl");
    let outf = args.get("output_format").and_then(|v| v.as_str()).unwrap_or("arcd");
    crate::visual::apply_stereo3d(input, &output, inf, outf).unwrap_or_else(|e| e)
}

fn execute_apply_telecine_claude(args: &Value) -> String {
    let input = args["input_file"].as_str().unwrap_or("");
    let output_raw = args["output_file"].as_str().unwrap_or("");
    let output = ensure_outputs_directory(output_raw);
    let pattern = args.get("pattern").and_then(|v| v.as_str()).unwrap_or("23");
    let first_field = args.get("first_field").and_then(|v| v.as_u64()).unwrap_or(0) as u32;
    crate::visual::apply_telecine(input, &output, pattern, first_field).unwrap_or_else(|e| e)
}
fn execute_apply_telecine_gemini(args: &HashMap<String, Value>) -> String {
    let input = args.get("input_file").and_then(|v| v.as_str()).unwrap_or("");
    let output_raw = args.get("output_file").and_then(|v| v.as_str()).unwrap_or("");
    let output = ensure_outputs_directory(output_raw);
    let pattern = args.get("pattern").and_then(|v| v.as_str()).unwrap_or("23");
    let first_field = args.get("first_field").and_then(|v| v.as_u64()).unwrap_or(0) as u32;
    crate::visual::apply_telecine(input, &output, pattern, first_field).unwrap_or_else(|e| e)
}

fn execute_apply_pullup_claude(args: &Value) -> String {
    let input = args["input_file"].as_str().unwrap_or("");
    let output_raw = args["output_file"].as_str().unwrap_or("");
    let output = ensure_outputs_directory(output_raw);
    crate::visual::apply_pullup(input, &output).unwrap_or_else(|e| e)
}
fn execute_apply_pullup_gemini(args: &HashMap<String, Value>) -> String {
    let input = args.get("input_file").and_then(|v| v.as_str()).unwrap_or("");
    let output_raw = args.get("output_file").and_then(|v| v.as_str()).unwrap_or("");
    let output = ensure_outputs_directory(output_raw);
    crate::visual::apply_pullup(input, &output).unwrap_or_else(|e| e)
}

fn execute_select_thumbnail_frame_claude(args: &Value) -> String {
    let input = args["input_file"].as_str().unwrap_or("");
    let output_raw = args["output_file"].as_str().unwrap_or("");
    let output = ensure_outputs_directory(output_raw);
    let n = args.get("n").and_then(|v| v.as_u64()).unwrap_or(100) as u32;
    crate::visual::select_thumbnail_frame(input, &output, n).unwrap_or_else(|e| e)
}
fn execute_select_thumbnail_frame_gemini(args: &HashMap<String, Value>) -> String {
    let input = args.get("input_file").and_then(|v| v.as_str()).unwrap_or("");
    let output_raw = args.get("output_file").and_then(|v| v.as_str()).unwrap_or("");
    let output = ensure_outputs_directory(output_raw);
    let n = args.get("n").and_then(|v| v.as_u64()).unwrap_or(100) as u32;
    crate::visual::select_thumbnail_frame(input, &output, n).unwrap_or_else(|e| e)
}

// ============================================================================
// PHASE I BATCH 3 — Executor Functions
// ============================================================================

fn execute_apply_gaussian_blur_claude(args: &Value) -> String {
    let input = args["input_file"].as_str().unwrap_or("");
    let output_raw = args["output_file"].as_str().unwrap_or("");
    let output = ensure_outputs_directory(output_raw);
    let sigma = args.get("sigma").and_then(|v| v.as_f64()).unwrap_or(3.0);
    let steps = args.get("steps").and_then(|v| v.as_u64()).unwrap_or(1) as u32;
    let planes = args.get("planes").and_then(|v| v.as_u64()).unwrap_or(15) as u32;
    crate::visual::apply_gaussian_blur(input, &output, sigma, steps, planes).unwrap_or_else(|e| e)
}

fn execute_apply_gaussian_blur_gemini(args: &HashMap<String, Value>) -> String {
    let input = args.get("input_file").and_then(|v| v.as_str()).unwrap_or("");
    let output_raw = args.get("output_file").and_then(|v| v.as_str()).unwrap_or("");
    let output = ensure_outputs_directory(output_raw);
    let sigma = args.get("sigma").and_then(|v| v.as_f64()).unwrap_or(3.0);
    let steps = args.get("steps").and_then(|v| v.as_u64()).unwrap_or(1) as u32;
    let planes = args.get("planes").and_then(|v| v.as_u64()).unwrap_or(15) as u32;
    crate::visual::apply_gaussian_blur(input, &output, sigma, steps, planes).unwrap_or_else(|e| e)
}

fn execute_apply_box_blur_claude(args: &Value) -> String {
    let input = args["input_file"].as_str().unwrap_or("");
    let output_raw = args["output_file"].as_str().unwrap_or("");
    let output = ensure_outputs_directory(output_raw);
    let size_x = args.get("size_x").and_then(|v| v.as_u64()).unwrap_or(3) as u32;
    let size_y = args.get("size_y").and_then(|v| v.as_u64()).unwrap_or(0) as u32;
    let planes = args.get("planes").and_then(|v| v.as_u64()).unwrap_or(15) as u32;
    crate::visual::apply_box_blur(input, &output, size_x, size_y, planes).unwrap_or_else(|e| e)
}

fn execute_apply_box_blur_gemini(args: &HashMap<String, Value>) -> String {
    let input = args.get("input_file").and_then(|v| v.as_str()).unwrap_or("");
    let output_raw = args.get("output_file").and_then(|v| v.as_str()).unwrap_or("");
    let output = ensure_outputs_directory(output_raw);
    let size_x = args.get("size_x").and_then(|v| v.as_u64()).unwrap_or(3) as u32;
    let size_y = args.get("size_y").and_then(|v| v.as_u64()).unwrap_or(0) as u32;
    let planes = args.get("planes").and_then(|v| v.as_u64()).unwrap_or(15) as u32;
    crate::visual::apply_box_blur(input, &output, size_x, size_y, planes).unwrap_or_else(|e| e)
}

fn execute_apply_smart_blur_claude(args: &Value) -> String {
    let input = args["input_file"].as_str().unwrap_or("");
    let output_raw = args["output_file"].as_str().unwrap_or("");
    let output = ensure_outputs_directory(output_raw);
    let luma_radius = args.get("luma_radius").and_then(|v| v.as_f64()).unwrap_or(1.0);
    let luma_strength = args.get("luma_strength").and_then(|v| v.as_f64()).unwrap_or(-0.3);
    let luma_threshold = args.get("luma_threshold").and_then(|v| v.as_i64()).unwrap_or(0) as i32;
    crate::visual::apply_smart_blur(input, &output, luma_radius, luma_strength, luma_threshold).unwrap_or_else(|e| e)
}

fn execute_apply_smart_blur_gemini(args: &HashMap<String, Value>) -> String {
    let input = args.get("input_file").and_then(|v| v.as_str()).unwrap_or("");
    let output_raw = args.get("output_file").and_then(|v| v.as_str()).unwrap_or("");
    let output = ensure_outputs_directory(output_raw);
    let luma_radius = args.get("luma_radius").and_then(|v| v.as_f64()).unwrap_or(1.0);
    let luma_strength = args.get("luma_strength").and_then(|v| v.as_f64()).unwrap_or(-0.3);
    let luma_threshold = args.get("luma_threshold").and_then(|v| v.as_i64()).unwrap_or(0) as i32;
    crate::visual::apply_smart_blur(input, &output, luma_radius, luma_strength, luma_threshold).unwrap_or_else(|e| e)
}

fn execute_add_film_grain_claude(args: &Value) -> String {
    let input = args["input_file"].as_str().unwrap_or("");
    let output_raw = args["output_file"].as_str().unwrap_or("");
    let output = ensure_outputs_directory(output_raw);
    let all_strength = args.get("all_strength").and_then(|v| v.as_u64()).unwrap_or(8) as u32;
    let flags = args.get("flags").and_then(|v| v.as_str()).unwrap_or("a");
    crate::visual::add_film_grain(input, &output, all_strength, flags).unwrap_or_else(|e| e)
}

fn execute_add_film_grain_gemini(args: &HashMap<String, Value>) -> String {
    let input = args.get("input_file").and_then(|v| v.as_str()).unwrap_or("");
    let output_raw = args.get("output_file").and_then(|v| v.as_str()).unwrap_or("");
    let output = ensure_outputs_directory(output_raw);
    let all_strength = args.get("all_strength").and_then(|v| v.as_u64()).unwrap_or(8) as u32;
    let flags = args.get("flags").and_then(|v| v.as_str()).unwrap_or("a");
    crate::visual::add_film_grain(input, &output, all_strength, flags).unwrap_or_else(|e| e)
}

fn execute_apply_rotate_angle_claude(args: &Value) -> String {
    let input = args["input_file"].as_str().unwrap_or("");
    let output_raw = args["output_file"].as_str().unwrap_or("");
    let output = ensure_outputs_directory(output_raw);
    let angle_rad = args.get("angle_rad").and_then(|v| v.as_f64()).unwrap_or(0.0);
    let fillcolor = args.get("fillcolor").and_then(|v| v.as_str()).unwrap_or("black");
    let expand = args.get("expand").and_then(|v| v.as_bool()).unwrap_or(false);
    crate::transform::apply_rotate_angle(input, &output, angle_rad, fillcolor, expand).unwrap_or_else(|e| e)
}

fn execute_apply_rotate_angle_gemini(args: &HashMap<String, Value>) -> String {
    let input = args.get("input_file").and_then(|v| v.as_str()).unwrap_or("");
    let output_raw = args.get("output_file").and_then(|v| v.as_str()).unwrap_or("");
    let output = ensure_outputs_directory(output_raw);
    let angle_rad = args.get("angle_rad").and_then(|v| v.as_f64()).unwrap_or(0.0);
    let fillcolor = args.get("fillcolor").and_then(|v| v.as_str()).unwrap_or("black");
    let expand = args.get("expand").and_then(|v| v.as_bool()).unwrap_or(false);
    crate::transform::apply_rotate_angle(input, &output, angle_rad, fillcolor, expand).unwrap_or_else(|e| e)
}

fn execute_apply_geq_claude(args: &Value) -> String {
    let input = args["input_file"].as_str().unwrap_or("");
    let output_raw = args["output_file"].as_str().unwrap_or("");
    let output = ensure_outputs_directory(output_raw);
    let lum_expr = args.get("lum_expr").and_then(|v| v.as_str()).unwrap_or("");
    let cb_expr = args.get("cb_expr").and_then(|v| v.as_str()).unwrap_or("");
    let cr_expr = args.get("cr_expr").and_then(|v| v.as_str()).unwrap_or("");
    crate::visual::apply_geq(input, &output, lum_expr, cb_expr, cr_expr).unwrap_or_else(|e| e)
}

fn execute_apply_geq_gemini(args: &HashMap<String, Value>) -> String {
    let input = args.get("input_file").and_then(|v| v.as_str()).unwrap_or("");
    let output_raw = args.get("output_file").and_then(|v| v.as_str()).unwrap_or("");
    let output = ensure_outputs_directory(output_raw);
    let lum_expr = args.get("lum_expr").and_then(|v| v.as_str()).unwrap_or("");
    let cb_expr = args.get("cb_expr").and_then(|v| v.as_str()).unwrap_or("");
    let cr_expr = args.get("cr_expr").and_then(|v| v.as_str()).unwrap_or("");
    crate::visual::apply_geq(input, &output, lum_expr, cb_expr, cr_expr).unwrap_or_else(|e| e)
}

fn execute_apply_colorchannelmixer_claude(args: &Value) -> String {
    let input = args["input_file"].as_str().unwrap_or("");
    let output_raw = args["output_file"].as_str().unwrap_or("");
    let output = ensure_outputs_directory(output_raw);
    let rr = args.get("rr").and_then(|v| v.as_f64()).unwrap_or(1.0);
    let rg = args.get("rg").and_then(|v| v.as_f64()).unwrap_or(0.0);
    let rb = args.get("rb").and_then(|v| v.as_f64()).unwrap_or(0.0);
    let gr = args.get("gr").and_then(|v| v.as_f64()).unwrap_or(0.0);
    let gg = args.get("gg").and_then(|v| v.as_f64()).unwrap_or(1.0);
    let gb = args.get("gb").and_then(|v| v.as_f64()).unwrap_or(0.0);
    let br = args.get("br").and_then(|v| v.as_f64()).unwrap_or(0.0);
    let bg = args.get("bg").and_then(|v| v.as_f64()).unwrap_or(0.0);
    let bb = args.get("bb").and_then(|v| v.as_f64()).unwrap_or(1.0);
    crate::visual::apply_colorchannelmixer(input, &output, rr, rg, rb, gr, gg, gb, br, bg, bb).unwrap_or_else(|e| e)
}

fn execute_apply_colorchannelmixer_gemini(args: &HashMap<String, Value>) -> String {
    let input = args.get("input_file").and_then(|v| v.as_str()).unwrap_or("");
    let output_raw = args.get("output_file").and_then(|v| v.as_str()).unwrap_or("");
    let output = ensure_outputs_directory(output_raw);
    let rr = args.get("rr").and_then(|v| v.as_f64()).unwrap_or(1.0);
    let rg = args.get("rg").and_then(|v| v.as_f64()).unwrap_or(0.0);
    let rb = args.get("rb").and_then(|v| v.as_f64()).unwrap_or(0.0);
    let gr = args.get("gr").and_then(|v| v.as_f64()).unwrap_or(0.0);
    let gg = args.get("gg").and_then(|v| v.as_f64()).unwrap_or(1.0);
    let gb = args.get("gb").and_then(|v| v.as_f64()).unwrap_or(0.0);
    let br = args.get("br").and_then(|v| v.as_f64()).unwrap_or(0.0);
    let bg = args.get("bg").and_then(|v| v.as_f64()).unwrap_or(0.0);
    let bb = args.get("bb").and_then(|v| v.as_f64()).unwrap_or(1.0);
    crate::visual::apply_colorchannelmixer(input, &output, rr, rg, rb, gr, gg, gb, br, bg, bb).unwrap_or_else(|e| e)
}

fn execute_apply_atadenoise_claude(args: &Value) -> String {
    let input = args["input_file"].as_str().unwrap_or("");
    let output_raw = args["output_file"].as_str().unwrap_or("");
    let output = ensure_outputs_directory(output_raw);
    let window_size = args.get("window_size").and_then(|v| v.as_u64()).unwrap_or(9) as u32;
    let threshold_a = args.get("threshold_a").and_then(|v| v.as_f64()).unwrap_or(0.02);
    let threshold_b = args.get("threshold_b").and_then(|v| v.as_f64()).unwrap_or(0.04);
    let planes = args.get("planes").and_then(|v| v.as_u64()).unwrap_or(7) as u32;
    crate::visual::apply_atadenoise(input, &output, window_size, threshold_a, threshold_b, planes).unwrap_or_else(|e| e)
}

fn execute_apply_atadenoise_gemini(args: &HashMap<String, Value>) -> String {
    let input = args.get("input_file").and_then(|v| v.as_str()).unwrap_or("");
    let output_raw = args.get("output_file").and_then(|v| v.as_str()).unwrap_or("");
    let output = ensure_outputs_directory(output_raw);
    let window_size = args.get("window_size").and_then(|v| v.as_u64()).unwrap_or(9) as u32;
    let threshold_a = args.get("threshold_a").and_then(|v| v.as_f64()).unwrap_or(0.02);
    let threshold_b = args.get("threshold_b").and_then(|v| v.as_f64()).unwrap_or(0.04);
    let planes = args.get("planes").and_then(|v| v.as_u64()).unwrap_or(7) as u32;
    crate::visual::apply_atadenoise(input, &output, window_size, threshold_a, threshold_b, planes).unwrap_or_else(|e| e)
}

fn execute_apply_vaguedenoiser_claude(args: &Value) -> String {
    let input = args["input_file"].as_str().unwrap_or("");
    let output_raw = args["output_file"].as_str().unwrap_or("");
    let output = ensure_outputs_directory(output_raw);
    let threshold = args.get("threshold").and_then(|v| v.as_f64()).unwrap_or(2.0);
    let method = args.get("method").and_then(|v| v.as_u64()).unwrap_or(0) as u32;
    let nsteps = args.get("nsteps").and_then(|v| v.as_u64()).unwrap_or(6) as u32;
    let percent = args.get("percent").and_then(|v| v.as_f64()).unwrap_or(85.0);
    let planes = args.get("planes").and_then(|v| v.as_u64()).unwrap_or(7) as u32;
    crate::visual::apply_vaguedenoiser(input, &output, threshold, method, nsteps, percent, planes).unwrap_or_else(|e| e)
}

fn execute_apply_vaguedenoiser_gemini(args: &HashMap<String, Value>) -> String {
    let input = args.get("input_file").and_then(|v| v.as_str()).unwrap_or("");
    let output_raw = args.get("output_file").and_then(|v| v.as_str()).unwrap_or("");
    let output = ensure_outputs_directory(output_raw);
    let threshold = args.get("threshold").and_then(|v| v.as_f64()).unwrap_or(2.0);
    let method = args.get("method").and_then(|v| v.as_u64()).unwrap_or(0) as u32;
    let nsteps = args.get("nsteps").and_then(|v| v.as_u64()).unwrap_or(6) as u32;
    let percent = args.get("percent").and_then(|v| v.as_f64()).unwrap_or(85.0);
    let planes = args.get("planes").and_then(|v| v.as_u64()).unwrap_or(7) as u32;
    crate::visual::apply_vaguedenoiser(input, &output, threshold, method, nsteps, percent, planes).unwrap_or_else(|e| e)
}

fn execute_apply_fftdnoiz_claude(args: &Value) -> String {
    let input = args["input_file"].as_str().unwrap_or("");
    let output_raw = args["output_file"].as_str().unwrap_or("");
    let output = ensure_outputs_directory(output_raw);
    let sigma = args.get("sigma").and_then(|v| v.as_f64()).unwrap_or(1.0);
    let amount = args.get("amount").and_then(|v| v.as_f64()).unwrap_or(0.96);
    let block_size = args.get("block_size").and_then(|v| v.as_u64()).unwrap_or(32) as u32;
    let overlap = args.get("overlap").and_then(|v| v.as_f64()).unwrap_or(0.5);
    let planes = args.get("planes").and_then(|v| v.as_u64()).unwrap_or(7) as u32;
    crate::visual::apply_fftdnoiz(input, &output, sigma, amount, block_size, overlap, planes).unwrap_or_else(|e| e)
}

fn execute_apply_fftdnoiz_gemini(args: &HashMap<String, Value>) -> String {
    let input = args.get("input_file").and_then(|v| v.as_str()).unwrap_or("");
    let output_raw = args.get("output_file").and_then(|v| v.as_str()).unwrap_or("");
    let output = ensure_outputs_directory(output_raw);
    let sigma = args.get("sigma").and_then(|v| v.as_f64()).unwrap_or(1.0);
    let amount = args.get("amount").and_then(|v| v.as_f64()).unwrap_or(0.96);
    let block_size = args.get("block_size").and_then(|v| v.as_u64()).unwrap_or(32) as u32;
    let overlap = args.get("overlap").and_then(|v| v.as_f64()).unwrap_or(0.5);
    let planes = args.get("planes").and_then(|v| v.as_u64()).unwrap_or(7) as u32;
    crate::visual::apply_fftdnoiz(input, &output, sigma, amount, block_size, overlap, planes).unwrap_or_else(|e| e)
}

fn execute_generate_waveform_video_claude(args: &Value) -> String {
    let input = args["input_file"].as_str().unwrap_or("");
    let output_raw = args["output_file"].as_str().unwrap_or("");
    let output = ensure_outputs_directory(output_raw);
    let width = args.get("width").and_then(|v| v.as_u64()).unwrap_or(1280) as u32;
    let height = args.get("height").and_then(|v| v.as_u64()).unwrap_or(240) as u32;
    let mode = args.get("mode").and_then(|v| v.as_str()).unwrap_or("line");
    let color = args.get("color").and_then(|v| v.as_str()).unwrap_or("white");
    crate::audio::generate_waveform_video(input, &output, width, height, mode, color).unwrap_or_else(|e| e)
}

fn execute_generate_waveform_video_gemini(args: &HashMap<String, Value>) -> String {
    let input = args.get("input_file").and_then(|v| v.as_str()).unwrap_or("");
    let output_raw = args.get("output_file").and_then(|v| v.as_str()).unwrap_or("");
    let output = ensure_outputs_directory(output_raw);
    let width = args.get("width").and_then(|v| v.as_u64()).unwrap_or(1280) as u32;
    let height = args.get("height").and_then(|v| v.as_u64()).unwrap_or(240) as u32;
    let mode = args.get("mode").and_then(|v| v.as_str()).unwrap_or("line");
    let color = args.get("color").and_then(|v| v.as_str()).unwrap_or("white");
    crate::audio::generate_waveform_video(input, &output, width, height, mode, color).unwrap_or_else(|e| e)
}

fn execute_apply_lut3d_claude(args: &Value) -> String {
    let input = args["input_file"].as_str().unwrap_or("");
    let output_raw = args["output_file"].as_str().unwrap_or("");
    let output = ensure_outputs_directory(output_raw);
    let lut_file = args.get("lut_file").and_then(|v| v.as_str()).unwrap_or("");
    let interp = args.get("interp").and_then(|v| v.as_str()).unwrap_or("tetrahedral");
    crate::visual::apply_lut3d(input, &output, lut_file, interp).unwrap_or_else(|e| e)
}

fn execute_apply_lut3d_gemini(args: &HashMap<String, Value>) -> String {
    let input = args.get("input_file").and_then(|v| v.as_str()).unwrap_or("");
    let output_raw = args.get("output_file").and_then(|v| v.as_str()).unwrap_or("");
    let output = ensure_outputs_directory(output_raw);
    let lut_file = args.get("lut_file").and_then(|v| v.as_str()).unwrap_or("");
    let interp = args.get("interp").and_then(|v| v.as_str()).unwrap_or("tetrahedral");
    crate::visual::apply_lut3d(input, &output, lut_file, interp).unwrap_or_else(|e| e)
}

fn execute_measure_siti_claude(args: &Value) -> String {
    let input = args["input_file"].as_str().unwrap_or("");
    crate::visual::measure_siti(input).unwrap_or_else(|e| e)
}

fn execute_measure_siti_gemini(args: &HashMap<String, Value>) -> String {
    let input = args.get("input_file").and_then(|v| v.as_str()).unwrap_or("");
    crate::visual::measure_siti(input).unwrap_or_else(|e| e)
}

fn execute_create_test_pattern_claude(args: &Value) -> String {
    let output_raw = args["output_file"].as_str().unwrap_or("");
    let output = ensure_outputs_directory(output_raw);
    let width = args.get("width").and_then(|v| v.as_u64()).unwrap_or(1920) as u32;
    let height = args.get("height").and_then(|v| v.as_u64()).unwrap_or(1080) as u32;
    let duration = args.get("duration").and_then(|v| v.as_f64()).unwrap_or(10.0);
    let pattern = args.get("pattern").and_then(|v| v.as_str()).unwrap_or("smptebars");
    let framerate = args.get("framerate").and_then(|v| v.as_f64()).unwrap_or(25.0);
    crate::core::create_test_pattern(&output, width, height, duration, pattern, framerate).unwrap_or_else(|e| e)
}

fn execute_create_test_pattern_gemini(args: &HashMap<String, Value>) -> String {
    let output_raw = args.get("output_file").and_then(|v| v.as_str()).unwrap_or("");
    let output = ensure_outputs_directory(output_raw);
    let width = args.get("width").and_then(|v| v.as_u64()).unwrap_or(1920) as u32;
    let height = args.get("height").and_then(|v| v.as_u64()).unwrap_or(1080) as u32;
    let duration = args.get("duration").and_then(|v| v.as_f64()).unwrap_or(10.0);
    let pattern = args.get("pattern").and_then(|v| v.as_str()).unwrap_or("smptebars");
    let framerate = args.get("framerate").and_then(|v| v.as_f64()).unwrap_or(25.0);
    crate::core::create_test_pattern(&output, width, height, duration, pattern, framerate).unwrap_or_else(|e| e)
}

fn execute_apply_amplify_claude(args: &Value) -> String {
    let input = args["input_file"].as_str().unwrap_or("");
    let output_raw = args["output_file"].as_str().unwrap_or("");
    let output = ensure_outputs_directory(output_raw);
    let radius = args.get("radius").and_then(|v| v.as_u64()).unwrap_or(2) as u32;
    let factor = args.get("factor").and_then(|v| v.as_f64()).unwrap_or(2.0);
    let threshold = args.get("threshold").and_then(|v| v.as_f64()).unwrap_or(10.0);
    let planes = args.get("planes").and_then(|v| v.as_u64()).unwrap_or(7) as u32;
    crate::visual::apply_amplify(input, &output, radius, factor, threshold, planes).unwrap_or_else(|e| e)
}

fn execute_apply_amplify_gemini(args: &HashMap<String, Value>) -> String {
    let input = args.get("input_file").and_then(|v| v.as_str()).unwrap_or("");
    let output_raw = args.get("output_file").and_then(|v| v.as_str()).unwrap_or("");
    let output = ensure_outputs_directory(output_raw);
    let radius = args.get("radius").and_then(|v| v.as_u64()).unwrap_or(2) as u32;
    let factor = args.get("factor").and_then(|v| v.as_f64()).unwrap_or(2.0);
    let threshold = args.get("threshold").and_then(|v| v.as_f64()).unwrap_or(10.0);
    let planes = args.get("planes").and_then(|v| v.as_u64()).unwrap_or(7) as u32;
    crate::visual::apply_amplify(input, &output, radius, factor, threshold, planes).unwrap_or_else(|e| e)
}

// ============================================================
// PHASE I BATCH 5 EXECUTORS
// ============================================================

fn execute_apply_threshold_claude(args: &Value) -> String {
    let input = args["input_file"].as_str().unwrap_or("");
    let output_raw = args["output_file"].as_str().unwrap_or("");
    let output = ensure_outputs_directory(output_raw);
    let planes = args.get("planes").and_then(|v| v.as_u64()).unwrap_or(15) as u32;
    crate::visual::apply_threshold(input, &output, planes).unwrap_or_else(|e| e)
}

fn execute_apply_threshold_gemini(args: &HashMap<String, Value>) -> String {
    let input = args.get("input_file").and_then(|v| v.as_str()).unwrap_or("");
    let output_raw = args.get("output_file").and_then(|v| v.as_str()).unwrap_or("");
    let output = ensure_outputs_directory(output_raw);
    let planes = args.get("planes").and_then(|v| v.as_u64()).unwrap_or(15) as u32;
    crate::visual::apply_threshold(input, &output, planes).unwrap_or_else(|e| e)
}

fn execute_apply_maskedclamp_claude(args: &Value) -> String {
    let input = args["input_file"].as_str().unwrap_or("");
    let output_raw = args["output_file"].as_str().unwrap_or("");
    let output = ensure_outputs_directory(output_raw);
    let undershoot = args.get("undershoot").and_then(|v| v.as_u64()).unwrap_or(0) as u32;
    let overshoot = args.get("overshoot").and_then(|v| v.as_u64()).unwrap_or(0) as u32;
    let planes = args.get("planes").and_then(|v| v.as_u64()).unwrap_or(15) as u32;
    crate::visual::apply_maskedclamp(input, &output, undershoot, overshoot, planes).unwrap_or_else(|e| e)
}

fn execute_apply_maskedclamp_gemini(args: &HashMap<String, Value>) -> String {
    let input = args.get("input_file").and_then(|v| v.as_str()).unwrap_or("");
    let output_raw = args.get("output_file").and_then(|v| v.as_str()).unwrap_or("");
    let output = ensure_outputs_directory(output_raw);
    let undershoot = args.get("undershoot").and_then(|v| v.as_u64()).unwrap_or(0) as u32;
    let overshoot = args.get("overshoot").and_then(|v| v.as_u64()).unwrap_or(0) as u32;
    let planes = args.get("planes").and_then(|v| v.as_u64()).unwrap_or(15) as u32;
    crate::visual::apply_maskedclamp(input, &output, undershoot, overshoot, planes).unwrap_or_else(|e| e)
}

fn execute_apply_roberts_claude(args: &Value) -> String {
    let input = args["input_file"].as_str().unwrap_or("");
    let output_raw = args["output_file"].as_str().unwrap_or("");
    let output = ensure_outputs_directory(output_raw);
    let planes = args.get("planes").and_then(|v| v.as_u64()).unwrap_or(15) as u32;
    let scale = args.get("scale").and_then(|v| v.as_f64()).unwrap_or(1.0);
    let delta = args.get("delta").and_then(|v| v.as_f64()).unwrap_or(0.0);
    crate::visual::apply_roberts(input, &output, planes, scale, delta).unwrap_or_else(|e| e)
}

fn execute_apply_roberts_gemini(args: &HashMap<String, Value>) -> String {
    let input = args.get("input_file").and_then(|v| v.as_str()).unwrap_or("");
    let output_raw = args.get("output_file").and_then(|v| v.as_str()).unwrap_or("");
    let output = ensure_outputs_directory(output_raw);
    let planes = args.get("planes").and_then(|v| v.as_u64()).unwrap_or(15) as u32;
    let scale = args.get("scale").and_then(|v| v.as_f64()).unwrap_or(1.0);
    let delta = args.get("delta").and_then(|v| v.as_f64()).unwrap_or(0.0);
    crate::visual::apply_roberts(input, &output, planes, scale, delta).unwrap_or_else(|e| e)
}

fn execute_apply_sobel_claude(args: &Value) -> String {
    let input = args["input_file"].as_str().unwrap_or("");
    let output_raw = args["output_file"].as_str().unwrap_or("");
    let output = ensure_outputs_directory(output_raw);
    let planes = args.get("planes").and_then(|v| v.as_u64()).unwrap_or(15) as u32;
    let scale = args.get("scale").and_then(|v| v.as_f64()).unwrap_or(1.0);
    let delta = args.get("delta").and_then(|v| v.as_f64()).unwrap_or(0.0);
    crate::visual::apply_sobel(input, &output, planes, scale, delta).unwrap_or_else(|e| e)
}

fn execute_apply_sobel_gemini(args: &HashMap<String, Value>) -> String {
    let input = args.get("input_file").and_then(|v| v.as_str()).unwrap_or("");
    let output_raw = args.get("output_file").and_then(|v| v.as_str()).unwrap_or("");
    let output = ensure_outputs_directory(output_raw);
    let planes = args.get("planes").and_then(|v| v.as_u64()).unwrap_or(15) as u32;
    let scale = args.get("scale").and_then(|v| v.as_f64()).unwrap_or(1.0);
    let delta = args.get("delta").and_then(|v| v.as_f64()).unwrap_or(0.0);
    crate::visual::apply_sobel(input, &output, planes, scale, delta).unwrap_or_else(|e| e)
}

fn execute_apply_prewitt_claude(args: &Value) -> String {
    let input = args["input_file"].as_str().unwrap_or("");
    let output_raw = args["output_file"].as_str().unwrap_or("");
    let output = ensure_outputs_directory(output_raw);
    let planes = args.get("planes").and_then(|v| v.as_u64()).unwrap_or(15) as u32;
    let scale = args.get("scale").and_then(|v| v.as_f64()).unwrap_or(1.0);
    let delta = args.get("delta").and_then(|v| v.as_f64()).unwrap_or(0.0);
    crate::visual::apply_prewitt(input, &output, planes, scale, delta).unwrap_or_else(|e| e)
}

fn execute_apply_prewitt_gemini(args: &HashMap<String, Value>) -> String {
    let input = args.get("input_file").and_then(|v| v.as_str()).unwrap_or("");
    let output_raw = args.get("output_file").and_then(|v| v.as_str()).unwrap_or("");
    let output = ensure_outputs_directory(output_raw);
    let planes = args.get("planes").and_then(|v| v.as_u64()).unwrap_or(15) as u32;
    let scale = args.get("scale").and_then(|v| v.as_f64()).unwrap_or(1.0);
    let delta = args.get("delta").and_then(|v| v.as_f64()).unwrap_or(0.0);
    crate::visual::apply_prewitt(input, &output, planes, scale, delta).unwrap_or_else(|e| e)
}

fn execute_apply_kirsch_claude(args: &Value) -> String {
    let input = args["input_file"].as_str().unwrap_or("");
    let output_raw = args["output_file"].as_str().unwrap_or("");
    let output = ensure_outputs_directory(output_raw);
    let planes = args.get("planes").and_then(|v| v.as_u64()).unwrap_or(15) as u32;
    let scale = args.get("scale").and_then(|v| v.as_f64()).unwrap_or(1.0);
    let delta = args.get("delta").and_then(|v| v.as_f64()).unwrap_or(0.0);
    crate::visual::apply_kirsch(input, &output, planes, scale, delta).unwrap_or_else(|e| e)
}

fn execute_apply_kirsch_gemini(args: &HashMap<String, Value>) -> String {
    let input = args.get("input_file").and_then(|v| v.as_str()).unwrap_or("");
    let output_raw = args.get("output_file").and_then(|v| v.as_str()).unwrap_or("");
    let output = ensure_outputs_directory(output_raw);
    let planes = args.get("planes").and_then(|v| v.as_u64()).unwrap_or(15) as u32;
    let scale = args.get("scale").and_then(|v| v.as_f64()).unwrap_or(1.0);
    let delta = args.get("delta").and_then(|v| v.as_f64()).unwrap_or(0.0);
    crate::visual::apply_kirsch(input, &output, planes, scale, delta).unwrap_or_else(|e| e)
}

fn execute_apply_video_limiter_claude(args: &Value) -> String {
    let input = args["input_file"].as_str().unwrap_or("");
    let output_raw = args["output_file"].as_str().unwrap_or("");
    let output = ensure_outputs_directory(output_raw);
    let min = args.get("min").and_then(|v| v.as_u64()).unwrap_or(0) as u32;
    let max = args.get("max").and_then(|v| v.as_u64()).unwrap_or(65535) as u32;
    let planes = args.get("planes").and_then(|v| v.as_u64()).unwrap_or(15) as u32;
    crate::visual::apply_video_limiter(input, &output, min, max, planes).unwrap_or_else(|e| e)
}

fn execute_apply_video_limiter_gemini(args: &HashMap<String, Value>) -> String {
    let input = args.get("input_file").and_then(|v| v.as_str()).unwrap_or("");
    let output_raw = args.get("output_file").and_then(|v| v.as_str()).unwrap_or("");
    let output = ensure_outputs_directory(output_raw);
    let min = args.get("min").and_then(|v| v.as_u64()).unwrap_or(0) as u32;
    let max = args.get("max").and_then(|v| v.as_u64()).unwrap_or(65535) as u32;
    let planes = args.get("planes").and_then(|v| v.as_u64()).unwrap_or(15) as u32;
    crate::visual::apply_video_limiter(input, &output, min, max, planes).unwrap_or_else(|e| e)
}

fn execute_apply_bilateral_claude(args: &Value) -> String {
    let input = args["input_file"].as_str().unwrap_or("");
    let output_raw = args["output_file"].as_str().unwrap_or("");
    let output = ensure_outputs_directory(output_raw);
    let sigma_s = args.get("sigmaS").and_then(|v| v.as_f64()).unwrap_or(0.1);
    let sigma_r = args.get("sigmaR").and_then(|v| v.as_f64()).unwrap_or(0.1);
    let planes = args.get("planes").and_then(|v| v.as_u64()).unwrap_or(1) as u32;
    crate::visual::apply_bilateral(input, &output, sigma_s, sigma_r, planes).unwrap_or_else(|e| e)
}

fn execute_apply_bilateral_gemini(args: &HashMap<String, Value>) -> String {
    let input = args.get("input_file").and_then(|v| v.as_str()).unwrap_or("");
    let output_raw = args.get("output_file").and_then(|v| v.as_str()).unwrap_or("");
    let output = ensure_outputs_directory(output_raw);
    let sigma_s = args.get("sigmaS").and_then(|v| v.as_f64()).unwrap_or(0.1);
    let sigma_r = args.get("sigmaR").and_then(|v| v.as_f64()).unwrap_or(0.1);
    let planes = args.get("planes").and_then(|v| v.as_u64()).unwrap_or(1) as u32;
    crate::visual::apply_bilateral(input, &output, sigma_s, sigma_r, planes).unwrap_or_else(|e| e)
}

fn execute_apply_unsharp_mask_claude(args: &Value) -> String {
    let input = args["input_file"].as_str().unwrap_or("");
    let output_raw = args["output_file"].as_str().unwrap_or("");
    let output = ensure_outputs_directory(output_raw);
    let lx = args.get("luma_x").and_then(|v| v.as_u64()).unwrap_or(5) as u32;
    let ly = args.get("luma_y").and_then(|v| v.as_u64()).unwrap_or(5) as u32;
    let la = args.get("luma_amount").and_then(|v| v.as_f64()).unwrap_or(1.0);
    let cx = args.get("chroma_x").and_then(|v| v.as_u64()).unwrap_or(5) as u32;
    let cy = args.get("chroma_y").and_then(|v| v.as_u64()).unwrap_or(5) as u32;
    let ca = args.get("chroma_amount").and_then(|v| v.as_f64()).unwrap_or(0.0);
    crate::visual::apply_unsharp_mask(input, &output, lx, ly, la, cx, cy, ca).unwrap_or_else(|e| e)
}

fn execute_apply_unsharp_mask_gemini(args: &HashMap<String, Value>) -> String {
    let input = args.get("input_file").and_then(|v| v.as_str()).unwrap_or("");
    let output_raw = args.get("output_file").and_then(|v| v.as_str()).unwrap_or("");
    let output = ensure_outputs_directory(output_raw);
    let lx = args.get("luma_x").and_then(|v| v.as_u64()).unwrap_or(5) as u32;
    let ly = args.get("luma_y").and_then(|v| v.as_u64()).unwrap_or(5) as u32;
    let la = args.get("luma_amount").and_then(|v| v.as_f64()).unwrap_or(1.0);
    let cx = args.get("chroma_x").and_then(|v| v.as_u64()).unwrap_or(5) as u32;
    let cy = args.get("chroma_y").and_then(|v| v.as_u64()).unwrap_or(5) as u32;
    let ca = args.get("chroma_amount").and_then(|v| v.as_f64()).unwrap_or(0.0);
    crate::visual::apply_unsharp_mask(input, &output, lx, ly, la, cx, cy, ca).unwrap_or_else(|e| e)
}

fn execute_apply_lagfun_claude(args: &Value) -> String {
    let input = args["input_file"].as_str().unwrap_or("");
    let output_raw = args["output_file"].as_str().unwrap_or("");
    let output = ensure_outputs_directory(output_raw);
    let decay = args.get("decay").and_then(|v| v.as_f64()).unwrap_or(0.95);
    let planes = args.get("planes").and_then(|v| v.as_u64()).unwrap_or(15) as u32;
    crate::visual::apply_lagfun(input, &output, decay, planes).unwrap_or_else(|e| e)
}

fn execute_apply_lagfun_gemini(args: &HashMap<String, Value>) -> String {
    let input = args.get("input_file").and_then(|v| v.as_str()).unwrap_or("");
    let output_raw = args.get("output_file").and_then(|v| v.as_str()).unwrap_or("");
    let output = ensure_outputs_directory(output_raw);
    let decay = args.get("decay").and_then(|v| v.as_f64()).unwrap_or(0.95);
    let planes = args.get("planes").and_then(|v| v.as_u64()).unwrap_or(15) as u32;
    crate::visual::apply_lagfun(input, &output, decay, planes).unwrap_or_else(|e| e)
}

fn execute_apply_tinterlace_claude(args: &Value) -> String {
    let input = args["input_file"].as_str().unwrap_or("");
    let output_raw = args["output_file"].as_str().unwrap_or("");
    let output = ensure_outputs_directory(output_raw);
    let mode = args.get("mode").and_then(|v| v.as_u64()).unwrap_or(0) as u32;
    let flags = args.get("flags").and_then(|v| v.as_str()).unwrap_or("vlpf");
    crate::visual::apply_tinterlace(input, &output, mode, flags).unwrap_or_else(|e| e)
}

fn execute_apply_tinterlace_gemini(args: &HashMap<String, Value>) -> String {
    let input = args.get("input_file").and_then(|v| v.as_str()).unwrap_or("");
    let output_raw = args.get("output_file").and_then(|v| v.as_str()).unwrap_or("");
    let output = ensure_outputs_directory(output_raw);
    let mode = args.get("mode").and_then(|v| v.as_u64()).unwrap_or(0) as u32;
    let flags = args.get("flags").and_then(|v| v.as_str()).unwrap_or("vlpf");
    crate::visual::apply_tinterlace(input, &output, mode, flags).unwrap_or_else(|e| e)
}

fn execute_apply_datascope_claude(args: &Value) -> String {
    let input = args["input_file"].as_str().unwrap_or("");
    let output_raw = args["output_file"].as_str().unwrap_or("");
    let output = ensure_outputs_directory(output_raw);
    let size = args.get("size").and_then(|v| v.as_str()).unwrap_or("hd720");
    let x = args.get("x").and_then(|v| v.as_u64()).unwrap_or(0) as u32;
    let y = args.get("y").and_then(|v| v.as_u64()).unwrap_or(0) as u32;
    let mode = args.get("mode").and_then(|v| v.as_u64()).unwrap_or(0) as u32;
    let opacity = args.get("opacity").and_then(|v| v.as_f64()).unwrap_or(0.75);
    let axis = args.get("axis").and_then(|v| v.as_bool()).unwrap_or(false);
    crate::visual::apply_datascope(input, &output, size, x, y, mode, opacity, axis).unwrap_or_else(|e| e)
}

fn execute_apply_datascope_gemini(args: &HashMap<String, Value>) -> String {
    let input = args.get("input_file").and_then(|v| v.as_str()).unwrap_or("");
    let output_raw = args.get("output_file").and_then(|v| v.as_str()).unwrap_or("");
    let output = ensure_outputs_directory(output_raw);
    let size = args.get("size").and_then(|v| v.as_str()).unwrap_or("hd720");
    let x = args.get("x").and_then(|v| v.as_u64()).unwrap_or(0) as u32;
    let y = args.get("y").and_then(|v| v.as_u64()).unwrap_or(0) as u32;
    let mode = args.get("mode").and_then(|v| v.as_u64()).unwrap_or(0) as u32;
    let opacity = args.get("opacity").and_then(|v| v.as_f64()).unwrap_or(0.75);
    let axis = args.get("axis").and_then(|v| v.as_bool()).unwrap_or(false);
    crate::visual::apply_datascope(input, &output, size, x, y, mode, opacity, axis).unwrap_or_else(|e| e)
}

fn execute_apply_fspp_claude(args: &Value) -> String {
    let input = args["input_file"].as_str().unwrap_or("");
    let output_raw = args["output_file"].as_str().unwrap_or("");
    let output = ensure_outputs_directory(output_raw);
    let quality = args.get("quality").and_then(|v| v.as_u64()).unwrap_or(4) as u32;
    let strength = args.get("strength").and_then(|v| v.as_f64()).unwrap_or(0.0);
    let use_bframe_qp = args.get("use_bframe_qp").and_then(|v| v.as_bool()).unwrap_or(false);
    crate::visual::apply_fspp(input, &output, quality, strength, use_bframe_qp).unwrap_or_else(|e| e)
}

fn execute_apply_fspp_gemini(args: &HashMap<String, Value>) -> String {
    let input = args.get("input_file").and_then(|v| v.as_str()).unwrap_or("");
    let output_raw = args.get("output_file").and_then(|v| v.as_str()).unwrap_or("");
    let output = ensure_outputs_directory(output_raw);
    let quality = args.get("quality").and_then(|v| v.as_u64()).unwrap_or(4) as u32;
    let strength = args.get("strength").and_then(|v| v.as_f64()).unwrap_or(0.0);
    let use_bframe_qp = args.get("use_bframe_qp").and_then(|v| v.as_bool()).unwrap_or(false);
    crate::visual::apply_fspp(input, &output, quality, strength, use_bframe_qp).unwrap_or_else(|e| e)
}

fn execute_apply_haas_claude(args: &Value) -> String {
    let input = args["input_file"].as_str().unwrap_or("");
    let output_raw = args["output_file"].as_str().unwrap_or("");
    let output = ensure_outputs_directory(output_raw);
    let level_in = args.get("level_in").and_then(|v| v.as_f64()).unwrap_or(1.0);
    let level_out = args.get("level_out").and_then(|v| v.as_f64()).unwrap_or(1.0);
    let side_gain = args.get("side_gain").and_then(|v| v.as_f64()).unwrap_or(1.0);
    let middle_source = args.get("middle_source").and_then(|v| v.as_str()).unwrap_or("mid");
    let middle_phase = args.get("middle_phase").and_then(|v| v.as_bool()).unwrap_or(false);
    let left_delay = args.get("left_delay").and_then(|v| v.as_f64()).unwrap_or(2.5);
    let left_balance = args.get("left_balance").and_then(|v| v.as_f64()).unwrap_or(-1.0);
    let right_delay = args.get("right_delay").and_then(|v| v.as_f64()).unwrap_or(2.5);
    let right_balance = args.get("right_balance").and_then(|v| v.as_f64()).unwrap_or(1.0);
    crate::audio::apply_haas(input, &output, level_in, level_out, side_gain, middle_source, middle_phase, left_delay, left_balance, right_delay, right_balance).unwrap_or_else(|e| e)
}

fn execute_apply_haas_gemini(args: &HashMap<String, Value>) -> String {
    let input = args.get("input_file").and_then(|v| v.as_str()).unwrap_or("");
    let output_raw = args.get("output_file").and_then(|v| v.as_str()).unwrap_or("");
    let output = ensure_outputs_directory(output_raw);
    let level_in = args.get("level_in").and_then(|v| v.as_f64()).unwrap_or(1.0);
    let level_out = args.get("level_out").and_then(|v| v.as_f64()).unwrap_or(1.0);
    let side_gain = args.get("side_gain").and_then(|v| v.as_f64()).unwrap_or(1.0);
    let middle_source = args.get("middle_source").and_then(|v| v.as_str()).unwrap_or("mid");
    let middle_phase = args.get("middle_phase").and_then(|v| v.as_bool()).unwrap_or(false);
    let left_delay = args.get("left_delay").and_then(|v| v.as_f64()).unwrap_or(2.5);
    let left_balance = args.get("left_balance").and_then(|v| v.as_f64()).unwrap_or(-1.0);
    let right_delay = args.get("right_delay").and_then(|v| v.as_f64()).unwrap_or(2.5);
    let right_balance = args.get("right_balance").and_then(|v| v.as_f64()).unwrap_or(1.0);
    crate::audio::apply_haas(input, &output, level_in, level_out, side_gain, middle_source, middle_phase, left_delay, left_balance, right_delay, right_balance).unwrap_or_else(|e| e)
}

fn execute_apply_aemphasis_claude(args: &Value) -> String {
    let input = args["input_file"].as_str().unwrap_or("");
    let output_raw = args["output_file"].as_str().unwrap_or("");
    let output = ensure_outputs_directory(output_raw);
    let level_in = args.get("level_in").and_then(|v| v.as_f64()).unwrap_or(1.0);
    let level_out = args.get("level_out").and_then(|v| v.as_f64()).unwrap_or(1.0);
    let mode = args.get("mode").and_then(|v| v.as_str()).unwrap_or("reproduction");
    let emph_type = args.get("emph_type").and_then(|v| v.as_str()).unwrap_or("cd");
    crate::audio::apply_aemphasis(input, &output, level_in, level_out, mode, emph_type).unwrap_or_else(|e| e)
}

fn execute_apply_aemphasis_gemini(args: &HashMap<String, Value>) -> String {
    let input = args.get("input_file").and_then(|v| v.as_str()).unwrap_or("");
    let output_raw = args.get("output_file").and_then(|v| v.as_str()).unwrap_or("");
    let output = ensure_outputs_directory(output_raw);
    let level_in = args.get("level_in").and_then(|v| v.as_f64()).unwrap_or(1.0);
    let level_out = args.get("level_out").and_then(|v| v.as_f64()).unwrap_or(1.0);
    let mode = args.get("mode").and_then(|v| v.as_str()).unwrap_or("reproduction");
    let emph_type = args.get("emph_type").and_then(|v| v.as_str()).unwrap_or("cd");
    crate::audio::apply_aemphasis(input, &output, level_in, level_out, mode, emph_type).unwrap_or_else(|e| e)
}

// ============================================================
// PHASE I BATCH 6 EXECUTORS
// ============================================================

fn execute_apply_colormatrix_claude(args: &Value) -> String {
    let input = args["input_file"].as_str().unwrap_or("");
    let output_raw = args["output_file"].as_str().unwrap_or("");
    let output = ensure_outputs_directory(output_raw);
    let src = args.get("src").and_then(|v| v.as_str()).unwrap_or("bt601");
    let dst = args.get("dst").and_then(|v| v.as_str()).unwrap_or("bt709");
    crate::visual::apply_colormatrix(input, &output, src, dst).unwrap_or_else(|e| e)
}

fn execute_apply_colormatrix_gemini(args: &HashMap<String, Value>) -> String {
    let input = args.get("input_file").and_then(|v| v.as_str()).unwrap_or("");
    let output_raw = args.get("output_file").and_then(|v| v.as_str()).unwrap_or("");
    let output = ensure_outputs_directory(output_raw);
    let src = args.get("src").and_then(|v| v.as_str()).unwrap_or("bt601");
    let dst = args.get("dst").and_then(|v| v.as_str()).unwrap_or("bt709");
    crate::visual::apply_colormatrix(input, &output, src, dst).unwrap_or_else(|e| e)
}

fn execute_apply_chromashift_claude(args: &Value) -> String {
    let input = args["input_file"].as_str().unwrap_or("");
    let output_raw = args["output_file"].as_str().unwrap_or("");
    let output = ensure_outputs_directory(output_raw);
    let cbh = args.get("cbh").and_then(|v| v.as_i64()).unwrap_or(0) as i32;
    let cbv = args.get("cbv").and_then(|v| v.as_i64()).unwrap_or(0) as i32;
    let crh = args.get("crh").and_then(|v| v.as_i64()).unwrap_or(0) as i32;
    let crv = args.get("crv").and_then(|v| v.as_i64()).unwrap_or(0) as i32;
    crate::visual::apply_chromashift(input, &output, cbh, cbv, crh, crv).unwrap_or_else(|e| e)
}

fn execute_apply_chromashift_gemini(args: &HashMap<String, Value>) -> String {
    let input = args.get("input_file").and_then(|v| v.as_str()).unwrap_or("");
    let output_raw = args.get("output_file").and_then(|v| v.as_str()).unwrap_or("");
    let output = ensure_outputs_directory(output_raw);
    let cbh = args.get("cbh").and_then(|v| v.as_i64()).unwrap_or(0) as i32;
    let cbv = args.get("cbv").and_then(|v| v.as_i64()).unwrap_or(0) as i32;
    let crh = args.get("crh").and_then(|v| v.as_i64()).unwrap_or(0) as i32;
    let crv = args.get("crv").and_then(|v| v.as_i64()).unwrap_or(0) as i32;
    crate::visual::apply_chromashift(input, &output, cbh, cbv, crh, crv).unwrap_or_else(|e| e)
}

fn execute_apply_cas_claude(args: &Value) -> String {
    let input = args["input_file"].as_str().unwrap_or("");
    let output_raw = args["output_file"].as_str().unwrap_or("");
    let output = ensure_outputs_directory(output_raw);
    let strength = args.get("strength").and_then(|v| v.as_f64()).unwrap_or(0.0);
    let planes = args.get("planes").and_then(|v| v.as_u64()).unwrap_or(7) as u32;
    crate::visual::apply_cas(input, &output, strength, planes).unwrap_or_else(|e| e)
}

fn execute_apply_cas_gemini(args: &HashMap<String, Value>) -> String {
    let input = args.get("input_file").and_then(|v| v.as_str()).unwrap_or("");
    let output_raw = args.get("output_file").and_then(|v| v.as_str()).unwrap_or("");
    let output = ensure_outputs_directory(output_raw);
    let strength = args.get("strength").and_then(|v| v.as_f64()).unwrap_or(0.0);
    let planes = args.get("planes").and_then(|v| v.as_u64()).unwrap_or(7) as u32;
    crate::visual::apply_cas(input, &output, strength, planes).unwrap_or_else(|e| e)
}

fn execute_apply_nlmeans_video_claude(args: &Value) -> String {
    let input = args["input_file"].as_str().unwrap_or("");
    let output_raw = args["output_file"].as_str().unwrap_or("");
    let output = ensure_outputs_directory(output_raw);
    let s = args.get("s").and_then(|v| v.as_f64()).unwrap_or(1.0);
    let p = args.get("p").and_then(|v| v.as_u64()).unwrap_or(3) as u32;
    let pc = args.get("pc").and_then(|v| v.as_u64()).unwrap_or(0) as u32;
    let r = args.get("r").and_then(|v| v.as_u64()).unwrap_or(7) as u32;
    let rc = args.get("rc").and_then(|v| v.as_u64()).unwrap_or(0) as u32;
    crate::visual::apply_nlmeans_video(input, &output, s, p, pc, r, rc).unwrap_or_else(|e| e)
}

fn execute_apply_nlmeans_video_gemini(args: &HashMap<String, Value>) -> String {
    let input = args.get("input_file").and_then(|v| v.as_str()).unwrap_or("");
    let output_raw = args.get("output_file").and_then(|v| v.as_str()).unwrap_or("");
    let output = ensure_outputs_directory(output_raw);
    let s = args.get("s").and_then(|v| v.as_f64()).unwrap_or(1.0);
    let p = args.get("p").and_then(|v| v.as_u64()).unwrap_or(3) as u32;
    let pc = args.get("pc").and_then(|v| v.as_u64()).unwrap_or(0) as u32;
    let r = args.get("r").and_then(|v| v.as_u64()).unwrap_or(7) as u32;
    let rc = args.get("rc").and_then(|v| v.as_u64()).unwrap_or(0) as u32;
    crate::visual::apply_nlmeans_video(input, &output, s, p, pc, r, rc).unwrap_or_else(|e| e)
}

fn execute_apply_spp_claude(args: &Value) -> String {
    let input = args["input_file"].as_str().unwrap_or("");
    let output_raw = args["output_file"].as_str().unwrap_or("");
    let output = ensure_outputs_directory(output_raw);
    let quality = args.get("quality").and_then(|v| v.as_u64()).unwrap_or(3) as u32;
    let qp = args.get("qp").and_then(|v| v.as_i64()).unwrap_or(0) as i32;
    let mode = args.get("mode").and_then(|v| v.as_str()).unwrap_or("hard");
    crate::visual::apply_spp(input, &output, quality, qp, mode).unwrap_or_else(|e| e)
}

fn execute_apply_spp_gemini(args: &HashMap<String, Value>) -> String {
    let input = args.get("input_file").and_then(|v| v.as_str()).unwrap_or("");
    let output_raw = args.get("output_file").and_then(|v| v.as_str()).unwrap_or("");
    let output = ensure_outputs_directory(output_raw);
    let quality = args.get("quality").and_then(|v| v.as_u64()).unwrap_or(3) as u32;
    let qp = args.get("qp").and_then(|v| v.as_i64()).unwrap_or(0) as i32;
    let mode = args.get("mode").and_then(|v| v.as_str()).unwrap_or("hard");
    crate::visual::apply_spp(input, &output, quality, qp, mode).unwrap_or_else(|e| e)
}

fn execute_apply_pp_claude(args: &Value) -> String {
    let input = args["input_file"].as_str().unwrap_or("");
    let output_raw = args["output_file"].as_str().unwrap_or("");
    let output = ensure_outputs_directory(output_raw);
    let subfilters = args.get("subfilters").and_then(|v| v.as_str()).unwrap_or("default");
    crate::visual::apply_pp(input, &output, subfilters).unwrap_or_else(|e| e)
}

fn execute_apply_pp_gemini(args: &HashMap<String, Value>) -> String {
    let input = args.get("input_file").and_then(|v| v.as_str()).unwrap_or("");
    let output_raw = args.get("output_file").and_then(|v| v.as_str()).unwrap_or("");
    let output = ensure_outputs_directory(output_raw);
    let subfilters = args.get("subfilters").and_then(|v| v.as_str()).unwrap_or("default");
    crate::visual::apply_pp(input, &output, subfilters).unwrap_or_else(|e| e)
}

fn execute_apply_mestimate_claude(args: &Value) -> String {
    let input = args["input_file"].as_str().unwrap_or("");
    let output_raw = args["output_file"].as_str().unwrap_or("");
    let output = ensure_outputs_directory(output_raw);
    let method = args.get("method").and_then(|v| v.as_str()).unwrap_or("esa");
    let mb_size = args.get("mb_size").and_then(|v| v.as_u64()).unwrap_or(16) as u32;
    let search_param = args.get("search_param").and_then(|v| v.as_u64()).unwrap_or(7) as u32;
    crate::visual::apply_mestimate(input, &output, method, mb_size, search_param).unwrap_or_else(|e| e)
}

fn execute_apply_mestimate_gemini(args: &HashMap<String, Value>) -> String {
    let input = args.get("input_file").and_then(|v| v.as_str()).unwrap_or("");
    let output_raw = args.get("output_file").and_then(|v| v.as_str()).unwrap_or("");
    let output = ensure_outputs_directory(output_raw);
    let method = args.get("method").and_then(|v| v.as_str()).unwrap_or("esa");
    let mb_size = args.get("mb_size").and_then(|v| v.as_u64()).unwrap_or(16) as u32;
    let search_param = args.get("search_param").and_then(|v| v.as_u64()).unwrap_or(7) as u32;
    crate::visual::apply_mestimate(input, &output, method, mb_size, search_param).unwrap_or_else(|e| e)
}

fn execute_apply_midequalizer_claude(args: &Value) -> String {
    let input = args["input_file"].as_str().unwrap_or("");
    let secondary = args.get("secondary_file").and_then(|v| v.as_str()).unwrap_or("");
    let output_raw = args["output_file"].as_str().unwrap_or("");
    let output = ensure_outputs_directory(output_raw);
    let planes = args.get("planes").and_then(|v| v.as_u64()).unwrap_or(15) as u32;
    crate::visual::apply_midequalizer(input, secondary, &output, planes).unwrap_or_else(|e| e)
}

fn execute_apply_midequalizer_gemini(args: &HashMap<String, Value>) -> String {
    let input = args.get("input_file").and_then(|v| v.as_str()).unwrap_or("");
    let secondary = args.get("secondary_file").and_then(|v| v.as_str()).unwrap_or("");
    let output_raw = args.get("output_file").and_then(|v| v.as_str()).unwrap_or("");
    let output = ensure_outputs_directory(output_raw);
    let planes = args.get("planes").and_then(|v| v.as_u64()).unwrap_or(15) as u32;
    crate::visual::apply_midequalizer(input, secondary, &output, planes).unwrap_or_else(|e| e)
}

fn execute_apply_median_spatial_claude(args: &Value) -> String {
    let input = args["input_file"].as_str().unwrap_or("");
    let output_raw = args["output_file"].as_str().unwrap_or("");
    let output = ensure_outputs_directory(output_raw);
    let radius = args.get("radius").and_then(|v| v.as_u64()).unwrap_or(1) as u32;
    let radius_v = args.get("radiusV").and_then(|v| v.as_u64()).unwrap_or(0) as u32;
    let percentile = args.get("percentile").and_then(|v| v.as_f64()).unwrap_or(0.5);
    let planes = args.get("planes").and_then(|v| v.as_u64()).unwrap_or(15) as u32;
    crate::visual::apply_median_spatial(input, &output, radius, radius_v, percentile, planes).unwrap_or_else(|e| e)
}

fn execute_apply_median_spatial_gemini(args: &HashMap<String, Value>) -> String {
    let input = args.get("input_file").and_then(|v| v.as_str()).unwrap_or("");
    let output_raw = args.get("output_file").and_then(|v| v.as_str()).unwrap_or("");
    let output = ensure_outputs_directory(output_raw);
    let radius = args.get("radius").and_then(|v| v.as_u64()).unwrap_or(1) as u32;
    let radius_v = args.get("radiusV").and_then(|v| v.as_u64()).unwrap_or(0) as u32;
    let percentile = args.get("percentile").and_then(|v| v.as_f64()).unwrap_or(0.5);
    let planes = args.get("planes").and_then(|v| v.as_u64()).unwrap_or(15) as u32;
    crate::visual::apply_median_spatial(input, &output, radius, radius_v, percentile, planes).unwrap_or_else(|e| e)
}

fn execute_apply_acrusher_claude(args: &Value) -> String {
    let input = args["input_file"].as_str().unwrap_or("");
    let output_raw = args["output_file"].as_str().unwrap_or("");
    let output = ensure_outputs_directory(output_raw);
    let level_in = args.get("level_in").and_then(|v| v.as_f64()).unwrap_or(1.0);
    let level_out = args.get("level_out").and_then(|v| v.as_f64()).unwrap_or(1.0);
    let bits = args.get("bits").and_then(|v| v.as_f64()).unwrap_or(8.0);
    let mix = args.get("mix").and_then(|v| v.as_f64()).unwrap_or(0.5);
    let mode = args.get("mode").and_then(|v| v.as_str()).unwrap_or("log");
    let dc = args.get("dc").and_then(|v| v.as_f64()).unwrap_or(1.0);
    let aa = args.get("aa").and_then(|v| v.as_f64()).unwrap_or(0.5);
    let samples = args.get("samples").and_then(|v| v.as_f64()).unwrap_or(1.0);
    let lfo = args.get("lfo").and_then(|v| v.as_bool()).unwrap_or(false);
    let lforange = args.get("lforange").and_then(|v| v.as_f64()).unwrap_or(20.0);
    let lforate = args.get("lforate").and_then(|v| v.as_f64()).unwrap_or(0.3);
    crate::audio::apply_acrusher(input, &output, level_in, level_out, bits, mix, mode, dc, aa, samples, lfo, lforange, lforate).unwrap_or_else(|e| e)
}

fn execute_apply_acrusher_gemini(args: &HashMap<String, Value>) -> String {
    let input = args.get("input_file").and_then(|v| v.as_str()).unwrap_or("");
    let output_raw = args.get("output_file").and_then(|v| v.as_str()).unwrap_or("");
    let output = ensure_outputs_directory(output_raw);
    let level_in = args.get("level_in").and_then(|v| v.as_f64()).unwrap_or(1.0);
    let level_out = args.get("level_out").and_then(|v| v.as_f64()).unwrap_or(1.0);
    let bits = args.get("bits").and_then(|v| v.as_f64()).unwrap_or(8.0);
    let mix = args.get("mix").and_then(|v| v.as_f64()).unwrap_or(0.5);
    let mode = args.get("mode").and_then(|v| v.as_str()).unwrap_or("log");
    let dc = args.get("dc").and_then(|v| v.as_f64()).unwrap_or(1.0);
    let aa = args.get("aa").and_then(|v| v.as_f64()).unwrap_or(0.5);
    let samples = args.get("samples").and_then(|v| v.as_f64()).unwrap_or(1.0);
    let lfo = args.get("lfo").and_then(|v| v.as_bool()).unwrap_or(false);
    let lforange = args.get("lforange").and_then(|v| v.as_f64()).unwrap_or(20.0);
    let lforate = args.get("lforate").and_then(|v| v.as_f64()).unwrap_or(0.3);
    crate::audio::apply_acrusher(input, &output, level_in, level_out, bits, mix, mode, dc, aa, samples, lfo, lforange, lforate).unwrap_or_else(|e| e)
}

fn execute_apply_atempo_claude(args: &Value) -> String {
    let input = args["input_file"].as_str().unwrap_or("");
    let output_raw = args["output_file"].as_str().unwrap_or("");
    let output = ensure_outputs_directory(output_raw);
    let tempo = args.get("tempo").and_then(|v| v.as_f64()).unwrap_or(1.0);
    crate::audio::apply_atempo(input, &output, tempo).unwrap_or_else(|e| e)
}

fn execute_apply_atempo_gemini(args: &HashMap<String, Value>) -> String {
    let input = args.get("input_file").and_then(|v| v.as_str()).unwrap_or("");
    let output_raw = args.get("output_file").and_then(|v| v.as_str()).unwrap_or("");
    let output = ensure_outputs_directory(output_raw);
    let tempo = args.get("tempo").and_then(|v| v.as_f64()).unwrap_or(1.0);
    crate::audio::apply_atempo(input, &output, tempo).unwrap_or_else(|e| e)
}

fn execute_apply_asetnsamples_claude(args: &Value) -> String {
    let input = args["input_file"].as_str().unwrap_or("");
    let output_raw = args["output_file"].as_str().unwrap_or("");
    let output = ensure_outputs_directory(output_raw);
    let nb_samples = args.get("nb_samples").and_then(|v| v.as_u64()).unwrap_or(1024) as u32;
    let pad = args.get("pad").and_then(|v| v.as_bool()).unwrap_or(true);
    crate::audio::apply_asetnsamples(input, &output, nb_samples, pad).unwrap_or_else(|e| e)
}

fn execute_apply_asetnsamples_gemini(args: &HashMap<String, Value>) -> String {
    let input = args.get("input_file").and_then(|v| v.as_str()).unwrap_or("");
    let output_raw = args.get("output_file").and_then(|v| v.as_str()).unwrap_or("");
    let output = ensure_outputs_directory(output_raw);
    let nb_samples = args.get("nb_samples").and_then(|v| v.as_u64()).unwrap_or(1024) as u32;
    let pad = args.get("pad").and_then(|v| v.as_bool()).unwrap_or(true);
    crate::audio::apply_asetnsamples(input, &output, nb_samples, pad).unwrap_or_else(|e| e)
}

fn execute_apply_apad_claude(args: &Value) -> String {
    let input = args["input_file"].as_str().unwrap_or("");
    let output_raw = args["output_file"].as_str().unwrap_or("");
    let output = ensure_outputs_directory(output_raw);
    let packet_size = args.get("packet_size").and_then(|v| v.as_u64()).unwrap_or(4096) as u32;
    let pad_len = args.get("pad_len").and_then(|v| v.as_i64()).unwrap_or(0);
    let whole_len = args.get("whole_len").and_then(|v| v.as_i64()).unwrap_or(0);
    let pad_dur = args.get("pad_dur").and_then(|v| v.as_f64()).unwrap_or(0.0);
    let whole_dur = args.get("whole_dur").and_then(|v| v.as_f64()).unwrap_or(0.0);
    crate::audio::apply_apad(input, &output, packet_size, pad_len, whole_len, pad_dur, whole_dur).unwrap_or_else(|e| e)
}

fn execute_apply_apad_gemini(args: &HashMap<String, Value>) -> String {
    let input = args.get("input_file").and_then(|v| v.as_str()).unwrap_or("");
    let output_raw = args.get("output_file").and_then(|v| v.as_str()).unwrap_or("");
    let output = ensure_outputs_directory(output_raw);
    let packet_size = args.get("packet_size").and_then(|v| v.as_u64()).unwrap_or(4096) as u32;
    let pad_len = args.get("pad_len").and_then(|v| v.as_i64()).unwrap_or(0);
    let whole_len = args.get("whole_len").and_then(|v| v.as_i64()).unwrap_or(0);
    let pad_dur = args.get("pad_dur").and_then(|v| v.as_f64()).unwrap_or(0.0);
    let whole_dur = args.get("whole_dur").and_then(|v| v.as_f64()).unwrap_or(0.0);
    crate::audio::apply_apad(input, &output, packet_size, pad_len, whole_len, pad_dur, whole_dur).unwrap_or_else(|e| e)
}

fn execute_apply_asubcut_claude(args: &Value) -> String {
    let input = args["input_file"].as_str().unwrap_or("");
    let output_raw = args["output_file"].as_str().unwrap_or("");
    let output = ensure_outputs_directory(output_raw);
    let cutoff = args.get("cutoff").and_then(|v| v.as_f64()).unwrap_or(20.0);
    let order = args.get("order").and_then(|v| v.as_u64()).unwrap_or(10) as u32;
    let level = args.get("level").and_then(|v| v.as_f64()).unwrap_or(1.0);
    crate::audio::apply_asubcut(input, &output, cutoff, order, level).unwrap_or_else(|e| e)
}

fn execute_apply_asubcut_gemini(args: &HashMap<String, Value>) -> String {
    let input = args.get("input_file").and_then(|v| v.as_str()).unwrap_or("");
    let output_raw = args.get("output_file").and_then(|v| v.as_str()).unwrap_or("");
    let output = ensure_outputs_directory(output_raw);
    let cutoff = args.get("cutoff").and_then(|v| v.as_f64()).unwrap_or(20.0);
    let order = args.get("order").and_then(|v| v.as_u64()).unwrap_or(10) as u32;
    let level = args.get("level").and_then(|v| v.as_f64()).unwrap_or(1.0);
    crate::audio::apply_asubcut(input, &output, cutoff, order, level).unwrap_or_else(|e| e)
}

fn execute_apply_asupercut_claude(args: &Value) -> String {
    let input = args["input_file"].as_str().unwrap_or("");
    let output_raw = args["output_file"].as_str().unwrap_or("");
    let output = ensure_outputs_directory(output_raw);
    let cutoff = args.get("cutoff").and_then(|v| v.as_f64()).unwrap_or(20000.0);
    let order = args.get("order").and_then(|v| v.as_u64()).unwrap_or(10) as u32;
    let level = args.get("level").and_then(|v| v.as_f64()).unwrap_or(1.0);
    crate::audio::apply_asupercut(input, &output, cutoff, order, level).unwrap_or_else(|e| e)
}

fn execute_apply_asupercut_gemini(args: &HashMap<String, Value>) -> String {
    let input = args.get("input_file").and_then(|v| v.as_str()).unwrap_or("");
    let output_raw = args.get("output_file").and_then(|v| v.as_str()).unwrap_or("");
    let output = ensure_outputs_directory(output_raw);
    let cutoff = args.get("cutoff").and_then(|v| v.as_f64()).unwrap_or(20000.0);
    let order = args.get("order").and_then(|v| v.as_u64()).unwrap_or(10) as u32;
    let level = args.get("level").and_then(|v| v.as_f64()).unwrap_or(1.0);
    crate::audio::apply_asupercut(input, &output, cutoff, order, level).unwrap_or_else(|e| e)
}

// PHASE I BATCH 7 EXECUTORS

fn execute_apply_xfade_transition_claude(args: &Value) -> String {
    let input = args["input_file"].as_str().unwrap_or("");
    let secondary = args["secondary_file"].as_str().unwrap_or("");
    let output_raw = args["output_file"].as_str().unwrap_or("");
    let output = ensure_outputs_directory(output_raw);
    let transition = args.get("transition").and_then(|v| v.as_str()).unwrap_or("fade");
    let duration = args.get("duration").and_then(|v| v.as_f64()).unwrap_or(1.0);
    let offset = args.get("offset").and_then(|v| v.as_f64()).unwrap_or(0.0);
    crate::visual::apply_xfade_transition(input, secondary, &output, transition, duration, offset).unwrap_or_else(|e| e)
}

fn execute_apply_xfade_transition_gemini(args: &HashMap<String, Value>) -> String {
    let input = args.get("input_file").and_then(|v| v.as_str()).unwrap_or("");
    let secondary = args.get("secondary_file").and_then(|v| v.as_str()).unwrap_or("");
    let output_raw = args.get("output_file").and_then(|v| v.as_str()).unwrap_or("");
    let output = ensure_outputs_directory(output_raw);
    let transition = args.get("transition").and_then(|v| v.as_str()).unwrap_or("fade");
    let duration = args.get("duration").and_then(|v| v.as_f64()).unwrap_or(1.0);
    let offset = args.get("offset").and_then(|v| v.as_f64()).unwrap_or(0.0);
    crate::visual::apply_xfade_transition(input, secondary, &output, transition, duration, offset).unwrap_or_else(|e| e)
}

fn execute_apply_color_key_claude(args: &Value) -> String {
    let input = args["input_file"].as_str().unwrap_or("");
    let output_raw = args["output_file"].as_str().unwrap_or("");
    let output = ensure_outputs_directory(output_raw);
    let color = args.get("color").and_then(|v| v.as_str()).unwrap_or("0x00FF00");
    let similarity = args.get("similarity").and_then(|v| v.as_f64()).unwrap_or(0.1);
    let blend = args.get("blend").and_then(|v| v.as_f64()).unwrap_or(0.0);
    crate::visual::apply_color_key(input, &output, color, similarity, blend).unwrap_or_else(|e| e)
}

fn execute_apply_color_key_gemini(args: &HashMap<String, Value>) -> String {
    let input = args.get("input_file").and_then(|v| v.as_str()).unwrap_or("");
    let output_raw = args.get("output_file").and_then(|v| v.as_str()).unwrap_or("");
    let output = ensure_outputs_directory(output_raw);
    let color = args.get("color").and_then(|v| v.as_str()).unwrap_or("0x00FF00");
    let similarity = args.get("similarity").and_then(|v| v.as_f64()).unwrap_or(0.1);
    let blend = args.get("blend").and_then(|v| v.as_f64()).unwrap_or(0.0);
    crate::visual::apply_color_key(input, &output, color, similarity, blend).unwrap_or_else(|e| e)
}

fn execute_apply_monochrome_claude(args: &Value) -> String {
    let input = args["input_file"].as_str().unwrap_or("");
    let output_raw = args["output_file"].as_str().unwrap_or("");
    let output = ensure_outputs_directory(output_raw);
    let cb = args.get("cb").and_then(|v| v.as_f64()).unwrap_or(0.0);
    let cr = args.get("cr").and_then(|v| v.as_f64()).unwrap_or(0.0);
    let size = args.get("size").and_then(|v| v.as_f64()).unwrap_or(1.0);
    crate::visual::apply_monochrome(input, &output, cb, cr, size).unwrap_or_else(|e| e)
}

fn execute_apply_monochrome_gemini(args: &HashMap<String, Value>) -> String {
    let input = args.get("input_file").and_then(|v| v.as_str()).unwrap_or("");
    let output_raw = args.get("output_file").and_then(|v| v.as_str()).unwrap_or("");
    let output = ensure_outputs_directory(output_raw);
    let cb = args.get("cb").and_then(|v| v.as_f64()).unwrap_or(0.0);
    let cr = args.get("cr").and_then(|v| v.as_f64()).unwrap_or(0.0);
    let size = args.get("size").and_then(|v| v.as_f64()).unwrap_or(1.0);
    crate::visual::apply_monochrome(input, &output, cb, cr, size).unwrap_or_else(|e| e)
}

fn execute_apply_maskedmerge_claude(args: &Value) -> String {
    let input = args["input_file"].as_str().unwrap_or("");
    let overlay = args["overlay_file"].as_str().unwrap_or("");
    let mask = args["mask_file"].as_str().unwrap_or("");
    let output_raw = args["output_file"].as_str().unwrap_or("");
    let output = ensure_outputs_directory(output_raw);
    let planes = args.get("planes").and_then(|v| v.as_u64()).unwrap_or(15) as u32;
    crate::visual::apply_maskedmerge(input, overlay, mask, &output, planes).unwrap_or_else(|e| e)
}

fn execute_apply_maskedmerge_gemini(args: &HashMap<String, Value>) -> String {
    let input = args.get("input_file").and_then(|v| v.as_str()).unwrap_or("");
    let overlay = args.get("overlay_file").and_then(|v| v.as_str()).unwrap_or("");
    let mask = args.get("mask_file").and_then(|v| v.as_str()).unwrap_or("");
    let output_raw = args.get("output_file").and_then(|v| v.as_str()).unwrap_or("");
    let output = ensure_outputs_directory(output_raw);
    let planes = args.get("planes").and_then(|v| v.as_u64()).unwrap_or(15) as u32;
    crate::visual::apply_maskedmerge(input, overlay, mask, &output, planes).unwrap_or_else(|e| e)
}

fn execute_convert_360_video_claude(args: &Value) -> String {
    let input = args["input_file"].as_str().unwrap_or("");
    let output_raw = args["output_file"].as_str().unwrap_or("");
    let output = ensure_outputs_directory(output_raw);
    let input_fmt = args.get("input_fmt").and_then(|v| v.as_str()).unwrap_or("equirect");
    let output_fmt = args.get("output_fmt").and_then(|v| v.as_str()).unwrap_or("flat");
    let width = args.get("width").and_then(|v| v.as_u64()).unwrap_or(1920) as u32;
    let height = args.get("height").and_then(|v| v.as_u64()).unwrap_or(1080) as u32;
    let h_fov = args.get("h_fov").and_then(|v| v.as_f64()).unwrap_or(90.0);
    let v_fov = args.get("v_fov").and_then(|v| v.as_f64()).unwrap_or(90.0);
    crate::visual::convert_360_video(input, &output, input_fmt, output_fmt, width, height, h_fov, v_fov).unwrap_or_else(|e| e)
}

fn execute_convert_360_video_gemini(args: &HashMap<String, Value>) -> String {
    let input = args.get("input_file").and_then(|v| v.as_str()).unwrap_or("");
    let output_raw = args.get("output_file").and_then(|v| v.as_str()).unwrap_or("");
    let output = ensure_outputs_directory(output_raw);
    let input_fmt = args.get("input_fmt").and_then(|v| v.as_str()).unwrap_or("equirect");
    let output_fmt = args.get("output_fmt").and_then(|v| v.as_str()).unwrap_or("flat");
    let width = args.get("width").and_then(|v| v.as_u64()).unwrap_or(1920) as u32;
    let height = args.get("height").and_then(|v| v.as_u64()).unwrap_or(1080) as u32;
    let h_fov = args.get("h_fov").and_then(|v| v.as_f64()).unwrap_or(90.0);
    let v_fov = args.get("v_fov").and_then(|v| v.as_f64()).unwrap_or(90.0);
    crate::visual::convert_360_video(input, &output, input_fmt, output_fmt, width, height, h_fov, v_fov).unwrap_or_else(|e| e)
}

fn execute_fix_banding_claude(args: &Value) -> String {
    let input = args["input_file"].as_str().unwrap_or("");
    let output_raw = args["output_file"].as_str().unwrap_or("");
    let output = ensure_outputs_directory(output_raw);
    let strength = args.get("strength").and_then(|v| v.as_f64()).unwrap_or(1.2);
    let radius = args.get("radius").and_then(|v| v.as_u64()).unwrap_or(16) as u32;
    crate::visual::fix_banding(input, &output, strength, radius).unwrap_or_else(|e| e)
}

fn execute_fix_banding_gemini(args: &HashMap<String, Value>) -> String {
    let input = args.get("input_file").and_then(|v| v.as_str()).unwrap_or("");
    let output_raw = args.get("output_file").and_then(|v| v.as_str()).unwrap_or("");
    let output = ensure_outputs_directory(output_raw);
    let strength = args.get("strength").and_then(|v| v.as_f64()).unwrap_or(1.2);
    let radius = args.get("radius").and_then(|v| v.as_u64()).unwrap_or(16) as u32;
    crate::visual::fix_banding(input, &output, strength, radius).unwrap_or_else(|e| e)
}

fn execute_apply_greyedge_claude(args: &Value) -> String {
    let input = args["input_file"].as_str().unwrap_or("");
    let output_raw = args["output_file"].as_str().unwrap_or("");
    let output = ensure_outputs_directory(output_raw);
    let difford = args.get("difford").and_then(|v| v.as_u64()).unwrap_or(1) as u32;
    let minknorm = args.get("minknorm").and_then(|v| v.as_f64()).unwrap_or(1.0);
    let sigma = args.get("sigma").and_then(|v| v.as_f64()).unwrap_or(1.0);
    crate::visual::apply_greyedge(input, &output, difford, minknorm, sigma).unwrap_or_else(|e| e)
}

fn execute_apply_greyedge_gemini(args: &HashMap<String, Value>) -> String {
    let input = args.get("input_file").and_then(|v| v.as_str()).unwrap_or("");
    let output_raw = args.get("output_file").and_then(|v| v.as_str()).unwrap_or("");
    let output = ensure_outputs_directory(output_raw);
    let difford = args.get("difford").and_then(|v| v.as_u64()).unwrap_or(1) as u32;
    let minknorm = args.get("minknorm").and_then(|v| v.as_f64()).unwrap_or(1.0);
    let sigma = args.get("sigma").and_then(|v| v.as_f64()).unwrap_or(1.0);
    crate::visual::apply_greyedge(input, &output, difford, minknorm, sigma).unwrap_or_else(|e| e)
}

fn execute_apply_fade_video_claude(args: &Value) -> String {
    let input = args["input_file"].as_str().unwrap_or("");
    let output_raw = args["output_file"].as_str().unwrap_or("");
    let output = ensure_outputs_directory(output_raw);
    let fade_type = args.get("fade_type").and_then(|v| v.as_str()).unwrap_or("in");
    let start_time = args.get("start_time").and_then(|v| v.as_f64()).unwrap_or(0.0);
    let duration = args.get("duration").and_then(|v| v.as_f64()).unwrap_or(1.0);
    let color = args.get("color").and_then(|v| v.as_str()).unwrap_or("black");
    crate::visual::apply_fade_video(input, &output, fade_type, start_time, duration, color).unwrap_or_else(|e| e)
}

fn execute_apply_fade_video_gemini(args: &HashMap<String, Value>) -> String {
    let input = args.get("input_file").and_then(|v| v.as_str()).unwrap_or("");
    let output_raw = args.get("output_file").and_then(|v| v.as_str()).unwrap_or("");
    let output = ensure_outputs_directory(output_raw);
    let fade_type = args.get("fade_type").and_then(|v| v.as_str()).unwrap_or("in");
    let start_time = args.get("start_time").and_then(|v| v.as_f64()).unwrap_or(0.0);
    let duration = args.get("duration").and_then(|v| v.as_f64()).unwrap_or(1.0);
    let color = args.get("color").and_then(|v| v.as_str()).unwrap_or("black");
    crate::visual::apply_fade_video(input, &output, fade_type, start_time, duration, color).unwrap_or_else(|e| e)
}

fn execute_normalize_loudness_claude(args: &Value) -> String {
    let input = args["input_file"].as_str().unwrap_or("");
    let output_raw = args["output_file"].as_str().unwrap_or("");
    let output = ensure_outputs_directory(output_raw);
    let i = args.get("i").and_then(|v| v.as_f64()).unwrap_or(-23.0);
    let lra = args.get("lra").and_then(|v| v.as_f64()).unwrap_or(7.0);
    let tp = args.get("tp").and_then(|v| v.as_f64()).unwrap_or(-2.0);
    crate::audio::normalize_loudness(input, &output, i, lra, tp).unwrap_or_else(|e| e)
}

fn execute_normalize_loudness_gemini(args: &HashMap<String, Value>) -> String {
    let input = args.get("input_file").and_then(|v| v.as_str()).unwrap_or("");
    let output_raw = args.get("output_file").and_then(|v| v.as_str()).unwrap_or("");
    let output = ensure_outputs_directory(output_raw);
    let i = args.get("i").and_then(|v| v.as_f64()).unwrap_or(-23.0);
    let lra = args.get("lra").and_then(|v| v.as_f64()).unwrap_or(7.0);
    let tp = args.get("tp").and_then(|v| v.as_f64()).unwrap_or(-2.0);
    crate::audio::normalize_loudness(input, &output, i, lra, tp).unwrap_or_else(|e| e)
}

fn execute_dynamic_audio_normalize_claude(args: &Value) -> String {
    let input = args["input_file"].as_str().unwrap_or("");
    let output_raw = args["output_file"].as_str().unwrap_or("");
    let output = ensure_outputs_directory(output_raw);
    let frame_len = args.get("frame_len").and_then(|v| v.as_u64()).unwrap_or(500) as u32;
    let gausssize = args.get("gausssize").and_then(|v| v.as_u64()).unwrap_or(31) as u32;
    let peak = args.get("peak").and_then(|v| v.as_f64()).unwrap_or(0.95);
    let max_gain = args.get("max_gain").and_then(|v| v.as_f64()).unwrap_or(10.0);
    let rms = args.get("rms").and_then(|v| v.as_f64()).unwrap_or(0.0);
    let coupling = args.get("coupling").and_then(|v| v.as_bool()).unwrap_or(true);
    crate::audio::dynamic_audio_normalize(input, &output, frame_len, gausssize, peak, max_gain, rms, coupling).unwrap_or_else(|e| e)
}

fn execute_dynamic_audio_normalize_gemini(args: &HashMap<String, Value>) -> String {
    let input = args.get("input_file").and_then(|v| v.as_str()).unwrap_or("");
    let output_raw = args.get("output_file").and_then(|v| v.as_str()).unwrap_or("");
    let output = ensure_outputs_directory(output_raw);
    let frame_len = args.get("frame_len").and_then(|v| v.as_u64()).unwrap_or(500) as u32;
    let gausssize = args.get("gausssize").and_then(|v| v.as_u64()).unwrap_or(31) as u32;
    let peak = args.get("peak").and_then(|v| v.as_f64()).unwrap_or(0.95);
    let max_gain = args.get("max_gain").and_then(|v| v.as_f64()).unwrap_or(10.0);
    let rms = args.get("rms").and_then(|v| v.as_f64()).unwrap_or(0.0);
    let coupling = args.get("coupling").and_then(|v| v.as_bool()).unwrap_or(true);
    crate::audio::dynamic_audio_normalize(input, &output, frame_len, gausssize, peak, max_gain, rms, coupling).unwrap_or_else(|e| e)
}

fn execute_resample_audio_claude(args: &Value) -> String {
    let input = args["input_file"].as_str().unwrap_or("");
    let output_raw = args["output_file"].as_str().unwrap_or("");
    let output = ensure_outputs_directory(output_raw);
    let sample_rate = args.get("sample_rate").and_then(|v| v.as_u64()).unwrap_or(44100) as u32;
    let resampler = args.get("resampler").and_then(|v| v.as_str()).unwrap_or("swr");
    crate::audio::resample_audio(input, &output, sample_rate, resampler).unwrap_or_else(|e| e)
}

fn execute_resample_audio_gemini(args: &HashMap<String, Value>) -> String {
    let input = args.get("input_file").and_then(|v| v.as_str()).unwrap_or("");
    let output_raw = args.get("output_file").and_then(|v| v.as_str()).unwrap_or("");
    let output = ensure_outputs_directory(output_raw);
    let sample_rate = args.get("sample_rate").and_then(|v| v.as_u64()).unwrap_or(44100) as u32;
    let resampler = args.get("resampler").and_then(|v| v.as_str()).unwrap_or("swr");
    crate::audio::resample_audio(input, &output, sample_rate, resampler).unwrap_or_else(|e| e)
}

fn execute_trim_audio_claude(args: &Value) -> String {
    let input = args["input_file"].as_str().unwrap_or("");
    let output_raw = args["output_file"].as_str().unwrap_or("");
    let output = ensure_outputs_directory(output_raw);
    let start = args.get("start").and_then(|v| v.as_f64()).unwrap_or(0.0);
    let end = args.get("end").and_then(|v| v.as_f64()).unwrap_or(0.0);
    let duration = args.get("duration").and_then(|v| v.as_f64()).unwrap_or(0.0);
    crate::audio::trim_audio(input, &output, start, end, duration).unwrap_or_else(|e| e)
}

fn execute_trim_audio_gemini(args: &HashMap<String, Value>) -> String {
    let input = args.get("input_file").and_then(|v| v.as_str()).unwrap_or("");
    let output_raw = args.get("output_file").and_then(|v| v.as_str()).unwrap_or("");
    let output = ensure_outputs_directory(output_raw);
    let start = args.get("start").and_then(|v| v.as_f64()).unwrap_or(0.0);
    let end = args.get("end").and_then(|v| v.as_f64()).unwrap_or(0.0);
    let duration = args.get("duration").and_then(|v| v.as_f64()).unwrap_or(0.0);
    crate::audio::trim_audio(input, &output, start, end, duration).unwrap_or_else(|e| e)
}

fn execute_apply_crystalizer_claude(args: &Value) -> String {
    let input = args["input_file"].as_str().unwrap_or("");
    let output_raw = args["output_file"].as_str().unwrap_or("");
    let output = ensure_outputs_directory(output_raw);
    let i = args.get("i").and_then(|v| v.as_f64()).unwrap_or(2.0);
    let clip = args.get("clip").and_then(|v| v.as_bool()).unwrap_or(false);
    crate::audio::apply_crystalizer(input, &output, i, clip).unwrap_or_else(|e| e)
}

fn execute_apply_crystalizer_gemini(args: &HashMap<String, Value>) -> String {
    let input = args.get("input_file").and_then(|v| v.as_str()).unwrap_or("");
    let output_raw = args.get("output_file").and_then(|v| v.as_str()).unwrap_or("");
    let output = ensure_outputs_directory(output_raw);
    let i = args.get("i").and_then(|v| v.as_f64()).unwrap_or(2.0);
    let clip = args.get("clip").and_then(|v| v.as_bool()).unwrap_or(false);
    crate::audio::apply_crystalizer(input, &output, i, clip).unwrap_or_else(|e| e)
}

fn execute_multiband_compress_claude(args: &Value) -> String {
    let input = args["input_file"].as_str().unwrap_or("");
    let output_raw = args["output_file"].as_str().unwrap_or("");
    let output = ensure_outputs_directory(output_raw);
    let params = args.get("params").and_then(|v| v.as_str()).unwrap_or("");
    crate::audio::multiband_compress(input, &output, params).unwrap_or_else(|e| e)
}

fn execute_multiband_compress_gemini(args: &HashMap<String, Value>) -> String {
    let input = args.get("input_file").and_then(|v| v.as_str()).unwrap_or("");
    let output_raw = args.get("output_file").and_then(|v| v.as_str()).unwrap_or("");
    let output = ensure_outputs_directory(output_raw);
    let params = args.get("params").and_then(|v| v.as_str()).unwrap_or("");
    crate::audio::multiband_compress(input, &output, params).unwrap_or_else(|e| e)
}

fn execute_apply_super_equalizer_claude(args: &Value) -> String {
    let input = args["input_file"].as_str().unwrap_or("");
    let output_raw = args["output_file"].as_str().unwrap_or("");
    let output = ensure_outputs_directory(output_raw);
    let bands = args.get("bands").and_then(|v| v.as_str()).unwrap_or("");
    crate::audio::apply_super_equalizer(input, &output, bands).unwrap_or_else(|e| e)
}

fn execute_apply_super_equalizer_gemini(args: &HashMap<String, Value>) -> String {
    let input = args.get("input_file").and_then(|v| v.as_str()).unwrap_or("");
    let output_raw = args.get("output_file").and_then(|v| v.as_str()).unwrap_or("");
    let output = ensure_outputs_directory(output_raw);
    let bands = args.get("bands").and_then(|v| v.as_str()).unwrap_or("");
    crate::audio::apply_super_equalizer(input, &output, bands).unwrap_or_else(|e| e)
}

// PHASE I BATCH 8 EXECUTORS

fn execute_extract_alpha_channel_claude(args: &Value) -> String {
    let input = args["input_file"].as_str().unwrap_or("");
    let output_raw = args["output_file"].as_str().unwrap_or("");
    let output = ensure_outputs_directory(output_raw);
    crate::visual::extract_alpha_channel(input, &output).unwrap_or_else(|e| e)
}

fn execute_extract_alpha_channel_gemini(args: &HashMap<String, Value>) -> String {
    let input = args.get("input_file").and_then(|v| v.as_str()).unwrap_or("");
    let output_raw = args.get("output_file").and_then(|v| v.as_str()).unwrap_or("");
    let output = ensure_outputs_directory(output_raw);
    crate::visual::extract_alpha_channel(input, &output).unwrap_or_else(|e| e)
}

fn execute_merge_alpha_channel_claude(args: &Value) -> String {
    let input = args["input_file"].as_str().unwrap_or("");
    let alpha = args["alpha_file"].as_str().unwrap_or("");
    let output_raw = args["output_file"].as_str().unwrap_or("");
    let output = ensure_outputs_directory(output_raw);
    crate::visual::merge_alpha_channel(input, alpha, &output).unwrap_or_else(|e| e)
}

fn execute_merge_alpha_channel_gemini(args: &HashMap<String, Value>) -> String {
    let input = args.get("input_file").and_then(|v| v.as_str()).unwrap_or("");
    let alpha = args.get("alpha_file").and_then(|v| v.as_str()).unwrap_or("");
    let output_raw = args.get("output_file").and_then(|v| v.as_str()).unwrap_or("");
    let output = ensure_outputs_directory(output_raw);
    crate::visual::merge_alpha_channel(input, alpha, &output).unwrap_or_else(|e| e)
}

fn execute_apply_framestep_claude(args: &Value) -> String {
    let input = args["input_file"].as_str().unwrap_or("");
    let output_raw = args["output_file"].as_str().unwrap_or("");
    let output = ensure_outputs_directory(output_raw);
    let step = args.get("step").and_then(|v| v.as_u64()).unwrap_or(1) as u32;
    crate::visual::apply_framestep(input, &output, step).unwrap_or_else(|e| e)
}

fn execute_apply_framestep_gemini(args: &HashMap<String, Value>) -> String {
    let input = args.get("input_file").and_then(|v| v.as_str()).unwrap_or("");
    let output_raw = args.get("output_file").and_then(|v| v.as_str()).unwrap_or("");
    let output = ensure_outputs_directory(output_raw);
    let step = args.get("step").and_then(|v| v.as_u64()).unwrap_or(1) as u32;
    crate::visual::apply_framestep(input, &output, step).unwrap_or_else(|e| e)
}

fn execute_apply_swaprect_claude(args: &Value) -> String {
    let input = args["input_file"].as_str().unwrap_or("");
    let output_raw = args["output_file"].as_str().unwrap_or("");
    let output = ensure_outputs_directory(output_raw);
    let x1 = args.get("x1").and_then(|v| v.as_u64()).unwrap_or(0) as u32;
    let y1 = args.get("y1").and_then(|v| v.as_u64()).unwrap_or(0) as u32;
    let x2 = args.get("x2").and_then(|v| v.as_u64()).unwrap_or(0) as u32;
    let y2 = args.get("y2").and_then(|v| v.as_u64()).unwrap_or(0) as u32;
    let w = args.get("w").and_then(|v| v.as_u64()).unwrap_or(100) as u32;
    let h = args.get("h").and_then(|v| v.as_u64()).unwrap_or(100) as u32;
    crate::visual::apply_swaprect(input, &output, x1, y1, x2, y2, w, h).unwrap_or_else(|e| e)
}

fn execute_apply_swaprect_gemini(args: &HashMap<String, Value>) -> String {
    let input = args.get("input_file").and_then(|v| v.as_str()).unwrap_or("");
    let output_raw = args.get("output_file").and_then(|v| v.as_str()).unwrap_or("");
    let output = ensure_outputs_directory(output_raw);
    let x1 = args.get("x1").and_then(|v| v.as_u64()).unwrap_or(0) as u32;
    let y1 = args.get("y1").and_then(|v| v.as_u64()).unwrap_or(0) as u32;
    let x2 = args.get("x2").and_then(|v| v.as_u64()).unwrap_or(0) as u32;
    let y2 = args.get("y2").and_then(|v| v.as_u64()).unwrap_or(0) as u32;
    let w = args.get("w").and_then(|v| v.as_u64()).unwrap_or(100) as u32;
    let h = args.get("h").and_then(|v| v.as_u64()).unwrap_or(100) as u32;
    crate::visual::apply_swaprect(input, &output, x1, y1, x2, y2, w, h).unwrap_or_else(|e| e)
}

fn execute_apply_fillborders_claude(args: &Value) -> String {
    let input = args["input_file"].as_str().unwrap_or("");
    let output_raw = args["output_file"].as_str().unwrap_or("");
    let output = ensure_outputs_directory(output_raw);
    let left = args.get("left").and_then(|v| v.as_u64()).unwrap_or(0) as u32;
    let right = args.get("right").and_then(|v| v.as_u64()).unwrap_or(0) as u32;
    let top = args.get("top").and_then(|v| v.as_u64()).unwrap_or(0) as u32;
    let bottom = args.get("bottom").and_then(|v| v.as_u64()).unwrap_or(0) as u32;
    let mode = args.get("mode").and_then(|v| v.as_str()).unwrap_or("smear");
    let color = args.get("color").and_then(|v| v.as_str()).unwrap_or("black");
    crate::visual::apply_fillborders(input, &output, left, right, top, bottom, mode, color).unwrap_or_else(|e| e)
}

fn execute_apply_fillborders_gemini(args: &HashMap<String, Value>) -> String {
    let input = args.get("input_file").and_then(|v| v.as_str()).unwrap_or("");
    let output_raw = args.get("output_file").and_then(|v| v.as_str()).unwrap_or("");
    let output = ensure_outputs_directory(output_raw);
    let left = args.get("left").and_then(|v| v.as_u64()).unwrap_or(0) as u32;
    let right = args.get("right").and_then(|v| v.as_u64()).unwrap_or(0) as u32;
    let top = args.get("top").and_then(|v| v.as_u64()).unwrap_or(0) as u32;
    let bottom = args.get("bottom").and_then(|v| v.as_u64()).unwrap_or(0) as u32;
    let mode = args.get("mode").and_then(|v| v.as_str()).unwrap_or("smear");
    let color = args.get("color").and_then(|v| v.as_str()).unwrap_or("black");
    crate::visual::apply_fillborders(input, &output, left, right, top, bottom, mode, color).unwrap_or_else(|e| e)
}

fn execute_apply_chromanr_claude(args: &Value) -> String {
    let input = args["input_file"].as_str().unwrap_or("");
    let output_raw = args["output_file"].as_str().unwrap_or("");
    let output = ensure_outputs_directory(output_raw);
    let thres = args.get("thres").and_then(|v| v.as_f64()).unwrap_or(30.0);
    let sizew = args.get("sizew").and_then(|v| v.as_u64()).unwrap_or(5) as u32;
    let sizeh = args.get("sizeh").and_then(|v| v.as_u64()).unwrap_or(5) as u32;
    let stepw = args.get("stepw").and_then(|v| v.as_u64()).unwrap_or(1) as u32;
    let steph = args.get("steph").and_then(|v| v.as_u64()).unwrap_or(1) as u32;
    crate::visual::apply_chromanr(input, &output, thres, sizew, sizeh, stepw, steph).unwrap_or_else(|e| e)
}

fn execute_apply_chromanr_gemini(args: &HashMap<String, Value>) -> String {
    let input = args.get("input_file").and_then(|v| v.as_str()).unwrap_or("");
    let output_raw = args.get("output_file").and_then(|v| v.as_str()).unwrap_or("");
    let output = ensure_outputs_directory(output_raw);
    let thres = args.get("thres").and_then(|v| v.as_f64()).unwrap_or(30.0);
    let sizew = args.get("sizew").and_then(|v| v.as_u64()).unwrap_or(5) as u32;
    let sizeh = args.get("sizeh").and_then(|v| v.as_u64()).unwrap_or(5) as u32;
    let stepw = args.get("stepw").and_then(|v| v.as_u64()).unwrap_or(1) as u32;
    let steph = args.get("steph").and_then(|v| v.as_u64()).unwrap_or(1) as u32;
    crate::visual::apply_chromanr(input, &output, thres, sizew, sizeh, stepw, steph).unwrap_or_else(|e| e)
}

fn execute_apply_weave_claude(args: &Value) -> String {
    let input = args["input_file"].as_str().unwrap_or("");
    let output_raw = args["output_file"].as_str().unwrap_or("");
    let output = ensure_outputs_directory(output_raw);
    let first_field = args.get("first_field").and_then(|v| v.as_str()).unwrap_or("top");
    crate::visual::apply_weave(input, &output, first_field).unwrap_or_else(|e| e)
}

fn execute_apply_weave_gemini(args: &HashMap<String, Value>) -> String {
    let input = args.get("input_file").and_then(|v| v.as_str()).unwrap_or("");
    let output_raw = args.get("output_file").and_then(|v| v.as_str()).unwrap_or("");
    let output = ensure_outputs_directory(output_raw);
    let first_field = args.get("first_field").and_then(|v| v.as_str()).unwrap_or("top");
    crate::visual::apply_weave(input, &output, first_field).unwrap_or_else(|e| e)
}

fn execute_apply_interlace_claude(args: &Value) -> String {
    let input = args["input_file"].as_str().unwrap_or("");
    let output_raw = args["output_file"].as_str().unwrap_or("");
    let output = ensure_outputs_directory(output_raw);
    let scan = args.get("scan").and_then(|v| v.as_str()).unwrap_or("tff");
    let lowpass = args.get("lowpass").and_then(|v| v.as_u64()).unwrap_or(1) as u32;
    crate::visual::apply_interlace(input, &output, scan, lowpass).unwrap_or_else(|e| e)
}

fn execute_apply_interlace_gemini(args: &HashMap<String, Value>) -> String {
    let input = args.get("input_file").and_then(|v| v.as_str()).unwrap_or("");
    let output_raw = args.get("output_file").and_then(|v| v.as_str()).unwrap_or("");
    let output = ensure_outputs_directory(output_raw);
    let scan = args.get("scan").and_then(|v| v.as_str()).unwrap_or("tff");
    let lowpass = args.get("lowpass").and_then(|v| v.as_u64()).unwrap_or(1) as u32;
    crate::visual::apply_interlace(input, &output, scan, lowpass).unwrap_or_else(|e| e)
}

fn execute_denoise_audio_fft_claude(args: &Value) -> String {
    let input = args["input_file"].as_str().unwrap_or("");
    let output_raw = args["output_file"].as_str().unwrap_or("");
    let output = ensure_outputs_directory(output_raw);
    let noise_floor = args.get("noise_floor").and_then(|v| v.as_f64()).unwrap_or(-25.0);
    let noise_reduction = args.get("noise_reduction").and_then(|v| v.as_f64()).unwrap_or(12.0);
    let track_noise = args.get("track_noise").and_then(|v| v.as_bool()).unwrap_or(false);
    crate::audio::denoise_audio_fft(input, &output, noise_floor, noise_reduction, track_noise).unwrap_or_else(|e| e)
}

fn execute_denoise_audio_fft_gemini(args: &HashMap<String, Value>) -> String {
    let input = args.get("input_file").and_then(|v| v.as_str()).unwrap_or("");
    let output_raw = args.get("output_file").and_then(|v| v.as_str()).unwrap_or("");
    let output = ensure_outputs_directory(output_raw);
    let noise_floor = args.get("noise_floor").and_then(|v| v.as_f64()).unwrap_or(-25.0);
    let noise_reduction = args.get("noise_reduction").and_then(|v| v.as_f64()).unwrap_or(12.0);
    let track_noise = args.get("track_noise").and_then(|v| v.as_bool()).unwrap_or(false);
    crate::audio::denoise_audio_fft(input, &output, noise_floor, noise_reduction, track_noise).unwrap_or_else(|e| e)
}

fn execute_loop_audio_claude(args: &Value) -> String {
    let input = args["input_file"].as_str().unwrap_or("");
    let output_raw = args["output_file"].as_str().unwrap_or("");
    let output = ensure_outputs_directory(output_raw);
    let loop_count = args.get("loop_count").and_then(|v| v.as_i64()).unwrap_or(1) as i32;
    let size = args.get("size").and_then(|v| v.as_u64()).unwrap_or(0) as u32;
    let start = args.get("start").and_then(|v| v.as_u64()).unwrap_or(0) as u32;
    crate::audio::loop_audio(input, &output, loop_count, size, start).unwrap_or_else(|e| e)
}

fn execute_loop_audio_gemini(args: &HashMap<String, Value>) -> String {
    let input = args.get("input_file").and_then(|v| v.as_str()).unwrap_or("");
    let output_raw = args.get("output_file").and_then(|v| v.as_str()).unwrap_or("");
    let output = ensure_outputs_directory(output_raw);
    let loop_count = args.get("loop_count").and_then(|v| v.as_i64()).unwrap_or(1) as i32;
    let size = args.get("size").and_then(|v| v.as_u64()).unwrap_or(0) as u32;
    let start = args.get("start").and_then(|v| v.as_u64()).unwrap_or(0) as u32;
    crate::audio::loop_audio(input, &output, loop_count, size, start).unwrap_or_else(|e| e)
}

fn execute_apply_dc_shift_claude(args: &Value) -> String {
    let input = args["input_file"].as_str().unwrap_or("");
    let output_raw = args["output_file"].as_str().unwrap_or("");
    let output = ensure_outputs_directory(output_raw);
    let shift = args.get("shift").and_then(|v| v.as_f64()).unwrap_or(0.0);
    let limitergain = args.get("limitergain").and_then(|v| v.as_f64()).unwrap_or(0.0);
    crate::audio::apply_dc_shift(input, &output, shift, limitergain).unwrap_or_else(|e| e)
}

fn execute_apply_dc_shift_gemini(args: &HashMap<String, Value>) -> String {
    let input = args.get("input_file").and_then(|v| v.as_str()).unwrap_or("");
    let output_raw = args.get("output_file").and_then(|v| v.as_str()).unwrap_or("");
    let output = ensure_outputs_directory(output_raw);
    let shift = args.get("shift").and_then(|v| v.as_f64()).unwrap_or(0.0);
    let limitergain = args.get("limitergain").and_then(|v| v.as_f64()).unwrap_or(0.0);
    crate::audio::apply_dc_shift(input, &output, shift, limitergain).unwrap_or_else(|e| e)
}

fn execute_measure_dynamic_range_claude(args: &Value) -> String {
    let input = args["input_file"].as_str().unwrap_or("");
    crate::audio::measure_dynamic_range(input).unwrap_or_else(|e| e)
}

fn execute_measure_dynamic_range_gemini(args: &HashMap<String, Value>) -> String {
    let input = args.get("input_file").and_then(|v| v.as_str()).unwrap_or("");
    crate::audio::measure_dynamic_range(input).unwrap_or_else(|e| e)
}

fn execute_apply_single_eq_band_claude(args: &Value) -> String {
    let input = args["input_file"].as_str().unwrap_or("");
    let output_raw = args["output_file"].as_str().unwrap_or("");
    let output = ensure_outputs_directory(output_raw);
    let frequency = args.get("frequency").and_then(|v| v.as_f64()).unwrap_or(1000.0);
    let width = args.get("width").and_then(|v| v.as_f64()).unwrap_or(1.0);
    let gain = args.get("gain").and_then(|v| v.as_f64()).unwrap_or(0.0);
    let width_type = args.get("width_type").and_then(|v| v.as_str()).unwrap_or("o");
    crate::audio::apply_single_eq_band(input, &output, frequency, width, gain, width_type).unwrap_or_else(|e| e)
}

fn execute_apply_single_eq_band_gemini(args: &HashMap<String, Value>) -> String {
    let input = args.get("input_file").and_then(|v| v.as_str()).unwrap_or("");
    let output_raw = args.get("output_file").and_then(|v| v.as_str()).unwrap_or("");
    let output = ensure_outputs_directory(output_raw);
    let frequency = args.get("frequency").and_then(|v| v.as_f64()).unwrap_or(1000.0);
    let width = args.get("width").and_then(|v| v.as_f64()).unwrap_or(1.0);
    let gain = args.get("gain").and_then(|v| v.as_f64()).unwrap_or(0.0);
    let width_type = args.get("width_type").and_then(|v| v.as_str()).unwrap_or("o");
    crate::audio::apply_single_eq_band(input, &output, frequency, width, gain, width_type).unwrap_or_else(|e| e)
}

fn execute_apply_stereotools_claude(args: &Value) -> String {
    let input = args["input_file"].as_str().unwrap_or("");
    let output_raw = args["output_file"].as_str().unwrap_or("");
    let output = ensure_outputs_directory(output_raw);
    let level_in = args.get("level_in").and_then(|v| v.as_f64()).unwrap_or(1.0);
    let level_out = args.get("level_out").and_then(|v| v.as_f64()).unwrap_or(1.0);
    let balance_in = args.get("balance_in").and_then(|v| v.as_f64()).unwrap_or(0.0);
    let balance_out = args.get("balance_out").and_then(|v| v.as_f64()).unwrap_or(0.0);
    let softclip = args.get("softclip").and_then(|v| v.as_bool()).unwrap_or(false);
    let mutel = args.get("mutel").and_then(|v| v.as_bool()).unwrap_or(false);
    let muter = args.get("muter").and_then(|v| v.as_bool()).unwrap_or(false);
    let phasel = args.get("phasel").and_then(|v| v.as_bool()).unwrap_or(false);
    let phaser = args.get("phaser").and_then(|v| v.as_bool()).unwrap_or(false);
    let mode = args.get("mode").and_then(|v| v.as_str()).unwrap_or("lr>lr");
    crate::audio::apply_stereotools(input, &output, level_in, level_out, balance_in, balance_out, softclip, mutel, muter, phasel, phaser, mode).unwrap_or_else(|e| e)
}

fn execute_apply_stereotools_gemini(args: &HashMap<String, Value>) -> String {
    let input = args.get("input_file").and_then(|v| v.as_str()).unwrap_or("");
    let output_raw = args.get("output_file").and_then(|v| v.as_str()).unwrap_or("");
    let output = ensure_outputs_directory(output_raw);
    let level_in = args.get("level_in").and_then(|v| v.as_f64()).unwrap_or(1.0);
    let level_out = args.get("level_out").and_then(|v| v.as_f64()).unwrap_or(1.0);
    let balance_in = args.get("balance_in").and_then(|v| v.as_f64()).unwrap_or(0.0);
    let balance_out = args.get("balance_out").and_then(|v| v.as_f64()).unwrap_or(0.0);
    let softclip = args.get("softclip").and_then(|v| v.as_bool()).unwrap_or(false);
    let mutel = args.get("mutel").and_then(|v| v.as_bool()).unwrap_or(false);
    let muter = args.get("muter").and_then(|v| v.as_bool()).unwrap_or(false);
    let phasel = args.get("phasel").and_then(|v| v.as_bool()).unwrap_or(false);
    let phaser = args.get("phaser").and_then(|v| v.as_bool()).unwrap_or(false);
    let mode = args.get("mode").and_then(|v| v.as_str()).unwrap_or("lr>lr");
    crate::audio::apply_stereotools(input, &output, level_in, level_out, balance_in, balance_out, softclip, mutel, muter, phasel, phaser, mode).unwrap_or_else(|e| e)
}

fn execute_apply_asetrate_claude(args: &Value) -> String {
    let input = args["input_file"].as_str().unwrap_or("");
    let output_raw = args["output_file"].as_str().unwrap_or("");
    let output = ensure_outputs_directory(output_raw);
    let sample_rate = args.get("sample_rate").and_then(|v| v.as_u64()).unwrap_or(44100) as u32;
    crate::audio::apply_asetrate(input, &output, sample_rate).unwrap_or_else(|e| e)
}

fn execute_apply_asetrate_gemini(args: &HashMap<String, Value>) -> String {
    let input = args.get("input_file").and_then(|v| v.as_str()).unwrap_or("");
    let output_raw = args.get("output_file").and_then(|v| v.as_str()).unwrap_or("");
    let output = ensure_outputs_directory(output_raw);
    let sample_rate = args.get("sample_rate").and_then(|v| v.as_u64()).unwrap_or(44100) as u32;
    crate::audio::apply_asetrate(input, &output, sample_rate).unwrap_or_else(|e| e)
}

// PHASE I BATCH 9 EXECUTORS

fn execute_scale_to_reference_claude(args: &Value) -> String {
    let input = args["input_file"].as_str().unwrap_or("");
    let ref_file = args["ref_file"].as_str().unwrap_or("");
    let output_raw = args["output_file"].as_str().unwrap_or("");
    let output = ensure_outputs_directory(output_raw);
    let flags = args.get("flags").and_then(|v| v.as_str()).unwrap_or("bilinear");
    crate::visual::scale_to_reference(input, ref_file, &output, flags).unwrap_or_else(|e| e)
}

fn execute_scale_to_reference_gemini(args: &HashMap<String, Value>) -> String {
    let input = args.get("input_file").and_then(|v| v.as_str()).unwrap_or("");
    let ref_file = args.get("ref_file").and_then(|v| v.as_str()).unwrap_or("");
    let output_raw = args.get("output_file").and_then(|v| v.as_str()).unwrap_or("");
    let output = ensure_outputs_directory(output_raw);
    let flags = args.get("flags").and_then(|v| v.as_str()).unwrap_or("bilinear");
    crate::visual::scale_to_reference(input, ref_file, &output, flags).unwrap_or_else(|e| e)
}

fn execute_apply_fieldorder_claude(args: &Value) -> String {
    let input = args["input_file"].as_str().unwrap_or("");
    let output_raw = args["output_file"].as_str().unwrap_or("");
    let output = ensure_outputs_directory(output_raw);
    let order = args.get("order").and_then(|v| v.as_str()).unwrap_or("tff");
    crate::visual::apply_fieldorder(input, &output, order).unwrap_or_else(|e| e)
}

fn execute_apply_fieldorder_gemini(args: &HashMap<String, Value>) -> String {
    let input = args.get("input_file").and_then(|v| v.as_str()).unwrap_or("");
    let output_raw = args.get("output_file").and_then(|v| v.as_str()).unwrap_or("");
    let output = ensure_outputs_directory(output_raw);
    let order = args.get("order").and_then(|v| v.as_str()).unwrap_or("tff");
    crate::visual::apply_fieldorder(input, &output, order).unwrap_or_else(|e| e)
}

fn execute_optimize_gif_palette_claude(args: &Value) -> String {
    let input = args["input_file"].as_str().unwrap_or("");
    let output_raw = args["output_file"].as_str().unwrap_or("");
    let output = ensure_outputs_directory(output_raw);
    let width = args.get("width").and_then(|v| v.as_u64()).unwrap_or(320) as u32;
    let fps = args.get("fps").and_then(|v| v.as_f64()).unwrap_or(10.0);
    let stats_mode = args.get("stats_mode").and_then(|v| v.as_str()).unwrap_or("diff");
    crate::visual::optimize_gif_palette(input, &output, width, fps, stats_mode).unwrap_or_else(|e| e)
}

fn execute_optimize_gif_palette_gemini(args: &HashMap<String, Value>) -> String {
    let input = args.get("input_file").and_then(|v| v.as_str()).unwrap_or("");
    let output_raw = args.get("output_file").and_then(|v| v.as_str()).unwrap_or("");
    let output = ensure_outputs_directory(output_raw);
    let width = args.get("width").and_then(|v| v.as_u64()).unwrap_or(320) as u32;
    let fps = args.get("fps").and_then(|v| v.as_f64()).unwrap_or(10.0);
    let stats_mode = args.get("stats_mode").and_then(|v| v.as_str()).unwrap_or("diff");
    crate::visual::optimize_gif_palette(input, &output, width, fps, stats_mode).unwrap_or_else(|e| e)
}

fn execute_apply_hsv_key_claude(args: &Value) -> String {
    let input = args["input_file"].as_str().unwrap_or("");
    let output_raw = args["output_file"].as_str().unwrap_or("");
    let output = ensure_outputs_directory(output_raw);
    let hue = args.get("hue").and_then(|v| v.as_f64()).unwrap_or(0.0);
    let saturation = args.get("saturation").and_then(|v| v.as_f64()).unwrap_or(0.0);
    let value = args.get("value").and_then(|v| v.as_f64()).unwrap_or(0.0);
    let similarity = args.get("similarity").and_then(|v| v.as_f64()).unwrap_or(0.1);
    let blend = args.get("blend").and_then(|v| v.as_f64()).unwrap_or(0.0);
    crate::visual::apply_hsv_key(input, &output, hue, saturation, value, similarity, blend).unwrap_or_else(|e| e)
}

fn execute_apply_hsv_key_gemini(args: &HashMap<String, Value>) -> String {
    let input = args.get("input_file").and_then(|v| v.as_str()).unwrap_or("");
    let output_raw = args.get("output_file").and_then(|v| v.as_str()).unwrap_or("");
    let output = ensure_outputs_directory(output_raw);
    let hue = args.get("hue").and_then(|v| v.as_f64()).unwrap_or(0.0);
    let saturation = args.get("saturation").and_then(|v| v.as_f64()).unwrap_or(0.0);
    let value = args.get("value").and_then(|v| v.as_f64()).unwrap_or(0.0);
    let similarity = args.get("similarity").and_then(|v| v.as_f64()).unwrap_or(0.1);
    let blend = args.get("blend").and_then(|v| v.as_f64()).unwrap_or(0.0);
    crate::visual::apply_hsv_key(input, &output, hue, saturation, value, similarity, blend).unwrap_or_else(|e| e)
}

fn execute_apply_lut_yuv_claude(args: &Value) -> String {
    let input = args["input_file"].as_str().unwrap_or("");
    let output_raw = args["output_file"].as_str().unwrap_or("");
    let output = ensure_outputs_directory(output_raw);
    let y_expr = args.get("y_expr").and_then(|v| v.as_str()).unwrap_or("val");
    let u_expr = args.get("u_expr").and_then(|v| v.as_str()).unwrap_or("val");
    let v_expr = args.get("v_expr").and_then(|v| v.as_str()).unwrap_or("val");
    crate::visual::apply_lut_yuv(input, &output, y_expr, u_expr, v_expr).unwrap_or_else(|e| e)
}

fn execute_apply_lut_yuv_gemini(args: &HashMap<String, Value>) -> String {
    let input = args.get("input_file").and_then(|v| v.as_str()).unwrap_or("");
    let output_raw = args.get("output_file").and_then(|v| v.as_str()).unwrap_or("");
    let output = ensure_outputs_directory(output_raw);
    let y_expr = args.get("y_expr").and_then(|v| v.as_str()).unwrap_or("val");
    let u_expr = args.get("u_expr").and_then(|v| v.as_str()).unwrap_or("val");
    let v_expr = args.get("v_expr").and_then(|v| v.as_str()).unwrap_or("val");
    crate::visual::apply_lut_yuv(input, &output, y_expr, u_expr, v_expr).unwrap_or_else(|e| e)
}

fn execute_apply_freezeframes_claude(args: &Value) -> String {
    let input = args["input_file"].as_str().unwrap_or("");
    let output_raw = args["output_file"].as_str().unwrap_or("");
    let output = ensure_outputs_directory(output_raw);
    let first = args.get("first").and_then(|v| v.as_u64()).unwrap_or(0) as u32;
    let last = args.get("last").and_then(|v| v.as_u64()).unwrap_or(0) as u32;
    let replace = args.get("replace").and_then(|v| v.as_u64()).unwrap_or(0) as u32;
    crate::visual::apply_freezeframes(input, &output, first, last, replace).unwrap_or_else(|e| e)
}

fn execute_apply_freezeframes_gemini(args: &HashMap<String, Value>) -> String {
    let input = args.get("input_file").and_then(|v| v.as_str()).unwrap_or("");
    let output_raw = args.get("output_file").and_then(|v| v.as_str()).unwrap_or("");
    let output = ensure_outputs_directory(output_raw);
    let first = args.get("first").and_then(|v| v.as_u64()).unwrap_or(0) as u32;
    let last = args.get("last").and_then(|v| v.as_u64()).unwrap_or(0) as u32;
    let replace = args.get("replace").and_then(|v| v.as_u64()).unwrap_or(0) as u32;
    crate::visual::apply_freezeframes(input, &output, first, last, replace).unwrap_or_else(|e| e)
}

fn execute_draw_signal_graph_claude(args: &Value) -> String {
    let input = args["input_file"].as_str().unwrap_or("");
    let output_raw = args["output_file"].as_str().unwrap_or("");
    let output = ensure_outputs_directory(output_raw);
    let signal = args.get("signal").and_then(|v| v.as_str()).unwrap_or("YAVG");
    let width = args.get("width").and_then(|v| v.as_u64()).unwrap_or(1280) as u32;
    let height = args.get("height").and_then(|v| v.as_u64()).unwrap_or(256) as u32;
    crate::visual::draw_signal_graph(input, &output, signal, width, height).unwrap_or_else(|e| e)
}

fn execute_draw_signal_graph_gemini(args: &HashMap<String, Value>) -> String {
    let input = args.get("input_file").and_then(|v| v.as_str()).unwrap_or("");
    let output_raw = args.get("output_file").and_then(|v| v.as_str()).unwrap_or("");
    let output = ensure_outputs_directory(output_raw);
    let signal = args.get("signal").and_then(|v| v.as_str()).unwrap_or("YAVG");
    let width = args.get("width").and_then(|v| v.as_u64()).unwrap_or(1280) as u32;
    let height = args.get("height").and_then(|v| v.as_u64()).unwrap_or(256) as u32;
    crate::visual::draw_signal_graph(input, &output, signal, width, height).unwrap_or_else(|e| e)
}

fn execute_measure_video_entropy_claude(args: &Value) -> String {
    let input = args["input_file"].as_str().unwrap_or("");
    crate::visual::measure_video_entropy(input).unwrap_or_else(|e| e)
}

fn execute_measure_video_entropy_gemini(args: &HashMap<String, Value>) -> String {
    let input = args.get("input_file").and_then(|v| v.as_str()).unwrap_or("");
    crate::visual::measure_video_entropy(input).unwrap_or_else(|e| e)
}

fn execute_apply_compensation_delay_claude(args: &Value) -> String {
    let input = args["input_file"].as_str().unwrap_or("");
    let output_raw = args["output_file"].as_str().unwrap_or("");
    let output = ensure_outputs_directory(output_raw);
    let mm = args.get("mm").and_then(|v| v.as_f64()).unwrap_or(0.0);
    let cm = args.get("cm").and_then(|v| v.as_f64()).unwrap_or(0.0);
    let m = args.get("m").and_then(|v| v.as_f64()).unwrap_or(0.0);
    let dry = args.get("dry").and_then(|v| v.as_f64()).unwrap_or(0.0);
    let wet = args.get("wet").and_then(|v| v.as_f64()).unwrap_or(1.0);
    let temp = args.get("temp").and_then(|v| v.as_f64()).unwrap_or(20.0);
    crate::audio::apply_compensation_delay(input, &output, mm, cm, m, dry, wet, temp).unwrap_or_else(|e| e)
}

fn execute_apply_compensation_delay_gemini(args: &HashMap<String, Value>) -> String {
    let input = args.get("input_file").and_then(|v| v.as_str()).unwrap_or("");
    let output_raw = args.get("output_file").and_then(|v| v.as_str()).unwrap_or("");
    let output = ensure_outputs_directory(output_raw);
    let mm = args.get("mm").and_then(|v| v.as_f64()).unwrap_or(0.0);
    let cm = args.get("cm").and_then(|v| v.as_f64()).unwrap_or(0.0);
    let m = args.get("m").and_then(|v| v.as_f64()).unwrap_or(0.0);
    let dry = args.get("dry").and_then(|v| v.as_f64()).unwrap_or(0.0);
    let wet = args.get("wet").and_then(|v| v.as_f64()).unwrap_or(1.0);
    let temp = args.get("temp").and_then(|v| v.as_f64()).unwrap_or(20.0);
    crate::audio::apply_compensation_delay(input, &output, mm, cm, m, dry, wet, temp).unwrap_or_else(|e| e)
}

fn execute_apply_earwax_claude(args: &Value) -> String {
    let input = args["input_file"].as_str().unwrap_or("");
    let output_raw = args["output_file"].as_str().unwrap_or("");
    let output = ensure_outputs_directory(output_raw);
    crate::audio::apply_earwax(input, &output).unwrap_or_else(|e| e)
}

fn execute_apply_earwax_gemini(args: &HashMap<String, Value>) -> String {
    let input = args.get("input_file").and_then(|v| v.as_str()).unwrap_or("");
    let output_raw = args.get("output_file").and_then(|v| v.as_str()).unwrap_or("");
    let output = ensure_outputs_directory(output_raw);
    crate::audio::apply_earwax(input, &output).unwrap_or_else(|e| e)
}

fn execute_apply_allpass_filter_claude(args: &Value) -> String {
    let input = args["input_file"].as_str().unwrap_or("");
    let output_raw = args["output_file"].as_str().unwrap_or("");
    let output = ensure_outputs_directory(output_raw);
    let frequency = args.get("frequency").and_then(|v| v.as_f64()).unwrap_or(3000.0);
    let width = args.get("width").and_then(|v| v.as_f64()).unwrap_or(0.707);
    let width_type = args.get("width_type").and_then(|v| v.as_str()).unwrap_or("q");
    let mix = args.get("mix").and_then(|v| v.as_f64()).unwrap_or(1.0);
    crate::audio::apply_allpass_filter(input, &output, frequency, width, width_type, mix).unwrap_or_else(|e| e)
}

fn execute_apply_allpass_filter_gemini(args: &HashMap<String, Value>) -> String {
    let input = args.get("input_file").and_then(|v| v.as_str()).unwrap_or("");
    let output_raw = args.get("output_file").and_then(|v| v.as_str()).unwrap_or("");
    let output = ensure_outputs_directory(output_raw);
    let frequency = args.get("frequency").and_then(|v| v.as_f64()).unwrap_or(3000.0);
    let width = args.get("width").and_then(|v| v.as_f64()).unwrap_or(0.707);
    let width_type = args.get("width_type").and_then(|v| v.as_str()).unwrap_or("q");
    let mix = args.get("mix").and_then(|v| v.as_f64()).unwrap_or(1.0);
    crate::audio::apply_allpass_filter(input, &output, frequency, width, width_type, mix).unwrap_or_else(|e| e)
}

fn execute_apply_highshelf_claude(args: &Value) -> String {
    let input = args["input_file"].as_str().unwrap_or("");
    let output_raw = args["output_file"].as_str().unwrap_or("");
    let output = ensure_outputs_directory(output_raw);
    let frequency = args.get("frequency").and_then(|v| v.as_f64()).unwrap_or(8000.0);
    let gain = args.get("gain").and_then(|v| v.as_f64()).unwrap_or(0.0);
    let width = args.get("width").and_then(|v| v.as_f64()).unwrap_or(0.5);
    let width_type = args.get("width_type").and_then(|v| v.as_str()).unwrap_or("s");
    let poles = args.get("poles").and_then(|v| v.as_u64()).unwrap_or(2) as u32;
    crate::audio::apply_highshelf(input, &output, frequency, gain, width, width_type, poles).unwrap_or_else(|e| e)
}

fn execute_apply_highshelf_gemini(args: &HashMap<String, Value>) -> String {
    let input = args.get("input_file").and_then(|v| v.as_str()).unwrap_or("");
    let output_raw = args.get("output_file").and_then(|v| v.as_str()).unwrap_or("");
    let output = ensure_outputs_directory(output_raw);
    let frequency = args.get("frequency").and_then(|v| v.as_f64()).unwrap_or(8000.0);
    let gain = args.get("gain").and_then(|v| v.as_f64()).unwrap_or(0.0);
    let width = args.get("width").and_then(|v| v.as_f64()).unwrap_or(0.5);
    let width_type = args.get("width_type").and_then(|v| v.as_str()).unwrap_or("s");
    let poles = args.get("poles").and_then(|v| v.as_u64()).unwrap_or(2) as u32;
    crate::audio::apply_highshelf(input, &output, frequency, gain, width, width_type, poles).unwrap_or_else(|e| e)
}

fn execute_apply_lowshelf_claude(args: &Value) -> String {
    let input = args["input_file"].as_str().unwrap_or("");
    let output_raw = args["output_file"].as_str().unwrap_or("");
    let output = ensure_outputs_directory(output_raw);
    let frequency = args.get("frequency").and_then(|v| v.as_f64()).unwrap_or(120.0);
    let gain = args.get("gain").and_then(|v| v.as_f64()).unwrap_or(0.0);
    let width = args.get("width").and_then(|v| v.as_f64()).unwrap_or(0.5);
    let width_type = args.get("width_type").and_then(|v| v.as_str()).unwrap_or("s");
    let poles = args.get("poles").and_then(|v| v.as_u64()).unwrap_or(2) as u32;
    crate::audio::apply_lowshelf(input, &output, frequency, gain, width, width_type, poles).unwrap_or_else(|e| e)
}

fn execute_apply_lowshelf_gemini(args: &HashMap<String, Value>) -> String {
    let input = args.get("input_file").and_then(|v| v.as_str()).unwrap_or("");
    let output_raw = args.get("output_file").and_then(|v| v.as_str()).unwrap_or("");
    let output = ensure_outputs_directory(output_raw);
    let frequency = args.get("frequency").and_then(|v| v.as_f64()).unwrap_or(120.0);
    let gain = args.get("gain").and_then(|v| v.as_f64()).unwrap_or(0.0);
    let width = args.get("width").and_then(|v| v.as_f64()).unwrap_or(0.5);
    let width_type = args.get("width_type").and_then(|v| v.as_str()).unwrap_or("s");
    let poles = args.get("poles").and_then(|v| v.as_u64()).unwrap_or(2) as u32;
    crate::audio::apply_lowshelf(input, &output, frequency, gain, width, width_type, poles).unwrap_or_else(|e| e)
}

fn execute_apply_surround_upmix_claude(args: &Value) -> String {
    let input = args["input_file"].as_str().unwrap_or("");
    let output_raw = args["output_file"].as_str().unwrap_or("");
    let output = ensure_outputs_directory(output_raw);
    let chl_out = args.get("chl_out").and_then(|v| v.as_str()).unwrap_or("5.1");
    let chl_in = args.get("chl_in").and_then(|v| v.as_str()).unwrap_or("stereo");
    let level_in = args.get("level_in").and_then(|v| v.as_f64()).unwrap_or(1.0);
    let level_out = args.get("level_out").and_then(|v| v.as_f64()).unwrap_or(1.0);
    crate::audio::apply_surround_upmix(input, &output, chl_out, chl_in, level_in, level_out).unwrap_or_else(|e| e)
}

fn execute_apply_surround_upmix_gemini(args: &HashMap<String, Value>) -> String {
    let input = args.get("input_file").and_then(|v| v.as_str()).unwrap_or("");
    let output_raw = args.get("output_file").and_then(|v| v.as_str()).unwrap_or("");
    let output = ensure_outputs_directory(output_raw);
    let chl_out = args.get("chl_out").and_then(|v| v.as_str()).unwrap_or("5.1");
    let chl_in = args.get("chl_in").and_then(|v| v.as_str()).unwrap_or("stereo");
    let level_in = args.get("level_in").and_then(|v| v.as_f64()).unwrap_or(1.0);
    let level_out = args.get("level_out").and_then(|v| v.as_f64()).unwrap_or(1.0);
    crate::audio::apply_surround_upmix(input, &output, chl_out, chl_in, level_in, level_out).unwrap_or_else(|e| e)
}

fn execute_detect_volume_levels_claude(args: &Value) -> String {
    let input = args["input_file"].as_str().unwrap_or("");
    crate::audio::detect_volume_levels(input).unwrap_or_else(|e| e)
}

fn execute_detect_volume_levels_gemini(args: &HashMap<String, Value>) -> String {
    let input = args.get("input_file").and_then(|v| v.as_str()).unwrap_or("");
    crate::audio::detect_volume_levels(input).unwrap_or_else(|e| e)
}

// PHASE I BATCH 10 EXECUTORS

fn execute_stabilize_video_2pass_claude(args: &Value) -> String {
    let input = args["input_file"].as_str().unwrap_or("");
    let output_raw = args["output_file"].as_str().unwrap_or("");
    let output = ensure_outputs_directory(output_raw);
    let shakiness = args.get("shakiness").and_then(|v| v.as_u64()).unwrap_or(5) as u32;
    let accuracy = args.get("accuracy").and_then(|v| v.as_u64()).unwrap_or(15) as u32;
    let smoothing = args.get("smoothing").and_then(|v| v.as_u64()).unwrap_or(10) as u32;
    let zoom = args.get("zoom").and_then(|v| v.as_f64()).unwrap_or(0.0);
    crate::visual::stabilize_video_2pass(input, &output, shakiness, accuracy, smoothing, zoom).unwrap_or_else(|e| e)
}

fn execute_stabilize_video_2pass_gemini(args: &HashMap<String, Value>) -> String {
    let input = args.get("input_file").and_then(|v| v.as_str()).unwrap_or("");
    let output_raw = args.get("output_file").and_then(|v| v.as_str()).unwrap_or("");
    let output = ensure_outputs_directory(output_raw);
    let shakiness = args.get("shakiness").and_then(|v| v.as_u64()).unwrap_or(5) as u32;
    let accuracy = args.get("accuracy").and_then(|v| v.as_u64()).unwrap_or(15) as u32;
    let smoothing = args.get("smoothing").and_then(|v| v.as_u64()).unwrap_or(10) as u32;
    let zoom = args.get("zoom").and_then(|v| v.as_f64()).unwrap_or(0.0);
    crate::visual::stabilize_video_2pass(input, &output, shakiness, accuracy, smoothing, zoom).unwrap_or_else(|e| e)
}

fn execute_apply_lut_rgb_claude(args: &Value) -> String {
    let input = args["input_file"].as_str().unwrap_or("");
    let output_raw = args["output_file"].as_str().unwrap_or("");
    let output = ensure_outputs_directory(output_raw);
    let r_expr = args.get("r_expr").and_then(|v| v.as_str()).unwrap_or("");
    let g_expr = args.get("g_expr").and_then(|v| v.as_str()).unwrap_or("");
    let b_expr = args.get("b_expr").and_then(|v| v.as_str()).unwrap_or("");
    crate::visual::apply_lut_rgb(input, &output, r_expr, g_expr, b_expr).unwrap_or_else(|e| e)
}

fn execute_apply_lut_rgb_gemini(args: &HashMap<String, Value>) -> String {
    let input = args.get("input_file").and_then(|v| v.as_str()).unwrap_or("");
    let output_raw = args.get("output_file").and_then(|v| v.as_str()).unwrap_or("");
    let output = ensure_outputs_directory(output_raw);
    let r_expr = args.get("r_expr").and_then(|v| v.as_str()).unwrap_or("");
    let g_expr = args.get("g_expr").and_then(|v| v.as_str()).unwrap_or("");
    let b_expr = args.get("b_expr").and_then(|v| v.as_str()).unwrap_or("");
    crate::visual::apply_lut_rgb(input, &output, r_expr, g_expr, b_expr).unwrap_or_else(|e| e)
}

fn execute_apply_hsvhold_claude(args: &Value) -> String {
    let input = args["input_file"].as_str().unwrap_or("");
    let output_raw = args["output_file"].as_str().unwrap_or("");
    let output = ensure_outputs_directory(output_raw);
    let hue = args.get("hue").and_then(|v| v.as_f64()).unwrap_or(0.0);
    let white = args.get("white").and_then(|v| v.as_f64()).unwrap_or(0.01);
    let black = args.get("black").and_then(|v| v.as_f64()).unwrap_or(0.01);
    let similarity = args.get("similarity").and_then(|v| v.as_f64()).unwrap_or(0.01);
    let blend = args.get("blend").and_then(|v| v.as_f64()).unwrap_or(0.0);
    crate::visual::apply_hsvhold(input, &output, hue, white, black, similarity, blend).unwrap_or_else(|e| e)
}

fn execute_apply_hsvhold_gemini(args: &HashMap<String, Value>) -> String {
    let input = args.get("input_file").and_then(|v| v.as_str()).unwrap_or("");
    let output_raw = args.get("output_file").and_then(|v| v.as_str()).unwrap_or("");
    let output = ensure_outputs_directory(output_raw);
    let hue = args.get("hue").and_then(|v| v.as_f64()).unwrap_or(0.0);
    let white = args.get("white").and_then(|v| v.as_f64()).unwrap_or(0.01);
    let black = args.get("black").and_then(|v| v.as_f64()).unwrap_or(0.01);
    let similarity = args.get("similarity").and_then(|v| v.as_f64()).unwrap_or(0.01);
    let blend = args.get("blend").and_then(|v| v.as_f64()).unwrap_or(0.0);
    crate::visual::apply_hsvhold(input, &output, hue, white, black, similarity, blend).unwrap_or_else(|e| e)
}

fn execute_convert_pixel_format_claude(args: &Value) -> String {
    let input = args["input_file"].as_str().unwrap_or("");
    let output_raw = args["output_file"].as_str().unwrap_or("");
    let output = ensure_outputs_directory(output_raw);
    let pix_fmt = args.get("pix_fmt").and_then(|v| v.as_str()).unwrap_or("yuv420p");
    crate::visual::convert_pixel_format(input, &output, pix_fmt).unwrap_or_else(|e| e)
}

fn execute_convert_pixel_format_gemini(args: &HashMap<String, Value>) -> String {
    let input = args.get("input_file").and_then(|v| v.as_str()).unwrap_or("");
    let output_raw = args.get("output_file").and_then(|v| v.as_str()).unwrap_or("");
    let output = ensure_outputs_directory(output_raw);
    let pix_fmt = args.get("pix_fmt").and_then(|v| v.as_str()).unwrap_or("yuv420p");
    crate::visual::convert_pixel_format(input, &output, pix_fmt).unwrap_or_else(|e| e)
}

fn execute_apply_setsar_claude(args: &Value) -> String {
    let input = args["input_file"].as_str().unwrap_or("");
    let output_raw = args["output_file"].as_str().unwrap_or("");
    let output = ensure_outputs_directory(output_raw);
    let sar = args.get("sar").and_then(|v| v.as_str()).unwrap_or("1/1");
    crate::visual::apply_setsar(input, &output, sar).unwrap_or_else(|e| e)
}

fn execute_apply_setsar_gemini(args: &HashMap<String, Value>) -> String {
    let input = args.get("input_file").and_then(|v| v.as_str()).unwrap_or("");
    let output_raw = args.get("output_file").and_then(|v| v.as_str()).unwrap_or("");
    let output = ensure_outputs_directory(output_raw);
    let sar = args.get("sar").and_then(|v| v.as_str()).unwrap_or("1/1");
    crate::visual::apply_setsar(input, &output, sar).unwrap_or_else(|e| e)
}

fn execute_apply_random_frames_claude(args: &Value) -> String {
    let input = args["input_file"].as_str().unwrap_or("");
    let output_raw = args["output_file"].as_str().unwrap_or("");
    let output = ensure_outputs_directory(output_raw);
    let frames = args.get("frames").and_then(|v| v.as_u64()).unwrap_or(30) as u32;
    let seed = args.get("seed").and_then(|v| v.as_i64()).unwrap_or(-1);
    crate::visual::apply_random_frames(input, &output, frames, seed).unwrap_or_else(|e| e)
}

fn execute_apply_random_frames_gemini(args: &HashMap<String, Value>) -> String {
    let input = args.get("input_file").and_then(|v| v.as_str()).unwrap_or("");
    let output_raw = args.get("output_file").and_then(|v| v.as_str()).unwrap_or("");
    let output = ensure_outputs_directory(output_raw);
    let frames = args.get("frames").and_then(|v| v.as_u64()).unwrap_or(30) as u32;
    let seed = args.get("seed").and_then(|v| v.as_i64()).unwrap_or(-1);
    crate::visual::apply_random_frames(input, &output, frames, seed).unwrap_or_else(|e| e)
}

fn execute_visualize_cqt_claude(args: &Value) -> String {
    let input = args["input_file"].as_str().unwrap_or("");
    let output_raw = args["output_file"].as_str().unwrap_or("");
    let output = ensure_outputs_directory(output_raw);
    let width = args.get("width").and_then(|v| v.as_u64()).unwrap_or(1920) as u32;
    let height = args.get("height").and_then(|v| v.as_u64()).unwrap_or(1080) as u32;
    let bar_h = args.get("bar_h").and_then(|v| v.as_u64()).unwrap_or(20) as u32;
    let axis_h = args.get("axis_h").and_then(|v| v.as_u64()).unwrap_or(30) as u32;
    crate::audio::visualize_cqt(input, &output, width, height, bar_h, axis_h).unwrap_or_else(|e| e)
}

fn execute_visualize_cqt_gemini(args: &HashMap<String, Value>) -> String {
    let input = args.get("input_file").and_then(|v| v.as_str()).unwrap_or("");
    let output_raw = args.get("output_file").and_then(|v| v.as_str()).unwrap_or("");
    let output = ensure_outputs_directory(output_raw);
    let width = args.get("width").and_then(|v| v.as_u64()).unwrap_or(1920) as u32;
    let height = args.get("height").and_then(|v| v.as_u64()).unwrap_or(1080) as u32;
    let bar_h = args.get("bar_h").and_then(|v| v.as_u64()).unwrap_or(20) as u32;
    let axis_h = args.get("axis_h").and_then(|v| v.as_u64()).unwrap_or(30) as u32;
    crate::audio::visualize_cqt(input, &output, width, height, bar_h, axis_h).unwrap_or_else(|e| e)
}

fn execute_visualize_frequencies_claude(args: &Value) -> String {
    let input = args["input_file"].as_str().unwrap_or("");
    let output_raw = args["output_file"].as_str().unwrap_or("");
    let output = ensure_outputs_directory(output_raw);
    let width = args.get("width").and_then(|v| v.as_u64()).unwrap_or(1024) as u32;
    let height = args.get("height").and_then(|v| v.as_u64()).unwrap_or(512) as u32;
    let mode = args.get("mode").and_then(|v| v.as_str()).unwrap_or("line");
    let ascale = args.get("ascale").and_then(|v| v.as_str()).unwrap_or("log");
    crate::audio::visualize_frequencies(input, &output, width, height, mode, ascale).unwrap_or_else(|e| e)
}

fn execute_visualize_frequencies_gemini(args: &HashMap<String, Value>) -> String {
    let input = args.get("input_file").and_then(|v| v.as_str()).unwrap_or("");
    let output_raw = args.get("output_file").and_then(|v| v.as_str()).unwrap_or("");
    let output = ensure_outputs_directory(output_raw);
    let width = args.get("width").and_then(|v| v.as_u64()).unwrap_or(1024) as u32;
    let height = args.get("height").and_then(|v| v.as_u64()).unwrap_or(512) as u32;
    let mode = args.get("mode").and_then(|v| v.as_str()).unwrap_or("line");
    let ascale = args.get("ascale").and_then(|v| v.as_str()).unwrap_or("log");
    crate::audio::visualize_frequencies(input, &output, width, height, mode, ascale).unwrap_or_else(|e| e)
}

fn execute_apply_audio_iir_claude(args: &Value) -> String {
    let input = args["input_file"].as_str().unwrap_or("");
    let output_raw = args["output_file"].as_str().unwrap_or("");
    let output = ensure_outputs_directory(output_raw);
    let zeros = args.get("zeros").and_then(|v| v.as_str()).unwrap_or("");
    let poles = args.get("poles").and_then(|v| v.as_str()).unwrap_or("");
    let gains = args.get("gains").and_then(|v| v.as_str()).unwrap_or("");
    crate::audio::apply_audio_iir(input, &output, zeros, poles, gains).unwrap_or_else(|e| e)
}

fn execute_apply_audio_iir_gemini(args: &HashMap<String, Value>) -> String {
    let input = args.get("input_file").and_then(|v| v.as_str()).unwrap_or("");
    let output_raw = args.get("output_file").and_then(|v| v.as_str()).unwrap_or("");
    let output = ensure_outputs_directory(output_raw);
    let zeros = args.get("zeros").and_then(|v| v.as_str()).unwrap_or("");
    let poles = args.get("poles").and_then(|v| v.as_str()).unwrap_or("");
    let gains = args.get("gains").and_then(|v| v.as_str()).unwrap_or("");
    crate::audio::apply_audio_iir(input, &output, zeros, poles, gains).unwrap_or_else(|e| e)
}

fn execute_apply_audio_expression_claude(args: &Value) -> String {
    let input = args["input_file"].as_str().unwrap_or("");
    let output_raw = args["output_file"].as_str().unwrap_or("");
    let output = ensure_outputs_directory(output_raw);
    let exprs = args.get("exprs").and_then(|v| v.as_str()).unwrap_or("val");
    crate::audio::apply_audio_expression(input, &output, exprs).unwrap_or_else(|e| e)
}

fn execute_apply_audio_expression_gemini(args: &HashMap<String, Value>) -> String {
    let input = args.get("input_file").and_then(|v| v.as_str()).unwrap_or("");
    let output_raw = args.get("output_file").and_then(|v| v.as_str()).unwrap_or("");
    let output = ensure_outputs_directory(output_raw);
    let exprs = args.get("exprs").and_then(|v| v.as_str()).unwrap_or("val");
    crate::audio::apply_audio_expression(input, &output, exprs).unwrap_or_else(|e| e)
}

fn execute_convert_audio_format_claude(args: &Value) -> String {
    let input = args["input_file"].as_str().unwrap_or("");
    let output_raw = args["output_file"].as_str().unwrap_or("");
    let output = ensure_outputs_directory(output_raw);
    let sample_fmts = args.get("sample_fmts").and_then(|v| v.as_str()).unwrap_or("");
    let sample_rates = args.get("sample_rates").and_then(|v| v.as_str()).unwrap_or("");
    let channel_layouts = args.get("channel_layouts").and_then(|v| v.as_str()).unwrap_or("");
    crate::audio::convert_audio_format(input, &output, sample_fmts, sample_rates, channel_layouts).unwrap_or_else(|e| e)
}

fn execute_convert_audio_format_gemini(args: &HashMap<String, Value>) -> String {
    let input = args.get("input_file").and_then(|v| v.as_str()).unwrap_or("");
    let output_raw = args.get("output_file").and_then(|v| v.as_str()).unwrap_or("");
    let output = ensure_outputs_directory(output_raw);
    let sample_fmts = args.get("sample_fmts").and_then(|v| v.as_str()).unwrap_or("");
    let sample_rates = args.get("sample_rates").and_then(|v| v.as_str()).unwrap_or("");
    let channel_layouts = args.get("channel_layouts").and_then(|v| v.as_str()).unwrap_or("");
    crate::audio::convert_audio_format(input, &output, sample_fmts, sample_rates, channel_layouts).unwrap_or_else(|e| e)
}

fn execute_apply_cross_correlate_claude(args: &Value) -> String {
    let input = args["input_file"].as_str().unwrap_or("");
    let secondary = args.get("secondary_file").and_then(|v| v.as_str()).unwrap_or("");
    let output_raw = args["output_file"].as_str().unwrap_or("");
    let output = ensure_outputs_directory(output_raw);
    let size = args.get("size").and_then(|v| v.as_u64()).unwrap_or(256) as u32;
    let algo = args.get("algo").and_then(|v| v.as_str()).unwrap_or("fast");
    crate::audio::apply_cross_correlate(input, secondary, &output, size, algo).unwrap_or_else(|e| e)
}

fn execute_apply_cross_correlate_gemini(args: &HashMap<String, Value>) -> String {
    let input = args.get("input_file").and_then(|v| v.as_str()).unwrap_or("");
    let secondary = args.get("secondary_file").and_then(|v| v.as_str()).unwrap_or("");
    let output_raw = args.get("output_file").and_then(|v| v.as_str()).unwrap_or("");
    let output = ensure_outputs_directory(output_raw);
    let size = args.get("size").and_then(|v| v.as_u64()).unwrap_or(256) as u32;
    let algo = args.get("algo").and_then(|v| v.as_str()).unwrap_or("fast");
    crate::audio::apply_cross_correlate(input, secondary, &output, size, algo).unwrap_or_else(|e| e)
}

fn execute_apply_audio_multiply_claude(args: &Value) -> String {
    let input = args["input_file"].as_str().unwrap_or("");
    let secondary = args.get("secondary_file").and_then(|v| v.as_str()).unwrap_or("");
    let output_raw = args["output_file"].as_str().unwrap_or("");
    let output = ensure_outputs_directory(output_raw);
    crate::audio::apply_audio_multiply(input, secondary, &output).unwrap_or_else(|e| e)
}

fn execute_apply_audio_multiply_gemini(args: &HashMap<String, Value>) -> String {
    let input = args.get("input_file").and_then(|v| v.as_str()).unwrap_or("");
    let secondary = args.get("secondary_file").and_then(|v| v.as_str()).unwrap_or("");
    let output_raw = args.get("output_file").and_then(|v| v.as_str()).unwrap_or("");
    let output = ensure_outputs_directory(output_raw);
    crate::audio::apply_audio_multiply(input, secondary, &output).unwrap_or_else(|e| e)
}

fn execute_apply_audio_contrast_claude(args: &Value) -> String {
    let input = args["input_file"].as_str().unwrap_or("");
    let output_raw = args["output_file"].as_str().unwrap_or("");
    let output = ensure_outputs_directory(output_raw);
    let contrast = args.get("contrast").and_then(|v| v.as_f64()).unwrap_or(33.0);
    crate::audio::apply_audio_contrast(input, &output, contrast).unwrap_or_else(|e| e)
}

fn execute_apply_audio_contrast_gemini(args: &HashMap<String, Value>) -> String {
    let input = args.get("input_file").and_then(|v| v.as_str()).unwrap_or("");
    let output_raw = args.get("output_file").and_then(|v| v.as_str()).unwrap_or("");
    let output = ensure_outputs_directory(output_raw);
    let contrast = args.get("contrast").and_then(|v| v.as_f64()).unwrap_or(33.0);
    crate::audio::apply_audio_contrast(input, &output, contrast).unwrap_or_else(|e| e)
}

fn execute_decode_hdcd_claude(args: &Value) -> String {
    let input = args["input_file"].as_str().unwrap_or("");
    let output_raw = args["output_file"].as_str().unwrap_or("");
    let output = ensure_outputs_directory(output_raw);
    let disable_autoconvert = args.get("disable_autoconvert").and_then(|v| v.as_bool()).unwrap_or(false);
    let process_stereo = args.get("process_stereo").and_then(|v| v.as_bool()).unwrap_or(false);
    let force_pe = args.get("force_pe").and_then(|v| v.as_bool()).unwrap_or(false);
    crate::audio::decode_hdcd(input, &output, disable_autoconvert, process_stereo, force_pe).unwrap_or_else(|e| e)
}

fn execute_decode_hdcd_gemini(args: &HashMap<String, Value>) -> String {
    let input = args.get("input_file").and_then(|v| v.as_str()).unwrap_or("");
    let output_raw = args.get("output_file").and_then(|v| v.as_str()).unwrap_or("");
    let output = ensure_outputs_directory(output_raw);
    let disable_autoconvert = args.get("disable_autoconvert").and_then(|v| v.as_bool()).unwrap_or(false);
    let process_stereo = args.get("process_stereo").and_then(|v| v.as_bool()).unwrap_or(false);
    let force_pe = args.get("force_pe").and_then(|v| v.as_bool()).unwrap_or(false);
    crate::audio::decode_hdcd(input, &output, disable_autoconvert, process_stereo, force_pe).unwrap_or_else(|e| e)
}

// ============================================================================
// WORKFLOW RECIPES — Executor Functions
// Multi-step chains exposed as single AI-callable tools
// ============================================================================

fn execute_youtube_ready_export_claude(args: &Value) -> String {
    let input = args["input_file"].as_str().unwrap_or("");
    let output_raw = args.get("output_file").and_then(|v| v.as_str()).unwrap_or("youtube_ready_output.mp4");
    let output = ensure_outputs_directory(output_raw);
    crate::workflows::youtube_ready_export(input, &output).unwrap_or_else(|e| e)
}

fn execute_youtube_ready_export_gemini(args: &HashMap<String, Value>) -> String {
    let input = args.get("input_file").and_then(|v| v.as_str()).unwrap_or("");
    let output_raw = args.get("output_file").and_then(|v| v.as_str()).unwrap_or("youtube_ready_output.mp4");
    let output = ensure_outputs_directory(output_raw);
    crate::workflows::youtube_ready_export(input, &output).unwrap_or_else(|e| e)
}

fn execute_podcast_cleanup_claude(args: &Value) -> String {
    let input = args["input_file"].as_str().unwrap_or("");
    let output_raw = args.get("output_file").and_then(|v| v.as_str()).unwrap_or("podcast_cleaned.wav");
    let output = ensure_outputs_directory(output_raw);
    crate::workflows::podcast_cleanup(input, &output).unwrap_or_else(|e| e)
}

fn execute_podcast_cleanup_gemini(args: &HashMap<String, Value>) -> String {
    let input = args.get("input_file").and_then(|v| v.as_str()).unwrap_or("");
    let output_raw = args.get("output_file").and_then(|v| v.as_str()).unwrap_or("podcast_cleaned.wav");
    let output = ensure_outputs_directory(output_raw);
    crate::workflows::podcast_cleanup(input, &output).unwrap_or_else(|e| e)
}

fn execute_cinematic_grade_claude(args: &Value) -> String {
    let input = args["input_file"].as_str().unwrap_or("");
    let output_raw = args.get("output_file").and_then(|v| v.as_str()).unwrap_or("cinematic_output.mp4");
    let output = ensure_outputs_directory(output_raw);
    crate::workflows::cinematic_grade(input, &output).unwrap_or_else(|e| e)
}

fn execute_cinematic_grade_gemini(args: &HashMap<String, Value>) -> String {
    let input = args.get("input_file").and_then(|v| v.as_str()).unwrap_or("");
    let output_raw = args.get("output_file").and_then(|v| v.as_str()).unwrap_or("cinematic_output.mp4");
    let output = ensure_outputs_directory(output_raw);
    crate::workflows::cinematic_grade(input, &output).unwrap_or_else(|e| e)
}

fn execute_create_gif_workflow_claude(args: &Value) -> String {
    let input = args["input_file"].as_str().unwrap_or("");
    let output_raw = args.get("output_file").and_then(|v| v.as_str()).unwrap_or("output.gif");
    let output = ensure_outputs_directory(output_raw);
    let start = args.get("start_seconds").and_then(|v| v.as_f64()).unwrap_or(0.0);
    let duration = args.get("duration_seconds").and_then(|v| v.as_f64()).unwrap_or(5.0);
    let width = args.get("width").and_then(|v| v.as_u64()).unwrap_or(480) as u32;
    let fps = args.get("fps").and_then(|v| v.as_f64()).unwrap_or(15.0);
    crate::workflows::create_gif(input, &output, start, duration, width, fps).unwrap_or_else(|e| e)
}

fn execute_create_gif_workflow_gemini(args: &HashMap<String, Value>) -> String {
    let input = args.get("input_file").and_then(|v| v.as_str()).unwrap_or("");
    let output_raw = args.get("output_file").and_then(|v| v.as_str()).unwrap_or("output.gif");
    let output = ensure_outputs_directory(output_raw);
    let start = args.get("start_seconds").and_then(|v| v.as_f64()).unwrap_or(0.0);
    let duration = args.get("duration_seconds").and_then(|v| v.as_f64()).unwrap_or(5.0);
    let width = args.get("width").and_then(|v| v.as_u64()).unwrap_or(480) as u32;
    let fps = args.get("fps").and_then(|v| v.as_f64()).unwrap_or(15.0);
    crate::workflows::create_gif(input, &output, start, duration, width, fps).unwrap_or_else(|e| e)
}

fn execute_talking_head_cleanup_claude(args: &Value) -> String {
    let input = args["input_file"].as_str().unwrap_or("");
    let output_raw = args.get("output_file").and_then(|v| v.as_str()).unwrap_or("talking_head_output.mp4");
    let output = ensure_outputs_directory(output_raw);
    crate::workflows::talking_head_cleanup(input, &output).unwrap_or_else(|e| e)
}

fn execute_talking_head_cleanup_gemini(args: &HashMap<String, Value>) -> String {
    let input = args.get("input_file").and_then(|v| v.as_str()).unwrap_or("");
    let output_raw = args.get("output_file").and_then(|v| v.as_str()).unwrap_or("talking_head_output.mp4");
    let output = ensure_outputs_directory(output_raw);
    crate::workflows::talking_head_cleanup(input, &output).unwrap_or_else(|e| e)
}

// =============================================================================
// BLENDER MCP EXECUTORS — Gemini dispatch
// =============================================================================

fn blender_client_or_err(ctx: &ToolExecutionContext) -> Result<&crate::blender_mcp_client::BlenderMCPClient, String> {
    ctx.app_state
        .blender_mcp_client
        .as_ref()
        .ok_or_else(|| "❌ BlenderMCPServer not configured. Set BLENDER_MCP_URL in .env to enable 3D rendering.".to_string())
}

// ── Shared async render helper ─────────────────────────────────────────────────
// All blender executors use render_async (submit_job + poll loop + download)
// instead of the sync call_tool path.  This is safe for any clip duration
// because it never holds an HTTP connection open during the actual render.

async fn blender_render(
    client: &crate::blender_mcp_client::BlenderMCPClient,
    tool: &str,
    args: Value,
    url_key: &str,
    ext: &str,
    label: &str,
) -> String {
    match client.render_async(tool, args, url_key, ext).await {
        Ok(path) => format!("✅ {label}: {path}"),
        Err(e)   => format!("❌ {tool} failed: {e}"),
    }
}

// ── Gemini blender tool executors ─────────────────────────────────────────────

async fn execute_blender_generate_scene_gemini(
    args: &HashMap<String, Value>,
    ctx: &ToolExecutionContext,
) -> String {
    let client = match blender_client_or_err(ctx) { Ok(c) => c, Err(e) => return e };
    let mut tool_args = json!({
        "prompt":   args.get("prompt").and_then(|v| v.as_str()).unwrap_or(""),
        "duration": args.get("duration").and_then(|v| v.as_f64()).unwrap_or(10.0),
        "style":    args.get("style").and_then(|v| v.as_str()).unwrap_or("cinematic"),
    });
    if let Some(u) = args.get("reference_image_url").and_then(|v| v.as_str()) {
        tool_args["reference_image_url"] = Value::String(u.to_string());
    }
    blender_render(&client, "blender_generate_scene", tool_args, "video_url", "mp4", "Blender scene rendered").await
}

async fn execute_blender_generate_thumbnail_gemini(
    args: &HashMap<String, Value>,
    ctx: &ToolExecutionContext,
) -> String {
    let client = match blender_client_or_err(ctx) { Ok(c) => c, Err(e) => return e };
    let tool_args = json!({
        "prompt":     args.get("prompt").and_then(|v| v.as_str()).unwrap_or(""),
        "title_text": args.get("title_text").and_then(|v| v.as_str()).unwrap_or(""),
        "style":      args.get("style").and_then(|v| v.as_str()).unwrap_or("youtube"),
    });
    blender_render(&client, "blender_generate_thumbnail", tool_args, "image_url", "png", "Blender thumbnail rendered").await
}

async fn execute_blender_generate_title_card_gemini(
    args: &HashMap<String, Value>,
    ctx: &ToolExecutionContext,
) -> String {
    let client = match blender_client_or_err(ctx) { Ok(c) => c, Err(e) => return e };
    let tool_args = json!({
        "title":    args.get("title").and_then(|v| v.as_str()).unwrap_or(""),
        "subtitle": args.get("subtitle").and_then(|v| v.as_str()).unwrap_or(""),
        "duration": args.get("duration").and_then(|v| v.as_f64()).unwrap_or(5.0),
        "style":    args.get("style").and_then(|v| v.as_str()).unwrap_or("cinematic"),
    });
    blender_render(&client, "blender_generate_title_card", tool_args, "video_url", "mp4", "Blender title card rendered").await
}

async fn execute_blender_generate_data_viz_gemini(
    args: &HashMap<String, Value>,
    ctx: &ToolExecutionContext,
) -> String {
    let client = match blender_client_or_err(ctx) { Ok(c) => c, Err(e) => return e };
    let tool_args = json!({
        "data_json":  args.get("data_json").and_then(|v| v.as_str()).unwrap_or("[]"),
        "chart_type": args.get("chart_type").and_then(|v| v.as_str()).unwrap_or("bar"),
        "title":      args.get("title").and_then(|v| v.as_str()).unwrap_or(""),
        "duration":   args.get("duration").and_then(|v| v.as_f64()).unwrap_or(10.0),
    });
    blender_render(&client, "blender_generate_data_viz", tool_args, "video_url", "mp4", "Blender data viz rendered").await
}

async fn execute_blender_generate_lower_third_gemini(
    args: &HashMap<String, Value>,
    ctx: &ToolExecutionContext,
) -> String {
    let client = match blender_client_or_err(ctx) { Ok(c) => c, Err(e) => return e };
    let tool_args = json!({
        "name_text":     args.get("name_text").and_then(|v| v.as_str()).unwrap_or(""),
        "subtitle_text": args.get("subtitle_text").and_then(|v| v.as_str()).unwrap_or(""),
        "style":         args.get("style").and_then(|v| v.as_str()).unwrap_or("modern"),
        "duration":      args.get("duration").and_then(|v| v.as_f64()).unwrap_or(5.0),
    });
    blender_render(&client, "blender_generate_lower_third", tool_args, "video_url", "mp4", "Blender lower third rendered").await
}

async fn execute_blender_generate_latex_gemini(
    args: &HashMap<String, Value>,
    ctx: &ToolExecutionContext,
) -> String {
    let client = match blender_client_or_err(ctx) { Ok(c) => c, Err(e) => return e };
    let tool_args = json!({
        "latex_expression": args.get("latex_expression").and_then(|v| v.as_str()).unwrap_or(""),
        "animation_type":   args.get("animation_type").and_then(|v| v.as_str()).unwrap_or("appear"),
        "duration":         args.get("duration").and_then(|v| v.as_f64()).unwrap_or(8.0),
        "background_style": args.get("background_style").and_then(|v| v.as_str()).unwrap_or("dark"),
    });
    blender_render(&client, "blender_generate_latex", tool_args, "video_url", "mp4", "Blender LaTeX animation rendered").await
}

async fn execute_blender_generate_ui_mockup_gemini(
    args: &HashMap<String, Value>,
    ctx: &ToolExecutionContext,
) -> String {
    let client = match blender_client_or_err(ctx) { Ok(c) => c, Err(e) => return e };
    let tool_args = json!({
        "device":         args.get("device").and_then(|v| v.as_str()).unwrap_or("iPhone"),
        "animation":      args.get("animation").and_then(|v| v.as_str()).unwrap_or("reveal"),
        "duration":       args.get("duration").and_then(|v| v.as_f64()).unwrap_or(5.0),
        "screenshot_url": args.get("screenshot_url").and_then(|v| v.as_str()).unwrap_or(""),
    });
    blender_render(&client, "blender_generate_ui_mockup", tool_args, "video_url", "mp4", "Blender UI mockup rendered").await
}

async fn execute_blender_generate_animation_gemini(
    args: &HashMap<String, Value>,
    ctx: &ToolExecutionContext,
) -> String {
    let client = match blender_client_or_err(ctx) { Ok(c) => c, Err(e) => return e };
    let tool_args = json!({
        "description": args.get("description").and_then(|v| v.as_str()).unwrap_or(""),
        "duration":    args.get("duration").and_then(|v| v.as_f64()).unwrap_or(10.0),
        "background":  args.get("background").and_then(|v| v.as_str()).unwrap_or("dark"),
        "quality":     args.get("quality").and_then(|v| v.as_str()).unwrap_or("m"),
    });
    blender_render(&client, "blender_generate_animation", tool_args, "video_url", "mp4", "Manim animation rendered").await
}

async fn execute_blender_generate_chart_gemini(
    args: &HashMap<String, Value>,
    ctx: &ToolExecutionContext,
) -> String {
    let client = match blender_client_or_err(ctx) { Ok(c) => c, Err(e) => return e };
    let data: Value = args.get("data")
        .and_then(|v| v.as_str())
        .and_then(|s| serde_json::from_str(s).ok())
        .unwrap_or(json!([]));
    let labels: Value = args.get("labels")
        .and_then(|v| v.as_str())
        .and_then(|s| serde_json::from_str(s).ok())
        .unwrap_or(json!([]));
    let colors: Value = args.get("colors")
        .and_then(|v| v.as_str())
        .and_then(|s| serde_json::from_str(s).ok())
        .unwrap_or(json!([]));
    let tool_args = json!({
        "chart_type": args.get("chart_type").and_then(|v| v.as_str()).unwrap_or("bar_chart"),
        "title":      args.get("title").and_then(|v| v.as_str()).unwrap_or(""),
        "data":       data,
        "labels":     labels,
        "duration":   args.get("duration").and_then(|v| v.as_f64()).unwrap_or(10.0),
        "colors":     colors,
    });
    blender_render(&client, "blender_generate_chart", tool_args, "video_url", "mp4", "Manim chart rendered").await
}

// ── Claude blender tool executors ──────────────────────────────────────────────

async fn execute_blender_generate_scene_claude(args: &Value, ctx: &ToolExecutionContext) -> String {
    let client = match blender_client_or_err(ctx) { Ok(c) => c, Err(e) => return e };
    let mut tool_args = json!({
        "prompt":   args["prompt"].as_str().unwrap_or(""),
        "duration": args["duration"].as_f64().unwrap_or(10.0),
        "style":    args["style"].as_str().unwrap_or("cinematic"),
    });
    if let Some(u) = args["reference_image_url"].as_str() {
        tool_args["reference_image_url"] = Value::String(u.to_string());
    }
    blender_render(&client, "blender_generate_scene", tool_args, "video_url", "mp4", "Blender scene rendered").await
}

async fn execute_blender_generate_thumbnail_claude(args: &Value, ctx: &ToolExecutionContext) -> String {
    let client = match blender_client_or_err(ctx) { Ok(c) => c, Err(e) => return e };
    let tool_args = json!({
        "prompt":     args["prompt"].as_str().unwrap_or(""),
        "title_text": args["title_text"].as_str().unwrap_or(""),
        "style":      args["style"].as_str().unwrap_or("youtube"),
    });
    blender_render(&client, "blender_generate_thumbnail", tool_args, "image_url", "png", "Blender thumbnail rendered").await
}

async fn execute_blender_generate_title_card_claude(args: &Value, ctx: &ToolExecutionContext) -> String {
    let client = match blender_client_or_err(ctx) { Ok(c) => c, Err(e) => return e };
    let tool_args = json!({
        "title":    args["title"].as_str().unwrap_or(""),
        "subtitle": args["subtitle"].as_str().unwrap_or(""),
        "duration": args["duration"].as_f64().unwrap_or(5.0),
        "style":    args["style"].as_str().unwrap_or("cinematic"),
    });
    blender_render(&client, "blender_generate_title_card", tool_args, "video_url", "mp4", "Blender title card rendered").await
}

async fn execute_blender_generate_data_viz_claude(args: &Value, ctx: &ToolExecutionContext) -> String {
    let client = match blender_client_or_err(ctx) { Ok(c) => c, Err(e) => return e };
    let tool_args = json!({
        "data_json":  args["data_json"].as_str().unwrap_or("[]"),
        "chart_type": args["chart_type"].as_str().unwrap_or("bar"),
        "title":      args["title"].as_str().unwrap_or("Data"),
        "duration":   args["duration"].as_f64().unwrap_or(10.0),
    });
    blender_render(&client, "blender_generate_data_viz", tool_args, "video_url", "mp4", "Blender data viz rendered").await
}

async fn execute_blender_generate_lower_third_claude(args: &Value, ctx: &ToolExecutionContext) -> String {
    let client = match blender_client_or_err(ctx) { Ok(c) => c, Err(e) => return e };
    let tool_args = json!({
        "name_text":     args["name_text"].as_str().unwrap_or(""),
        "subtitle_text": args["subtitle_text"].as_str().unwrap_or(""),
        "style":         args["style"].as_str().unwrap_or("modern"),
        "duration":      args["duration"].as_f64().unwrap_or(5.0),
    });
    blender_render(&client, "blender_generate_lower_third", tool_args, "video_url", "mp4", "Blender lower third rendered").await
}

async fn execute_blender_generate_latex_claude(args: &Value, ctx: &ToolExecutionContext) -> String {
    let client = match blender_client_or_err(ctx) { Ok(c) => c, Err(e) => return e };
    let tool_args = json!({
        "latex_expression": args["latex_expression"].as_str().unwrap_or(""),
        "animation_type":   args["animation_type"].as_str().unwrap_or("appear"),
        "duration":         args["duration"].as_f64().unwrap_or(8.0),
        "background_style": args["background_style"].as_str().unwrap_or("dark"),
    });
    blender_render(&client, "blender_generate_latex", tool_args, "video_url", "mp4", "Blender LaTeX animation rendered").await
}

async fn execute_blender_generate_ui_mockup_claude(args: &Value, ctx: &ToolExecutionContext) -> String {
    let client = match blender_client_or_err(ctx) { Ok(c) => c, Err(e) => return e };
    let tool_args = json!({
        "device":         args["device"].as_str().unwrap_or("iPhone"),
        "animation":      args["animation"].as_str().unwrap_or("reveal"),
        "duration":       args["duration"].as_f64().unwrap_or(5.0),
        "screenshot_url": args["screenshot_url"].as_str().unwrap_or(""),
    });
    blender_render(&client, "blender_generate_ui_mockup", tool_args, "video_url", "mp4", "Blender UI mockup rendered").await
}

async fn execute_blender_generate_animation_claude(args: &Value, ctx: &ToolExecutionContext) -> String {
    let client = match blender_client_or_err(ctx) { Ok(c) => c, Err(e) => return e };
    let tool_args = json!({
        "description": args["description"].as_str().unwrap_or(""),
        "duration":    args["duration"].as_f64().unwrap_or(10.0),
        "background":  args["background"].as_str().unwrap_or("dark"),
        "quality":     args["quality"].as_str().unwrap_or("m"),
    });
    blender_render(&client, "blender_generate_animation", tool_args, "video_url", "mp4", "Manim animation rendered").await
}

async fn execute_blender_generate_chart_claude(args: &Value, ctx: &ToolExecutionContext) -> String {
    let client = match blender_client_or_err(ctx) { Ok(c) => c, Err(e) => return e };
    let data: Value = args["data"].as_str()
        .and_then(|s| serde_json::from_str(s).ok())
        .unwrap_or(json!([]));
    let labels: Value = args["labels"].as_str()
        .and_then(|s| serde_json::from_str(s).ok())
        .unwrap_or(json!([]));
    let colors: Value = args["colors"].as_str()
        .and_then(|s| serde_json::from_str(s).ok())
        .unwrap_or(json!([]));
    let tool_args = json!({
        "chart_type": args["chart_type"].as_str().unwrap_or("bar_chart"),
        "title":      args["title"].as_str().unwrap_or(""),
        "data":       data,
        "labels":     labels,
        "duration":   args["duration"].as_f64().unwrap_or(10.0),
        "colors":     colors,
    });
    blender_render(&client, "blender_generate_chart", tool_args, "video_url", "mp4", "Manim chart rendered").await
}

/// Generic passthrough for new tools — forwards all args to BlenderMCPServer as-is.
async fn execute_blender_passthrough_gemini(
    tool_name: &str,
    args: &HashMap<String, Value>,
    ctx: &ToolExecutionContext,
) -> String {
    let client = match blender_client_or_err(ctx) { Ok(c) => c, Err(e) => return e };
    let tool_args: Value = args.iter()
        .map(|(k, v)| (k.clone(), v.clone()))
        .collect::<serde_json::Map<String, Value>>()
        .into();
    blender_render(&client, tool_name, tool_args, "video_url", "mp4", &format!("{} rendered", tool_name)).await
}

/// Generic Claude passthrough for new blender/manim tools.
async fn execute_blender_simple_manim_claude(
    tool_name: &str,
    args: &Value,
    ctx: &ToolExecutionContext,
) -> String {
    let client = match blender_client_or_err(ctx) { Ok(c) => c, Err(e) => return e };
    let tool_args = args.as_object()
        .map(|m| serde_json::Value::Object(m.clone()))
        .unwrap_or(json!({}));
    blender_render(&client, tool_name, tool_args, "video_url", "mp4", &format!("{} rendered", tool_name)).await
}
