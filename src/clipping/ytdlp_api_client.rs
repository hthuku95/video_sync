#![allow(dead_code, unused_imports)]
/*!
 * HTTP client for FastAPI yt-dlp microservice
 *
 * Replaces Strategy #3 (yt-dlp CLI subprocess) with reliable HTTP API calls
 * to the standalone FastAPI microservice.
 */

use reqwest::Client;
use serde::{Deserialize, Serialize};
use std::env;
use std::path::Path;
use std::time::Duration;
use tokio::fs::File;
use tokio::io::AsyncWriteExt;
use tracing::{error, info, warn};

// ============================================================================
// Request/Response Models (matches FastAPI schemas)
// ============================================================================

#[derive(Debug, Serialize)]
struct DownloadRequest {
    video_url: String,
    #[serde(skip_serializing_if = "Option::is_none")]
    job_id: Option<String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    quality: Option<String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    format: Option<String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    prefer_base64: Option<bool>,
    #[serde(skip_serializing_if = "Option::is_none")]
    timeout_seconds: Option<u32>,
}

#[derive(Debug, Deserialize)]
struct DownloadResponse {
    success: bool,
    method: String,
    #[serde(skip_serializing_if = "Option::is_none")]
    download_url: Option<String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    file_data: Option<String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    expires_at: Option<String>,
    metadata: VideoMetadata,
}

#[derive(Debug, Deserialize)]
struct VideoMetadata {
    title: String,
    duration_seconds: f64,
    #[serde(skip_serializing_if = "Option::is_none")]
    width: Option<u32>,
    #[serde(skip_serializing_if = "Option::is_none")]
    height: Option<u32>,
    #[serde(skip_serializing_if = "Option::is_none")]
    file_size_bytes: Option<u64>,
    format: String,
}

#[derive(Debug, Deserialize)]
struct ErrorResponse {
    success: bool,
    error: ErrorDetail,
}

#[derive(Debug, Deserialize)]
struct ErrorDetail {
    code: String,
    message: String,
    is_transient: bool,
    #[serde(skip_serializing_if = "Option::is_none")]
    retry_after_seconds: Option<u32>,
}

#[derive(Debug, Serialize)]
struct InfoRequest {
    video_url: String,
    #[serde(skip_serializing_if = "Option::is_none")]
    include_formats: Option<bool>,
}

#[derive(Debug, Deserialize)]
struct InfoResponse {
    success: bool,
    metadata: VideoMetadata,
}

// ============================================================================
// VideoDownloadResult (shared with other strategies)
// ============================================================================

/// Shared result type used across all download strategies
#[derive(Debug, Clone)]
pub struct VideoDownloadResult {
    pub file_path: String,
    pub title: String,
    pub duration_seconds: f64,
    pub width: Option<u32>,
    pub height: Option<u32>,
}

// ============================================================================
// YtdlpApiClient
// ============================================================================

#[derive(Debug)]
pub struct YtdlpApiClient {
    base_url: String,
    http_client: Client,
}

impl YtdlpApiClient {
    /// Create new client from YTDLP_API_URL environment variable
    ///
    /// Returns Err if YTDLP_API_URL is not set (fast-fail, no unnecessary retries)
    pub fn new() -> Result<Self, String> {
        let base_url = env::var("YTDLP_API_URL")
            .map_err(|_| "YTDLP_API_URL environment variable not set".to_string())?;

        if base_url.is_empty() {
            return Err("YTDLP_API_URL is empty".to_string());
        }

        let http_client = Client::builder()
            .timeout(Duration::from_secs(600)) // 10 minutes per request (proxy strategies can be slow)
            .connect_timeout(Duration::from_secs(30))
            .build()
            .map_err(|e| format!("Failed to create HTTP client: {}", e))?;

        info!("YtdlpApiClient initialized with base_url: {}", base_url);

        Ok(Self {
            base_url,
            http_client,
        })
    }

    /// Wake up the YTDLP microservice before the first real download call.
    ///
    /// In production we currently front this service with Cloud Run. We prefer
    /// the explicit health endpoint, but in practice a service can be fully
    /// alive while a single probe path is slow or temporarily unhealthy.
    ///
    /// To avoid false negatives, we treat the service as awake if any of a
    /// small set of lightweight documented endpoints responds successfully.
    async fn warm_up_service(client: &reqwest::Client, base_url: &str) -> Result<(), String> {
        let probe_endpoints = [
            "/api/v1/health",
            "/api/v1/strategies",
            "/openapi.json",
            "/docs",
            "/",
        ];
        let deadline = tokio::time::Instant::now() + Duration::from_secs(60);
        info!("⏳ Warming up YTDLP microservice at {}...", base_url);

        loop {
            for endpoint in probe_endpoints {
                let url = format!("{}{}", base_url, endpoint);

                match client.get(&url).timeout(Duration::from_secs(10)).send().await {
                    Ok(resp) if resp.status().is_success() => {
                        info!(
                            "✅ YTDLP microservice is awake (probe passed at {})",
                            endpoint
                        );
                        return Ok(());
                    }
                    Ok(resp) => {
                        warn!(
                            "YTDLP probe {} returned {} — trying next probe",
                            endpoint,
                            resp.status()
                        );
                    }
                    Err(e) => {
                        warn!("YTDLP probe {} failed: {} — trying next probe", endpoint, e);
                    }
                }
            }

            if tokio::time::Instant::now() >= deadline {
                return Err(
                    "YTDLP microservice did not become healthy within 60 seconds".to_string(),
                );
            }

            tokio::time::sleep(Duration::from_secs(5)).await;
        }
    }

    /// Download video via FastAPI microservice
    ///
    /// Replaces Strategy #3 (yt-dlp CLI subprocess) in the fallback chain
    pub async fn download_video(
        video_url: &str,
        output_path: &str,
    ) -> Result<VideoDownloadResult, String> {
        info!(
            "🌐 YtdlpApiClient::download_video starting: {} → {}",
            video_url, output_path
        );

        // Create client (fast-fail if env var not set)
        let client = Self::new()?;

        // Wake up Render free-tier service before the download request
        Self::warm_up_service(&client.http_client, &client.base_url).await?;

        // Extract job_id from output path for tracking
        let job_id = Path::new(output_path)
            .file_stem()
            .and_then(|s| s.to_str())
            .map(|s| s.to_string());

        // Retry logic: 3 attempts with exponential backoff (5s, 15s, 45s)
        let max_retries = 3;
        let mut retry_count = 0;

        loop {
            match client
                ._download_attempt(video_url, output_path, job_id.clone())
                .await
            {
                Ok(result) => {
                    info!(
                        "✅ YtdlpApiClient download succeeded on attempt {}/{}",
                        retry_count + 1,
                        max_retries
                    );
                    return Ok(result);
                }
                Err(e) => {
                    retry_count += 1;

                    // Check if error is transient
                    let is_transient = e.contains("transient")
                        || e.contains("timeout")
                        || e.contains("network")
                        || e.contains("429")
                        || e.contains("500")
                        || e.contains("502")
                        || e.contains("503")
                        || e.contains("504");

                    if !is_transient || retry_count >= max_retries {
                        error!("❌ YtdlpApiClient download failed permanently: {}", e);
                        return Err(e);
                    }

                    let backoff_secs = match retry_count {
                        1 => 5,
                        2 => 15,
                        _ => 45,
                    };

                    warn!("⚠️ YtdlpApiClient attempt {}/{} failed (transient): {}. Retrying in {}s...",
                          retry_count, max_retries, e, backoff_secs);

                    tokio::time::sleep(Duration::from_secs(backoff_secs)).await;
                }
            }
        }
    }

    /// Single download attempt (no retries)
    async fn _download_attempt(
        &self,
        video_url: &str,
        output_path: &str,
        job_id: Option<String>,
    ) -> Result<VideoDownloadResult, String> {
        // Build request payload
        let request_payload = DownloadRequest {
            video_url: video_url.to_string(),
            job_id,
            quality: Some("720p".to_string()), // Default quality
            format: Some("mp4".to_string()),
            prefer_base64: Some(false), // Always use URL mode for large files
            timeout_seconds: Some(600), // 10 minutes (matches HTTP client timeout)
        };

        // POST to /api/v1/download
        let endpoint = format!("{}/api/v1/download", self.base_url);
        info!("📤 POST {}", endpoint);

        let response = self
            .http_client
            .post(&endpoint)
            .json(&request_payload)
            .send()
            .await
            .map_err(|e| format!("HTTP request failed: {}", e))?;

        let status = response.status();
        info!("📥 Response status: {}", status);

        // Handle error responses
        if !status.is_success() {
            let error_text = response
                .text()
                .await
                .unwrap_or_else(|_| "Unknown error".to_string());

            // Try to parse as ErrorResponse
            if let Ok(error_response) = serde_json::from_str::<ErrorResponse>(&error_text) {
                let is_transient_tag = if error_response.error.is_transient {
                    " (transient)"
                } else {
                    ""
                };
                return Err(format!(
                    "FastAPI error [{}]{}: {}",
                    error_response.error.code, is_transient_tag, error_response.error.message
                ));
            }

            return Err(format!("HTTP {} error: {}", status, error_text));
        }

        // Parse success response
        let response_text = response
            .text()
            .await
            .map_err(|e| format!("Failed to read response body: {}", e))?;

        let download_response: DownloadResponse =
            serde_json::from_str(&response_text).map_err(|e| {
                format!(
                    "Failed to parse response JSON: {} (body: {})",
                    e, response_text
                )
            })?;

        if !download_response.success {
            return Err("Response indicated failure but had 200 status".to_string());
        }

        info!("📦 Download method: {}", download_response.method);

        // Handle different response methods — stream to disk, never buffer entire video in RAM
        let mut file = File::create(output_path)
            .await
            .map_err(|e| format!("Failed to create output file: {}", e))?;

        match download_response.method.as_str() {
            "url" => {
                // Pattern A/B: stream directly from download_url → disk (no full-file buffer)
                let download_url = download_response
                    .download_url
                    .ok_or("download_url missing in URL mode")?;

                let full_url = if download_url.starts_with("http") {
                    download_url
                } else {
                    format!("{}{}", self.base_url, download_url)
                };

                info!("📥 Streaming download from: {}", full_url);

                let file_response = self
                    .http_client
                    .get(&full_url)
                    .send()
                    .await
                    .map_err(|e| format!("Failed to start file download: {}", e))?;

                if !file_response.status().is_success() {
                    return Err(format!(
                        "File download failed with status: {}",
                        file_response.status()
                    ));
                }

                use futures::StreamExt;
                let mut stream = file_response.bytes_stream();
                let mut bytes_written: u64 = 0;
                while let Some(chunk) = stream.next().await {
                    let chunk = chunk.map_err(|e| format!("Stream read error: {}", e))?;
                    file.write_all(&chunk)
                        .await
                        .map_err(|e| format!("Failed to write chunk: {}", e))?;
                    bytes_written += chunk.len() as u64;
                }
                info!("💾 Streamed {} bytes to {}", bytes_written, output_path);
            }
            "base64" => {
                // Pattern C: base64 payload — cap at 600 MB decoded to protect RAM
                let file_data = download_response
                    .file_data
                    .ok_or("file_data missing in base64 mode")?;

                const MAX_BASE64_CHARS: usize = 800_000_000; // ~600 MB decoded
                if file_data.len() > MAX_BASE64_CHARS {
                    return Err(format!(
                        "base64 payload too large ({} chars, max {}). Use URL mode.",
                        file_data.len(),
                        MAX_BASE64_CHARS
                    ));
                }

                info!("📦 Decoding base64 data ({} chars)", file_data.len());
                use base64::prelude::*;
                let decoded = BASE64_STANDARD
                    .decode(&file_data)
                    .map_err(|e| format!("Failed to decode base64: {}", e))?;
                info!("💾 Writing {} bytes to {}", decoded.len(), output_path);
                file.write_all(&decoded)
                    .await
                    .map_err(|e| format!("Failed to write base64 data: {}", e))?;
            }
            method => {
                return Err(format!("Unknown download method: {}", method));
            }
        }

        file.flush()
            .await
            .map_err(|e| format!("Failed to flush file: {}", e))?;

        // Validate file size
        let file_size = tokio::fs::metadata(output_path)
            .await
            .map_err(|e| format!("Failed to read file metadata: {}", e))?
            .len();

        if file_size == 0 {
            return Err("Downloaded file is empty".to_string());
        }

        info!("✅ File written successfully: {} bytes", file_size);

        // Build result
        Ok(VideoDownloadResult {
            file_path: output_path.to_string(),
            title: download_response.metadata.title,
            duration_seconds: download_response.metadata.duration_seconds,
            width: download_response.metadata.width,
            height: download_response.metadata.height,
        })
    }

}

#[cfg(test)]
mod tests {
    use super::*;

    #[tokio::test]
    async fn test_client_creation_fails_without_env_var() {
        env::remove_var("YTDLP_API_URL");
        let result = YtdlpApiClient::new();
        assert!(result.is_err());
        assert!(result.unwrap_err().contains("YTDLP_API_URL"));
    }

    #[tokio::test]
    async fn test_client_creation_succeeds_with_env_var() {
        env::set_var("YTDLP_API_URL", "http://localhost:8000");
        let result = YtdlpApiClient::new();
        assert!(result.is_ok());
        env::remove_var("YTDLP_API_URL");
    }
}
