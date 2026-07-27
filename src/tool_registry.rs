use crate::claude_client::{ClaudeTool, InputSchema, PropertyDefinition as ClaudePropertyDefinition};
use crate::gemini_client::{
    FunctionDeclaration, Parameters, PropertyDefinition as GeminiPropertyDefinition,
};
use std::collections::{BTreeSet, HashMap};

#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
pub enum AgentExecutionProfile {
    FullProduction,
}

#[derive(Debug, Clone)]
pub struct CompletionContract {
    pub mandatory_tool_names: Vec<&'static str>,
}

impl CompletionContract {
    pub fn full_generation() -> Self {
        Self {
            mandatory_tool_names: vec!["set_chat_title", "submit_final_answer"],
        }
    }
}

#[derive(Debug, Clone)]
pub struct CanonicalTool {
    pub declaration: FunctionDeclaration,
}

pub struct ToolRegistry;

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum DurableToolPolicy {
    ReadOnly,
    FastTransform,
    HeavyRender,
    ExternalService,
    UploadPublish,
    ReviewQa,
    CompletionControl,
}

impl DurableToolPolicy {
    pub fn as_str(self) -> &'static str {
        match self {
            DurableToolPolicy::ReadOnly => "read_only",
            DurableToolPolicy::FastTransform => "fast_transform",
            DurableToolPolicy::HeavyRender => "heavy_render",
            DurableToolPolicy::ExternalService => "external_service",
            DurableToolPolicy::UploadPublish => "upload_publish",
            DurableToolPolicy::ReviewQa => "review_qa",
            DurableToolPolicy::CompletionControl => "completion_control",
        }
    }

    pub fn requires_durable_node(self) -> bool {
        matches!(
            self,
            DurableToolPolicy::FastTransform
                | DurableToolPolicy::HeavyRender
                | DurableToolPolicy::ExternalService
                | DurableToolPolicy::UploadPublish
                | DurableToolPolicy::ReviewQa
        )
    }

    pub fn max_attempts(self) -> i32 {
        match self {
            DurableToolPolicy::ReadOnly => 1,
            DurableToolPolicy::FastTransform => 2,
            DurableToolPolicy::HeavyRender => 3,
            DurableToolPolicy::ExternalService => 3,
            DurableToolPolicy::UploadPublish => 2,
            DurableToolPolicy::ReviewQa => 2,
            DurableToolPolicy::CompletionControl => 1,
        }
    }

    pub fn timeout_hint_seconds(self) -> i32 {
        match self {
            DurableToolPolicy::ReadOnly => 60,
            DurableToolPolicy::FastTransform => 600,
            DurableToolPolicy::HeavyRender => 3600,
            DurableToolPolicy::ExternalService => 1800,
            DurableToolPolicy::UploadPublish => 1200,
            DurableToolPolicy::ReviewQa => 900,
            DurableToolPolicy::CompletionControl => 60,
        }
    }
}

impl ToolRegistry {
    pub fn completion_contract(_profile: AgentExecutionProfile) -> CompletionContract {
        CompletionContract::full_generation()
    }

    pub fn tools_for_profile(profile: AgentExecutionProfile) -> Vec<CanonicalTool> {
        match profile {
            AgentExecutionProfile::FullProduction => {
                let mut declarations =
                    crate::gemini_client::GeminiClient::create_video_editing_tools();
                declarations.push(Self::long_form_video_tool());
                declarations.push(Self::clip_compilation_tool());
                let contract = Self::completion_contract(profile);
                let mandatory: BTreeSet<&str> =
                    contract.mandatory_tool_names.iter().copied().collect();

                declarations
                    .into_iter()
                    .map(|declaration| {
                        let _is_completion_critical =
                            mandatory.contains(declaration.name.as_str());
                        CanonicalTool { declaration }
                    })
                    .collect()
            }
        }
    }

    /// Tools to KEEP as separate tools despite starting with `apply_` prefix.
    const KEEP_APPLY_TOOLS: &'static [&'static str] = &[
        "apply_filter", "apply_filter_chain", "apply_audio_effect",
        "apply_lut", "apply_lut3d", "apply_lut_rgb", "apply_lut_yuv",
        "apply_xfade_transition", "apply_maskedmerge",
    ];

    /// Tool name prefixes whose individual variants are consolidated into parameterized tools.
    const CONSOLIDATED_PREFIXES: &'static [&'static str] = &[
        "blender_generate_",
        "encode_",
        "detect_",
        "measure_",
        "compare_",
        "extract_",
        "apply_",
    ];

    fn _is_consolidated(tool_name: &str) -> bool {
        // Keep these specific consolidated tools, filter out individual variants
        let keep = [
            "blender_generate_scene_type",
            "manim_execute_script",
            "export_video",
            "analyze_video",
            "extract_audio",
            "extract_frames",
            "apply_filter", "apply_filter_chain", "apply_audio_effect",
            "apply_lut", "apply_lut3d", "apply_lut_rgb", "apply_lut_yuv",
            "apply_xfade_transition", "apply_maskedmerge",
            "apply_ffmpeg_filter", "apply_audio_ffmpeg_filter",
        ];
        if keep.contains(&tool_name) {
            return false; // don't filter out these consolidated tools
        }
        for prefix in Self::CONSOLIDATED_PREFIXES {
            if tool_name.starts_with(prefix) {
                return true; // filter out individual wrappers under these prefixes
            }
        }
        false // keep everything else
    }

    fn _filter_tools(tools: Vec<FunctionDeclaration>) -> Vec<FunctionDeclaration> {
        let mut filtered: Vec<FunctionDeclaration> = tools
            .into_iter()
            .filter(|t| !Self::_is_consolidated(&t.name))
            .collect();

        // Add consolidated parameterized tools
        filtered.push(crate::gemini_client::GeminiClient::apply_ffmpeg_filter_tool());
        filtered.push(crate::gemini_client::GeminiClient::apply_audio_ffmpeg_filter_tool());
        filtered.push(crate::gemini_client::GeminiClient::blender_generate_scene_type_tool());
        filtered.push(crate::gemini_client::GeminiClient::manim_execute_script_tool());
        filtered.push(crate::gemini_client::GeminiClient::export_video_tool());

        filtered
    }

    fn _filter_claude_tools(tools: Vec<FunctionDeclaration>) -> Vec<ClaudeTool> {
        let filtered: Vec<FunctionDeclaration> = tools
            .into_iter()
            .filter(|t| !Self::_is_consolidated(&t.name))
            .collect();

        let mut claude_tools: Vec<ClaudeTool> = filtered
            .into_iter()
            .map(gemini_to_claude_tool)
            .collect();

        claude_tools.push(gemini_to_claude_tool(
            crate::gemini_client::GeminiClient::apply_ffmpeg_filter_tool()
        ));
        claude_tools.push(gemini_to_claude_tool(
            crate::gemini_client::GeminiClient::apply_audio_ffmpeg_filter_tool()
        ));
        claude_tools.push(gemini_to_claude_tool(
            crate::gemini_client::GeminiClient::blender_generate_scene_type_tool()
        ));
        claude_tools.push(gemini_to_claude_tool(
            crate::gemini_client::GeminiClient::manim_execute_script_tool()
        ));
        claude_tools.push(gemini_to_claude_tool(
            crate::gemini_client::GeminiClient::export_video_tool()
        ));

        claude_tools
    }

    pub fn gemini_tools_for_profile(profile: AgentExecutionProfile) -> Vec<FunctionDeclaration> {
        let tools: Vec<FunctionDeclaration> = Self::tools_for_profile(profile)
            .into_iter()
            .map(|tool| tool.declaration)
            .collect();
        Self::_filter_tools(tools)
    }

    pub fn claude_tools_for_profile(profile: AgentExecutionProfile) -> Vec<ClaudeTool> {
        let tools: Vec<FunctionDeclaration> = Self::tools_for_profile(profile)
            .into_iter()
            .map(|tool| tool.declaration)
            .collect();
        Self::_filter_claude_tools(tools)
    }

    pub fn filter_gemini_tools_for_profile(
        profile: AgentExecutionProfile,
        tool_names: &[String],
    ) -> Vec<FunctionDeclaration> {
        let allowed: BTreeSet<&str> = tool_names.iter().map(String::as_str).collect();
        Self::gemini_tools_for_profile(profile)
            .into_iter()
            .filter(|tool| allowed.contains(tool.name.as_str()))
            .collect()
    }

    pub fn filter_claude_tools_for_profile(
        profile: AgentExecutionProfile,
        tool_names: &[String],
    ) -> Vec<ClaudeTool> {
        let allowed: BTreeSet<&str> = tool_names.iter().map(String::as_str).collect();
        Self::claude_tools_for_profile(profile)
            .into_iter()
            .filter(|tool| allowed.contains(tool.name.as_str()))
            .collect()
    }

    pub fn durable_policy_for_tool(tool_name: &str) -> DurableToolPolicy {
        let name = tool_name.trim();
        if matches!(name, "set_chat_title" | "submit_final_answer") {
            return DurableToolPolicy::CompletionControl;
        }

        if name.starts_with("review_")
            || name.starts_with("view_")
            || name.contains("_qa")
            || name.contains("quality")
        {
            return DurableToolPolicy::ReviewQa;
        }

        if name.starts_with("upload_")
            || name.starts_with("post_")
            || name.starts_with("publish_")
            || name.contains("youtube_upload")
            || name.contains("delivery")
        {
            return DurableToolPolicy::UploadPublish;
        }

        if name.starts_with("blender_")
            || name.starts_with("generate_long_form_video")
            || name.starts_with("auto_generate_video")
            || name.starts_with("generate_video")
            || name.starts_with("create_blank_video")
            || name.starts_with("merge_video")
            || name.starts_with("merge_videos")
            || name.starts_with("export_")
            || name.starts_with("convert_")
            || name.starts_with("compress_")
            || name.starts_with("render_")
            || name.contains("manim")
            || name.contains("latex")
        {
            return DurableToolPolicy::HeavyRender;
        }

        if name.starts_with("generate_text_to_speech")
            || name.starts_with("generate_sound_effect")
            || name.starts_with("generate_music")
            || name.starts_with("add_voiceover")
            || name.starts_with("transcribe_")
            || name.starts_with("pexels_")
            || name.starts_with("sketchfab_")
            || name.starts_with("search_youtube")
            || name.starts_with("analyze_youtube")
            || name.starts_with("optimize_youtube")
            || name.starts_with("suggest_content")
        {
            return DurableToolPolicy::ExternalService;
        }

        if name.ends_with("_video")
            || name.contains("_video")
            || name.starts_with("add_")
            || name.starts_with("trim_")
            || name.starts_with("split_")
            || name.starts_with("crop_")
            || name.starts_with("resize_")
            || name.starts_with("rotate_")
            || name.starts_with("flip_")
            || name.starts_with("scale_")
            || name.starts_with("stabilize_")
            || name.starts_with("apply_")
            || name.starts_with("adjust_")
            || name.starts_with("fade_")
            || name.starts_with("extract_")
            || name.starts_with("create_thumbnail")
        {
            return DurableToolPolicy::FastTransform;
        }

        DurableToolPolicy::ReadOnly
    }

    fn long_form_video_tool() -> FunctionDeclaration {
        let mut properties = HashMap::new();
        properties.insert(
            "title".to_string(),
            gemini_property(
                "string",
                "Working title for the long-form video, fallback summary, SaaS demo, or educational explainer.",
            ),
        );
        properties.insert(
            "brief".to_string(),
            gemini_property(
                "string",
                "Detailed creative brief. Include product, audience, offer, desired scenes, CTA, references, and any required talking points.",
            ),
        );
        properties.insert(
            "target_duration_seconds".to_string(),
            gemini_property(
                "number",
                "Target final duration in seconds. The system supports arbitrary length by planning and rendering multiple bounded segments.",
            ),
        );
        properties.insert(
            "segment_duration_seconds".to_string(),
            gemini_property(
                "number",
                "Optional preferred segment length in seconds. Leave blank unless there is a specific pacing requirement.",
            ),
        );
        properties.insert(
            "style".to_string(),
            gemini_property(
                "string",
                "Visual style, for example cinematic SaaS launch, bold social ad, documentary explainer, educational motion graphics, or premium tech.",
            ),
        );
        properties.insert(
            "offer_type".to_string(),
            gemini_property(
                "string",
                "Business lane: landing_page, education, product_mockup, clipping, business_explainer, thumbnails, or full_stack.",
            ),
        );
        properties.insert(
            "reference_url".to_string(),
            gemini_property(
                "string",
                "Optional public product, app, landing page, source video, or brand reference URL.",
            ),
        );
        properties.insert(
            "narration_speaker".to_string(),
            gemini_property(
                "string",
                "Optional VibeVoice narrator/speaker name or style to use for segment narration.",
            ),
        );
        properties.insert(
            "include_narration".to_string(),
            gemini_property(
                "boolean",
                "Whether to generate narration/audio for the planned segments. Defaults to true.",
            ),
        );

        FunctionDeclaration {
            name: "generate_long_form_video".to_string(),
            description: "Start a durable LangGraph-style long-form video workflow. Use this when the task needs a video longer than a normal short clip, fallback summary videos, SaaS/app demo packs, explainers, courses, or any generated video that should be built from multiple creative tools. The workflow plans bounded segments, calls Blender/Manim/LaTeX/UI/data-viz/VibeVoice where useful, then assembles the final video. It supports any target duration by increasing the number of segments instead of doing one fragile giant render.".to_string(),
            parameters: Parameters {
                param_type: "object".to_string(),
                properties,
                required: vec![
                    "title".to_string(),
                    "brief".to_string(),
                    "target_duration_seconds".to_string(),
                ],
            },
        }
    }

    fn clip_compilation_tool() -> FunctionDeclaration {
        let mut properties = HashMap::new();
        properties.insert(
            "source_url".to_string(),
            gemini_property(
                "string",
                "The URL of the video to download and clip. Supports YouTube, Kick, Twitch, or any yt-dlp compatible URL.",
            ),
        );
        properties.insert(
            "clip_duration_seconds".to_string(),
            gemini_property(
                "number",
                "Duration in seconds for each clip. Defaults to 15 if not specified.",
            ),
        );
        properties.insert(
            "max_clips".to_string(),
            gemini_property(
                "integer",
                "Maximum number of clips to generate. Defaults to 3 if not specified.",
            ),
        );
        properties.insert(
            "include_captions".to_string(),
            gemini_property(
                "boolean",
                "Whether to add auto-generated captions/subtitles to the clips. Defaults to true.",
            ),
        );
        properties.insert(
            "description".to_string(),
            gemini_property(
                "string",
                "A brief description of the content for context. Used for caption generation and logging.",
            ),
        );
        properties.insert(
            "clip_times".to_string(),
            gemini_array_property(
                "Explicit start times (in seconds) for each clip. When provided, the tool trims at these exact positions instead of auto-detecting. Use your vision capability to analyze the video and determine the most engaging moments, then pass them here. Example: [12.5, 48.2, 93.7]",
                "number",
            ),
        );
        properties.insert(
            "smart_selection".to_string(),
            gemini_property(
                "boolean",
                "Whether to use smart scene detection + audio energy analysis to pick clip positions. Defaults to true when clip_times is not provided. Set to false to use evenly-spaced intervals (legacy behavior).",
            ),
        );
        properties.insert(
            "kick_style".to_string(),
            gemini_property(
                "boolean",
                "Enable Kick-compliant clip editing: 9:16 vertical (1080x1920) with blurred background, logo watermark, styled captions, and outro card. Defaults to false. When enabled, logo_url and streamer_name can customize the output.",
            ),
        );
        properties.insert(
            "logo_url".to_string(),
            gemini_property(
                "string",
                "URL to the Kick streamer logo/watermark image (PNG with transparency). Use with kick_style=true. Download via download_asset tool first, then pass the R2 URL here.",
            ),
        );
        properties.insert(
            "streamer_name".to_string(),
            gemini_property(
                "string",
                "The streamer/channel name (e.g. 'Neon') for the outro card. Use with kick_style=true.",
            ),
        );

        FunctionDeclaration {
            name: "generate_clip_compilation".to_string(),
            description: "Download a video from YouTube, Kick, Twitch, or any public URL, extract highlight clips, add captions/subtitles, and upload the clips to R2. Returns an array of R2 URLs for the finished clips. Perfect for clipping/kick_auto_clipper services. Uses yt-dlp for stream and FFmpeg for editing — NEVER uses Blender or Manim. When clip_times is provided, trims at those exact timestamps. Otherwise uses smart scene detection + audio energy to find the best moments.".to_string(),
            parameters: Parameters {
                param_type: "object".to_string(),
                properties,
                required: vec!["source_url".to_string()],
            },
        }
    }
}

fn gemini_property(prop_type: &str, description: &str) -> GeminiPropertyDefinition {
    GeminiPropertyDefinition {
        prop_type: prop_type.to_string(),
        description: description.to_string(),
        items: None,
    }
}

fn gemini_array_property(description: &str, item_type: &str) -> GeminiPropertyDefinition {
    GeminiPropertyDefinition {
        prop_type: "array".to_string(),
        description: description.to_string(),
        items: Some(Box::new(GeminiPropertyDefinition {
            prop_type: item_type.to_string(),
            description: String::new(),
            items: None,
        })),
    }
}

fn gemini_to_claude_tool(declaration: FunctionDeclaration) -> ClaudeTool {
    ClaudeTool {
        name: declaration.name,
        description: declaration.description,
        input_schema: InputSchema {
            schema_type: declaration.parameters.param_type,
            properties: declaration
                .parameters
                .properties
                .into_iter()
                .map(|(name, property)| (name, gemini_to_claude_property(property)))
                .collect(),
            required: declaration.parameters.required,
        },
    }
}

fn gemini_to_claude_property(property: GeminiPropertyDefinition) -> ClaudePropertyDefinition {
    ClaudePropertyDefinition {
        prop_type: property.prop_type,
        description: property.description,
        items: property
            .items
            .map(|nested| Box::new(gemini_to_claude_property(*nested))),
    }
}

#[allow(dead_code)]
fn claude_to_gemini_tool(tool: ClaudeTool) -> FunctionDeclaration {
    FunctionDeclaration {
        name: tool.name,
        description: tool.description,
        parameters: Parameters {
            param_type: tool.input_schema.schema_type,
            properties: tool
                .input_schema
                .properties
                .into_iter()
                .map(|(name, property)| (name, claude_to_gemini_property(property)))
                .collect(),
            required: tool.input_schema.required,
        },
    }
}

#[allow(dead_code)]
fn claude_to_gemini_property(property: ClaudePropertyDefinition) -> GeminiPropertyDefinition {
    GeminiPropertyDefinition {
        prop_type: property.prop_type,
        description: property.description,
        items: property
            .items
            .map(|nested| Box::new(claude_to_gemini_property(*nested))),
    }
}
