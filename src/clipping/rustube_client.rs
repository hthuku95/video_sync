// Pure Rust YouTube video downloader using rustube
// Fallback strategy #1 in the 5-tier system (no external dependencies)

use rustube::{Id, Video};
use std::path::Path;
use tokio::fs;

// Use shared types from apify_client to ensure compatibility
use crate::clipping::apify_client::VideoDownloadResult;
use crate::r2_client::R2Client;

pub struct RustubeClient;

impl RustubeClient {
    /// Download a YouTube video using pure Rust (rustube)
    pub async fn download_video(
        video_url: &str,
        output_path: &str,
        r2_client: Option<&R2Client>,
        r2_key: Option<&str>,
    ) -> Result<VideoDownloadResult, String> {
        tracing::info!("📥 Downloading video from YouTube using Rustube: {}", video_url);

        let video_id = Self::extract_video_id(video_url)?;
        tracing::info!("🔍 Fetching video metadata for ID: {}", video_id);

        let id = Id::from_raw(&video_id)
            .map_err(|e| format!("Invalid video ID {}: {}", video_id, e))?;
        let video = Video::from_id(id.into_owned()).await
            .map_err(|e| format!("Failed to fetch video: {}", e))?;

        let video_details = video.video_details();
        let title = video_details.title.clone();
        let duration_secs = video_details.length_seconds as f64;
        tracing::info!("📹 Video: {} ({}s)", title, duration_secs);

        let streams = video.streams();
        let stream = streams.iter()
            .filter(|s| s.includes_video_track && s.includes_audio_track)
            .max_by_key(|s| s.width.unwrap_or(0))
            .or_else(|| streams.iter().filter(|s| s.includes_video_track).max_by_key(|s| s.width.unwrap_or(0)))
            .ok_or_else(|| "No suitable video stream found".to_string())?;

        let width = stream.width.map(|w| w as i32);
        let height = stream.height.map(|h| h as i32);

        use tokio::time::{timeout, Duration};
        let temp_path = format!("/tmp/rustube_{}_{}.mp4", video_id, std::process::id());

        let download_future = async {
            let downloaded_path = stream.download().await
                .map_err(|e| format!("Failed to download video: {}", e))?;
            fs::rename(&downloaded_path, &temp_path).await
                .map_err(|e| format!("Failed to move downloaded file: {}", e))?;
            Ok::<(), String>(())
        };

        timeout(Duration::from_secs(3600), download_future).await
            .map_err(|_| "Download timed out after 1 hour".to_string())?
            .map_err(|e: String| e)?;

        if let (Some(client), Some(key)) = (r2_client, r2_key) {
            let r2_url = client.upload_file(&temp_path, key).await
                .map_err(|e| format!("R2 upload failed: {e}"))?;
            let _ = fs::remove_file(&temp_path).await;
            tracing::info!("✅ Video uploaded to R2: {r2_url}");
            Ok(VideoDownloadResult {
                file_path: output_path.to_string(),
                title,
                duration_seconds: Some(duration_secs),
                width,
                height,
                r2_url: Some(r2_url),
            })
        } else {
            let dest = if Path::new(output_path).parent().is_some() {
                if let Some(parent) = Path::new(output_path).parent() {
                    fs::create_dir_all(parent).await.map_err(|e| format!("Failed to create dir: {e}"))?;
                }
                output_path
            } else { output_path };
            fs::rename(&temp_path, dest).await
                .map_err(|e| format!("Failed to save final file: {}", e))?;
            Self::validate_download(output_path).await?;
            tracing::info!("✅ Video saved: {}", output_path);
            Ok(VideoDownloadResult {
                file_path: output_path.to_string(),
                title,
                duration_seconds: Some(duration_secs),
                width,
                height,
                r2_url: None,
            })
        }
    }

    /// Extract video ID from various YouTube URL formats
    fn extract_video_id(url: &str) -> Result<String, String> {
        // Handle different YouTube URL formats:
        // - https://www.youtube.com/watch?v=VIDEO_ID
        // - https://youtu.be/VIDEO_ID
        // - https://youtube.com/watch?v=VIDEO_ID
        // - VIDEO_ID (raw ID)

        if let Some(id) = url.strip_prefix("https://www.youtube.com/watch?v=") {
            return Ok(id.split('&').next().unwrap_or(id).to_string());
        }

        if let Some(id) = url.strip_prefix("https://youtu.be/") {
            return Ok(id.split('?').next().unwrap_or(id).to_string());
        }

        if let Some(id) = url.strip_prefix("https://youtube.com/watch?v=") {
            return Ok(id.split('&').next().unwrap_or(id).to_string());
        }

        if let Some(id) = url.strip_prefix("http://www.youtube.com/watch?v=") {
            return Ok(id.split('&').next().unwrap_or(id).to_string());
        }

        if let Some(id) = url.strip_prefix("http://youtube.com/watch?v=") {
            return Ok(id.split('&').next().unwrap_or(id).to_string());
        }

        // If it doesn't match any pattern, assume it's a raw video ID
        if url.len() == 11
            && url
                .chars()
                .all(|c| c.is_alphanumeric() || c == '_' || c == '-')
        {
            return Ok(url.to_string());
        }

        Err(format!("Could not extract video ID from URL: {}", url))
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

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_extract_video_id() {
        assert_eq!(
            RustubeClient::extract_video_id("https://www.youtube.com/watch?v=dQw4w9WgXcQ").unwrap(),
            "dQw4w9WgXcQ"
        );

        assert_eq!(
            RustubeClient::extract_video_id("https://youtu.be/dQw4w9WgXcQ").unwrap(),
            "dQw4w9WgXcQ"
        );

        assert_eq!(
            RustubeClient::extract_video_id("https://youtube.com/watch?v=dQw4w9WgXcQ&t=10s")
                .unwrap(),
            "dQw4w9WgXcQ"
        );

        assert_eq!(
            RustubeClient::extract_video_id("dQw4w9WgXcQ").unwrap(),
            "dQw4w9WgXcQ"
        );
    }
}
