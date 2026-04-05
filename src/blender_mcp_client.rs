// BlenderMCPClient — HTTP client for BlenderMCPServer
//
// Calls the REST endpoint at POST /api/call_tool, downloads the resulting
// file from the R2 presigned URL, and returns the local path inside outputs/.
//
// Mirrors the ElevenLabsClient pattern: struct + new() + convenience methods.

use reqwest::Client;
use serde_json::{json, Value};

#[derive(Clone)]
pub struct BlenderMCPClient {
    client: Client,
    pub base_url: String,
    api_key: String,
}

impl BlenderMCPClient {
    pub fn new(base_url: String, api_key: String) -> Self {
        // Blender renders take 90-150s on CPU-only Render instances.
        // Set a 3-minute timeout so sync calls don't time out mid-render.
        let client = Client::builder()
            .timeout(std::time::Duration::from_secs(180))
            .build()
            .unwrap_or_default();
        Self {
            client,
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

    /// Generate a Manim animation from a natural language description.
    /// Returns a local file path inside `outputs/`.
    pub async fn generate_animation(
        &self,
        description: &str,
        duration: f64,
        background: &str,
        quality: &str,
    ) -> Result<String, String> {
        let args = json!({
            "description": description,
            "duration":    duration,
            "background":  background,
            "quality":     quality,
        });
        self.render_async("blender_generate_animation", args, "video_url", "mp4").await
    }

    /// Generate an animated data visualisation (bar/line/pie/counter/scatter).
    /// Returns a local file path inside `outputs/`.
    pub async fn generate_chart(
        &self,
        chart_type: &str,
        title: &str,
        data: serde_json::Value,
        labels: serde_json::Value,
        duration: f64,
        colors: serde_json::Value,
    ) -> Result<String, String> {
        let args = json!({
            "chart_type": chart_type,
            "title":      title,
            "data":       data,
            "labels":     labels,
            "duration":   duration,
            "colors":     colors,
        });
        self.render_async("blender_generate_chart", args, "video_url", "mp4").await
    }

    /// Submit a render job and poll until completion, then download the result.
    /// Use this for all renders — it is safe for any duration because it never
    /// holds an HTTP connection open during the actual render.
    ///
    /// * `tool`    — e.g. "blender_generate_scene"
    /// * `args`    — tool-specific args JSON
    /// * `url_key` — field in the result object that holds the file URL
    ///               ("video_url" for MP4 tools, "image_url" for thumbnail)
    /// * `ext`     — file extension for the local copy ("mp4" or "png")
    pub async fn render_async(
        &self,
        tool: &str,
        args: serde_json::Value,
        url_key: &str,
        ext: &str,
    ) -> Result<String, String> {
        let ts = std::time::SystemTime::now()
            .duration_since(std::time::UNIX_EPOCH)
            .unwrap_or_default()
            .as_secs();
        let prefix = tool.replace("blender_generate_", "blender_");
        let filename = format!("{prefix}_{ts}.{ext}");

        let job_id = self.submit_job(tool, args).await?;

        // Poll every 5 seconds for up to 15 minutes (180 polls)
        for _ in 0..180 {
            tokio::time::sleep(std::time::Duration::from_secs(5)).await;
            let status = self.poll_job(&job_id).await?;
            match status.get("state").and_then(|s| s.as_str()) {
                Some("completed") => {
                    let url = status
                        .get("result")
                        .and_then(|r| r.get(url_key))
                        .and_then(|v| v.as_str())
                        .ok_or_else(|| format!("Job result missing '{url_key}'"))?;
                    return self.download_to_outputs(url, &filename).await;
                }
                Some("error") | Some("failed") => {
                    let msg = status
                        .get("error")
                        .and_then(|v| v.as_str())
                        .unwrap_or("unknown error");
                    return Err(msg.to_string());
                }
                _ => {
                    // If the job is not found (404 after a server redeploy), fail fast
                    if status.get("error").is_some() && status.get("state").is_none() {
                        let msg = status
                            .get("error")
                            .and_then(|v| v.as_str())
                            .unwrap_or("job not found");
                        return Err(format!("Blender job {job_id}: {msg}"));
                    }
                    // pending / running — keep polling
                }
            }
        }
        Err(format!("Blender job {job_id} timed out after 900s"))
    }

    /// Submit a long-running job to the Phase 5 async queue.
    /// Returns the job_id to poll via `poll_job`.
    /// Retries up to 3 times on transient parse/connection errors (handles
    /// server cold-start or brief Render proxy glitches).
    pub async fn submit_job(&self, tool: &str, args: serde_json::Value) -> Result<String, String> {
        let body = json!({"tool": tool, "args": args});
        let url = format!("{}/api/jobs", self.base_url);
        let mut last_err = String::new();
        for attempt in 0..3u8 {
            if attempt > 0 {
                tokio::time::sleep(std::time::Duration::from_secs(5)).await;
            }
            let resp = match self
                .client
                .post(&url)
                .bearer_auth(&self.api_key)
                .json(&body)
                .send()
                .await
            {
                Ok(r) => r,
                Err(e) => { last_err = format!("BlenderMCP submit_job HTTP error: {e}"); continue; }
            };
            let json: serde_json::Value = match resp.json().await {
                Ok(j) => j,
                Err(e) => { last_err = format!("BlenderMCP submit_job parse error: {e}"); continue; }
            };
            match json.get("job_id").and_then(|v| v.as_str()).map(|s| s.to_string()) {
                Some(id) => return Ok(id),
                None => { last_err = format!("BlenderMCP submit_job: no job_id in response: {json}"); }
            }
        }
        Err(last_err)
    }

    /// Analyze a video via BlenderMCPServer's `/api/analyze-video` endpoint.
    ///
    /// This offloads video analysis to the Python service which uses a SEPARATE Gemini API key
    /// (`BLENDER_GEMINI_API_KEY`), keeping it fully isolated from the Rust app's quota.
    ///
    /// Used as a fallback when the primary Gemini client returns 429.
    pub async fn analyze_video(
        &self,
        video_url: &str,
        clips_requested: u32,
        min_duration_sec: f64,
        max_duration_sec: f64,
        high_performing_factors: &[String],
    ) -> Result<crate::clipping::gemini_video_analyzer::VideoAnalysis, String> {
        let body = json!({
            "video_url": video_url,
            "clips_requested": clips_requested,
            "min_duration": min_duration_sec,
            "max_duration": max_duration_sec,
            "high_performing_factors": high_performing_factors,
        });
        let url = format!("{}/api/analyze-video", self.base_url);
        let resp = self
            .client
            .post(&url)
            .bearer_auth(&self.api_key)
            .json(&body)
            .send()
            .await
            .map_err(|e| format!("BlenderMCP analyze_video HTTP error: {e}"))?;

        if !resp.status().is_success() {
            let status = resp.status();
            let text = resp.text().await.unwrap_or_default();
            return Err(format!("BlenderMCP analyze_video {status}: {text}"));
        }

        let val: serde_json::Value = resp
            .json()
            .await
            .map_err(|e| format!("BlenderMCP analyze_video parse error: {e}"))?;

        serde_json::from_value(val)
            .map_err(|e| format!("BlenderMCP analyze_video deserialize error: {e}"))
    }

    /// Poll job status.  Returns the full JSON status object.
    /// Retries up to 3 times on transient parse errors.
    pub async fn poll_job(&self, job_id: &str) -> Result<serde_json::Value, String> {
        let url = format!("{}/api/jobs/{}", self.base_url, job_id);
        let mut last_err = String::new();
        for attempt in 0..3u8 {
            if attempt > 0 {
                tokio::time::sleep(std::time::Duration::from_secs(3)).await;
            }
            let resp = match self
                .client
                .get(&url)
                .bearer_auth(&self.api_key)
                .send()
                .await
            {
                Ok(r) => r,
                Err(e) => { last_err = format!("BlenderMCP poll_job HTTP error: {e}"); continue; }
            };
            match resp.json::<serde_json::Value>().await {
                Ok(j) => return Ok(j),
                Err(e) => { last_err = format!("BlenderMCP poll_job parse error: {e}"); }
            }
        }
        Err(last_err)
    }
}
