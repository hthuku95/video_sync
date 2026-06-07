/// Client for FFmpeg MCP microservice — processes media directly on R2,
/// zero local disk usage. Falls back to local FFmpeg if unavailable.
use reqwest::Client;

const DEFAULT_MCP_URL: &str = "http://172.31.42.118:8001";

#[derive(Debug, Clone)]
pub struct FfmpegMcpClient {
    client: Client,
    base_url: String,
}

#[derive(serde::Deserialize)]
struct ProcessResponse {
    output_url: String,
    output_key: String,
}

#[derive(serde::Deserialize)]
struct HealthResponse {
    status: String,
    r2_configured: bool,
}

impl FfmpegMcpClient {
    pub fn from_env() -> Option<Self> {
        let url = std::env::var("FFMPEG_MCP_URL").unwrap_or_else(|_| DEFAULT_MCP_URL.to_string());
        Some(Self {
            client: Client::new(),
            base_url: url.trim_end_matches('/').to_string(),
        })
    }

    pub async fn health(&self) -> Result<bool, String> {
        let resp = self
            .client
            .get(format!("{}/health", self.base_url))
            .send()
            .await
            .map_err(|e| format!("FFmpeg MCP unreachable: {}", e))?;
        let h: HealthResponse = resp.json().await.map_err(|e| format!("bad health: {}", e))?;
        Ok(h.status == "ok")
    }

    /// Process a media file. `input_url` can be an R2 presigned URL or any HTTP URL.
    /// `ffmpeg_args` are the arguments AFTER `-i <input_url>` and BEFORE the output path.
    /// Returns the presigned URL of the processed file on R2.
    pub async fn process(
        &self,
        input_url: &str,
        ffmpeg_args: &[String],
        output_key: &str,
    ) -> Result<String, String> {
        let body = serde_json::json!({
            "input_url": input_url,
            "ffmpeg_args": ffmpeg_args,
            "output_key": output_key,
        });

        let resp = self
            .client
            .post(format!("{}/process", self.base_url))
            .header("Content-Type", "application/json")
            .json(&body)
            .send()
            .await
            .map_err(|e| format!("FFmpeg MCP request failed: {}", e))?;

        let status = resp.status();
        if !status.is_success() {
            let err = resp.text().await.unwrap_or_default();
            return Err(format!("FFmpeg MCP error {}: {}", status, err));
        }

        let pr: ProcessResponse = resp
            .json()
            .await
            .map_err(|e| format!("FFmpeg MCP bad response: {}", e))?;
        Ok(pr.output_url)
    }
}
