/// Full-toolbelt-by-default tool access for agents.
///
/// Historically this module preselected a narrow subset of tools before the
/// agent could reason about the task. That architecture became a reliability
/// bottleneck in production because completion-critical and generation-critical
/// tools could be omitted before the model even started planning.
///
/// The system now defaults to returning the full allowed video toolbelt so the
/// agent can choose tools for itself at runtime.
///
/// A legacy opt-in preselection mode is still available through
/// `AI_TOOL_PRESELECTION_MODE=enabled` for experiments, but it is no longer the
/// primary path anywhere in the app.
use crate::gemini_client::FunctionDeclaration;

const ESSENTIAL_VIDEO_TOOL_NAMES: &[&str] = &["set_chat_title", "submit_final_answer"];

/// Maximum video tools to include when legacy preselection mode is explicitly enabled.
const MAX_VIDEO_TOOLS: usize = 30;

fn preselection_enabled() -> bool {
    matches!(
        std::env::var("AI_TOOL_PRESELECTION_MODE")
            .ok()
            .map(|value| value.trim().to_ascii_lowercase()),
        Some(value) if value == "enabled" || value == "true" || value == "1"
    )
}

fn all_video_tools_with_essentials() -> Vec<FunctionDeclaration> {
    let all_video_tools = crate::tool_registry::ToolRegistry::gemini_tools_for_profile(
        crate::tool_registry::AgentExecutionProfile::FullProduction,
    );
    build_tool_list(
        all_video_tools
            .iter()
            .map(|tool| tool.name.clone())
            .collect(),
        all_video_tools,
    )
}

/// Returns the full allowed video toolbelt by default.
///
/// Legacy AI preselection is available only when
/// `AI_TOOL_PRESELECTION_MODE=enabled`.
pub async fn select_tools_for_request(
    user_request: &str,
    nvidia_nim_client: Option<&crate::nvidia_nim_client::NvidiaNimClient>,
    gemini_client: Option<&crate::gemini_client::GeminiClient>,
) -> Vec<FunctionDeclaration> {
    if !preselection_enabled() {
        let tools = all_video_tools_with_essentials();
        tracing::info!(
            "🔓 Full toolbelt mode enabled: exposing {} video tools for request: \"{}\"",
            tools.len(),
            user_request.chars().take(80).collect::<String>()
        );
        return tools;
    }

    // All available video editing tool definitions
    let all_video_tools = crate::tool_registry::ToolRegistry::gemini_tools_for_profile(
        crate::tool_registry::AgentExecutionProfile::FullProduction,
    );

    // Build compact catalog: "tool_name: description\n" for each tool
    let catalog: String = all_video_tools
        .iter()
        .map(|t| {
            // Use just the first sentence of the description to keep the catalog concise
            let short_desc = t
                .description
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
        "Legacy AI tool selector picked {} tools for request: \"{}\"",
        selected_names.len(),
        user_request.chars().take(80).collect::<String>()
    );

    // Build the final tool list from AI-selected video tools
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

    tracing::warn!(
        "Could not parse tool names from AI response: {}",
        &response[..response.len().min(200)]
    );
    vec![]
}

/// Build the final FunctionDeclaration list from selected tool names.
/// Always includes all 3 control tool declarations (added separately by the caller).
fn build_tool_list(
    selected_names: Vec<String>,
    all_video_tools: Vec<FunctionDeclaration>,
) -> Vec<FunctionDeclaration> {
    let essential_tools: Vec<FunctionDeclaration> = ESSENTIAL_VIDEO_TOOL_NAMES
        .iter()
        .filter_map(|name| {
            all_video_tools
                .iter()
                .find(|candidate| candidate.name == *name)
                .cloned()
        })
        .collect();

    let ensure_essential_tools = |mut tools: Vec<FunctionDeclaration>| {
        for essential_tool in &essential_tools {
            if tools.iter().any(|tool| tool.name == essential_tool.name) {
                continue;
            }

            tools.push(essential_tool.clone());
        }
        tools
    };

    if selected_names.is_empty() {
        tracing::warn!(
            "Legacy AI tool selection returned empty — falling back to full toolbelt instead"
        );
        return ensure_essential_tools(all_video_tools);
    }

    // Filter to AI-selected tools, preserving the AI's ordering preference
    let mut result: Vec<FunctionDeclaration> = selected_names
        .iter()
        .filter_map(|name| all_video_tools.iter().find(|t| &t.name == name).cloned())
        .take(MAX_VIDEO_TOOLS)
        .collect();

    // If the AI selected fewer than 10 tools, pad from the canonical registry order.
    if result.len() < 10 {
        for tool in &all_video_tools {
            if result.len() >= MAX_VIDEO_TOOLS {
                break;
            }
            if !result.iter().any(|t| t.name == tool.name) {
                result.push(tool.clone());
            }
        }
    }

    ensure_essential_tools(result)
}
