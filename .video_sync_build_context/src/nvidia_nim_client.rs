/// NVIDIA NIM client for Gemma 4 31B — text generation AND tool calling.
/// Uses OpenAI-compatible chat completions endpoint — 40 RPM free tier.
/// Gemma 4 has native function calling via special tokens (not prompt engineering).
/// Docs: https://ai.google.dev/gemma/docs/capabilities/text/function-calling-gemma4
/// API key: https://build.nvidia.com/google/gemma-4-31b-it
use reqwest::Client;

const NVIDIA_NIM_ENDPOINT: &str = "https://integrate.api.nvidia.com/v1/chat/completions";
const NVIDIA_GEMMA_MODEL: &str = "google/gemma-4-31b-it";

#[derive(Debug, Clone)]
pub struct NvidiaNimClient {
    client: Client,
    api_key: String,
}

/// A single tool call returned by Gemma 4.
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
    pub fn new(api_key: String) -> Self {
        Self {
            client: Client::new(),
            api_key,
        }
    }

    // ─── Plain text generation ────────────────────────────────────────────────

    /// Generate text using Gemma 4 31B via NVIDIA NIM.
    /// Falls back gracefully on 429 (rate limited) after 2 retries.
    pub async fn generate_text(
        &self,
        prompt: &str,
    ) -> Result<String, Box<dyn std::error::Error + Send + Sync>> {
        self.generate_text_with_tokens(prompt, 1024).await
    }

    /// Generate text with a custom max_tokens limit.
    pub async fn generate_text_with_tokens(
        &self,
        prompt: &str,
        max_tokens: u32,
    ) -> Result<String, Box<dyn std::error::Error + Send + Sync>> {
        let body = serde_json::json!({
            "model": NVIDIA_GEMMA_MODEL,
            "messages": [{"role": "user", "content": prompt}],
            "max_tokens": max_tokens,
            "temperature": 0.1,
        });

        let max_attempts = 3u32;
        let mut last_err: Box<dyn std::error::Error + Send + Sync> =
            "NVIDIA NIM: no attempts made".into();

        for attempt in 0..max_attempts {
            let resp = self
                .client
                .post(NVIDIA_NIM_ENDPOINT)
                .header("Authorization", format!("Bearer {}", self.api_key))
                .header("Content-Type", "application/json")
                .json(&body)
                .send()
                .await?;

            let status = resp.status();

            if status.is_success() {
                let json: serde_json::Value = resp.json().await?;
                let text = json["choices"][0]["message"]["content"]
                    .as_str()
                    .ok_or("NVIDIA NIM: no content in response")?
                    .to_string();
                return Ok(text);
            }

            let err_body = resp.text().await.unwrap_or_default();

            if status.as_u16() == 429 && attempt < max_attempts - 1 {
                let wait = 15u64; // NVIDIA's 40 RPM = retry after ~15s
                tracing::warn!(
                    "⏳ NVIDIA NIM 429 (attempt {}/{}). Waiting {}s…",
                    attempt + 1,
                    max_attempts,
                    wait
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

    // ─── Tool calling (Gemma 4 native function calling) ───────────────────────

    /// Convert Gemini-format FunctionDeclarations to OpenAI-format tools array.
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
                        // Preserve array item types if present
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
    /// Returns `NimResponse::Text` when Gemma answers directly, or
    /// `NimResponse::ToolCalls` when it wants to call tools.
    pub async fn generate_single(
        &self,
        messages: &[serde_json::Value],
        tools: &[crate::gemini_client::FunctionDeclaration],
    ) -> Result<NimResponse, Box<dyn std::error::Error + Send + Sync>> {
        let openai_tools = Self::to_openai_tools(tools);

        let body = serde_json::json!({
            "model": NVIDIA_GEMMA_MODEL,
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
            let resp = self
                .client
                .post(NVIDIA_NIM_ENDPOINT)
                .header("Authorization", format!("Bearer {}", self.api_key))
                .header("Content-Type", "application/json")
                .json(&body)
                .send()
                .await?;

            let status = resp.status();

            if status.is_success() {
                let json: serde_json::Value = resp.json().await?;
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
                            Some(NimToolCall {
                                id,
                                name,
                                arguments,
                            })
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
                    attempt + 1,
                    max_attempts
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
