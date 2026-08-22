/// NVIDIA NIM client — text generation AND tool calling.
/// Uses OpenAI-compatible chat completions endpoint — 40 RPM free tier.
///
/// ## Model configuration
/// - `NVIDIA_NIM_MODEL` — text + tool-calling model (default: google/gemma-4-31b-it)
/// - `NVIDIA_NIM_VISION_MODEL` — vision + tool-calling model (default: nvidia/nemotron-3-nano-omni-30b-a3b-reasoning)
///
/// Only models with tool-calling support are listed below. Models that support
/// vision or audio are flagged accordingly.
///
/// | Model ID | Tools | Vision | Audio |
/// |---|---|---|---|
/// | google/gemma-4-31b-it | ✅ | ❌ | ❌ |
/// | nvidia/nemotron-3-super-120b-a12b | ✅ | ❌ | ❌ |
/// | meta/llama-3.3-70b-instruct | ✅ | ❌ | ❌ |
/// | mistralai/mistral-nemo | ✅ | ❌ | ❌ |
/// | qwen/qwen3-coder-480b | ✅ | ❌ | ❌ |
/// | minimaxai/minimax-m2.7 | ✅ | ❌ | ❌ |
/// | moonshotai/kimi-k2.5 | ✅ | ❌ | ❌ |
/// | z-ai/glm-5.1 | ✅ | ❌ | ❌ |
/// | nvidia/nemotron-3-nano-omni-30b-a3b-reasoning | ✅ | ✅ | ✅ |
///
/// Get API key: https://build.nvidia.com/settings/api-keys
use base64::Engine as _;
use reqwest::Client;

const NVIDIA_NIM_ENDPOINT: &str = "https://integrate.api.nvidia.com/v1/chat/completions";
const NVIDIA_FLUX_IMAGE_ENDPOINT: &str =
    "https://ai.api.nvidia.com/v1/genai/black-forest-labs/flux.1-dev";
const NVIDIA_DEFAULT_MODEL: &str = "meta/llama-3.3-70b-instruct";
const NVIDIA_DEFAULT_VISION_MODEL: &str = "nvidia/nemotron-3-nano-omni-30b-a3b-reasoning";

/// Generate an image with FLUX.1-dev via NVIDIA's hosted NIM API (free tier).
/// Returns decoded image bytes (JPEG, 1024x1024). Verified working Aug 22 2026
/// (~6s per image). Used as the PRIMARY background-image provider so UI backgrounds
/// don't depend on Gemini prepaid credits; Gemini remains a fallback.
pub async fn generate_image_flux(api_key: &str, prompt: &str) -> Result<Vec<u8>, String> {
    if api_key.trim().is_empty() {
        return Err("NVIDIA_API_KEY not configured".to_string());
    }

    let client = Client::builder()
        .timeout(std::time::Duration::from_secs(90))
        .build()
        .map_err(|e| format!("HTTP client build failed: {}", e))?;

    let body = serde_json::json!({
        "prompt": prompt,
        "mode": "base",
        "seed": 0,
        "steps": 30,
    });

    let resp = client
        .post(NVIDIA_FLUX_IMAGE_ENDPOINT)
        .header("Authorization", format!("Bearer {}", api_key))
        .header("Accept", "application/json")
        .json(&body)
        .send()
        .await
        .map_err(|e| format!("NIM image request failed: {}", e))?;

    let status = resp.status();
    if !status.is_success() {
        let err_text = resp.text().await.unwrap_or_default();
        return Err(format!("NIM image generation {} : {}", status, err_text));
    }

    let payload: serde_json::Value = resp
        .json()
        .await
        .map_err(|e| format!("NIM image response parse failed: {}", e))?;

    let b64 = payload
        .get("artifacts")
        .and_then(|a| a.get(0))
        .and_then(|a| a.get("base64"))
        .and_then(|v| v.as_str())
        .ok_or_else(|| "NIM image response missing artifacts[0].base64".to_string())?;

    let bytes = base64::engine::general_purpose::STANDARD
        .decode(b64)
        .map_err(|e| format!("NIM image base64 decode failed: {}", e))?;

    if bytes.len() < 100 {
        return Err("NIM generated image too small".to_string());
    }

    Ok(bytes)
}


// ─── Model capabilities ─────────────────────────────────────────────────────

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct NimCapabilities {
    pub tool_calling: bool,
    pub vision: bool,
    pub audio: bool,
}

impl NimCapabilities {
    const fn text_tools() -> Self {
        Self { tool_calling: true, vision: false, audio: false }
    }
    const fn omni() -> Self {
        Self { tool_calling: true, vision: true, audio: true }
    }
}

/// Known NIM model with capability metadata.
#[derive(Debug, Clone)]
pub struct NimModelInfo {
    pub id: &'static str,
    pub name: &'static str,
    pub capabilities: NimCapabilities,
}

pub const NIM_MODELS: &[NimModelInfo] = &[
    // Tool-calling text models
    NimModelInfo { id: "google/gemma-4-31b-it", name: "Gemma 4 31B", capabilities: NimCapabilities::text_tools() },
    NimModelInfo { id: "nvidia/nemotron-3-super-120b-a12b", name: "Nemotron 3 Super 120B", capabilities: NimCapabilities::text_tools() },
    NimModelInfo { id: "meta/llama-3.3-70b-instruct", name: "Llama 3.3 70B", capabilities: NimCapabilities::text_tools() },
    NimModelInfo { id: "mistralai/mistral-nemo", name: "Mistral Nemo", capabilities: NimCapabilities::text_tools() },
    NimModelInfo { id: "qwen/qwen3-coder-480b", name: "Qwen3 Coder 480B", capabilities: NimCapabilities::text_tools() },
    NimModelInfo { id: "minimaxai/minimax-m2.7", name: "MiniMax M2.7", capabilities: NimCapabilities::text_tools() },
    NimModelInfo { id: "moonshotai/kimi-k2.5", name: "Kimi K2.5", capabilities: NimCapabilities::text_tools() },
    NimModelInfo { id: "z-ai/glm-5.1", name: "GLM 5.1", capabilities: NimCapabilities::text_tools() },
    // Vision + tool-calling models (Gemini multimodal fallback)
    NimModelInfo { id: "nvidia/nemotron-3-nano-omni-30b-a3b-reasoning", name: "Nemotron Omni 30B", capabilities: NimCapabilities::omni() },
];

pub fn lookup_model(model_id: &str) -> Option<&'static NimModelInfo> {
    NIM_MODELS.iter().find(|m| m.id == model_id)
}

pub fn infer_capabilities(model_id: &str) -> NimCapabilities {
    lookup_model(model_id).map(|m| m.capabilities).unwrap_or(NimCapabilities::text_tools())
}

// ─── Client ─────────────────────────────────────────────────────────────────

#[derive(Debug, Clone)]
pub struct NvidiaNimClient {
    client: Client,
    api_key: String,
    model: String,
    pub capabilities: NimCapabilities,
}

/// A single tool call returned by a NIM model.
#[derive(Debug, Clone)]
pub struct NimToolCall {
    pub id: String,
    pub name: String,
    pub arguments: serde_json::Value,
}

/// Either a final text answer or one or more tool calls to execute.
#[derive(Debug)]
pub enum NimResponse {
    Text(String),
    ToolCalls(Vec<NimToolCall>),
}

impl NvidiaNimClient {
    /// Create a new text + tool-calling NIM client.
    /// Model from `NVIDIA_NIM_MODEL` env var (default: google/gemma-4-31b-it).
    pub fn new(api_key: String) -> Self {
        let model = std::env::var("NVIDIA_NIM_MODEL")
            .unwrap_or_else(|_| NVIDIA_DEFAULT_MODEL.to_string());
        let capabilities = infer_capabilities(&model);
        tracing::info!(
            "NVIDIA NIM text client initialized: {} (tools={}, vision={}, audio={})",
            model, capabilities.tool_calling, capabilities.vision, capabilities.audio
        );
        Self {
            client: Client::builder()
                .timeout(std::time::Duration::from_secs(60))
                .build()
                .unwrap_or_default(),
            api_key,
            model,
            capabilities,
        }
    }

    /// Create a NIM client with an explicit model (bypasses env var).
    pub fn with_model(api_key: String, model: String) -> Self {
        let capabilities = infer_capabilities(&model);
        tracing::info!(
            "NVIDIA NIM client initialized: {} (tools={}, vision={}, audio={})",
            model, capabilities.tool_calling, capabilities.vision, capabilities.audio
        );
        Self {
            client: Client::builder()
                .timeout(std::time::Duration::from_secs(60))
                .build()
                .unwrap_or_default(),
            api_key,
            model,
            capabilities,
        }
    }

    /// Create a vision-capable NIM client from `NVIDIA_NIM_VISION_MODEL` env var.
    pub fn new_vision(api_key: String) -> Self {
        let model = std::env::var("NVIDIA_NIM_VISION_MODEL")
            .unwrap_or_else(|_| NVIDIA_DEFAULT_VISION_MODEL.to_string());
        let capabilities = infer_capabilities(&model);
        tracing::info!(
            "NVIDIA NIM vision client initialized: {} (tools={}, vision={}, audio={})",
            model, capabilities.tool_calling, capabilities.vision, capabilities.audio
        );
        Self {
            client: Client::new(),
            api_key,
            model,
            capabilities,
        }
    }

    // ─── Content formatting helpers ──────────────────────────────────────

    /// Build an OpenAI vision content-parts array from text + optional image bytes.
    fn build_vision_content(text: &str, image_bytes: Option<&[u8]>) -> serde_json::Value {
        match image_bytes {
            Some(bytes) => {
                let mime_type = detect_mime(bytes);
                let b64 = base64::engine::general_purpose::STANDARD.encode(bytes);
                let data_uri = format!("data:{};base64,{}", mime_type, b64);
                serde_json::json!([
                    {"type": "text", "text": text},
                    {"type": "image_url", "image_url": {"url": data_uri}},
                ])
            }
            None => serde_json::json!(text),
        }
    }

    /// Build a full user message with vision content.
    pub fn vision_message(text: &str, image_bytes: &[u8]) -> serde_json::Value {
        serde_json::json!({
            "role": "user",
            "content": Self::build_vision_content(text, Some(image_bytes)),
        })
    }

    // ─── Plain text generation ───────────────────────────────────────────

    pub async fn generate_text(
        &self,
        prompt: &str,
    ) -> Result<String, Box<dyn std::error::Error + Send + Sync>> {
        self.generate_text_with_tokens(prompt, 1024).await
    }

    pub async fn generate_text_with_tokens(
        &self,
        prompt: &str,
        max_tokens: u32,
    ) -> Result<String, Box<dyn std::error::Error + Send + Sync>> {
        let body = serde_json::json!({
            "model": self.model,
            "messages": [{"role": "user", "content": prompt}],
            "max_tokens": max_tokens,
            "temperature": 0.1,
        });

        let max_attempts = 3u32;
        let mut last_err: Box<dyn std::error::Error + Send + Sync> =
            "NVIDIA NIM: no attempts made".into();

        for attempt in 0..max_attempts {
            let resp = match self
                .client
                .post(NVIDIA_NIM_ENDPOINT)
                .header("Authorization", format!("Bearer {}", self.api_key))
                .header("Content-Type", "application/json")
                .json(&body)
                .send()
                .await
            {
                Ok(r) => r,
                Err(e) => {
                    let is_last = attempt == max_attempts - 1;
                    if is_last {
                        return Err(format!("NVIDIA NIM connection error: {e}").into());
                    }
                    let backoff = 5u64 * (attempt as u64 + 1);
                    tracing::warn!(
                        "NVIDIA NIM connection error (attempt {}/{}, retry in {}s): {e}",
                        attempt + 1, max_attempts, backoff
                    );
                    tokio::time::sleep(tokio::time::Duration::from_secs(backoff)).await;
                    last_err = format!("NVIDIA NIM connection error: {e}").into();
                    continue;
                }
            };

            let status = resp.status();

            if status.is_success() {
                let json: serde_json::Value = match resp.json().await {
                    Ok(j) => j,
                    Err(e) => {
                        last_err = format!("NVIDIA NIM parse error: {e}").into();
                        if attempt < max_attempts - 1 {
                            tokio::time::sleep(tokio::time::Duration::from_secs(2)).await;
                        }
                        continue;
                    }
                };
                let text = json["choices"][0]["message"]["content"]
                    .as_str()
                    .map(|s| s.to_string())
                    .unwrap_or_default();
                return Ok(text);
            }

            let err_body = resp.text().await.unwrap_or_default();

            if status.as_u16() == 429 && attempt < max_attempts - 1 {
                let wait = 2u64;
                tracing::warn!(
                    "⏳ NVIDIA NIM 429 (attempt {}/{}). Waiting {wait}s…",
                    attempt + 1, max_attempts
                );
                tokio::time::sleep(tokio::time::Duration::from_secs(wait)).await;
                last_err = format!("NVIDIA NIM 429: {}", err_body).into();
                continue;
            }

            last_err = format!("NVIDIA NIM error {}: {}", status, err_body).into();
            if attempt < max_attempts - 1 {
                tokio::time::sleep(tokio::time::Duration::from_secs(2)).await;
            }
        }

        Err(last_err)
    }

    // ─── Multimodal analysis ─────────────────────────────────────────────

    /// Analyze an image from bytes using a vision-capable NIM model.
    pub async fn analyze_image_bytes(
        &self,
        image_bytes: &[u8],
        analysis_prompt: &str,
    ) -> Result<String, Box<dyn std::error::Error + Send + Sync>> {
        let content = Self::build_vision_content(analysis_prompt, Some(image_bytes));

        let body = serde_json::json!({
            "model": self.model,
            "messages": [{"role": "user", "content": content}],
            "max_tokens": 2048,
            "temperature": 0.3,
        });

        let resp = self
            .client
            .post(NVIDIA_NIM_ENDPOINT)
            .header("Authorization", format!("Bearer {}", self.api_key))
            .header("Content-Type", "application/json")
            .json(&body)
            .send()
            .await?;

        let status = resp.status();
        if !status.is_success() {
            let err_body = resp.text().await.unwrap_or_default();
            return Err(format!("NVIDIA NIM vision error {}: {}", status, err_body).into());
        }

        let json: serde_json::Value = resp.json().await?;
        let text = json["choices"][0]["message"]["content"]
            .as_str()
            .unwrap_or("")
            .to_string();

        if text.is_empty() {
            Err("NVIDIA NIM vision: empty response".into())
        } else {
            Ok(text)
        }
    }

    /// Analyze audio from bytes using a vision/audio-capable NIM model.
    pub async fn analyze_audio_bytes(
        &self,
        audio_bytes: &[u8],
        mime_type: &str,
        analysis_prompt: &str,
    ) -> Result<String, Box<dyn std::error::Error + Send + Sync>> {
        let b64 = base64::engine::general_purpose::STANDARD.encode(audio_bytes);
        let data_uri = format!("data:{};base64,{}", mime_type, b64);

        let content = serde_json::json!([
            {"type": "text", "text": analysis_prompt},
            {"type": "image_url", "image_url": {"url": data_uri}},
        ]);

        let body = serde_json::json!({
            "model": self.model,
            "messages": [{"role": "user", "content": content}],
            "max_tokens": 2048,
            "temperature": 0.2,
        });

        let resp = self
            .client
            .post(NVIDIA_NIM_ENDPOINT)
            .header("Authorization", format!("Bearer {}", self.api_key))
            .header("Content-Type", "application/json")
            .json(&body)
            .send()
            .await?;

        let status = resp.status();
        if !status.is_success() {
            let err_body = resp.text().await.unwrap_or_default();
            return Err(format!("NVIDIA NIM audio error {}: {}", status, err_body).into());
        }

        let json: serde_json::Value = resp.json().await?;
        let text = json["choices"][0]["message"]["content"]
            .as_str()
            .unwrap_or("")
            .to_string();

        if text.is_empty() {
            Err("NVIDIA NIM audio: empty response".into())
        } else {
            Ok(text)
        }
    }

    // ─── Tool calling ────────────────────────────────────────────────────

    fn to_openai_tools(decls: &[crate::gemini_client::FunctionDeclaration]) -> serde_json::Value {
        let tools: Vec<serde_json::Value> = decls
            .iter()
            .map(|d| {
                let props: serde_json::Map<String, serde_json::Value> = d
                    .parameters
                    .properties
                    .iter()
                    .map(|(k, v)| {
                        let mut prop = serde_json::json!({
                            "type": v.prop_type,
                            "description": v.description,
                        });
                        if let Some(ref items) = v.items {
                            prop["items"] = serde_json::json!({ "type": items });
                        }
                        (k.clone(), prop)
                    })
                    .collect();

                serde_json::json!({
                    "type": "function",
                    "function": {
                        "name": d.name,
                        "description": d.description,
                        "parameters": {
                            "type": "object",
                            "properties": props,
                            "required": d.parameters.required,
                        }
                    }
                })
            })
            .collect();

        serde_json::Value::Array(tools)
    }

    /// Single API call with tool definitions.
    /// Messages should be OpenAI-format array. For vision content, use `vision_message()`
    /// or build content-parts arrays in the message.
    pub async fn generate_single(
        &self,
        messages: &[serde_json::Value],
        tools: &[crate::gemini_client::FunctionDeclaration],
    ) -> Result<NimResponse, Box<dyn std::error::Error + Send + Sync>> {
        let openai_tools = Self::to_openai_tools(tools);

        let body = serde_json::json!({
            "model": self.model,
            "messages": messages,
            "tools": openai_tools,
            "tool_choice": "auto",
            "max_tokens": 2048,
            "temperature": 0.5,
        });

        let max_attempts = 3u32;
        let mut last_err: Box<dyn std::error::Error + Send + Sync> =
            "NVIDIA NIM: no attempts made".into();

        for attempt in 0..max_attempts {
            let resp = match self
                .client
                .post(NVIDIA_NIM_ENDPOINT)
                .header("Authorization", format!("Bearer {}", self.api_key))
                .header("Content-Type", "application/json")
                .json(&body)
                .send()
                .await
            {
                Ok(r) => r,
                Err(e) => {
                    let is_last = attempt == max_attempts - 1;
                    if is_last {
                        return Err(format!("NVIDIA NIM connection error: {e}").into());
                    }
                    let backoff = 5u64 * (attempt as u64 + 1);
                    tracing::warn!(
                        "NVIDIA NIM connection error (attempt {}/{}, retry in {}s): {e}",
                        attempt + 1, max_attempts, backoff
                    );
                    tokio::time::sleep(tokio::time::Duration::from_secs(backoff)).await;
                    last_err = format!("NVIDIA NIM connection error: {e}").into();
                    continue;
                }
            };

            let status = resp.status();

            if status.is_success() {
                let json: serde_json::Value = match resp.json().await {
                    Ok(j) => j,
                    Err(e) => {
                        last_err = format!("NVIDIA NIM parse error: {e}").into();
                        if attempt < max_attempts - 1 {
                            tokio::time::sleep(tokio::time::Duration::from_secs(2)).await;
                        }
                        continue;
                    }
                };
                let choice = &json["choices"][0];
                let finish_reason = choice["finish_reason"].as_str().unwrap_or("");

                if finish_reason == "tool_calls" {
                    let tool_calls: Vec<NimToolCall> = choice["message"]["tool_calls"]
                        .as_array()
                        .unwrap_or(&vec![])
                        .iter()
                        .filter_map(|tc| {
                            let id = tc["id"].as_str()?.to_string();
                            let name = tc["function"]["name"].as_str()?.to_string();
                            let args_str = tc["function"]["arguments"].as_str().unwrap_or("{}");
                            let arguments =
                                serde_json::from_str(args_str).unwrap_or(serde_json::json!({}));
                            Some(NimToolCall { id, name, arguments })
                        })
                        .collect();
                    return Ok(NimResponse::ToolCalls(tool_calls));
                }

                let text = choice["message"]["content"]
                    .as_str()
                    .unwrap_or("")
                    .to_string();
                return Ok(NimResponse::Text(text));
            }

            let err_body = resp.text().await.unwrap_or_default();

            if status.as_u16() == 429 && attempt < max_attempts - 1 {
                tracing::warn!(
                    "⏳ NVIDIA NIM tool call 429 (attempt {}/{}). Waiting 15s…",
                    attempt + 1, max_attempts
                );
                tokio::time::sleep(tokio::time::Duration::from_secs(15)).await;
                last_err = format!("NVIDIA NIM 429: {}", err_body).into();
                continue;
            }

            last_err = format!("NVIDIA NIM error {}: {}", status, err_body).into();
            if attempt < max_attempts - 1 {
                tokio::time::sleep(tokio::time::Duration::from_secs(2)).await;
            }
        }

        Err(last_err)
    }
}

// ─── Helpers ────────────────────────────────────────────────────────────────

fn detect_mime(bytes: &[u8]) -> &'static str {
    if bytes.starts_with(&[0xFF, 0xD8, 0xFF]) {
        "image/jpeg"
    } else if bytes.starts_with(&[0x89, 0x50, 0x4E, 0x47]) {
        "image/png"
    } else if bytes.starts_with(&[0x47, 0x49, 0x46]) {
        "image/gif"
    } else if bytes.starts_with(&[0x52, 0x49, 0x46, 0x46]) {
        "image/webp"
    } else {
        "image/png"
    }
}
