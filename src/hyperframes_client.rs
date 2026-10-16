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
    /// "shim" (default, media-box HTTP) or "lambda" (Step Functions + S3).
    pub mode: String,
    pub state_machine_arn: String,
    pub render_bucket: String,
}

impl HyperFramesClient {
    pub fn new(base_url: String) -> Self {
        // CPU renders take 30-120s for short compositions; poll loop uses
        // short requests so the client timeout only guards individual calls.
        let client = Client::builder()
            .timeout(std::time::Duration::from_secs(60))
            .build()
            .unwrap_or_default();
        let mode = std::env::var("HYPERFRAMES_MODE")
            .unwrap_or_else(|_| "shim".to_string())
            .to_ascii_lowercase();
        let state_machine_arn = std::env::var("HYPERFRAMES_STATE_MACHINE_ARN").unwrap_or_default();
        let render_bucket = std::env::var("HYPERFRAMES_RENDER_BUCKET").unwrap_or_default();
        Self {
            client,
            base_url: base_url.trim_end_matches('/').to_string(),
            mode,
            state_machine_arn,
            render_bucket,
        }
    }

    /// Lambda path: tar the HTML, upload to S3, start execution, poll to
    /// SUCCEEDED, download OutputS3Uri. Returns local outputs/ path.
    pub async fn render_lambda(
        &self,
        html: &str,
        filename: &str,
        timeout_secs: u64,
    ) -> Result<String, String> {
        if self.state_machine_arn.is_empty() || self.render_bucket.is_empty() {
            return Err(
                "Lambda mode needs HYPERFRAMES_STATE_MACHINE_ARN + HYPERFRAMES_RENDER_BUCKET"
                    .to_string(),
            );
        }
        // Stage project dir (index.html) and tar.gz it with system tar.
        let stamp = std::time::SystemTime::now()
            .duration_since(std::time::UNIX_EPOCH)
            .map(|d| d.as_millis())
            .unwrap_or(0);
        let proj_dir = std::format!("/tmp/hflambda_{stamp}");
        std::fs::create_dir_all(&proj_dir)
            .map_err(|e| format!("Failed to stage lambda project: {e}"))?;
        std::fs::write(std::format!("{proj_dir}/index.html"), html)
            .map_err(|e| format!("Failed to write index.html: {e}"))?;
        let tar_path = std::format!("/tmp/hflambda_{stamp}.tar.gz");
        let tar_out = std::process::Command::new("tar")
            .args(["-czf", &tar_path, "-C", &proj_dir, "index.html"])
            .output()
            .map_err(|e| format!("tar spawn failed: {e}"))?;
        if !tar_out.status.success() {
            return Err("tar project archive failed".to_string());
        }
        let project_key = std::format!("projects/rust-{stamp}.tar.gz");
        let output_key = std::format!("rust-out-{stamp}.mp4");

        let sdk_config = aws_config::load_defaults(aws_config::BehaviorVersion::latest()).await;
        let s3 = aws_sdk_s3::Client::new(&sdk_config);
        let sfn = aws_sdk_sfn::Client::new(&sdk_config);

        let tar_bytes =
            std::fs::read(&tar_path).map_err(|e| format!("Failed to read tarball: {e}"))?;
        s3.put_object()
            .bucket(&self.render_bucket)
            .key(&project_key)
            .body(tar_bytes.into())
            .send()
            .await
            .map_err(|e| format!("S3 project upload failed: {e}"))?;

        let input = serde_json::json!({
            "PlanProtocol": "v1",
            "ProjectS3Uri": std::format!("s3://{}/{project_key}", self.render_bucket),
            "PlanOutputS3Prefix": std::format!("s3://{}/renders/rust-{stamp}/", self.render_bucket),
            "OutputS3Uri": std::format!("s3://{}/{output_key}", self.render_bucket),
            "Config": {"fps": 30, "width": 1920, "height": 1080, "format": "mp4"},
        })
        .to_string();
        let exec_arn = sfn
            .start_execution()
            .state_machine_arn(&self.state_machine_arn)
            .input(input)
            .send()
            .await
            .map_err(|e| format!("Step Functions start failed: {e}"))?
            .execution_arn
            .ok_or("Step Functions returned no execution ARN")?;
        tracing::info!("🎬 HyperFrames Lambda execution: {exec_arn}");

        let deadline = tokio::time::Instant::now() + std::time::Duration::from_secs(timeout_secs);
        loop {
            if tokio::time::Instant::now() > deadline {
                return Err(format!("Lambda render still not done after {timeout_secs}s"));
            }
            tokio::time::sleep(std::time::Duration::from_secs(20)).await;
            let desc = sfn
                .describe_execution()
                .execution_arn(&exec_arn)
                .send()
                .await
                .map_err(|e| format!("Step Functions poll failed: {e}"))?;
            match desc.status() {
                aws_sdk_sfn::types::ExecutionStatus::Succeeded => break,
                aws_sdk_sfn::types::ExecutionStatus::Failed
                | aws_sdk_sfn::types::ExecutionStatus::TimedOut
                | aws_sdk_sfn::types::ExecutionStatus::Aborted => {
                    return Err(format!(
                        "Lambda render ended {:?}: {}",
                        desc.status(),
                        desc.output().unwrap_or("no output")
                    ))
                }
                _ => {}
            }
        }

        let obj = s3
            .get_object()
            .bucket(&self.render_bucket)
            .key(&output_key)
            .send()
            .await
            .map_err(|e| format!("S3 output fetch failed: {e}"))?;
        let bytes = obj
            .body
            .collect()
            .await
            .map_err(|e| format!("S3 body read failed: {e}"))?
            .into_bytes();
        if (bytes.len() as u64) < 10_000 {
            return Err(format!("Lambda render too small ({} bytes)", bytes.len()));
        }
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
        let dest = std::format!("outputs/{safe}");
        std::fs::write(&dest, &bytes).map_err(|e| format!("Failed to write {dest}: {e}"))?;
        let _ = std::fs::remove_file(&tar_path);
        let _ = std::fs::remove_dir_all(&proj_dir);
        tracing::info!("✅ HyperFrames Lambda render ready: {dest}");
        Ok(dest)
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
    /// Routes to Step Functions + S3 when HYPERFRAMES_MODE=lambda.
    pub async fn render_html(
        &self,
        html: &str,
        resolution: Option<&str>,
        filename: &str,
        timeout_secs: u64,
    ) -> Result<String, String> {
        if self.mode == "lambda" {
            return self.render_lambda(html, filename, timeout_secs).await;
        }
        let job_id = self.submit(html, resolution).await?;
        tracing::info!("🎬 HyperFrames job submitted: {job_id}");
        self.poll(&job_id, timeout_secs).await?;
        let path = self.download(&job_id, filename).await?;
        tracing::info!("✅ HyperFrames render ready: {path}");
        Ok(path)
    }
}
