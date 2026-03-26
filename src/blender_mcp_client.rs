// BlenderMCPClient — HTTP client for BlenderMCPServer
//
// Calls the REST endpoint at POST /api/call_tool, downloads the resulting
// file from the R2 presigned URL, and returns the local path inside outputs/.
//
// Mirrors the ElevenLabsClient pattern: struct + new() + convenience methods.

use reqwest::Client;
use serde_json::{json, Value};
use std::path::Path;

#[derive(Clone)]
pub struct BlenderMCPClient {
    client: Client,
    pub base_url: String,
    api_key: String,
}

impl BlenderMCPClient {
    pub fn new(base_url: String, api_key: String) -> Self {
        Self {
            client: Client::new(),
            base_url: base_url.trim_end_matches('/').to_string(),
            api_key,
        }
    }

    // -------------------------------------------------------------------------
    // Core call_tool — calls POST /api/call_tool, returns raw JSON result
    // -------------------------------------------------------------------------

    pub async fn call_tool(
        &self,
        tool_name: &str,
        args: Value,
    ) -> Result<Value, String> {
        let url = format!("{}/api/call_tool", self.base_url);
        let body = json!({
            "tool": tool_name,
            "args": args,
        });

        let mut req = self.client.post(&url).json(&body);
        if !self.api_key.is_empty() {
            req = req.header("Authorization", format!("Bearer {}", self.api_key));
        }

        let resp = req
            .send()
            .await
            .map_err(|e| format!("BlenderMCPClient request failed: {e}"))?;

        if !resp.status().is_success() {
            let status = resp.status();
            let body = resp.text().await.unwrap_or_default();
            return Err(format!("BlenderMCPServer error {status}: {body}"));
        }

        let json: Value = resp
            .json()
            .await
            .map_err(|e| format!("Failed to parse BlenderMCPServer response: {e}"))?;

        if let Some(err) = json.get("error") {
            return Err(format!("BlenderMCPServer tool error: {err}"));
        }

        json.get("result")
            .cloned()
            .ok_or_else(|| "BlenderMCPServer response missing 'result' field".to_string())
    }

    // -------------------------------------------------------------------------
    // Download helper — fetches a presigned URL and saves to outputs/
    // -------------------------------------------------------------------------

    async fn download_to_outputs(
        &self,
        url: &str,
        filename: &str,
    ) -> Result<String, String> {
        std::fs::create_dir_all("outputs")
            .map_err(|e| format!("Failed to create outputs/ dir: {e}"))?;

        let dest = format!("outputs/{filename}");
        let resp = self
            .client
            .get(url)
            .send()
            .await
            .map_err(|e| format!("Download failed: {e}"))?;

        if !resp.status().is_success() {
            return Err(format!(
                "Download HTTP error {}: {}",
                resp.status(),
                url
            ));
        }

        let bytes = resp
            .bytes()
            .await
            .map_err(|e| format!("Failed to read download body: {e}"))?;

        std::fs::write(&dest, &bytes)
            .map_err(|e| format!("Failed to write {dest}: {e}"))?;

        Ok(dest)
    }

    // -------------------------------------------------------------------------
    // Tool convenience methods
    // -------------------------------------------------------------------------

    /// Generate a 3D Blender scene clip. Returns local path inside outputs/.
    pub async fn generate_scene(
        &self,
        prompt: &str,
        duration: f64,
        style: &str,
        reference_image_url: Option<&str>,
    ) -> Result<String, String> {
        let mut args = json!({
            "prompt": prompt,
            "duration": duration,
            "style": style,
        });
        if let Some(url) = reference_image_url {
            args["reference_image_url"] = Value::String(url.to_string());
        }

        let result = self.call_tool("blender_generate_scene", args).await?;

        let video_url = result
            .get("video_url")
            .and_then(|v| v.as_str())
            .ok_or_else(|| "blender_generate_scene response missing video_url".to_string())?;

        let filename = format!(
            "blender_scene_{}.mp4",
            std::time::SystemTime::now()
                .duration_since(std::time::UNIX_EPOCH)
                .unwrap_or_default()
                .as_secs()
        );
        self.download_to_outputs(video_url, &filename).await
    }

    /// Generate a 3D thumbnail image. Returns local path inside outputs/.
    pub async fn generate_thumbnail(
        &self,
        prompt: &str,
        title_text: &str,
        style: &str,
    ) -> Result<String, String> {
        let args = json!({
            "prompt": prompt,
            "title_text": title_text,
            "style": style,
        });
        let result = self.call_tool("blender_generate_thumbnail", args).await?;
        let image_url = result
            .get("image_url")
            .and_then(|v| v.as_str())
            .ok_or_else(|| "blender_generate_thumbnail response missing image_url".to_string())?;
        if image_url.is_empty() {
            return Err("blender_generate_thumbnail: server returned empty image_url".to_string());
        }
        let filename = format!(
            "blender_thumb_{}.png",
            std::time::SystemTime::now()
                .duration_since(std::time::UNIX_EPOCH)
                .unwrap_or_default()
                .as_secs()
        );
        self.download_to_outputs(image_url, &filename).await
    }

    /// Generate an animated title card clip. Returns local path inside outputs/.
    pub async fn generate_title_card(
        &self,
        title: &str,
        subtitle: &str,
        duration: f64,
        style: &str,
    ) -> Result<String, String> {
        let args = json!({
            "title": title,
            "subtitle": subtitle,
            "duration": duration,
            "style": style,
        });
        let result = self.call_tool("blender_generate_title_card", args).await?;
        let video_url = result
            .get("video_url")
            .and_then(|v| v.as_str())
            .ok_or_else(|| "blender_generate_title_card response missing video_url".to_string())?;
        if video_url.is_empty() {
            return Err("blender_generate_title_card: server returned empty video_url".to_string());
        }
        let filename = format!(
            "blender_title_{}.mp4",
            std::time::SystemTime::now()
                .duration_since(std::time::UNIX_EPOCH)
                .unwrap_or_default()
                .as_secs()
        );
        self.download_to_outputs(video_url, &filename).await
    }

    /// Generate a data visualisation clip. Returns local path inside outputs/.
    pub async fn generate_data_viz(
        &self,
        data_json: &str,
        chart_type: &str,
        title: &str,
        duration: f64,
    ) -> Result<String, String> {
        let args = json!({
            "data_json": data_json,
            "chart_type": chart_type,
            "title": title,
            "duration": duration,
        });
        let result = self.call_tool("blender_generate_data_viz", args).await?;
        let video_url = result
            .get("video_url")
            .and_then(|v| v.as_str())
            .ok_or_else(|| "blender_generate_data_viz response missing video_url".to_string())?;
        if video_url.is_empty() {
            return Err("blender_generate_data_viz: server returned empty video_url".to_string());
        }
        let filename = format!(
            "blender_viz_{}.mp4",
            std::time::SystemTime::now()
                .duration_since(std::time::UNIX_EPOCH)
                .unwrap_or_default()
                .as_secs()
        );
        self.download_to_outputs(video_url, &filename).await
    }

    /// Generate an animated lower-third overlay clip. Returns local path inside outputs/.
    pub async fn generate_lower_third(
        &self,
        name_text: &str,
        subtitle_text: &str,
        style: &str,
        duration: f64,
    ) -> Result<String, String> {
        let args = json!({
            "name_text": name_text,
            "subtitle_text": subtitle_text,
            "style": style,
            "duration": duration,
        });
        let result = self.call_tool("blender_generate_lower_third", args).await?;
        let video_url = result
            .get("video_url")
            .and_then(|v| v.as_str())
            .ok_or_else(|| "blender_generate_lower_third response missing video_url".to_string())?;
        if video_url.is_empty() {
            return Err("blender_generate_lower_third: server returned empty video_url".to_string());
        }
        let filename = format!(
            "blender_lower_{}.mp4",
            std::time::SystemTime::now()
                .duration_since(std::time::UNIX_EPOCH)
                .unwrap_or_default()
                .as_secs()
        );
        self.download_to_outputs(video_url, &filename).await
    }

    /// Generate a LaTeX/Manim equation animation clip. Returns local path inside outputs/.
    pub async fn generate_latex(
        &self,
        latex_expression: &str,
        animation_type: &str,
        duration: f64,
        background_style: &str,
    ) -> Result<String, String> {
        let args = json!({
            "latex_expression": latex_expression,
            "animation_type": animation_type,
            "duration": duration,
            "background_style": background_style,
        });
        let result = self.call_tool("blender_generate_latex", args).await?;
        let video_url = result
            .get("video_url")
            .and_then(|v| v.as_str())
            .ok_or_else(|| "blender_generate_latex response missing video_url".to_string())?;
        if video_url.is_empty() {
            return Err("blender_generate_latex: server returned empty video_url".to_string());
        }
        let filename = format!(
            "blender_latex_{}.mp4",
            std::time::SystemTime::now()
                .duration_since(std::time::UNIX_EPOCH)
                .unwrap_or_default()
                .as_secs()
        );
        self.download_to_outputs(video_url, &filename).await
    }

    /// Generate a device UI mockup (iPhone/MacBook/browser/iPad) with optional animation.
    /// Returns a local file path inside `outputs/`.
    pub async fn generate_ui_mockup(
        &self,
        device: &str,
        animation: &str,
        duration: f64,
        screenshot_url: &str,
        screenshot_spec: Option<&serde_json::Value>,
        background_color: Option<&[f64; 3]>,
        accent_color: Option<&[f64; 3]>,
    ) -> Result<String, String> {
        let mut args = json!({
            "device": device,
            "animation": animation,
            "duration": duration,
            "screenshot_url": screenshot_url,
        });
        if let Some(spec) = screenshot_spec {
            args["screenshot_spec"] = serde_json::Value::String(spec.to_string());
        }
        if let Some(bg) = background_color {
            args["background_color"] = json!(bg);
        }
        if let Some(acc) = accent_color {
            args["accent_color"] = json!(acc);
        }

        let result = self.call_tool("blender_generate_ui_mockup", args).await?;

        // Static renders return image_url; animated return video_url
        let (url, ext) = if animation == "static" {
            (
                result
                    .get("image_url")
                    .and_then(|v| v.as_str())
                    .ok_or_else(|| "blender_generate_ui_mockup response missing image_url".to_string())?,
                "png",
            )
        } else {
            (
                result
                    .get("video_url")
                    .and_then(|v| v.as_str())
                    .ok_or_else(|| "blender_generate_ui_mockup response missing video_url".to_string())?,
                "mp4",
            )
        };

        let filename = format!(
            "blender_mockup_{device}_{animation}_{}.{ext}",
            std::time::SystemTime::now()
                .duration_since(std::time::UNIX_EPOCH)
                .unwrap_or_default()
                .as_secs()
        );
        self.download_to_outputs(url, &filename).await
    }

    /// Submit a long-running job to the Phase 5 async queue.
    /// Returns the job_id to poll via `poll_job`.
    pub async fn submit_job(&self, tool: &str, args: serde_json::Value) -> Result<String, String> {
        let body = json!({"tool": tool, "args": args});
        let url = format!("{}/api/jobs", self.base_url);
        let resp = self
            .client
            .post(&url)
            .bearer_auth(&self.api_key)
            .json(&body)
            .send()
            .await
            .map_err(|e| format!("BlenderMCP submit_job HTTP error: {e}"))?;
        let json: serde_json::Value = resp
            .json()
            .await
            .map_err(|e| format!("BlenderMCP submit_job parse error: {e}"))?;
        json.get("job_id")
            .and_then(|v| v.as_str())
            .map(|s| s.to_string())
            .ok_or_else(|| format!("BlenderMCP submit_job: no job_id in response: {json}"))
    }

    /// Poll job status.  Returns the full JSON status object.
    pub async fn poll_job(&self, job_id: &str) -> Result<serde_json::Value, String> {
        let url = format!("{}/api/jobs/{}", self.base_url, job_id);
        let resp = self
            .client
            .get(&url)
            .bearer_auth(&self.api_key)
            .send()
            .await
            .map_err(|e| format!("BlenderMCP poll_job HTTP error: {e}"))?;
        resp.json::<serde_json::Value>()
            .await
            .map_err(|e| format!("BlenderMCP poll_job parse error: {e}"))
    }
}
