// yt-dlp client wrapper using command-line tool
// Fallback strategy #2 in the 5-tier system (CLI wrapper, battle-tested)

use std::path::Path;
use std::process::Stdio;
use tokio::process::Command;

// Use shared types from apify_client to ensure compatibility
use crate::clipping::apify_client::{VideoDownloadResult, VideoInfo};

pub struct YtDlpClient;

impl YtDlpClient {
    /// Download a YouTube video using yt-dlp command-line tool
    pub async fn download_video(
        video_url: &str,
        output_path: &str,
    ) -> Result<VideoDownloadResult, String> {
        // Ensure parent directory exists
        if let Some(parent) = Path::new(output_path).parent() {
            if let Err(e) = tokio::fs::create_dir_all(parent).await {
                return Err(format!("Failed to create output directory: {}", e));
            }
        }

        tracing::info!("📥 Downloading video from YouTube: {}", video_url);

        // Check if yt-dlp is installed
        Self::check_ytdlp_installed().await?;

        // Run yt-dlp command with explicit merge format and 1-hour timeout
        tracing::info!("⏳ Starting download with 1-hour timeout");

        use tokio::time::{timeout, Duration};

        // Use absolute path to yt-dlp to avoid PATH issues in subprocess
        // Try multiple locations in order of preference
        // IMPORTANT: Check /usr/local/bin FIRST as that's where Dockerfile installs it
        let ytdlp_binary = if Path::new("/usr/local/bin/yt-dlp").exists() {
            "/usr/local/bin/yt-dlp"
        } else if Path::new("/usr/bin/yt-dlp").exists() {
            "/usr/bin/yt-dlp"
        } else {
            "yt-dlp" // Fallback to PATH lookup
        };

        tracing::debug!("Using yt-dlp binary at: {}", ytdlp_binary);

        let output = timeout(
            Duration::from_secs(3600),  // 1 hour timeout for large videos
            Command::new(ytdlp_binary)
                .arg("--format")
                .arg("bestvideo[ext=mp4]+bestaudio[ext=m4a]/best[ext=mp4]/best")
                .arg("--merge-output-format")
                .arg("mp4")
                .arg("--output")
                .arg(output_path)
                .arg("--no-playlist")
                .arg("--print")
                .arg("after_move:filepath,title,duration,width,height")
                // Anti-bot detection measures
                .arg("--user-agent")
                .arg("Mozilla/5.0 (Windows NT 10.0; Win64; x64) AppleWebKit/537.36 (KHTML, like Gecko) Chrome/120.0.0.0 Safari/537.36")
                .arg("--add-header")
                .arg("Accept-Language:en-US,en;q=0.9")
                .arg("--extractor-args")
                .arg("youtube:player_client=android,web")
                .arg("--no-check-certificates")
                .arg(video_url)
                .stdout(Stdio::piped())
                .stderr(Stdio::piped())
                .output()
        )
        .await
        .map_err(|_| "❌ Download timed out after 1 hour".to_string())?
        .map_err(|e| format!("Failed to execute yt-dlp: {}. Make sure yt-dlp is installed.", e))?;

        if !output.status.success() {
            let stderr = String::from_utf8_lossy(&output.stderr);
            tracing::error!("yt-dlp error: {}", stderr);
            return Err(format!("yt-dlp download failed: {}", stderr));
        }

        let stdout = String::from_utf8_lossy(&output.stdout);
        tracing::debug!("yt-dlp output: {}", stdout);

        // Parse output (yt-dlp prints filepath, title, duration, width, height)
        let lines: Vec<&str> = stdout.lines().collect();

        // Validate the downloaded file
        tracing::info!("🔍 Validating downloaded video file");

        // Check 1: File exists
        if !Path::new(output_path).exists() {
            return Err(format!("❌ Downloaded file does not exist: {}", output_path));
        }

        // Check 2: File size is reasonable (> 1MB for videos)
        let metadata = tokio::fs::metadata(output_path)
            .await
            .map_err(|e| format!("Failed to read file metadata: {}", e))?;

        if metadata.len() < 1_000_000 {
            return Err(format!(
                "❌ Downloaded file is suspiciously small ({} bytes): {}",
                metadata.len(),
                output_path
            ));
        }

        tracing::info!("✅ File exists and has reasonable size ({:.2} MB)",
            metadata.len() as f64 / 1_000_000.0);

        // Check 3: Validate with ffprobe (use existing validate_video_file)
        match crate::core::validate_video_file(output_path) {
            Ok(true) => {
                tracing::info!("✅ Downloaded video validated successfully");
            }
            Ok(false) => {
                // Clean up corrupted download
                let _ = tokio::fs::remove_file(output_path).await;
                return Err(format!("❌ Downloaded video is corrupted or unreadable: {}", output_path));
            }
            Err(e) => {
                tracing::warn!("⚠️ Video validation check failed: {}", e);
                // Continue anyway - validation might fail for other reasons
            }
        }

        // Check 4: Verify duration matches expected
        match crate::core::analyze_video(output_path) {
            Ok(video_meta) => {
                if video_meta.duration_seconds < 1.0 {
                    // Clean up corrupted download
                    let _ = tokio::fs::remove_file(output_path).await;
                    return Err(format!(
                        "❌ Downloaded video is too short ({:.1}s), likely corrupted",
                        video_meta.duration_seconds
                    ));
                }
                tracing::info!(
                    "✅ Video validated: {}x{} @ {:.1}fps, duration: {:.1}s, format: {}",
                    video_meta.width,
                    video_meta.height,
                    video_meta.fps,
                    video_meta.duration_seconds,
                    video_meta.format
                );
            }
            Err(e) => {
                tracing::warn!("⚠️ Unable to analyze video metadata: {}", e);
            }
        }

        Ok(VideoDownloadResult {
            file_path: output_path.to_string(),
            title: lines.get(1).unwrap_or(&"Unknown Title").to_string(),
            duration_seconds: lines.get(2).and_then(|s| s.parse().ok()),
            width: lines.get(3).and_then(|s| s.parse().ok()),
            height: lines.get(4).and_then(|s| s.parse().ok()),
        })
    }

    /// Get video metadata without downloading
    pub async fn get_video_info(video_url: &str) -> Result<VideoInfo, String> {
        tracing::info!("ℹ️ Fetching video metadata: {}", video_url);

        Self::check_ytdlp_installed().await?;

        // Run yt-dlp with --print-json to get metadata
        let output = Command::new("yt-dlp")
            .arg("--print-json")
            .arg("--skip-download")
            .arg(video_url)
            .stdout(Stdio::piped())
            .stderr(Stdio::piped())
            .output()
            .await
            .map_err(|e| format!("Failed to execute yt-dlp: {}", e))?;

        if !output.status.success() {
            let stderr = String::from_utf8_lossy(&output.stderr);
            return Err(format!("yt-dlp info extraction failed: {}", stderr));
        }

        let stdout = String::from_utf8_lossy(&output.stdout);
        let json: serde_json::Value = serde_json::from_str(&stdout)
            .map_err(|e| format!("Failed to parse yt-dlp JSON output: {}", e))?;

        Ok(VideoInfo {
            video_id: json["id"].as_str().unwrap_or("").to_string(),
            title: json["title"].as_str().unwrap_or("Unknown").to_string(),
            duration_seconds: json["duration"].as_f64(),
            channel_id: json["channel_id"].as_str().map(|s| s.to_string()),
            channel_name: json["channel"].as_str().map(|s| s.to_string()),
            upload_date: json["upload_date"].as_str().map(|s| s.to_string()),
        })
    }

    /// Check if yt-dlp is installed
    async fn check_ytdlp_installed() -> Result<(), String> {
        use std::path::Path;

        // Check multiple possible locations for yt-dlp binary
        // IMPORTANT: Check /usr/local/bin FIRST as that's where Dockerfile installs it
        let possible_paths = [
            "/usr/local/bin/yt-dlp",     // Dockerfile install location (PRIMARY)
            "/usr/bin/yt-dlp",           // Alternative location / symlink
        ];

        // First, try to find yt-dlp at known locations
        for path in &possible_paths {
            if Path::new(path).exists() {
                tracing::debug!("✅ Found yt-dlp at: {}", path);
                // Verify it's executable
                let output = Command::new(path)
                    .arg("--version")
                    .stdout(Stdio::null())
                    .stderr(Stdio::null())
                    .status()
                    .await;

                if let Ok(status) = output {
                    if status.success() {
                        return Ok(());
                    }
                }
            }
        }

        // Fallback: Try PATH lookup
        let output = Command::new("yt-dlp")
            .arg("--version")
            .stdout(Stdio::null())
            .stderr(Stdio::null())
            .status()
            .await;

        match output {
            Ok(status) if status.success() => {
                tracing::debug!("✅ Found yt-dlp via PATH");
                Ok(())
            }
            _ => {
                tracing::error!("❌ yt-dlp not found in any of: {:?} or PATH", possible_paths);
                Err(
                    "yt-dlp is not installed. Install it with: pip install yt-dlp OR apt install yt-dlp"
                        .to_string()
                )
            },
        }
    }
}
