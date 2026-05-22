// Pure Rust YouTube video downloader using rusty_ytdl
// Eliminates Python/yt-dlp dependency entirely - production-ready alternative to rustube

use rusty_ytdl::{Video, VideoOptions, VideoQuality, VideoSearchOptions};
use std::path::Path;
use tokio::fs;
use tokio::io::AsyncWriteExt;

// Use the same types as apify_client to ensure compatibility
use crate::clipping::apify_client::{VideoDownloadResult, VideoInfo};

pub struct RustyYtdlClient;

impl RustyYtdlClient {
    /// Download a YouTube video using pure Rust (rusty_ytdl)
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

        tracing::info!(
            "📥 Downloading video from YouTube using rusty_ytdl: {}",
            video_url
        );

        // Configure video options for best quality MP4
        let video_options = VideoOptions {
            quality: VideoQuality::Highest,
            filter: VideoSearchOptions::VideoAudio, // Get streams with both video and audio
            ..Default::default()
        };

        // Create video instance
        let video = Video::new_with_options(video_url, video_options)
            .map_err(|e| format!("Failed to create video instance: {}", e))?;

        // Get video info
        let video_info = video
            .get_info()
            .await
            .map_err(|e| format!("Failed to fetch video info: {}", e))?;

        let title = video_info.video_details.title.clone();
        let duration_secs = video_info.video_details.length_seconds.parse::<f64>().ok();
        let video_id = video_info.video_details.video_id.clone();

        tracing::info!("📹 Video: {} (ID: {})", title, video_id);
        if let Some(duration) = duration_secs {
            tracing::info!("⏱️ Duration: {:.1}s", duration);
        }

        // Download video stream with timeout
        use tokio::time::{timeout, Duration};

        tracing::info!("⬇️ Starting download with 1-hour timeout");

        let temp_path = format!("{}.tmp", output_path);

        let download_future = async {
            // Open stream
            let stream = video
                .stream()
                .await
                .map_err(|e| format!("Failed to open video stream: {}", e))?;

            // Create output file
            let mut file = fs::File::create(&temp_path)
                .await
                .map_err(|e| format!("Failed to create output file: {}", e))?;

            let mut total_bytes = 0u64;
            let mut chunk_count = 0u32;

            // Download chunks and write to file
            while let Some(chunk) = stream
                .chunk()
                .await
                .map_err(|e| format!("Failed to read chunk: {}", e))?
            {
                file.write_all(&chunk)
                    .await
                    .map_err(|e| format!("Failed to write chunk: {}", e))?;

                total_bytes += chunk.len() as u64;
                chunk_count += 1;

                // Log progress every 100 chunks (~10MB at typical chunk size)
                if chunk_count % 100 == 0 {
                    tracing::info!(
                        "📦 Downloaded {:.2} MB ({} chunks)",
                        total_bytes as f64 / 1_000_000.0,
                        chunk_count
                    );
                }
            }

            file.flush()
                .await
                .map_err(|e| format!("Failed to flush file: {}", e))?;

            tracing::info!(
                "💾 Download complete: {:.2} MB ({} chunks)",
                total_bytes as f64 / 1_000_000.0,
                chunk_count
            );

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

        tracing::info!("✅ Saved video to: {}", output_path);

        // Validate downloaded file
        Self::validate_download(output_path).await?;

        tracing::info!("✅ Video downloaded successfully: {}", output_path);

        Ok(VideoDownloadResult {
            file_path: output_path.to_string(),
            title,
            duration_seconds: duration_secs,
            width: None, // rusty_ytdl doesn't expose dimensions easily
            height: None,
        })
    }

    /// Get video metadata without downloading
    pub async fn get_video_info(video_url: &str) -> Result<VideoInfo, String> {
        tracing::info!("ℹ️ Fetching video metadata: {}", video_url);

        let video =
            Video::new(video_url).map_err(|e| format!("Failed to create video instance: {}", e))?;

        let info = video
            .get_info()
            .await
            .map_err(|e| format!("Failed to fetch video metadata: {}", e))?;

        let details = &info.video_details;

        Ok(VideoInfo {
            video_id: details.video_id.clone(),
            title: details.title.clone(),
            duration_seconds: details.length_seconds.parse::<f64>().ok(),
            channel_id: Some(details.channel_id.clone()),
            channel_name: details.author.as_ref().map(|a| a.name.clone()),
            upload_date: Some(details.publish_date.clone()),
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

#[cfg(test)]
mod tests {
    use super::*;

    #[tokio::test]
    async fn test_get_video_info() {
        // Test with a known stable video (change to a real test video ID)
        let result =
            RustyYtdlClient::get_video_info("https://www.youtube.com/watch?v=dQw4w9WgXcQ").await;
        assert!(result.is_ok(), "Should fetch video info successfully");
    }
}
