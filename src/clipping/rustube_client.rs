// Pure Rust YouTube video downloader using rustube
// Fallback strategy #1 in the 5-tier system (no external dependencies)

use rustube::{Id, Video};
use std::path::Path;
use tokio::fs;

// Use shared types from apify_client to ensure compatibility
use crate::clipping::apify_client::{VideoDownloadResult, VideoInfo};

pub struct RustubeClient;

impl RustubeClient {
    /// Download a YouTube video using pure Rust (rustube)
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

        tracing::info!("📥 Downloading video from YouTube using Rustube: {}", video_url);

        // Extract video ID from URL
        let video_id = Self::extract_video_id(video_url)?;

        tracing::info!("🔍 Fetching video metadata for ID: {}", video_id);

        // Fetch video using rustube
        let id = Id::from_raw(&video_id)
            .map_err(|e| format!("Invalid video ID {}: {}", video_id, e))?;

        let video = Video::from_id(id.into_owned())
            .await
            .map_err(|e| format!("Failed to fetch video: {}", e))?;

        let video_details = video.video_details();
        let title = video_details.title.clone();
        let duration_secs = video_details.length_seconds as f64;

        tracing::info!("📹 Video: {} ({}s)", title, duration_secs);

        // Get all streams
        let streams = video.streams();

        // Try to get a stream with both video and audio (best quality)
        let stream = streams
            .iter()
            .filter(|s| s.includes_video_track && s.includes_audio_track)
            .max_by_key(|s| s.width.unwrap_or(0))
            .or_else(|| {
                // Fallback: get best video stream
                tracing::warn!("⚠️ No combined video+audio stream found, using best video-only stream");
                streams
                    .iter()
                    .filter(|s| s.includes_video_track)
                    .max_by_key(|s| s.width.unwrap_or(0))
            })
            .ok_or_else(|| "No suitable video stream found".to_string())?;

        let width = stream.width.map(|w| w as i32);
        let height = stream.height.map(|h| h as i32);

        tracing::info!(
            "⬇️ Downloading stream: {}x{}, codec: {:?}, mime: {}",
            width.unwrap_or(0),
            height.unwrap_or(0),
            stream.codecs,
            stream.mime
        );

        // Download the video stream
        use tokio::time::{timeout, Duration};

        tracing::info!("⏳ Starting download with 1-hour timeout");

        // Download to a temporary location first, then move to final location
        let temp_path = format!("{}.tmp", output_path);

        let download_future = async {
            // Download returns PathBuf to the downloaded file
            let downloaded_path = stream
                .download()
                .await
                .map_err(|e| format!("Failed to download video: {}", e))?;

            // Move the downloaded file to our desired location
            fs::rename(&downloaded_path, &temp_path)
                .await
                .map_err(|e| format!("Failed to move downloaded file: {}", e))?;

            Ok::<(), String>(())
        };

        timeout(Duration::from_secs(3600), download_future)
            .await
            .map_err(|_| "Download timed out after 1 hour".to_string())?
            .map_err(|e: String| e)?;

        // Move from temp to final location
        fs::rename(&temp_path, output_path)
            .await
            .map_err(|e| format!("Failed to save final file: {}", e))?;

        tracing::info!("💾 Saved video to: {}", output_path);

        // Validate downloaded file
        Self::validate_download(output_path).await?;

        tracing::info!("✅ Video downloaded successfully: {}", output_path);

        Ok(VideoDownloadResult {
            file_path: output_path.to_string(),
            title,
            duration_seconds: Some(duration_secs),
            width,
            height,
        })
    }

    /// Get video metadata without downloading
    pub async fn get_video_info(video_url: &str) -> Result<VideoInfo, String> {
        tracing::info!("ℹ️ Fetching video metadata: {}", video_url);

        let video_id = Self::extract_video_id(video_url)?;

        let id = Id::from_raw(&video_id)
            .map_err(|e| format!("Invalid video ID: {}", e))?;

        let video = Video::from_id(id.into_owned())
            .await
            .map_err(|e| format!("Failed to fetch video metadata: {}", e))?;

        let details = video.video_details();

        Ok(VideoInfo {
            video_id: details.video_id.to_string(),
            title: details.title.clone(),
            duration_seconds: Some(details.length_seconds as f64),
            channel_id: Some(details.channel_id.clone()),
            channel_name: Some(details.author.clone()),
            upload_date: None, // rustube doesn't provide this easily
        })
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
        if url.len() == 11 && url.chars().all(|c| c.is_alphanumeric() || c == '_' || c == '-') {
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
            RustubeClient::extract_video_id("https://youtube.com/watch?v=dQw4w9WgXcQ&t=10s").unwrap(),
            "dQw4w9WgXcQ"
        );

        assert_eq!(
            RustubeClient::extract_video_id("dQw4w9WgXcQ").unwrap(),
            "dQw4w9WgXcQ"
        );
    }
}
