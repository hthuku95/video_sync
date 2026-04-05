/// NVIDIA NIM client for Gemma 4 31B text generation.
/// Uses OpenAI-compatible chat completions endpoint — 40 RPM free tier.
/// API key format: nvapi-...
/// Get a free key at https://build.nvidia.com/google/gemma-4-31b-it
use reqwest::Client;

const NVIDIA_NIM_ENDPOINT: &str = "https://integrate.api.nvidia.com/v1/chat/completions";
const NVIDIA_GEMMA_MODEL: &str = "google/gemma-4-31b-it";

#[derive(Debug, Clone)]
pub struct NvidiaNimClient {
    client: Client,
    api_key: String,
}

impl NvidiaNimClient {
    pub fn new(api_key: String) -> Self {
        Self {
            client: Client::new(),
            api_key,
        }
    }

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
                    attempt + 1, max_attempts, wait
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
}
