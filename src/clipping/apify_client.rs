// Apify YouTube downloader with yt-dlp fallback
use std::path::Path;
use std::sync::{Arc, Mutex};
use tokio::fs;
use tokio::time::{sleep, Duration, timeout, Instant};
use tokio::io::AsyncWriteExt;
use reqwest::Client;
use serde::{Deserialize, Serialize};
use serde_json::json;

#[derive(Debug)]
pub struct VideoDownloadResult {
    pub file_path: String,
    pub title: String,
    pub duration_seconds: Option<f64>,
    pub width: Option<i32>,
    pub height: Option<i32>,
}

#[derive(Debug)]
pub struct VideoInfo {
    pub video_id: String,
    pub title: String,
    pub duration_seconds: Option<f64>,
    pub channel_id: Option<String>,
    pub channel_name: Option<String>,
    pub upload_date: Option<String>,
}

// Apify API response structures
#[derive(Deserialize)]
struct ApifyRunResponse {
    data: ApifyRun,
}

#[derive(Deserialize)]
struct ApifyRun {
    id: String,
    #[serde(rename = "defaultDatasetId")]
    default_dataset_id: Option<String>,
    status: String, // "READY", "RUNNING", "SUCCEEDED", "FAILED", etc.
}

#[derive(Deserialize)]
struct ApifyDatasetItem {
    title: Option<String>,
    duration: Option<f64>,
    #[serde(rename = "videoUrl")]
    video_url: Option<String>,
    #[serde(rename = "downloadUrl")]
    download_url: Option<String>,
}

/// Circuit breaker states for Apify API
#[derive(Debug, Clone, PartialEq)]
enum CircuitState {
    Closed,   // Normal operation, requests allowed
    Open,     // Too many failures, skip Apify
    HalfOpen, // Testing if Apify recovered
}

/// Circuit breaker for Apify API resilience
struct CircuitBreaker {
    state: CircuitState,
    failure_count: u32,
    success_count: u32,
    last_failure_time: Option<Instant>,
    last_state_change: Instant,
}

impl CircuitBreaker {
    fn new() -> Self {
        Self {
            state: CircuitState::Closed,
            failure_count: 0,
            success_count: 0,
            last_failure_time: None,
            last_state_change: Instant::now(),
        }
    }

    /// Check if requests should be allowed
    fn should_allow_request(&mut self) -> bool {
        match self.state {
            CircuitState::Closed => true,
            CircuitState::Open => {
                // Check if enough time has passed to try half-open
                let timeout_secs = std::env::var("APIFY_CIRCUIT_BREAKER_TIMEOUT_SECONDS")
                    .ok()
                    .and_then(|v| v.parse::<u64>().ok())
                    .unwrap_or(300); // Default: 5 minutes

                if self.last_state_change.elapsed().as_secs() >= timeout_secs {
                    tracing::info!("🟡 Circuit breaker: OPEN → HALF_OPEN (testing recovery)");
                    self.state = CircuitState::HalfOpen;
                    self.last_state_change = Instant::now();
                    true
                } else {
                    false
                }
            }
            CircuitState::HalfOpen => true,
        }
    }

    /// Record successful request
    fn record_success(&mut self) {
        self.success_count += 1;
        self.failure_count = 0;
        self.last_failure_time = None;

        match self.state {
            CircuitState::HalfOpen => {
                let success_threshold = std::env::var("APIFY_CIRCUIT_BREAKER_SUCCESS_THRESHOLD")
                    .ok()
                    .and_then(|v| v.parse::<u32>().ok())
                    .unwrap_or(2);

                if self.success_count >= success_threshold {
                    tracing::info!("🟢 Circuit breaker: HALF_OPEN → CLOSED (recovery confirmed)");
                    self.state = CircuitState::Closed;
                    self.success_count = 0;
                    self.last_state_change = Instant::now();
                }
            }
            _ => {}
        }
    }

    /// Record failed request
    fn record_failure(&mut self, is_auth_error: bool) {
        self.failure_count += 1;
        self.success_count = 0;
        self.last_failure_time = Some(Instant::now());

        let failure_threshold = std::env::var("APIFY_CIRCUIT_BREAKER_FAILURE_THRESHOLD")
            .ok()
            .and_then(|v| v.parse::<u32>().ok())
            .unwrap_or(5);

        // Auth errors (403) trigger circuit immediately after 3 failures
        let threshold = if is_auth_error { 3 } else { failure_threshold };

        match self.state {
            CircuitState::Closed => {
                if self.failure_count >= threshold {
                    tracing::warn!(
                        "🔴 Circuit breaker: CLOSED → OPEN ({} consecutive failures)",
                        self.failure_count
                    );
                    self.state = CircuitState::Open;
                    self.last_state_change = Instant::now();
                }
            }
            CircuitState::HalfOpen => {
                tracing::warn!("🔴 Circuit breaker: HALF_OPEN → OPEN (recovery failed)");
                self.state = CircuitState::Open;
                self.failure_count = 0;
                self.last_state_change = Instant::now();
            }
            _ => {}
        }
    }

    fn get_state(&self) -> CircuitState {
        self.state.clone()
    }
}

pub struct ApifyClient {
    api_token: String,
    actor_id: String,
    http_client: Client,
    circuit_breaker: Arc<Mutex<CircuitBreaker>>,
}

impl ApifyClient {
    pub fn new(api_token: String, actor_id: String) -> Self {
        Self {
            api_token,
            actor_id,
            http_client: Client::builder()
                .timeout(Duration::from_secs(3600))
                .build()
                .expect("Failed to create HTTP client"),
            circuit_breaker: Arc::new(Mutex::new(CircuitBreaker::new())),
        }
    }

    /// Check if status code indicates a transient failure (should retry)
    fn is_transient_error(status: u16) -> bool {
        matches!(status, 429 | 503 | 504 | 408)
    }

    /// Retry logic with exponential backoff for transient failures
    /// Returns Ok(response) on success, Err(message) on permanent failure
    async fn retry_with_backoff<F, Fut>(
        &self,
        operation: F,
        operation_name: &str,
    ) -> Result<reqwest::Response, String>
    where
        F: Fn() -> Fut,
        Fut: std::future::Future<Output = Result<reqwest::Response, reqwest::Error>>,
    {
        let max_retries = 3;
        let mut delays = vec![
            Duration::from_secs(5),  // 1st retry: 5 seconds
            Duration::from_secs(15), // 2nd retry: 15 seconds
            Duration::from_secs(45), // 3rd retry: 45 seconds
        ];

        for attempt in 0..=max_retries {
            match operation().await {
                Ok(response) => {
                    let status = response.status();

                    if status.is_success() {
                        return Ok(response);
                    }

                    // Check if it's a transient error that should be retried
                    if Self::is_transient_error(status.as_u16()) && attempt < max_retries {
                        let delay = delays[attempt];
                        tracing::warn!(
                            "⚠️ {} returned {} (transient error), retrying in {}s (attempt {}/{})",
                            operation_name,
                            status,
                            delay.as_secs(),
                            attempt + 1,
                            max_retries
                        );
                        sleep(delay).await;
                        continue;
                    }

                    // Permanent error (403, 400, etc.) or max retries exceeded
                    return Ok(response); // Let caller handle the error
                }
                Err(e) => {
                    // Network errors - retry if not max attempts
                    if attempt < max_retries {
                        let delay = delays[attempt];
                        tracing::warn!(
                            "⚠️ {} network error: {}, retrying in {}s (attempt {}/{})",
                            operation_name,
                            e,
                            delay.as_secs(),
                            attempt + 1,
                            max_retries
                        );
                        sleep(delay).await;
                        continue;
                    }

                    return Err(format!("Network error after {} retries: {}", max_retries, e));
                }
            }
        }

        Err(format!("{} failed after {} retries", operation_name, max_retries))
    }

    /// Validate API token on startup
    ///
    /// Tests API token by making a simple GET request to the actor endpoint.
    /// Returns Ok(()) if token is valid, Err(message) if invalid or other error.
    pub async fn validate_token(&self) -> Result<(), String> {
        let url = format!(
            "https://api.apify.com/v2/acts/{}/runs/last",
            self.actor_id
        );

        let response = self
            .http_client
            .get(&url)
            .header("Authorization", format!("Bearer {}", self.api_token))
            .send()
            .await
            .map_err(|e| format!("Network error during token validation: {}", e))?;

        if response.status().is_success() || response.status() == 404 {
            // 404 means actor exists but no runs yet - token is valid
            Ok(())
        } else if response.status() == 401 || response.status() == 403 {
            Err(format!("Invalid Apify API token (status: {})", response.status()))
        } else {
            Err(format!("Unexpected API response: {}", response.status()))
        }
    }

    /// Download video using Apify (primary) with yt-dlp fallback
    pub async fn download_video(
        &self,
        video_url: &str,
        output_path: &str,
    ) -> Result<VideoDownloadResult, String> {
        // Ensure parent directory exists
        if let Some(parent) = Path::new(output_path).parent() {
            fs::create_dir_all(parent)
                .await
                .map_err(|e| format!("Failed to create output directory: {}", e))?;
        }

        tracing::info!("📥 Attempting video download: {}", video_url);

        // Check circuit breaker before attempting Apify
        let should_try_apify = {
            let mut breaker = self.circuit_breaker.lock().unwrap();
            breaker.should_allow_request()
        };

        if !should_try_apify {
            let state = {
                let breaker = self.circuit_breaker.lock().unwrap();
                breaker.get_state()
            };
            tracing::warn!(
                "⚠️ Circuit breaker {:?} - Skipping Apify, using yt-dlp directly",
                state
            );
            return self.download_via_ytdlp(video_url, output_path).await;
        }

        // Try Apify first
        match self.download_via_apify(video_url, output_path).await {
            Ok(result) => {
                tracing::info!("✅ Apify download successful");
                // Record success in circuit breaker
                let mut breaker = self.circuit_breaker.lock().unwrap();
                breaker.record_success();
                return Ok(result);
            }
            Err(e) => {
                // Record failure in circuit breaker
                let is_auth_error = e.contains("403") || e.contains("401");
                {
                    let mut breaker = self.circuit_breaker.lock().unwrap();
                    breaker.record_failure(is_auth_error);
                }

                tracing::warn!("⚠️ Apify download failed: {}", e);
                tracing::info!("🔄 Falling back to yt-dlp...");
            }
        }

        // Fallback to yt-dlp
        self.download_via_ytdlp(video_url, output_path).await
    }

    /// Download via Apify API (with residential proxies)
    async fn download_via_apify(
        &self,
        video_url: &str,
        output_path: &str,
    ) -> Result<VideoDownloadResult, String> {
        tracing::info!("🎬 Starting Apify actor run for: {}", video_url);

        // Step 1: Start Apify actor run
        let run_url = format!(
            "https://api.apify.com/v2/acts/{}/runs?token={}",
            self.actor_id, self.api_token
        );

        let input = json!({
            "startUrls": [video_url],
            "quality": "720",
            "includeFailedVideos": false,
            "proxy": {
                "useApifyProxy": true,
                "apifyProxyGroups": ["RESIDENTIAL"]
            }
        });

        // Use retry logic for starting Apify run
        let run_response = self
            .retry_with_backoff(
                || {
                    self.http_client
                        .post(&run_url)
                        .header("Authorization", format!("Bearer {}", self.api_token))
                        .json(&input)
                        .send()
                },
                "Start Apify run",
            )
            .await
            .map_err(|e| format!("Failed to start Apify run: {}", e))?;

        if !run_response.status().is_success() {
            let status = run_response.status();
            let error_body = run_response
                .text()
                .await
                .unwrap_or_else(|_| "Could not read error body".to_string());

            tracing::error!(
                "❌ Apify API error: status={}, body={}",
                status,
                error_body
            );

            return Err(format!("Apify API error: {} - {}", status, error_body));
        }

        let run_data: ApifyRunResponse = run_response
            .json()
            .await
            .map_err(|e| format!("Failed to parse run response: {}", e))?;

        let run_id = run_data.data.id;
        tracing::info!("📊 Apify run started: {}", run_id);

        // Step 2: Poll for completion (max 10 minutes)
        let status_url = format!(
            "https://api.apify.com/v2/acts/{}/runs/{}?token={}",
            self.actor_id, run_id, self.api_token
        );

        let max_polls = 120; // 10 minutes (5 sec intervals)
        let mut dataset_id = None;

        for i in 0..max_polls {
            sleep(Duration::from_secs(5)).await;

            // Use retry logic for status checks (transient failures are common)
            let status_response = self
                .retry_with_backoff(
                    || {
                        self.http_client
                            .get(&status_url)
                            .header("Authorization", format!("Bearer {}", self.api_token))
                            .send()
                    },
                    "Check run status",
                )
                .await
                .map_err(|e| format!("Failed to check run status: {}", e))?;

            let status_data: ApifyRunResponse = status_response
                .json()
                .await
                .map_err(|e| format!("Failed to parse status: {}", e))?;

            tracing::info!("⏳ Apify run status: {} (poll {}/{})", status_data.data.status, i+1, max_polls);

            match status_data.data.status.as_str() {
                "SUCCEEDED" => {
                    dataset_id = status_data.data.default_dataset_id;
                    break;
                }
                "FAILED" | "TIMED-OUT" | "ABORTED" => {
                    return Err(format!("Apify run failed: {}", status_data.data.status));
                }
                _ => continue, // RUNNING, READY, etc.
            }
        }

        let dataset_id = dataset_id.ok_or("Apify run timed out after 10 minutes")?;

        // Step 3: Fetch dataset items
        tracing::info!("📦 Fetching dataset: {}", dataset_id);
        let dataset_url = format!(
            "https://api.apify.com/v2/datasets/{}/items?token={}",
            dataset_id, self.api_token
        );

        // Use retry logic for dataset fetching
        let items_response = self
            .retry_with_backoff(
                || {
                    self.http_client
                        .get(&dataset_url)
                        .header("Authorization", format!("Bearer {}", self.api_token))
                        .send()
                },
                "Fetch dataset",
            )
            .await
            .map_err(|e| format!("Failed to fetch dataset: {}", e))?;

        let items: Vec<ApifyDatasetItem> = items_response
            .json()
            .await
            .map_err(|e| format!("Failed to parse dataset: {}", e))?;

        let item = items.first().ok_or("No video in dataset")?;

        let download_url = item
            .download_url
            .as_ref()
            .ok_or("No download URL in dataset")?;

        // Step 4: Download video file from Apify storage
        tracing::info!("⬇️ Downloading video from Apify storage");
        let video_response = self
            .http_client
            .get(download_url)
            .send()
            .await
            .map_err(|e| format!("Failed to download video: {}", e))?;

        // Write to file
        let mut file = fs::File::create(output_path)
            .await
            .map_err(|e| format!("Failed to create output file: {}", e))?;

        let bytes = video_response
            .bytes()
            .await
            .map_err(|e| format!("Failed to read video bytes: {}", e))?;

        file.write_all(&bytes)
            .await
            .map_err(|e| format!("Failed to write video file: {}", e))?;

        tracing::info!("💾 Video saved: {}", output_path);

        // Validate downloaded file
        Self::validate_download(output_path).await?;

        Ok(VideoDownloadResult {
            file_path: output_path.to_string(),
            title: item.title.clone().unwrap_or_else(|| "Unknown".to_string()),
            duration_seconds: item.duration,
            width: None,
            height: None,
        })
    }

    /// Fallback: Download via yt-dlp subprocess
    async fn download_via_ytdlp(
        &self,
        video_url: &str,
        output_path: &str,
    ) -> Result<VideoDownloadResult, String> {
        use tokio::process::Command;
        use std::process::Stdio;

        tracing::info!("🔧 Using yt-dlp fallback for: {}", video_url);

        // First get video info
        let info = Self::get_video_info_ytdlp(video_url).await?;

        // Download video
        let mut cmd = Command::new("yt-dlp");
        cmd.args([
            "--format", "bestvideo[ext=mp4]+bestaudio[ext=m4a]/best[ext=mp4]/best",
            "--merge-output-format", "mp4",
            "--output", output_path,
            "--user-agent", "Mozilla/5.0 (Windows NT 10.0; Win64; x64) AppleWebKit/537.36",
            "--retries", "3",
            "--no-playlist",
            "--socket-timeout", "600",
            video_url,
        ]);

        cmd.stdout(Stdio::piped());
        cmd.stderr(Stdio::piped());

        let output = timeout(Duration::from_secs(3600), cmd.output())
            .await
            .map_err(|_| "yt-dlp timed out after 1 hour".to_string())?
            .map_err(|e| format!("Failed to execute yt-dlp: {}", e))?;

        if !output.status.success() {
            let stderr = String::from_utf8_lossy(&output.stderr);
            return Err(format!("yt-dlp failed: {}", stderr));
        }

        Self::validate_download(output_path).await?;

        Ok(VideoDownloadResult {
            file_path: output_path.to_string(),
            title: info.title,
            duration_seconds: info.duration_seconds,
            width: None,
            height: None,
        })
    }

    /// Get video info via yt-dlp
    async fn get_video_info_ytdlp(video_url: &str) -> Result<VideoInfo, String> {
        use tokio::process::Command;
        use std::process::Stdio;
        use serde_json::Value;

        let mut cmd = Command::new("yt-dlp");
        cmd.args(["--dump-json", "--no-playlist", video_url]);
        cmd.stdout(Stdio::piped());
        cmd.stderr(Stdio::piped());

        let output = cmd
            .output()
            .await
            .map_err(|e| format!("Failed to get video info: {}", e))?;

        if !output.status.success() {
            return Err("Failed to fetch video metadata".to_string());
        }

        let stdout = String::from_utf8_lossy(&output.stdout);
        let json: Value = serde_json::from_str(&stdout)
            .map_err(|e| format!("Failed to parse metadata: {}", e))?;

        Ok(VideoInfo {
            video_id: json["id"].as_str().unwrap_or("").to_string(),
            title: json["title"].as_str().unwrap_or("Unknown").to_string(),
            duration_seconds: json["duration"].as_f64(),
            channel_id: json["channel_id"].as_str().map(|s| s.to_string()),
            channel_name: json["uploader"].as_str().map(|s| s.to_string()),
            upload_date: json["upload_date"].as_str().map(|s| s.to_string()),
        })
    }

    /// Validate downloaded video file
    async fn validate_download(output_path: &str) -> Result<(), String> {
        tracing::info!("🔍 Validating downloaded video");

        if !Path::new(output_path).exists() {
            return Err(format!("File does not exist: {}", output_path));
        }

        let metadata = fs::metadata(output_path)
            .await
            .map_err(|e| format!("Failed to read metadata: {}", e))?;

        if metadata.len() < 1_000_000 {
            return Err(format!(
                "File too small ({} bytes): {}",
                metadata.len(),
                output_path
            ));
        }

        tracing::info!(
            "✅ File validated ({:.2} MB)",
            metadata.len() as f64 / 1_000_000.0
        );

        // Optional: ffprobe validation
        match crate::core::validate_video_file(output_path) {
            Ok(true) => Ok(()),
            Ok(false) => {
                let _ = fs::remove_file(output_path).await;
                Err("Video corrupted".to_string())
            }
            Err(e) => {
                tracing::warn!("⚠️ Validation warning: {}", e);
                Ok(()) // Continue anyway
            }
        }
    }

    /// Get video info (use Apify or fallback)
    pub async fn get_video_info(&self, video_url: &str) -> Result<VideoInfo, String> {
        // For simplicity, use yt-dlp for metadata (faster than running full Apify job)
        Self::get_video_info_ytdlp(video_url).await
    }
}
