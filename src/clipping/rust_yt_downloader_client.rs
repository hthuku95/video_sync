// rust-yt-downloader client wrapper
// Fallback strategy #3 in the 5-tier system (feature-rich yt-dlp wrapper)

use std::path::Path;
use tokio::fs;

// Use shared types from apify_client to ensure compatibility
use crate::clipping::apify_client::VideoDownloadResult;
use crate::r2_client::R2Client;

pub struct RustYtDownloaderClient;

impl RustYtDownloaderClient {
    /// Download a YouTube video using rust-yt-downloader (CLI wrapper for yt-dlp)
    pub async fn download_video(
        video_url: &str,
        output_path: &str,
        r2_client: Option<&R2Client>,
        r2_key: Option<&str>,
    ) -> Result<VideoDownloadResult, String> {
        tracing::info!("📥 Downloading video using rust-yt-downloader: {}", video_url);

        use rust_yt_downloader::YtDlpClient;
        if !YtDlpClient::is_available() {
            return Err("yt-dlp is not installed or not available in PATH".to_string());
        }

        tracing::info!("🔍 Fetching video metadata");
        let video_info = tokio::task::spawn_blocking({
            let url = video_url.to_string();
            let client = YtDlpClient::new();
            move || client.get_video_info(&url)
        }).await
        .map_err(|e| format!("Task join error: {}", e))?
        .map_err(|e| format!("Failed to fetch video info: {}", e))?;

        let title = video_info.title.clone();
        let duration_secs = Some(video_info.duration as f64);

        use tokio::time::{timeout, Duration};
        let temp_path = format!("/tmp/rustytdownloader_{}_{}.mp4", std::process::id(), std::time::SystemTime::now()
            .duration_since(std::time::UNIX_EPOCH).map(|d| d.as_nanos()).unwrap_or(0));

        tracing::info!("⬇️ Starting download with 1-hour timeout");
        let download_future = tokio::task::spawn_blocking({
            let url = video_url.to_string();
            let output = temp_path.clone();
            let client = YtDlpClient::new();
            move || client.download(&url, &output, None)
        });

        timeout(Duration::from_secs(3600), download_future).await
            .map_err(|_| "Download timed out after 1 hour".to_string())?
            .map_err(|e| format!("Task join error: {}", e))?
            .map_err(|e| format!("Failed to download video: {}", e))?;

        if let (Some(client), Some(key)) = (r2_client, r2_key) {
            let r2_url = client.upload_file(&temp_path, key).await
                .map_err(|e| format!("R2 upload failed: {e}"))?;
            let _ = fs::remove_file(&temp_path).await;
            tracing::info!("✅ Video uploaded to R2: {r2_url}");
            Ok(VideoDownloadResult {
                file_path: output_path.to_string(),
                title,
                duration_seconds: duration_secs,
                width: None,
                height: None,
                r2_url: Some(r2_url),
            })
        } else {
            if let Some(parent) = Path::new(output_path).parent() {
                fs::create_dir_all(parent).await
                    .map_err(|e| format!("Failed to create output directory: {}", e))?;
            }
            fs::rename(&temp_path, output_path).await
                .map_err(|e| format!("Failed to save final file: {}", e))?;
            Self::validate_download(output_path).await?;
            tracing::info!("✅ Video saved: {}", output_path);
            Ok(VideoDownloadResult {
                file_path: output_path.to_string(),
                title,
                duration_seconds: duration_secs,
                width: None,
                height: None,
                r2_url: None,
            })
        }
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
