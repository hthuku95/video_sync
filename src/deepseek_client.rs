/// DeepSeek V4 client — text generation AND tool calling.
/// Uses OpenAI-compatible chat completions API.
/// Models: deepseek-v4-pro (top reasoning), deepseek-v4-flash (fast/cheap)
/// Docs: https://api-docs.deepseek.com/
use reqwest::Client;

const DEEPSEEK_BASE_URL: &str = "https://api.deepseek.com";
const DEEPSEEK_DEFAULT_MODEL: &str = "deepseek-v4-flash";

#[derive(Debug, Clone)]
pub struct DeepSeekClient {
    client: Client,
    api_key: String,
    model: String,
}

/// A single tool call returned by DeepSeek.
#[derive(Debug, Clone)]
pub struct DeepSeekToolCall {
    pub id: String,
    pub name: String,
    pub arguments: serde_json::Value,
}

/// Either a final text answer or one or more tool calls to execute.
#[derive(Debug)]
pub enum DeepSeekResponse {
    Text(String),
    ToolCalls(Vec<DeepSeekToolCall>),
}

impl DeepSeekClient {
    pub fn new(api_key: String) -> Self {
        let model = std::env::var("DEEPSEEK_MODEL")
            .unwrap_or_else(|_| DEEPSEEK_DEFAULT_MODEL.to_string());
        Self {
            client: Client::new(),
            api_key,
            model,
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
            "DeepSeek: no attempts made".into();

        for attempt in 0..max_attempts {
            let resp = self
                .client
                .post(format!("{}/chat/completions", DEEPSEEK_BASE_URL))
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
                    .ok_or("DeepSeek: no content in response")?
                    .to_string();
                return Ok(text);
            }

            let err_body = resp.text().await.unwrap_or_default();

            // Non-retryable: insufficient balance
            if err_body.contains("Insufficient Balance") || status.as_u16() == 402 {
                return Err(
                    format!("DeepSeek: Insufficient Balance - add credits").into(),
                );
            }

            if (status.as_u16() == 429 || status.as_u16() == 503) && attempt < max_attempts - 1 {
                let wait = 10u64 * 2u64.pow(attempt);
                tracing::warn!(
                    "⏳ DeepSeek {} (attempt {}/{}). Waiting {}s…",
                    status.as_u16(),
                    attempt + 1,
                    max_attempts,
                    wait
                );
                tokio::time::sleep(tokio::time::Duration::from_secs(wait.min(60))).await;
                last_err = format!("DeepSeek {}: {}", status.as_u16(), err_body).into();
                continue;
            }

            last_err = format!("DeepSeek error {}: {}", status, err_body).into();
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
            DeepSeekResponse,
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
            "DeepSeek: no attempts made".into();

        for attempt in 0..max_attempts {
            let resp = self
                .client
                .post(format!("{}/chat/completions", DEEPSEEK_BASE_URL))
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
                    let tool_calls: Vec<DeepSeekToolCall> = choice["message"]["tool_calls"]
                        .as_array()
                        .unwrap_or(&vec![])
                        .iter()
                        .filter_map(|tc| {
                            let id = tc["id"].as_str()?.to_string();
                            let name = tc["function"]["name"].as_str()?.to_string();
                            let args_str = tc["function"]["arguments"].as_str().unwrap_or("{}");
                            let arguments =
                                serde_json::from_str(args_str).unwrap_or(serde_json::json!({}));
                            Some(DeepSeekToolCall {
                                id,
                                name,
                                arguments,
                            })
                        })
                        .collect();
                    return Ok((DeepSeekResponse::ToolCalls(tool_calls), usage));
                }

                let text = choice["message"]["content"]
                    .as_str()
                    .unwrap_or("")
                    .to_string();
                return Ok((DeepSeekResponse::Text(text), usage));
            }

            let err_body = resp.text().await.unwrap_or_default();

            // Non-retryable: insufficient balance
            if err_body.contains("Insufficient Balance") || status.as_u16() == 402 {
                return Err(
                    format!("DeepSeek: Insufficient Balance - add credits").into(),
                );
            }

            if (status.as_u16() == 429 || status.as_u16() == 503) && attempt < max_attempts - 1 {
                let wait = 10u64 * 2u64.pow(attempt);
                tracing::warn!(
                    "⏳ DeepSeek tool call {} (attempt {}/{}). Waiting {}s…",
                    status.as_u16(),
                    attempt + 1,
                    max_attempts,
                    wait
                );
                tokio::time::sleep(tokio::time::Duration::from_secs(wait.min(60))).await;
                last_err = format!("DeepSeek {}: {}", status.as_u16(), err_body).into();
                continue;
            }

            last_err = format!("DeepSeek error {}: {}", status, err_body).into();
            if attempt < max_attempts - 1 {
                tokio::time::sleep(tokio::time::Duration::from_secs(2)).await;
            }
        }

        Err(last_err)
    }
}
