// Dynamic tool selection for optimized AI agent performance
// Reduces schema complexity by selecting only relevant tools based on context

use std::collections::HashSet;

/// Tool categories for video editing tasks
#[derive(Debug, Clone, PartialEq, Eq, Hash)]
pub enum ToolCategory {
    CoreEditing,       // trim, merge, split, crop, rotate, etc.
    VisualEffects,     // overlays, filters, color adjustment
    AudioProcessing,   // volume, audio extraction/addition
    AIGeneration,      // TTS, sound effects, music, image generation
    StockMedia,        // Pexels integration
    AnalysisReview,    // Video/image analysis, review
    YouTubeIntegration,// YouTube optimization and trends
    PlatformExport,    // Export and final answer tools
}

impl ToolCategory {
    /// Get all tool names in this category
    pub fn tools(&self) -> Vec<&'static str> {
        match self {
            ToolCategory::CoreEditing => vec![
                "trim_video",
                "merge_videos",
                "split_video",
                "crop_video",
                "rotate_video",
                "flip_video",
                "resize_video",
                "scale_video",
                "convert_format",
                "compress_video",
                "stabilize_video",
            ],
            ToolCategory::VisualEffects => vec![
                "add_text_overlay",
                "add_overlay",
                "apply_filter",
                "adjust_color",
                "picture_in_picture",
                "chroma_key",
                "split_screen",
                "add_subtitles",
                "create_thumbnail",
            ],
            ToolCategory::AudioProcessing => vec![
                "adjust_volume",
                "extract_audio",
                "add_audio",
                "fade_audio",
                "add_voiceover_to_video",
                "adjust_speed",
            ],
            ToolCategory::AIGeneration => vec![
                "generate_text_to_speech",
                "generate_sound_effect",
                "generate_music",
                "generate_image",
                "generate_video_script",
                "auto_generate_video",
            ],
            ToolCategory::StockMedia => vec![
                "pexels_search",
                "pexels_download_video",
                "pexels_download_photo",
                "pexels_get_trending",
                "pexels_get_curated",
                "create_blank_video",
            ],
            ToolCategory::AnalysisReview => vec![
                "analyze_video",
                "analyze_image",
                "view_video",
                "review_video",
                "view_image",
                "extract_frames",
            ],
            ToolCategory::YouTubeIntegration => vec![
                "optimize_youtube_metadata",
                "analyze_youtube_performance",
                "suggest_content_ideas",
                "search_youtube_trends",
                "search_youtube_channels",
            ],
            ToolCategory::PlatformExport => vec![
                "export_for_platform",
                "submit_final_answer",
                "set_chat_title",
            ],
        }
    }
}

/// Tool selector that dynamically chooses relevant tools based on user intent
pub struct ToolSelector;

impl ToolSelector {
    /// Select relevant tools based on user prompt
    /// Returns a filtered list of tool names (max 20 tools as per Google's recommendation)
    pub fn select_tools(user_prompt: &str) -> Vec<String> {
        let prompt_lower = user_prompt.to_lowercase();

        // Detect categories based on keywords
        let mut selected_categories = Vec::new();

        // Always include core editing and platform export (essential for all tasks)
        selected_categories.push(ToolCategory::CoreEditing);
        selected_categories.push(ToolCategory::PlatformExport);

        // Visual effects keywords
        if Self::contains_any(&prompt_lower, &[
            "overlay", "text", "subtitle", "filter", "color", "picture in picture",
            "pip", "chroma", "green screen", "split screen", "thumbnail", "effect"
        ]) {
            selected_categories.push(ToolCategory::VisualEffects);
        }

        // Audio processing keywords
        if Self::contains_any(&prompt_lower, &[
            "audio", "sound", "volume", "music", "voiceover", "voice", "mute",
            "extract audio", "add audio", "fade", "speed", "slow", "fast"
        ]) {
            selected_categories.push(ToolCategory::AudioProcessing);
        }

        // AI generation keywords
        if Self::contains_any(&prompt_lower, &[
            "generate", "create", "tts", "text to speech", "sound effect",
            "background music", "script", "auto", "ai", "artificial"
        ]) {
            selected_categories.push(ToolCategory::AIGeneration);
        }

        // Stock media keywords
        if Self::contains_any(&prompt_lower, &[
            "pexels", "stock", "footage", "photo", "image", "download",
            "trending", "curated", "blank video", "template"
        ]) {
            selected_categories.push(ToolCategory::StockMedia);
        }

        // Analysis & review keywords
        if Self::contains_any(&prompt_lower, &[
            "analyze", "analysis", "review", "check", "inspect", "view",
            "metadata", "information", "details", "frames", "extract frame"
        ]) {
            selected_categories.push(ToolCategory::AnalysisReview);
        }

        // YouTube integration keywords
        if Self::contains_any(&prompt_lower, &[
            "youtube", "yt", "optimize", "seo", "metadata", "performance",
            "trends", "trending", "viral", "channel", "content ideas"
        ]) {
            selected_categories.push(ToolCategory::YouTubeIntegration);
        }

        // Collect all tools from selected categories
        let mut tools = HashSet::new();
        for category in &selected_categories {
            for tool in category.tools() {
                tools.insert(tool.to_string());
            }
        }

        // Convert to Vec and limit to 20 tools (Google's recommendation)
        let mut tool_list: Vec<String> = tools.into_iter().collect();
        tool_list.sort(); // Sort for consistency

        // If we exceed 20 tools, prioritize core editing
        if tool_list.len() > 20 {
            tracing::warn!(
                "Tool selection exceeded 20 tools ({}), applying prioritization",
                tool_list.len()
            );

            // Keep core editing and platform export, trim others
            let mut prioritized = Vec::new();

            // Add core editing tools first (highest priority)
            for tool in ToolCategory::CoreEditing.tools() {
                if tool_list.contains(&tool.to_string()) {
                    prioritized.push(tool.to_string());
                }
            }

            // Add platform export tools (essential)
            for tool in ToolCategory::PlatformExport.tools() {
                if tool_list.contains(&tool.to_string()) {
                    prioritized.push(tool.to_string());
                }
            }

            // Add remaining tools up to 20 total
            for tool in tool_list {
                if !prioritized.contains(&tool) && prioritized.len() < 20 {
                    prioritized.push(tool);
                }
            }

            tool_list = prioritized;
        }

        tracing::info!(
            "Dynamic tool selection: {} categories, {} tools selected",
            selected_categories.len(),
            tool_list.len()
        );

        tool_list
    }

    /// Helper function to check if text contains any of the keywords
    fn contains_any(text: &str, keywords: &[&str]) -> bool {
        keywords.iter().any(|keyword| text.contains(keyword))
    }

    /// Get all available tools (for backwards compatibility or fallback)
    pub fn all_tools() -> Vec<String> {
        let all_categories = vec![
            ToolCategory::CoreEditing,
            ToolCategory::VisualEffects,
            ToolCategory::AudioProcessing,
            ToolCategory::AIGeneration,
            ToolCategory::StockMedia,
            ToolCategory::AnalysisReview,
            ToolCategory::YouTubeIntegration,
            ToolCategory::PlatformExport,
        ];

        let mut tools = HashSet::new();
        for category in all_categories {
            for tool in category.tools() {
                tools.insert(tool.to_string());
            }
        }

        let mut tool_list: Vec<String> = tools.into_iter().collect();
        tool_list.sort();
        tool_list
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_basic_editing_prompt() {
        let tools = ToolSelector::select_tools("trim my video from 0:10 to 0:30");
        assert!(tools.contains(&"trim_video".to_string()));
        assert!(tools.len() <= 20);
    }

    #[test]
    fn test_audio_prompt() {
        let tools = ToolSelector::select_tools("add background music to my video");
        assert!(tools.contains(&"add_audio".to_string()));
        assert!(tools.len() <= 20);
    }

    #[test]
    fn test_complex_prompt() {
        let tools = ToolSelector::select_tools(
            "create a video with text overlay, background music, and upload to YouTube"
        );
        assert!(tools.contains(&"add_text_overlay".to_string()));
        assert!(tools.contains(&"add_audio".to_string()));
        assert!(tools.len() <= 20);
    }

    #[test]
    fn test_tool_count_limit() {
        let tools = ToolSelector::select_tools(
            "I need to trim, merge, add text, add music, optimize for YouTube, \
             apply filters, and create thumbnails"
        );
        assert!(tools.len() <= 20, "Tool count should not exceed 20");
    }

    #[test]
    fn test_all_tools() {
        let tools = ToolSelector::all_tools();
        assert!(tools.len() > 40); // Should have all ~52 tools
    }
}
