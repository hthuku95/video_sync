/// Ollama client — self-hosted LLM via Ollama OpenAI-compatible API.
/// Default model: gemma3:4b (multimodal, fast, ~3.3GB).
/// Configurable via OLLAMA_BASE_URL and OLLAMA_MODEL env vars.
use reqwest::Client;

const OLLAMA_DEFAULT_URL: &str = "http://172.31.42.118:11434";
const OLLAMA_DEFAULT_MODEL: &str = "gemma3:4b";

#[derive(Debug, Clone)]
pub struct OllamaClient {
    client: Client,
    base_url: String,
    model: String,
}

impl OllamaClient {
    pub fn new() -> Self {
        let base_url = std::env::var("OLLAMA_BASE_URL")
            .unwrap_or_else(|_| OLLAMA_DEFAULT_URL.to_string());
        let model = std::env::var("OLLAMA_MODEL")
            .unwrap_or_else(|_| OLLAMA_DEFAULT_MODEL.to_string());
        Self {
            client: Client::new(),
            base_url,
            model,
        }
    }

    pub async fn generate_text(
        &self,
        prompt: &str,
    ) -> Result<String, Box<dyn std::error::Error + Send + Sync>> {
        let body = serde_json::json!({
            "model": self.model,
            "messages": [{"role": "user", "content": prompt}],
            "max_tokens": 2048,
            "temperature": 0.1,
            "stream": false,
        });

        let resp = self
            .client
            .post(format!("{}/v1/chat/completions", self.base_url))
            .header("Content-Type", "application/json")
            .json(&body)
            .send()
            .await?;

        let status = resp.status();
        if !status.is_success() {
            let err = resp.text().await.unwrap_or_default();
            return Err(format!("Ollama error {}: {}", status, err).into());
        }

        let json: serde_json::Value = resp.json().await?;
        let text = json["choices"][0]["message"]["content"]
            .as_str()
            .ok_or("Ollama: no content in response")?
            .to_string();
        Ok(text)
    }
}
