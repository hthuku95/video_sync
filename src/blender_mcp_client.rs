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

    pub async fn call_tool(&self, tool_name: &str, args: Value) -> Result<Value, String> {
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

    async fn download_to_outputs(&self, url: &str, filename: &str) -> Result<String, String> {
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
            return Err(format!("Download HTTP error {}: {}", resp.status(), url));
        }

        let bytes = resp
            .bytes()
            .await
            .map_err(|e| format!("Failed to read download body: {e}"))?;

        std::fs::write(&dest, &bytes).map_err(|e| format!("Failed to write {dest}: {e}"))?;

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
        self.render_async("blender_execute_bpy_script", args, "video_url", "mp4")
            .await
    }

    /// Generate a 3D thumbnail image. Returns local path inside outputs/.
    pub async fn generate_thumbnail(
        &self,
        prompt: &str,
        title_text: &str,
        style: &str,
    ) -> Result<String, String> {
        let combined_prompt = format!("Generate a 3D thumbnail image. Scene: {}. Title text: {}", prompt, title_text);
        let args = json!({
            "prompt": combined_prompt,
            "duration": 0,
            "style": style,
            "reference_image_url": "",
        });
        self.render_async("blender_execute_bpy_script", args, "image_url", "png")
            .await
    }

    /// Generate an animated title card clip. Returns local path inside outputs/.
    pub async fn generate_title_card(
        &self,
        title: &str,
        subtitle: &str,
        duration: f64,
        style: &str,
    ) -> Result<String, String> {
        let combined_prompt = format!("Generate an animated title card. Title: {}. Subtitle: {}. Style: {}", title, subtitle, style);
        let args = json!({
            "prompt": combined_prompt,
            "duration": duration,
            "style": style,
            "reference_image_url": "",
        });
        self.render_async("blender_execute_bpy_script", args, "video_url", "mp4")
            .await
    }

    /// Generate a data visualisation clip. Returns local path inside outputs/.
    pub async fn generate_data_viz(
        &self,
        data_json: &str,
        chart_type: &str,
        title: &str,
        duration: f64,
    ) -> Result<String, String> {
        let combined_prompt = format!("Generate a data visualization chart. Type: {}. Title: {}. Data: {}", chart_type, title, data_json);
        let args = json!({
            "prompt": combined_prompt,
            "duration": duration,
            "style": "default",
            "reference_image_url": "",
        });
        self.render_async("blender_execute_bpy_script", args, "video_url", "mp4")
            .await
    }

    /// Generate an animated lower-third overlay clip. Returns local path inside outputs/.
    pub async fn generate_lower_third(
        &self,
        name_text: &str,
        subtitle_text: &str,
        style: &str,
        duration: f64,
    ) -> Result<String, String> {
        let combined_prompt = format!("Generate an animated lower-third overlay. Name: {}. Subtitle: {}. Style: {}", name_text, subtitle_text, style);
        let args = json!({
            "prompt": combined_prompt,
            "duration": duration,
            "style": style,
            "reference_image_url": "",
        });
        self.render_async("blender_execute_bpy_script", args, "video_url", "mp4")
            .await
    }

    /// Generate a LaTeX equation animation clip. Returns local path inside outputs/.
    pub async fn generate_latex(
        &self,
        latex_expression: &str,
        animation_type: &str,
        duration: f64,
        background_style: &str,
    ) -> Result<String, String> {
        let description = format!("Create a LaTeX equation animation. Expression: {}. Animation type: {}. Background style: {}", latex_expression, animation_type, background_style);
        self.render_async(
            "manim_execute_script",
            json!({
                "description": description,
                "duration": duration,
                "background": background_style,
                "transparent": false,
                "quality": "m",
            }),
            "video_url",
            "mp4",
        ).await
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
        let mut desc = format!("Generate a UI mockup on a {}. Animation: {}. Screenshot URL: {}", device, animation, screenshot_url);
        if let Some(spec) = screenshot_spec {
            desc.push_str(&format!(". Screenshot spec: {}", spec));
        }
        if let Some(_bg) = background_color {
            desc.push_str(". With custom background color");
        }
        if let Some(_acc) = accent_color {
            desc.push_str(". With custom accent color");
        }
        let (url_key, ext) = if animation == "static" {
            ("image_url", "png")
        } else {
            ("video_url", "mp4")
        };
        let args = json!({
            "prompt": desc,
            "duration": duration,
            "style": "default",
            "reference_image_url": screenshot_url,
        });
        self.render_async("blender_execute_bpy_script", args, url_key, ext)
            .await
    }

    /// Generate a Manim animation from a natural language description.
    /// Returns a local file path inside `outputs/`.
    pub async fn generate_animation(
        &self,
        description: &str,
        duration: f64,
        background_style: &str,
        composite_over_scene: bool,
    ) -> Result<String, String> {
        self.render_async(
            "manim_execute_script",
            json!({
                "description": description,
                "duration": duration,
                "background": background_style,
                "transparent": composite_over_scene,
                "quality": "m",
            }),
            "video_url",
            "mp4",
        ).await
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
        let description = format!("Create a {} chart titled '{}'. Data: {}. Labels: {}. Colors: {}", chart_type, title, data, labels, colors);
        self.render_async(
            "manim_execute_script",
            json!({
                "description": description,
                "duration": duration,
                "background": "dark",
                "transparent": false,
                "quality": "m",
            }),
            "video_url",
            "mp4",
        ).await
    }

    /// Consolidated bpy script execution — replaces all individual blender_generate_* tools.
    pub async fn execute_bpy_script(
        &self,
        prompt: &str,
        duration: f64,
        style: &str,
        reference_image_url: &str,
    ) -> Result<String, String> {
        self.render_async(
            "blender_execute_bpy_script",
            json!({
                "prompt": prompt,
                "duration": duration,
                "style": style,
                "reference_image_url": reference_image_url,
            }),
            "video_url",
            "mp4",
        )
        .await
    }

    /// Consolidated Manim script execution — replaces all individual blender_generate_* Manim tools.
    pub async fn execute_manim_script(
        &self,
        description: &str,
        duration: f64,
        background: &str,
        transparent: bool,
        quality: &str,
    ) -> Result<String, String> {
        self.render_async(
            "manim_execute_script",
            json!({
                "description": description,
                "duration": duration,
                "background": background,
                "transparent": transparent,
                "quality": quality,
            }),
            "video_url",
            if transparent { "mov" } else { "mp4" },
        )
        .await
    }

    /// Submit a render job and poll until completion, then download the result.
    /// Use this for all renders — it is safe for any duration because it never
    /// holds an HTTP connection open during the actual render.
    ///
    /// * `tool`    — e.g. "blender_execute_bpy_script"
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
        let filename = format!("{tool}_{ts}.{ext}");

        let job_id = self.submit_job(tool, args).await?;

        // Poll every 5 seconds for up to 30 minutes (360 polls).
        // This leaves enough room for heavier reference-driven renders that
        // now run behind the durable Blender workflow.
        for _ in 0..360 {
            tokio::time::sleep(std::time::Duration::from_secs(5)).await;
            let status = self.poll_job(&job_id).await?;
            match status.get("state").and_then(|s| s.as_str()) {
                Some("completed") => {
                    let result = status
                        .get("result")
                        .ok_or_else(|| "Job result missing 'result'".to_string())?;
                    let url = if url_key == "video_url" {
                        result
                            .get("narrated_video_url")
                            .and_then(|v| v.as_str())
                            .or_else(|| result.get(url_key).and_then(|v| v.as_str()))
                    } else {
                        result.get(url_key).and_then(|v| v.as_str())
                    }
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
        Err(format!("Blender job {job_id} timed out after 1800s"))
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
                Err(e) => {
                    last_err = format!("BlenderMCP submit_job HTTP error: {e}");
                    continue;
                }
            };
            let status = resp.status();
            let bytes = match resp.bytes().await {
                Ok(b) => b,
                Err(e) => {
                    last_err = format!("BlenderMCP submit_job body read error: {e}");
                    continue;
                }
            };
            if !status.is_success() {
                last_err = format!(
                    "BlenderMCP submit_job HTTP {}: {}",
                    status,
                    Self::body_snippet_from_bytes(&bytes),
                );
                continue;
            }
            let json: serde_json::Value = match Self::parse_json_body(&bytes) {
                Ok(j) => j,
                Err(e) => {
                    last_err = format!("BlenderMCP submit_job parse error: {e}");
                    continue;
                }
            };
            match json
                .get("job_id")
                .and_then(|v| v.as_str())
                .map(|s| s.to_string())
            {
                Some(id) => return Ok(id),
                None => {
                    last_err = format!("BlenderMCP submit_job: no job_id in response: {json}");
                }
            }
        }
        Err(last_err)
    }

    /// Submit a tool as a background job and poll until completion.
    /// Returns the full JSON result (unlike render_async which downloads the file).
    /// Safe for any duration — never holds an HTTP connection open during render.
    pub async fn call_tool_async(&self, tool: &str, args: serde_json::Value) -> Result<serde_json::Value, String> {
        let job_id = self.submit_job(tool, args).await?;

        // Poll every 5 seconds for up to 30 minutes (360 polls).
        for _ in 0..360 {
            tokio::time::sleep(std::time::Duration::from_secs(5)).await;
            let status = self.poll_job(&job_id).await?;
            match status.get("state").and_then(|s| s.as_str()) {
                Some("completed") => {
                    return status
                        .get("result")
                        .ok_or_else(|| "Job result missing 'result'".to_string())
                        .cloned();
                }
                Some("error") | Some("failed") => {
                    let msg = status
                        .get("error")
                        .and_then(|v| v.as_str())
                        .unwrap_or("unknown error");
                    return Err(msg.to_string());
                }
                _ => {
                    if status.get("error").is_some() && status.get("state").is_none() {
                        let msg = status
                            .get("error")
                            .and_then(|v| v.as_str())
                            .unwrap_or("job not found");
                        return Err(format!("Blender job {job_id}: {msg}"));
                    }
                }
            }
        }
        Err(format!("Blender job {job_id} timed out after 1800s"))
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
        let mut saw_transient_issue = false;
        for attempt in 0..5u8 {
            if attempt > 0 {
                tokio::time::sleep(std::time::Duration::from_secs(3 + (attempt as u64 * 2))).await;
            }
            let resp = match self
                .client
                .get(&url)
                .bearer_auth(&self.api_key)
                .send()
                .await
            {
                Ok(r) => r,
                Err(e) => {
                    last_err = format!("BlenderMCP poll_job HTTP error: {e}");
                    continue;
                }
            };
            let status = resp.status();
            let bytes = match resp.bytes().await {
                Ok(b) => b,
                Err(e) => {
                    saw_transient_issue = true;
                    last_err = format!("BlenderMCP poll_job body read error: {e}");
                    continue;
                }
            };

            if status == reqwest::StatusCode::NOT_FOUND {
                return Err(format!(
                    "BlenderMCP poll_job 404 for job {job_id}: {}",
                    Self::body_snippet_from_bytes(&bytes)
                ));
            }

            if !status.is_success() {
                let snippet = Self::body_snippet_from_bytes(&bytes);
                last_err = format!("BlenderMCP poll_job HTTP {status}: {snippet}");
                if status.is_server_error() || status == reqwest::StatusCode::TOO_MANY_REQUESTS {
                    saw_transient_issue = true;
                    continue;
                }
                return Err(last_err);
            }

            match Self::parse_json_body(&bytes) {
                Ok(j) => return Ok(j),
                Err(e) => {
                    saw_transient_issue = true;
                    last_err = format!("BlenderMCP poll_job parse error: {e}");
                }
            }
        }
        if saw_transient_issue {
            return Ok(json!({
                "state": "running",
                "poll_error": last_err,
            }));
        }
        Err(last_err)
    }

    fn parse_json_body(bytes: &[u8]) -> Result<serde_json::Value, String> {
        if let Ok(value) = serde_json::from_slice::<serde_json::Value>(bytes) {
            return Ok(value);
        }

        let text = String::from_utf8_lossy(bytes);
        if let (Some(start), Some(end)) = (text.find('{'), text.rfind('}')) {
            if start <= end {
                if let Ok(value) = serde_json::from_str::<serde_json::Value>(&text[start..=end]) {
                    return Ok(value);
                }
            }
        }

        Err(format!(
            "unable to decode JSON body: {}",
            Self::body_snippet_from_text(&text),
        ))
    }

    fn body_snippet_from_bytes(bytes: &[u8]) -> String {
        Self::body_snippet_from_text(&String::from_utf8_lossy(bytes))
    }

    fn body_snippet_from_text(text: &str) -> String {
        let compact = text.replace('\r', " ").replace('\n', " ");
        let trimmed = compact.trim();
        if trimmed.len() <= 240 {
            trimmed.to_string()
        } else {
            format!("{}...", &trimmed[..240])
        }
    }
}
