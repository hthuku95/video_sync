// yt-dlp client wrapper using command-line tool
// Fallback strategy #2 in the 5-tier system (CLI wrapper, battle-tested)

use std::path::Path;
use std::process::Stdio;
use tokio::io::AsyncRead;
use tokio::process::Command;

// Use shared types from apify_client to ensure compatibility
use crate::clipping::apify_client::VideoDownloadResult;
use crate::r2_client::R2Client;

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

    /// Download a YouTube video using yt-dlp command-line tool.
    /// When r2_client and r2_key are set, streams directly to R2 (zero local disk).
    pub async fn download_video(
        video_url: &str,
        output_path: &str,
        r2_client: Option<&R2Client>,
        r2_key: Option<&str>,
    ) -> Result<VideoDownloadResult, String> {
        Self::check_ytdlp_installed().await?;
        use tokio::time::{timeout, Duration};
        let ytdlp_binary = if Path::new("/usr/local/bin/yt-dlp").exists() {
            "/usr/local/bin/yt-dlp"
        } else if Path::new("/usr/bin/yt-dlp").exists() {
            "/usr/bin/yt-dlp"
        } else {
            "yt-dlp"
        };

        // R2 streaming path: yt-dlp -o - stdout pipe → R2
        if let (Some(client), Some(key)) = (r2_client, r2_key) {
            let r2_bucket = client.bucket.clone();

            // 1. Extract metadata first
            tracing::info!("📋 R2 stream: extracting metadata via yt-dlp JSON");
            let meta_cmd = Command::new(ytdlp_binary)
                .arg("--dump-single-json")
                .arg("--no-playlist")
                .arg("--no-check-certificates")
                .arg(video_url)
                .stdout(Stdio::piped())
                .stderr(Stdio::piped())
                .output()
                .await
                .map_err(|e| format!("yt-dlp metadata extraction failed: {e}"))?;

            if !meta_cmd.status.success() {
                let stderr = String::from_utf8_lossy(&meta_cmd.stderr);
                return Err(format!("yt-dlp metadata failed: {stderr}"));
            }

            let meta_json: serde_json::Value = serde_json::from_slice(&meta_cmd.stdout)
                .map_err(|e| format!("Failed to parse yt-dlp JSON output: {e}"))?;

            let title = meta_json["title"].as_str().unwrap_or("Unknown").to_string();
            let duration = meta_json["duration"].as_f64().unwrap_or(0.0);
            let width = meta_json["width"].as_i64().map(|w| w as i32);
            let height = meta_json["height"].as_i64().map(|h| h as i32);

            // 2. Stream binary via stdout pipe → R2
            tracing::info!("📥 R2 stream: yt-dlp -o - → R2 ({key})");
            let mut stream_cmd = Command::new(ytdlp_binary);
            stream_cmd
                .arg("--format")
                .arg("bestvideo[ext=mp4]+bestaudio[ext=m4a]/best[ext=mp4]/best")
                .arg("--merge-output-format").arg("mp4")
                .arg("-o").arg("-")
                .arg("--no-playlist")
                .arg("--user-agent")
                .arg("Mozilla/5.0 (Windows NT 10.0; Win64; x64) AppleWebKit/537.36")
                .arg("--extractor-args").arg(Self::extractor_args())
                .arg("--no-check-certificates")
                .arg(video_url)
                .stdout(Stdio::piped())
                .stderr(Stdio::piped());
            let temp_cookie_path = Self::apply_optional_auth_args(&mut stream_cmd).await?;
            let mut child = stream_cmd.spawn()
                .map_err(|e| format!("Failed to spawn yt-dlp process: {e}"))?;
            let mut stdout = child.stdout.take()
                .ok_or("No stdout from yt-dlp process")?;

            let result = timeout(
                Duration::from_secs(3600),
                client.upload_stream(key, &mut stdout),
            ).await;

            // Wait for process to finish
            let status = child.wait().await.ok();
            if let Some(path) = temp_cookie_path.as_deref() {
                let _ = tokio::fs::remove_file(path).await;
            }

            match result {
                Ok(Ok(())) => {
                    let r2_url = client.presign_get(key, 7 * 24 * 3600).await
                        .map_err(|e| format!("Failed to presign R2 URL: {e}"))?;
                    tracing::info!("✅ R2 stream complete: {r2_url}");
                    Ok(VideoDownloadResult {
                        file_path: output_path.to_string(),
                        title,
                        duration_seconds: Some(duration),
                        width,
                        height,
                        r2_url: Some(r2_url),
                    })
                }
                Ok(Err(e)) => Err(format!("R2 upload failed: {e}")),
                Err(_) => Err("yt-dlp R2 stream timed out after 1 hour".to_string()),
            }
        } else {
            // Legacy file-based path
            if let Some(parent) = Path::new(output_path).parent() {
                if let Err(e) = tokio::fs::create_dir_all(parent).await {
                    return Err(format!("Failed to create output directory: {}", e));
                }
            }

            tracing::info!("📥 Downloading video from YouTube: {}", video_url);
            tracing::info!("⏳ Starting download with 1-hour timeout");

            let mut command = Command::new(ytdlp_binary);
            command
                .arg("--format")
                .arg("bestvideo[ext=mp4]+bestaudio[ext=m4a]/best[ext=mp4]/best")
                .arg("--merge-output-format").arg("mp4")
                .arg("--output").arg(output_path)
                .arg("--no-playlist")
                .arg("--print").arg("after_move:filepath,title,duration,width,height")
                .arg("--user-agent")
                .arg("Mozilla/5.0 (Windows NT 10.0; Win64; x64) AppleWebKit/537.36 (KHTML, like Gecko) Chrome/120.0.0.0 Safari/537.36")
                .arg("--add-header").arg("Accept-Language:en-US,en;q=0.9")
                .arg("--extractor-args").arg(Self::extractor_args())
                .arg("--no-check-certificates")
                .arg(video_url)
                .stdout(Stdio::piped())
                .stderr(Stdio::piped());
            let temp_cookie_path = Self::apply_optional_auth_args(&mut command).await?;

            let output = timeout(Duration::from_secs(3600), command.output()).await
                .map_err(|_| "❌ Download timed out after 1 hour".to_string())?
                .map_err(|e| format!("Failed to execute yt-dlp: {e}"))?;
            if let Some(path) = temp_cookie_path.as_deref() {
                let _ = tokio::fs::remove_file(path).await;
            }

            if !output.status.success() {
                let stderr = String::from_utf8_lossy(&output.stderr);
                tracing::error!("yt-dlp error: {}", stderr);
                let lower = stderr.to_lowercase();
                let auth_hint = if lower.contains("private video") || lower.contains("sign in")
                    || lower.contains("members-only") || lower.contains("login")
                    || lower.contains("confirm you're not a bot") {
                    format!(" {}", Self::auth_hint())
                } else { String::new() };
                return Err(format!("yt-dlp download failed: {}{}", stderr, auth_hint));
            }

            let stdout = String::from_utf8_lossy(&output.stdout);
            let lines: Vec<&str> = stdout.lines().collect();

            tracing::info!("🔍 Validating downloaded video file");
            if !Path::new(output_path).exists() {
                return Err(format!("❌ Downloaded file does not exist: {}", output_path));
            }
            let metadata = tokio::fs::metadata(output_path).await
                .map_err(|e| format!("Failed to read file metadata: {e}"))?;
            if metadata.len() < 1_000_000 {
                return Err(format!("❌ File too small ({} bytes)", metadata.len()));
            }
            match crate::core::validate_video_file(output_path) {
                Ok(false) => { let _ = tokio::fs::remove_file(output_path).await; return Err("❌ Video corrupted".to_string()); }
                _ => {}
            }

            Ok(VideoDownloadResult {
                file_path: output_path.to_string(),
                title: lines.get(1).unwrap_or(&"Unknown Title").to_string(),
                duration_seconds: lines.get(2).and_then(|s| s.parse().ok()),
                width: lines.get(3).and_then(|s| s.parse().ok()),
                height: lines.get(4).and_then(|s| s.parse().ok()),
                r2_url: None,
            })
        }
    }

    /// Extract HLS stream URL from a video URL using yt-dlp -g.
    /// Returns the raw m3u8 URL for direct ffmpeg download.
    pub async fn extract_hls_url(video_url: &str) -> Result<String, String> {
        Self::check_ytdlp_installed().await?;
        let ytdlp_binary = if Path::new("/usr/local/bin/yt-dlp").exists() {
            "/usr/local/bin/yt-dlp"
        } else if Path::new("/usr/bin/yt-dlp").exists() {
            "/usr/bin/yt-dlp"
        } else {
            "yt-dlp"
        };

        let output = tokio::process::Command::new(ytdlp_binary)
            .arg("-g")
            .arg("--no-playlist")
            .arg(video_url)
            .stdout(Stdio::piped())
            .stderr(Stdio::piped())
            .output()
            .await
            .map_err(|e| format!("Failed to execute yt-dlp -g: {}", e))?;

        if !output.status.success() {
            let stderr = String::from_utf8_lossy(&output.stderr);
            return Err(format!("yt-dlp -g failed: {}", stderr));
        }

        let stdout = String::from_utf8_lossy(&output.stdout);
        let hls_url = stdout.lines().next().unwrap_or("").trim().to_string();
        if hls_url.is_empty() || !hls_url.contains(".m3u8") {
            return Err(format!("No HLS URL found in yt-dlp output: {}", stdout));
        }
        Ok(hls_url)
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
