// rust-yt-downloader client wrapper
// Fallback strategy #3 in the 5-tier system (feature-rich yt-dlp wrapper)

use std::path::Path;
use tokio::fs;

// Use shared types from apify_client to ensure compatibility
use crate::clipping::apify_client::VideoDownloadResult;

pub struct RustYtDownloaderClient;

impl RustYtDownloaderClient {
    /// Download a YouTube video using rust-yt-downloader (CLI wrapper for yt-dlp)
    pub async fn download_video(
        video_url: &str,
        output_path: &str,
    ) -> Result<VideoDownloadResult, String> {
        // Ensure parent directory exists
        if let Some(parent) = Path::new(output_path).parent() {
            fs::create_dir_all(parent)
                .await
                .map_err(|e| format!("Failed to create output directory: {}", e))?;
        }

        tracing::info!("📥 Downloading video using rust-yt-downloader: {}", video_url);

        // Use rust_yt_downloader crate (requires yt-dlp installed)
        use rust_yt_downloader::YtDlpClient;

        // Check if yt-dlp is available
        if !YtDlpClient::is_available() {
            return Err("yt-dlp is not installed or not available in PATH".to_string());
        }

        // Get video info first
        tracing::info!("🔍 Fetching video metadata");

        let video_info = tokio::task::spawn_blocking({
            let url = video_url.to_string();
            let client = YtDlpClient::new(); // Create client inside closure
            move || client.get_video_info(&url)
        })
        .await
        .map_err(|e| format!("Task join error: {}", e))?
        .map_err(|e| format!("Failed to fetch video info: {}", e))?;

        let title = video_info.title.clone();
        let duration_secs = Some(video_info.duration as f64);

        tracing::info!("📹 Video: {}", title);
        tracing::info!("⏱️ Duration: {:.1}s", video_info.duration);

        // Download video with timeout
        use tokio::time::{timeout, Duration};

        tracing::info!("⬇️ Starting download with 1-hour timeout");

        let download_future = tokio::task::spawn_blocking({
            let url = video_url.to_string();
            let output = output_path.to_string();
            let client = YtDlpClient::new(); // Create separate client for download
            move || client.download(&url, &output, None) // None means best quality
        });

        let result = timeout(Duration::from_secs(3600), download_future)
            .await
            .map_err(|_| "Download timed out after 1 hour".to_string())?
            .map_err(|e| format!("Task join error: {}", e))?
            .map_err(|e| format!("Failed to download video: {}", e))?;

        tracing::info!("💾 Video saved: {}", output_path);

        // Validate downloaded file
        Self::validate_download(output_path).await?;

        tracing::info!("✅ Video downloaded successfully with rust-yt-downloader: {}", output_path);

        Ok(VideoDownloadResult {
            file_path: output_path.to_string(),
            title,
            duration_seconds: duration_secs,
            width: None,  // rust-yt-downloader doesn't expose dimensions easily
            height: None,
        })
    }

    /// Validate the downloaded video file
    async fn validate_download(output_path: &str) -> Result<(), String> {
        tracing::info!("🔍 Validating downloaded video file");

        // Check 1: File exists
        if !Path::new(output_path).exists() {
            return Err(format!("Downloaded file does not exist: {}", output_path));
        }

        // Check 2: File size is reasonable (> 1MB for videos)
        let metadata = fs::metadata(output_path)
            .await
            .map_err(|e| format!("Failed to read file metadata: {}", e))?;

        if metadata.len() < 1_000_000 {
            return Err(format!(
                "Downloaded file is suspiciously small ({} bytes): {}",
                metadata.len(),
                output_path
            ));
        }

        tracing::info!(
            "✅ File exists and has reasonable size ({:.2} MB)",
            metadata.len() as f64 / 1_000_000.0
        );

        // Check 3: Validate with ffprobe (use existing validate_video_file)
        match crate::core::validate_video_file(output_path) {
            Ok(true) => {
                tracing::info!("✅ Downloaded video validated successfully");
                Ok(())
            }
            Ok(false) => {
                // Clean up corrupted download
                let _ = fs::remove_file(output_path).await;
                Err(format!(
                    "Downloaded video is corrupted or unreadable: {}",
                    output_path
                ))
            }
            Err(e) => {
                tracing::warn!("⚠️ Video validation check failed: {}", e);
                // Continue anyway - validation might fail for other reasons
                Ok(())
            }
        }
    }
}
