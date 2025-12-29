// yt-dlp client wrapper using command-line tool
// Calls yt-dlp executable directly to avoid dependency conflicts

use std::path::Path;
use std::process::Stdio;
use tokio::process::Command;

/// Result of video download
#[derive(Debug)]
pub struct VideoDownloadResult {
    pub file_path: String,
    pub title: String,
    pub duration_seconds: Option<f64>,
    pub width: Option<i32>,
    pub height: Option<i32>,
}

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

        // Run yt-dlp command
        let output = Command::new("yt-dlp")
            .arg("--format")
            .arg("bestvideo[ext=mp4]+bestaudio[ext=m4a]/best[ext=mp4]/best")
            .arg("--output")
            .arg(output_path)
            .arg("--no-playlist")
            .arg("--print")
            .arg("after_move:filepath,title,duration,width,height")
            .arg(video_url)
            .stdout(Stdio::piped())
            .stderr(Stdio::piped())
            .output()
            .await
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
        let output = Command::new("yt-dlp")
            .arg("--version")
            .stdout(Stdio::null())
            .stderr(Stdio::null())
            .status()
            .await;

        match output {
            Ok(status) if status.success() => Ok(()),
            _ => Err(
                "yt-dlp is not installed. Install it with: pip install yt-dlp OR apt install yt-dlp"
                    .to_string(),
            ),
        }
    }
}

/// Video metadata from yt-dlp
#[derive(Debug)]
pub struct VideoInfo {
    pub video_id: String,
    pub title: String,
    pub duration_seconds: Option<f64>,
    pub channel_id: Option<String>,
    pub channel_name: Option<String>,
    pub upload_date: Option<String>,
}
