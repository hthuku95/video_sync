// yt-dlp client wrapper using command-line tool
// Fallback strategy #2 in the 5-tier system (CLI wrapper, battle-tested)

use std::path::Path;
use std::process::Stdio;
use tokio::process::Command;

// Use shared types from apify_client to ensure compatibility
use crate::clipping::apify_client::VideoDownloadResult;

pub struct YtDlpClient;

impl YtDlpClient {
    fn extractor_args() -> String {
        std::env::var("YTDLP_EXTRACTOR_ARGS")
            .ok()
            .filter(|value| !value.trim().is_empty())
            .unwrap_or_else(|| "youtube:player_client=android,web".to_string())
    }

    fn auth_hint() -> &'static str {
        "If this video requires authentication, configure YTDLP_COOKIES_FROM_BROWSER or YTDLP_COOKIES_FILE."
    }

    async fn apply_optional_auth_args(command: &mut Command) -> Result<Option<String>, String> {
        if let Ok(cookie_file) = std::env::var("YTDLP_COOKIES_FILE") {
            let cookie_file = cookie_file.trim();
            if !cookie_file.is_empty() {
                tracing::info!("🍪 Using YTDLP_COOKIES_FILE for authenticated yt-dlp access");
                command.arg("--cookies").arg(cookie_file);
                return Ok(None);
            }
        }

        if let Ok(browser) = std::env::var("YTDLP_COOKIES_FROM_BROWSER") {
            let browser = browser.trim();
            if !browser.is_empty() {
                tracing::info!(
                    "🍪 Using YTDLP_COOKIES_FROM_BROWSER for authenticated yt-dlp access"
                );
                command.arg("--cookies-from-browser").arg(browser);
                return Ok(None);
            }
        }

        if let Ok(cookies_b64) = std::env::var("YTDLP_COOKIES_B64") {
            let cookies_b64 = cookies_b64.trim();
            if !cookies_b64.is_empty() {
                tracing::info!("🍪 Decoding YTDLP_COOKIES_B64 for authenticated yt-dlp access");
                use base64::prelude::*;
                let decoded = BASE64_STANDARD
                    .decode(cookies_b64)
                    .map_err(|e| format!("Failed to decode YTDLP_COOKIES_B64: {}", e))?;
                let temp_path = std::env::temp_dir().join(format!(
                    "videosync-ytdlp-cookies-{}-{}.txt",
                    std::process::id(),
                    std::time::SystemTime::now()
                        .duration_since(std::time::UNIX_EPOCH)
                        .map(|d| d.as_millis())
                        .unwrap_or(0)
                ));
                tokio::fs::write(&temp_path, decoded)
                    .await
                    .map_err(|e| format!("Failed to materialize yt-dlp cookies file: {}", e))?;
                let temp_path_string = temp_path.to_string_lossy().to_string();
                command.arg("--cookies").arg(&temp_path_string);
                return Ok(Some(temp_path_string));
            }
        }

        Ok(None)
    }

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

        let mut command = Command::new(ytdlp_binary);
        command
            .arg("--format")
            .arg("bestvideo[ext=mp4]+bestaudio[ext=m4a]/best[ext=mp4]/best")
            .arg("--merge-output-format")
            .arg("mp4")
            .arg("--output")
            .arg(output_path)
            .arg("--no-playlist")
            .arg("--print")
            .arg("after_move:filepath,title,duration,width,height")
            .arg("--user-agent")
            .arg("Mozilla/5.0 (Windows NT 10.0; Win64; x64) AppleWebKit/537.36 (KHTML, like Gecko) Chrome/120.0.0.0 Safari/537.36")
            .arg("--add-header")
            .arg("Accept-Language:en-US,en;q=0.9")
            .arg("--extractor-args")
            .arg(Self::extractor_args())
            .arg("--no-check-certificates")
            .arg(video_url)
            .stdout(Stdio::piped())
            .stderr(Stdio::piped());
        let temp_cookie_path = Self::apply_optional_auth_args(&mut command).await?;

        let output = timeout(
            Duration::from_secs(3600),  // 1 hour timeout for large videos
            command.output()
        )
        .await
        .map_err(|_| "❌ Download timed out after 1 hour".to_string())?
        .map_err(|e| format!("Failed to execute yt-dlp: {}. Make sure yt-dlp is installed.", e))?;
        if let Some(path) = temp_cookie_path.as_deref() {
            let _ = tokio::fs::remove_file(path).await;
        }

        if !output.status.success() {
            let stderr = String::from_utf8_lossy(&output.stderr);
            tracing::error!("yt-dlp error: {}", stderr);
            let lower = stderr.to_lowercase();
            let auth_hint = if lower.contains("private video")
                || lower.contains("sign in")
                || lower.contains("members-only")
                || lower.contains("login")
                || lower.contains("confirm you're not a bot")
            {
                format!(" {}", Self::auth_hint())
            } else {
                String::new()
            };
            return Err(format!("yt-dlp download failed: {}{}", stderr, auth_hint));
        }

        let stdout = String::from_utf8_lossy(&output.stdout);
        tracing::debug!("yt-dlp output: {}", stdout);

        // Parse output (yt-dlp prints filepath, title, duration, width, height)
        let lines: Vec<&str> = stdout.lines().collect();

        // Validate the downloaded file
        tracing::info!("🔍 Validating downloaded video file");

        // Check 1: File exists
        if !Path::new(output_path).exists() {
            return Err(format!(
                "❌ Downloaded file does not exist: {}",
                output_path
            ));
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

        tracing::info!(
            "✅ File exists and has reasonable size ({:.2} MB)",
            metadata.len() as f64 / 1_000_000.0
        );

        // Check 3: Validate with ffprobe (use existing validate_video_file)
        match crate::core::validate_video_file(output_path) {
            Ok(true) => {
                tracing::info!("✅ Downloaded video validated successfully");
            }
            Ok(false) => {
                // Clean up corrupted download
                let _ = tokio::fs::remove_file(output_path).await;
                return Err(format!(
                    "❌ Downloaded video is corrupted or unreadable: {}",
                    output_path
                ));
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

    /// Check if yt-dlp is installed
    async fn check_ytdlp_installed() -> Result<(), String> {
        use std::path::Path;

        // Check multiple possible locations for yt-dlp binary
        // IMPORTANT: Check /usr/local/bin FIRST as that's where Dockerfile installs it
        let possible_paths = [
            "/usr/local/bin/yt-dlp", // Dockerfile install location (PRIMARY)
            "/usr/bin/yt-dlp",       // Alternative location / symlink
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
                tracing::error!(
                    "❌ yt-dlp not found in any of: {:?} or PATH",
                    possible_paths
                );
                Err(
                    "yt-dlp is not installed. Install it with: pip install yt-dlp OR apt install yt-dlp"
                        .to_string()
                )
            }
        }
    }
}
