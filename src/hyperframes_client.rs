//! HyperFramesClient — HTTP client for the HyperFrames render shim.
//!
//! The shim runs on the media ASG next to the ytdlp service (CPU-only
//! headless-Chrome rendering, no GPU). Flow: POST /render {html} → poll
//! GET /render/:id → download GET /render/:id/file into outputs/.
//!
//! Mirrors the BlenderMCPClient pattern: struct + new() + convenience methods.
//! Used by the `hyperframes_render` agent tool (education + landing_page
//! services take Blender's old slots; Manim is untouched).

use reqwest::Client;
use serde_json::Value;

#[derive(Clone)]
pub struct HyperFramesClient {
    client: Client,
    pub base_url: String,
}

impl HyperFramesClient {
    pub fn new(base_url: String) -> Self {
        // CPU renders take 30-120s for short compositions; poll loop uses
        // short requests so the client timeout only guards individual calls.
        let client = Client::builder()
            .timeout(std::time::Duration::from_secs(60))
            .build()
            .unwrap_or_default();
        Self {
            client,
            base_url: base_url.trim_end_matches('/').to_string(),
        }
    }

    /// Submit an HTML composition. Returns the shim job_id.
    pub async fn submit(
        &self,
        html: &str,
        resolution: Option<&str>,
    ) -> Result<String, String> {
        let mut body = serde_json::json!({ "html": html });
        if let Some(r) = resolution {
            if ["portrait", "landscape", "square"].contains(&r) {
                body["resolution"] = serde_json::Value::String(r.to_string());
            }
        }
        let resp = self
            .client
            .post(format!("{}/render", self.base_url))
            .json(&body)
            .send()
            .await
            .map_err(|e| format!("HyperFrames submit failed: {e}"))?;
        if !resp.status().is_success() {
            let status = resp.status();
            let text = resp.text().await.unwrap_or_default();
            return Err(format!("HyperFrames submit HTTP {status}: {text}"));
        }
        let json: Value = resp
            .json()
            .await
            .map_err(|e| format!("HyperFrames submit parse failed: {e}"))?;
        json.get("job_id")
            .and_then(|v| v.as_str())
            .map(String::from)
            .ok_or_else(|| "HyperFrames submit response missing job_id".to_string())
    }

    /// Poll one job until done/failed. Short requests, 10s cadence.
    pub async fn poll(&self, job_id: &str, timeout_secs: u64) -> Result<(), String> {
        let deadline = tokio::time::Instant::now() + std::time::Duration::from_secs(timeout_secs);
        loop {
            if tokio::time::Instant::now() > deadline {
                return Err(format!(
                    "HyperFrames job {job_id} still not done after {timeout_secs}s (transient — retry)"
                ));
            }
            tokio::time::sleep(std::time::Duration::from_secs(10)).await;
            let resp = self
                .client
                .get(format!("{}/render/{}", self.base_url, job_id))
                .send()
                .await
                .map_err(|e| format!("HyperFrames poll failed: {e}"))?;
            if !resp.status().is_success() {
                continue;
            }
            let json: Value = match resp.json().await {
                Ok(j) => j,
                Err(_) => continue,
            };
            match json.get("status").and_then(|v| v.as_str()) {
                Some("done") => return Ok(()),
                Some("failed") => {
                    let err = json
                        .get("error")
                        .and_then(|v| v.as_str())
                        .unwrap_or("render failed");
                    return Err(format!("HyperFrames render failed: {err}"));
                }
                _ => {}
            }
        }
    }

    /// Download the finished MP4 into outputs/. Returns the local path.
    pub async fn download(&self, job_id: &str, filename: &str) -> Result<String, String> {
        std::fs::create_dir_all("outputs")
            .map_err(|e| format!("Failed to create outputs/ dir: {e}"))?;
        let safe: String = filename
            .chars()
            .map(|c| {
                if c.is_ascii_alphanumeric() || c == '-' || c == '_' || c == '.' {
                    c
                } else {
                    '_'
                }
            })
            .collect();
        let dest = format!("outputs/{safe}");
        let resp = self
            .client
            .get(format!("{}/render/{}/file", self.base_url, job_id))
            .send()
            .await
            .map_err(|e| format!("HyperFrames download failed: {e}"))?;
        if !resp.status().is_success() {
            return Err(format!("HyperFrames file HTTP {}", resp.status()));
        }
        let bytes = resp
            .bytes()
            .await
            .map_err(|e| format!("Failed to read render body: {e}"))?;
        if (bytes.len() as u64) < 10_000 {
            return Err(format!(
                "HyperFrames render too small ({} bytes)",
                bytes.len()
            ));
        }
        std::fs::write(&dest, &bytes).map_err(|e| format!("Failed to write {dest}: {e}"))?;
        Ok(dest)
    }

    /// Full pipeline: submit HTML → poll → download. Returns local outputs/ path.
    pub async fn render_html(
        &self,
        html: &str,
        resolution: Option<&str>,
        filename: &str,
        timeout_secs: u64,
    ) -> Result<String, String> {
        let job_id = self.submit(html, resolution).await?;
        tracing::info!("🎬 HyperFrames job submitted: {job_id}");
        self.poll(&job_id, timeout_secs).await?;
        let path = self.download(&job_id, filename).await?;
        tracing::info!("✅ HyperFrames render ready: {path}");
        Ok(path)
    }
}
