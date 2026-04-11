/// AI-powered tool selection — replaces the keyword-matching ToolSelector.
///
/// Instead of hardcoded keyword lists, we give the AI a compact catalog of all
/// 320+ tool names + one-line descriptions and let it intelligently pick the
/// most relevant tools for the user's specific request.
///
/// This works for ANY request including novel ones that keyword matching misses
/// (e.g. "create cinematic travel reels" has no FFmpeg-specific keywords but the
/// AI understands it needs auto_generate_video, add_text_overlay, add_audio, etc.)
///
/// Uses Gemma 4 via NVIDIA NIM (free, 40 RPM) — does not consume Gemini quota.
/// Falls back to Gemini if NIM is unavailable. Falls back to a general tool set
/// if both fail, so the agent always has something to work with.

use crate::gemini_client::FunctionDeclaration;

/// Control tools that are always included regardless of request type.
/// These let the agent manage its own workflow.
const CONTROL_TOOL_NAMES: &[&str] = &[
    "start_background_job",
    "check_job_status",
    "search_memory",
];

/// Maximum video tools to include (control tools are on top of this).
/// Keeps the total well under Gemini's practical schema complexity limit.
const MAX_VIDEO_TOOLS: usize = 30;

/// Select the most relevant tools for `user_request` using AI.
///
/// Returns the full `FunctionDeclaration` structs ready to pass to the model,
/// including the 3 control tools plus up to MAX_VIDEO_TOOLS video tools.
pub async fn select_tools_for_request(
    user_request: &str,
    nvidia_nim_client: Option<&crate::nvidia_nim_client::NvidiaNimClient>,
    gemini_client: Option<&crate::gemini_client::GeminiClient>,
) -> Vec<FunctionDeclaration> {
    // All available video editing tool definitions
    let all_video_tools = crate::gemini_client::GeminiClient::create_video_editing_tools();

    // Build compact catalog: "tool_name: description\n" for each tool
    let catalog: String = all_video_tools
        .iter()
        .map(|t| {
            // Use just the first sentence of the description to keep the catalog concise
            let short_desc = t.description
                .split(". ")
                .next()
                .unwrap_or(&t.description)
                .trim_end_matches('.');
            format!("{}: {}", t.name, short_desc)
        })
        .collect::<Vec<_>>()
        .join("\n");

    let selection_prompt = format!(
        r#"You are a tool selector for an AI video editing assistant that has {total} available tools.

USER REQUEST: "{request}"

Your job: select the {max} most relevant tools from the catalog below that would be needed to fully handle this request. Think about ALL the steps: planning, generating content, editing, effects, audio, and export.

TOOL CATALOG:
{catalog}

Return ONLY a valid JSON array of tool names. No explanation, no markdown, no commentary:
["tool1", "tool2", ...]"#,
        total = all_video_tools.len(),
        request = user_request,
        max = MAX_VIDEO_TOOLS,
        catalog = catalog,
    );

    // Try Gemma 4 via NVIDIA NIM first (free quota, doesn't touch Gemini)
    let selected_names = if let Some(nim) = nvidia_nim_client {
        match nim.generate_text_with_tokens(&selection_prompt, 512).await {
            Ok(text) => parse_tool_names(&text),
            Err(e) => {
                tracing::warn!("AI tool selection via NIM failed: {} — trying Gemini", e);
                try_gemini_selection(gemini_client, &selection_prompt).await
            }
        }
    } else {
        try_gemini_selection(gemini_client, &selection_prompt).await
    };

    tracing::info!(
        "AI tool selector picked {} tools for request: \"{}\"",
        selected_names.len(),
        user_request.chars().take(80).collect::<String>()
    );

    // Build the final tool list: control tools + AI-selected video tools
    build_tool_list(selected_names, all_video_tools)
}

async fn try_gemini_selection(
    gemini_client: Option<&crate::gemini_client::GeminiClient>,
    prompt: &str,
) -> Vec<String> {
    if let Some(gemini) = gemini_client {
        match gemini.generate_text(prompt).await {
            Ok(text) => return parse_tool_names(&text),
            Err(e) => tracing::warn!("AI tool selection via Gemini failed: {}", e),
        }
    }
    // Both providers failed — return empty (caller will use general fallback)
    vec![]
}

/// Parse a JSON array of tool names from the AI response.
/// Handles responses that have extra text before/after the JSON array.
fn parse_tool_names(response: &str) -> Vec<String> {
    // Find the first '[' and last ']' to extract the JSON array
    let start = response.find('[');
    let end = response.rfind(']');

    if let (Some(s), Some(e)) = (start, end) {
        if let Ok(names) = serde_json::from_str::<Vec<String>>(&response[s..=e]) {
            return names;
        }
    }

    tracing::warn!("Could not parse tool names from AI response: {}", &response[..response.len().min(200)]);
    vec![]
}

/// Build the final FunctionDeclaration list from selected tool names.
/// Always includes all 3 control tool declarations (added separately by the caller).
fn build_tool_list(
    selected_names: Vec<String>,
    all_video_tools: Vec<FunctionDeclaration>,
) -> Vec<FunctionDeclaration> {
    if selected_names.is_empty() {
        // Fallback: use the general-purpose set
        tracing::info!("AI tool selection returned empty — using general tool set as fallback");
        let general_names = crate::tool_selector::ToolSelector::general_tools();
        return all_video_tools
            .into_iter()
            .filter(|t| general_names.contains(&t.name))
            .collect();
    }

    // Filter to AI-selected tools, preserving the AI's ordering preference
    let mut result: Vec<FunctionDeclaration> = selected_names
        .iter()
        .filter_map(|name| all_video_tools.iter().find(|t| &t.name == name).cloned())
        .take(MAX_VIDEO_TOOLS)
        .collect();

    // If the AI selected fewer than 10 tools, pad with general tools to ensure coverage
    if result.len() < 10 {
        let general_names = crate::tool_selector::ToolSelector::general_tools();
        for tool in &all_video_tools {
            if result.len() >= MAX_VIDEO_TOOLS {
                break;
            }
            if general_names.contains(&tool.name) && !result.iter().any(|t| t.name == tool.name) {
                result.push(tool.clone());
            }
        }
    }

    result
}

/// Names of the control tools always included by the agent (exported for reference).
pub const ALWAYS_INCLUDED_TOOLS: &[&str] = CONTROL_TOOL_NAMES;
