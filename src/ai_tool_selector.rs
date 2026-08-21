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
use crate::ollama_client::OllamaClient;

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
    let all_tools = crate::tool_registry::ToolRegistry::gemini_tools_for_profile(
        crate::tool_registry::AgentExecutionProfile::FullProduction,
    );
    tracing::info!(
        "🔓 Full toolbelt mode enabled: exposing {} video tools",
        all_tools.len()
    );
    all_tools
}

/// Returns the full allowed video toolbelt by default.
///
/// Legacy AI preselection is available only when
/// `AI_TOOL_PRESELECTION_MODE=enabled`.
pub async fn select_tools_for_request(
    user_request: &str,
    ollama_client: Option<&OllamaClient>,
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

    // Try Ollama first (self-hosted, free, GPU auto-scaled), then NIM, then Gemini
    let selected_names = if let Some(ollama) = ollama_client {
        match ollama.generate_text(&selection_prompt).await {
            Ok(text) => parse_tool_names(&text),
            Err(e) => {
                tracing::warn!("AI tool selection via Ollama failed: {} — trying NIM", e);
                try_nim_selection(nvidia_nim_client, gemini_client, &selection_prompt).await
            }
        }
    } else {
        try_nim_selection(nvidia_nim_client, gemini_client, &selection_prompt).await
    };

    tracing::info!(
        "Legacy AI tool selector picked {} tools for request: \"{}\"",
        selected_names.len(),
        user_request.chars().take(80).collect::<String>()
    );

    // Build the final tool list from AI-selected video tools
    build_tool_list(selected_names, all_video_tools)
}

async fn try_nim_selection(
    nvidia_nim_client: Option<&crate::nvidia_nim_client::NvidiaNimClient>,
    gemini_client: Option<&crate::gemini_client::GeminiClient>,
    prompt: &str,
) -> Vec<String> {
    if let Some(nim) = nvidia_nim_client {
        match nim.generate_text_with_tokens(prompt, 512).await {
            Ok(text) => return parse_tool_names(&text),
            Err(e) => tracing::warn!("AI tool selection via NIM failed: {} — trying Gemini", e),
        }
    }
    try_gemini_selection(gemini_client, prompt).await
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

// ─── Service-scoped toolbelts + search_tools discovery ────────────────────────
//
// Sending all ~223 tool schemas on every turn of every request inflates each
// Ollama prompt to ~40K tokens (schemas alone ~22K), which wedges the GPU
// cluster (minutes-long prefill per turn, 5-minute client timeouts, queue
// collapse). Industry pattern (Anthropic Tool Search Tool, RAG-MCP): advertise
// a small service-scoped set plus a `search_tools` meta-tool; the agent pulls
// additional tools into its active set only when it actually needs them.
//
// Kill switch: AGENT_TOOL_DISCOVERY=off reverts to the full toolbelt.

/// True unless explicitly disabled via env. Default ON.
pub fn tool_discovery_enabled() -> bool {
    !matches!(
        std::env::var("AGENT_TOOL_DISCOVERY")
            .ok()
            .map(|v| v.trim().to_ascii_lowercase()),
        Some(v) if v == "off" || v == "false" || v == "0"
    )
}

/// Mandatory tool names per service scope, mirroring each service prompt's
/// mandatory tool sequence in agentic_service_pipeline.rs. Control tools
/// (set_chat_title / submit_final_answer) and search_tools are added separately.
fn service_tool_names(scope: &str) -> Option<Vec<&'static str>> {
    let normalized = scope.trim().to_ascii_lowercase();
    let names: Vec<&'static str> = match normalized.as_str() {
        "clipping" | "kick_auto_clipper" => vec![
            "generate_clip_compilation",
            "download_asset",
            "analyze_video",
            "trim_video",
            "split_video",
            "add_subtitles",
            "merge_videos",
            "review_video",
            "export_video",
        ],
        "landing_page" => vec![
            "blender_generate_scene_type",
            "manim_execute_script",
            "merge_videos",
            "review_video",
            "export_video",
        ],
        "education" => vec![
            "manim_execute_script",
            "blender_generate_scene_type",
            "merge_videos",
            "generate_text_to_speech",
            "review_video",
            "export_video",
        ],
        "manim_explainer"
        | "whiteboard_animation"
        | "kinetic_typography"
        | "animated_infographic"
        | "algorithm_viz"
        | "investor_pitch"
        | "year_in_review"
        | "isometric_explainer" => vec![
            "manim_execute_script",
            "merge_videos",
            "generate_text_to_speech",
            "review_video",
            "export_video",
        ],
        "product_mockup" => vec![
            "blender_generate_scene_type",
            "view_image",
            "review_video",
            "export_video",
        ],
        "thumbnails" => vec!["generate_image", "view_image"],
        "business_explainer" => vec![
            "blender_generate_scene_type",
            "manim_execute_script",
            "merge_videos",
            "generate_text_to_speech",
            "review_video",
            "export_video",
        ],
        "voice_audio" => vec!["generate_text_to_speech", "generate_music"],
        _ => return None,
    };
    Some(names)
}

/// The `search_tools` meta-tool declaration. When scoped mode is active this is
/// always advertised; executing it returns matching tools from the FULL catalog
/// and dynamically expands the active toolset for subsequent turns.
pub fn search_tools_declaration() -> FunctionDeclaration {
    FunctionDeclaration {
        name: "search_tools".to_string(),
        description: (
            "Search the FULL tool catalog (~200 tools) by keyword when you need a \
             capability that is not in your current active toolset. Returns the top \
             matching tools with their names and descriptions; matched tools are \
             automatically added to your active toolset and become callable on your \
             next turn. Use this INSTEAD of guessing a tool name that was not given \
             to you."
        )
        .to_string(),
        parameters: crate::gemini_client::Parameters {
            param_type: "object".to_string(),
            properties: {
                let mut props = std::collections::HashMap::new();
                props.insert(
                    "query".to_string(),
                    crate::gemini_client::PropertyDefinition {
                        prop_type: "string".to_string(),
                        description: (
                            "Space-separated keywords describing the capability needed, \
                             e.g. 'audio fade volume', 'crop resize video', 'stock footage search'"
                        )
                        .to_string(),
                        items: None,
                    },
                );
                props.insert(
                    "limit".to_string(),
                    crate::gemini_client::PropertyDefinition {
                        prop_type: "integer".to_string(),
                        description: "Max tools to return (default 5, max 12)".to_string(),
                        items: None,
                    },
                );
                props
            },
            required: vec!["query".to_string()],
        },
    }
}

/// Lightweight keyword scoring over the full catalog (BM25-lite: term overlap
/// weighted by name-hit bonus). Pure Rust, no external index — fast enough at
/// ~223 entries that no caching is needed.
pub fn search_catalog(query: &str, limit: usize) -> Vec<(String, String)> {
    let terms: Vec<String> = query
        .split(|c: char| c.is_whitespace() || c == ',')
        .map(|t| t.trim().to_ascii_lowercase())
        .filter(|t| t.len() >= 3)
        .collect();
    if terms.is_empty() {
        return vec![];
    }

    let catalog = crate::tool_registry::ToolRegistry::gemini_tools_for_profile(
        crate::tool_registry::AgentExecutionProfile::FullProduction,
    );

    let mut scored: Vec<(f64, &FunctionDeclaration)> = catalog
        .iter()
        .map(|tool| {
            let name_lower = tool.name.to_ascii_lowercase();
            let desc_lower = tool.description.to_ascii_lowercase();
            let mut score = 0.0f64;
            for term in &terms {
                if name_lower.contains(term.as_str()) {
                    score += 3.0;
                    if name_lower.starts_with(term.as_str()) || name_lower.split('_').any(|p| p == term) {
                        score += 2.0;
                    }
                }
                // Count occurrences in description (capped to avoid spam weighting)
                let occurrences = desc_lower.matches(term.as_str()).count().min(3);
                score += occurrences as f64 * 1.0;
            }
            (score, tool)
        })
        .filter(|(score, _)| *score > 0.0)
        .collect();

    scored.sort_by(|a, b| b.0.partial_cmp(&a.0).unwrap_or(std::cmp::Ordering::Equal));

    scored
        .into_iter()
        .take(limit.clamp(1, 12))
        .map(|(_, tool)| {
            let short_desc = tool
                .description
                .split(". ")
                .next()
                .unwrap_or(&tool.description)
                .trim_end_matches('.')
                .to_string();
            (tool.name.clone(), short_desc)
        })
        .collect()
}

/// Execute a search_tools call: returns the JSON result text AND the names of
/// matched tools so the caller can expand the active toolset.
pub fn execute_search_tools(args: &std::collections::HashMap<String, serde_json::Value>) -> (String, Vec<String>) {
    let query = args
        .get("query")
        .and_then(|v| v.as_str())
        .unwrap_or("");
    let limit = args
        .get("limit")
        .and_then(|v| v.as_u64())
        .unwrap_or(5) as usize;

    if query.trim().is_empty() {
        return (
            r#"{"error": "query is required"}"#.to_string(),
            vec![],
        );
    }

    let matches = search_catalog(query, limit);
    if matches.is_empty() {
        return (
            serde_json::json!({
                "matches": [],
                "note": "No tools matched. Try different keywords."
            })
            .to_string(),
            vec![],
        );
    }

    let names: Vec<String> = matches.iter().map(|(n, _)| n.clone()).collect();
    let matches_json: Vec<serde_json::Value> = matches
        .iter()
        .map(|(name, desc)| {
            serde_json::json!({ "name": name, "description": desc })
        })
        .collect();
    let text = serde_json::json!({
        "matches": matches_json,
        "note": "These tools are NOW ACTIVE in your toolset — call them directly on your next turn."
    })
    .to_string();
    (text, names)
}

/// Build the service-scoped toolbelt: mandatory sequence + search_tools.
/// Falls back to the full belt when the scope is unknown or discovery is off.
pub fn service_scoped_tools(scope: &str) -> Vec<FunctionDeclaration> {
    if !tool_discovery_enabled() {
        return all_video_tools_with_essentials();
    }
    match service_tool_names(scope) {
        Some(names) => {
            let name_strings: Vec<String> = names.iter().map(|s| s.to_string()).collect();
            let mut tools =
                crate::tool_registry::ToolRegistry::filter_gemini_tools_for_profile(
                    crate::tool_registry::AgentExecutionProfile::FullProduction,
                    &name_strings,
                );
            tracing::info!(
                "🎯 Service-scoped toolbelt for '{}': {} tools (+search_tools)",
                scope,
                tools.len()
            );
            tools.push(search_tools_declaration());
            tools
        }
        None => {
            tracing::info!(
                "🎯 No scoped toolbelt for '{}' — falling back to full toolbelt",
                scope
            );
            all_video_tools_with_essentials()
        }
    }
}
