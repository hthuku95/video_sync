/// Qwen client via Alibaba Cloud DashScope (OpenAI-compatible API).
/// Uses DashScope's `compatible-mode/v1/chat/completions` endpoint so the
/// request/response shape is identical to DeepSeek/OpenAI.
/// Models: qwen3.7-plus (default, multimodal text+image+video, overridable via QWEN_MODEL).
/// Docs: https://help.aliyun.com/zh/model-studio/compatibility-of-openai-with-dashscope
use reqwest::Client;

/// Default base URL is the International (`intl`) endpoint — the account in use
/// is an Alibaba Cloud International account (eu-central-1). Overridable via
/// `QWEN_BASE_URL` (e.g. set to `https://dashscope.aliyuncs.com/compatible-mode/v1`
/// for a mainland-China account).
const QWEN_DEFAULT_BASE_URL: &str =
    "https://dashscope-intl.aliyuncs.com/compatible-mode/v1";
fn qwen_base_url() -> String {
    std::env::var("QWEN_BASE_URL").unwrap_or_else(|_| QWEN_DEFAULT_BASE_URL.to_string())
}
const QWEN_DEFAULT_MODEL: &str = "qwen3.7-plus";

/// DashScope multimodal embedding REST endpoint (NOT the OpenAI-compatible
/// `/embeddings` route — that only serves text-only embedders). Used by
/// `embed_contents` below. `dashscope-intl` = Alibaba Cloud International.
const QWEN_MULTIMODAL_EMBEDDING_URL: &str =
    "https://dashscope-intl.aliyuncs.com/api/v1/services/embeddings/multimodal-embedding/multimodal-embedding";
const QWEN_DEFAULT_EMBEDDING_MODEL: &str = "qwen3-vl-embedding";

#[derive(Debug, Clone)]
pub struct QwenClient {
    client: Client,
    api_key: String,
    model: String,
    embedding_model: String,
}

/// A single tool call returned by Qwen.
#[derive(Debug, Clone)]
pub struct QwenToolCall {
    pub id: String,
    pub name: String,
    pub arguments: serde_json::Value,
}

/// Either a final text answer or one or more tool calls to execute.
#[derive(Debug)]
pub enum QwenResponse {
    Text(String),
    ToolCalls(Vec<QwenToolCall>),
}

impl QwenClient {
    pub fn new(api_key: String) -> Self {
        let model = std::env::var("QWEN_MODEL")
            .unwrap_or_else(|_| QWEN_DEFAULT_MODEL.to_string());
        let embedding_model = std::env::var("QWEN_EMBEDDING_MODEL")
            .unwrap_or_else(|_| QWEN_DEFAULT_EMBEDDING_MODEL.to_string());
        Self {
            client: Client::new(),
            api_key,
            model,
            embedding_model,
        }
    }

    // ─── Plain text generation ────────────────────────────────────────────────

    pub async fn generate_text(
        &self,
        prompt: &str,
    ) -> Result<String, Box<dyn std::error::Error + Send + Sync>> {
        self.generate_text_with_tokens(prompt, 2048).await
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
            "Qwen: no attempts made".into();

        for attempt in 0..max_attempts {
            let resp = self
                .client
                .post(format!("{}/chat/completions", qwen_base_url()))
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
                    .ok_or("Qwen: no content in response")?
                    .to_string();
                return Ok(text);
            }

            let err_body = resp.text().await.unwrap_or_default();

            // Non-retryable: auth / insufficient balance / invalid model
            if status.as_u16() == 401
                || status.as_u16() == 402
                || status.as_u16() == 400
            {
                return Err(format!("Qwen {}: {}", status.as_u16(), err_body).into());
            }

            if (status.as_u16() == 429 || status.as_u16() == 503)
                && attempt < max_attempts - 1
            {
                let wait = 10u64 * 2u64.pow(attempt);
                tracing::warn!(
                    "⏳ Qwen {} (attempt {}/{}). Waiting {}s…",
                    status.as_u16(),
                    attempt + 1,
                    max_attempts,
                    wait
                );
                tokio::time::sleep(tokio::time::Duration::from_secs(wait.min(60))).await;
                last_err = format!("Qwen {}: {}", status.as_u16(), err_body).into();
                continue;
            }

            last_err = format!("Qwen error {}: {}", status, err_body).into();
            if attempt < max_attempts - 1 {
                tokio::time::sleep(tokio::time::Duration::from_secs(2)).await;
            }
        }

        Err(last_err)
    }

    // ─── Tool calling ─────────────────────────────────────────────────────────

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

    pub async fn generate_single(
        &self,
        messages: &[serde_json::Value],
        tools: &[crate::gemini_client::FunctionDeclaration],
    ) -> Result<
        (
            QwenResponse,
            crate::usage::UsageInfo,
        ),
        Box<dyn std::error::Error + Send + Sync>,
    > {
        let openai_tools = Self::to_openai_tools(tools);

        let body = serde_json::json!({
            "model": self.model,
            "messages": messages,
            "tools": openai_tools,
            "tool_choice": "auto",
            "max_tokens": 4096,
            "temperature": 0.5,
        });

        let max_attempts = 3u32;
        let mut last_err: Box<dyn std::error::Error + Send + Sync> =
            "Qwen: no attempts made".into();

        for attempt in 0..max_attempts {
            let resp = self
                .client
                .post(format!("{}/chat/completions", qwen_base_url()))
                .header("Authorization", format!("Bearer {}", self.api_key))
                .header("Content-Type", "application/json")
                .json(&body)
                .send()
                .await?;

            let status = resp.status();

            if status.is_success() {
                let json: serde_json::Value = resp.json().await?;
                let usage = crate::usage::UsageInfo::from_openai(&json);
                let choice = &json["choices"][0];
                let finish_reason = choice["finish_reason"].as_str().unwrap_or("");

                if finish_reason == "tool_calls" {
                    let tool_calls: Vec<QwenToolCall> = choice["message"]["tool_calls"]
                        .as_array()
                        .unwrap_or(&vec![])
                        .iter()
                        .filter_map(|tc| {
                            let id = tc["id"].as_str()?.to_string();
                            let name = tc["function"]["name"].as_str()?.to_string();
                            let args_str = tc["function"]["arguments"].as_str().unwrap_or("{}");
                            let arguments =
                                serde_json::from_str(args_str).unwrap_or(serde_json::json!({}));
                            Some(QwenToolCall {
                                id,
                                name,
                                arguments,
                            })
                        })
                        .collect();
                    return Ok((QwenResponse::ToolCalls(tool_calls), usage));
                }

                let text = choice["message"]["content"]
                    .as_str()
                    .unwrap_or("")
                    .to_string();
                return Ok((QwenResponse::Text(text), usage));
            }

            let err_body = resp.text().await.unwrap_or_default();

            // Non-retryable: auth / insufficient balance / invalid model
            if status.as_u16() == 401
                || status.as_u16() == 402
                || status.as_u16() == 400
            {
                return Err(format!("Qwen {}: {}", status.as_u16(), err_body).into());
            }

            if (status.as_u16() == 429 || status.as_u16() == 503)
                && attempt < max_attempts - 1
            {
                let wait = 10u64 * 2u64.pow(attempt);
                tracing::warn!(
                    "⏳ Qwen tool call {} (attempt {}/{}). Waiting {}s…",
                    status.as_u16(),
                    attempt + 1,
                    max_attempts,
                    wait
                );
                tokio::time::sleep(tokio::time::Duration::from_secs(wait.min(60))).await;
                last_err = format!("Qwen {}: {}", status.as_u16(), err_body).into();
                continue;
            }

            last_err = format!("Qwen error {}: {}", status, err_body).into();
            if attempt < max_attempts - 1 {
                tokio::time::sleep(tokio::time::Duration::from_secs(2)).await;
            }
        }

        Err(last_err)
    }

    // ─── Multimodal embedding (DashScope REST) ─────────────────────────────────
    //
    // Uses `qwen3-vl-embedding` via the DashScope multimodal-embedding REST
    // endpoint (NOT the OpenAI-compatible /embeddings route). Accepts text,
    // image URLs, and video URLs as input contents. `dimension` defaults to
    // 1536 to stay Qdrant-compatible with gemini-embedding-2. Fused vectors
    // (`enable_fusion=true`) return a single `embedding` array.

    /// Embed a list of content parts: `{"text": "..."}`, `{"image": "https://..."}`,
    /// or `{"video": "https://..."}`. Returns the fused 1536-d vector.
    pub async fn embed_contents(
        &self,
        contents: &[serde_json::Value],
        dimension: Option<usize>,
    ) -> Result<Vec<f32>, Box<dyn std::error::Error + Send + Sync>> {
        let dimension = dimension.unwrap_or(1536);
        let body = serde_json::json!({
            "model": self.embedding_model,
            "input": { "contents": contents },
            "enable_fusion": true,
            "parameters": { "dimension": dimension },
        });

        let max_attempts = 3u32;
        let mut last_err: Box<dyn std::error::Error + Send + Sync> =
            "Qwen: no embedding attempts made".into();

        for attempt in 0..max_attempts {
            let resp = self
                .client
                .post(QWEN_MULTIMODAL_EMBEDDING_URL)
                .header("Authorization", format!("Bearer {}", self.api_key))
                .header("Content-Type", "application/json")
                .json(&body)
                .send()
                .await?;

            let status = resp.status();

            if status.is_success() {
                let json: serde_json::Value = resp.json().await?;
                // Fused output: output.embeddings[0].embedding
                let fused = json["output"]["embeddings"][0]["embedding"]
                    .as_array()
                    .cloned();
                let vec = if let Some(fused) = fused {
                    fused
                        .iter()
                        .filter_map(|v| v.as_f64().map(|f| f as f32))
                        .collect::<Vec<f32>>()
                } else {
                    // Fallback: merge independent per-modality vectors in a
                    // fixed order (text, image, video) — matches Gemini-embedding-2
                    // single-vector semantics as closely as possible.
                    let mut merged: Vec<f32> = Vec::new();
                    for key in ["text_embedding", "image_embedding", "video_embedding"] {
                        if let Some(arr) = json["output"]["embeddings"][0][key].as_array() {
                            merged.extend(
                                arr.iter()
                                    .filter_map(|v| v.as_f64().map(|f| f as f32)),
                            );
                        }
                    }
                    merged
                };
                if vec.is_empty() {
                    return Err(format!(
                        "Qwen embedding: empty vector (raw: {})",
                        json
                    )
                    .into());
                }
                return Ok(vec);
            }

            let err_body = resp.text().await.unwrap_or_default();

            if status.as_u16() == 401
                || status.as_u16() == 402
                || status.as_u16() == 400
            {
                return Err(format!("Qwen embedding {}: {}", status.as_u16(), err_body).into());
            }

            if (status.as_u16() == 429 || status.as_u16() == 503)
                && attempt < max_attempts - 1
            {
                let wait = 10u64 * 2u64.pow(attempt);
                tracing::warn!(
                    "⏳ Qwen embedding {} (attempt {}/{}). Waiting {}s…",
                    status.as_u16(),
                    attempt + 1,
                    max_attempts,
                    wait
                );
                tokio::time::sleep(tokio::time::Duration::from_secs(wait.min(60))).await;
                last_err = format!("Qwen embedding {}: {}", status.as_u16(), err_body).into();
                continue;
            }

            last_err = format!("Qwen embedding error {}: {}", status, err_body).into();
            if attempt < max_attempts - 1 {
                tokio::time::sleep(tokio::time::Duration::from_secs(2)).await;
            }
        }

        Err(last_err)
    }

    /// Convenience wrapper: embed a plain-text query (still routed through the
    /// multimodal `qwen3-vl-embedding` REST endpoint so it stays multimodal).
    pub async fn embed_text(
        &self,
        text: &str,
        dimension: Option<usize>,
    ) -> Result<Vec<f32>, Box<dyn std::error::Error + Send + Sync>> {
        let contents = vec![serde_json::json!({ "text": text })];
        self.embed_contents(&contents, dimension).await
    }
}