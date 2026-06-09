#![allow(dead_code, unused_imports)]
// utils.rs - Pure FFmpeg utility functions (ZERO GStreamer!)
use std::process::Command;
use std::time::Duration;
use tokio::process::Command as TokioCommand;

/// Format duration in HH:MM:SS.mmm format
pub fn format_duration(seconds: f64) -> String {
    let hours = (seconds / 3600.0) as u32;
    let minutes = ((seconds % 3600.0) / 60.0) as u32;
    let secs = (seconds % 60.0) as u32;
    let millis = ((seconds % 1.0) * 1000.0) as u32;
    format!("{:02}:{:02}:{:02}.{:03}", hours, minutes, secs, millis)
}

/// Extract `-i <input_url>` from args and separate output path from ffmpeg flags.
fn parse_ffmpeg_args(args: &[String]) -> (bool, Option<String>, Vec<String>, Option<String>) {
    let i_pos = match args.iter().position(|a| a == "-i") {
        Some(p) => p,
        None => return (false, None, vec![], None),
    };
    let input_url = match args.get(i_pos + 1) {
        Some(url) => url,
        None => return (false, None, vec![], None),
    };
    let is_cloud = is_cloud_url(input_url);
    let rest: Vec<String> = args.iter().skip(i_pos + 2).cloned().collect();
    let output_pos = rest.iter().rposition(|a| !a.starts_with('-'));
    let output_path = output_pos.map(|p| rest[p].clone());
    let end = output_pos.unwrap_or(rest.len());
    let ffmpeg_args: Vec<String> = rest[..end].to_vec();
    (is_cloud, Some(input_url.clone()), ffmpeg_args, output_path)
}

fn write_marker_file(path: &str, presigned_url: &str) -> Result<(), String> {
    std::fs::write(path, format!("FFMPEG_MCP_OUTPUT_URL={presigned_url}\n"))
        .map_err(|e| format!("Failed to write MCP marker file: {e}"))
}

pub fn read_marker_file(path: &str) -> Option<String> {
    let content = std::fs::read_to_string(path).ok()?;
    if let Some(line) = content.lines().next() {
        if let Some(url) = line.strip_prefix("FFMPEG_MCP_OUTPUT_URL=") {
            return Some(url.to_string());
        }
    }
    None
}

/// Execute FFmpeg with MCP routing when input is a cloud URL.
/// Falls back to local FFmpeg otherwise.
fn execute_ffmpeg_maybe_via_mcp(
    command: &mut Command,
    generate_output_key: impl Fn(&str) -> String,
) -> Result<String, String> {
    if std::env::var("FFMPEG_MCP_URL").is_err() {
        return run_local_ffmpeg(command);
    }

    let program = command.get_program().to_string_lossy().to_string();
    let args: Vec<String> = command.get_args().map(|a| a.to_string_lossy().to_string()).collect();

    let (is_cloud, input_url, ffmpeg_args, output_path) = parse_ffmpeg_args(&args);
    if !is_cloud || input_url.is_none() || output_path.is_none() {
        return run_local_ffmpeg(command);
    }

    let input_url = input_url.unwrap();
    let output_path = output_path.unwrap();
    let output_key = generate_output_key(&output_path);

    let mcp_future = async {
        crate::ffmpeg_mcp_client::FfmpegMcpClient::from_env()
            .ok_or("FFmpeg MCP not configured".to_string())?
            .process(&input_url, &ffmpeg_args, &output_key)
            .await
    };

    let output_url = match tokio::runtime::Handle::try_current() {
        Ok(handle) => tokio::task::block_in_place(|| handle.block_on(mcp_future)),
        Err(_) => {
            let rt = tokio::runtime::Builder::new_current_thread()
                .enable_all()
                .build()
                .map_err(|e| format!("Failed to build runtime: {e}"))?;
            rt.block_on(mcp_future)
        }
    }.map_err(|e| format!("FFmpeg MCP failed: {e}"))?;

    write_marker_file(&output_path, &output_url)?;
    tracing::info!("✅ FFmpeg MCP: {output_path} → {output_url}");
    Ok(String::new())
}

fn run_local_ffmpeg(command: &mut Command) -> Result<String, String> {
    let output = command
        .output()
        .map_err(|e| format!("Failed to execute FFmpeg: {}", e))?;

    if !output.status.success() {
        let stderr = String::from_utf8_lossy(&output.stderr);
        return Err(format!("FFmpeg error: {}", stderr));
    }

    Ok(String::from_utf8_lossy(&output.stdout).to_string())
}

/// Execute FFmpeg command with error handling and progress info
/// Routes through MCP when input is a cloud URL.
pub fn execute_ffmpeg_command(mut command: Command) -> Result<String, String> {
    println!("Executing FFmpeg: {:?}", command);
    execute_ffmpeg_maybe_via_mcp(&mut command, |path| {
        let name = path.split('/').last().unwrap_or("output");
        format!("processed/{}", name)
    })
}

/// Execute FFmpeg command synchronously with timeout.
/// Routes through MCP when input is a cloud URL.
/// Default timeout is 5 minutes (300 seconds)
pub fn execute_ffmpeg_command_with_sync_timeout(
    command: Command,
    timeout_secs: Option<u64>,
) -> Result<String, String> {
    // Try MCP first if input is a cloud URL
    let mut cmd = command;
    let args: Vec<String> = cmd.get_args().map(|a| a.to_string_lossy().to_string()).collect();
    let (is_cloud, input_url, ffmpeg_args, output_path) = parse_ffmpeg_args(&args);

    if is_cloud && input_url.is_some() && output_path.is_some() && std::env::var("FFMPEG_MCP_URL").is_ok() {
        let input_url = input_url.unwrap();
        let output_path = output_path.unwrap();
        let output_key = format!("processed/{}", output_path.split('/').last().unwrap_or("output"));

        let mcp_future = async {
            crate::ffmpeg_mcp_client::FfmpegMcpClient::from_env()
                .ok_or("MCP not configured")?
                .process(&input_url, &ffmpeg_args, &output_key)
                .await
        };

        let result = match tokio::runtime::Handle::try_current() {
            Ok(handle) => tokio::task::block_in_place(|| handle.block_on(mcp_future)),
            Err(_) => {
                let rt = tokio::runtime::Builder::new_current_thread()
                    .enable_all()
                    .build().map_err(|e| format!("Runtime: {e}"))?;
                rt.block_on(mcp_future)
            }
        };

        match result {
            Ok(output_url) => {
                write_marker_file(&output_path, &output_url)?;
                return Ok(String::new());
            }
            Err(mcp_err) => {
                tracing::warn!("FFmpeg MCP failed, falling back to local: {mcp_err}");
                // Fall through to local processing
            }
        }
    }

    // Local FFmpeg path
    let timeout = std::time::Duration::from_secs(timeout_secs.unwrap_or(300));
    let program = cmd.get_program().to_string_lossy().to_string();

    let mut child = std::process::Command::new(&program)
        .args(&args)
        .stdout(std::process::Stdio::null())
        .stderr(std::process::Stdio::piped())
        .spawn()
        .map_err(|e| format!("Failed to spawn FFmpeg ({}): {}", program, e))?;

    let stderr_reader = child.stderr.take().map(|mut stderr| {
        std::thread::spawn(move || {
            let mut stderr_buf = Vec::new();
            use std::io::Read;
            let _ = stderr.read_to_end(&mut stderr_buf);
            stderr_buf
        })
    });

    let deadline = std::time::Instant::now() + timeout;
    loop {
        match child.try_wait() {
            Ok(Some(status)) => {
                let stderr_buf = stderr_reader
                    .map(|reader| reader.join().unwrap_or_default())
                    .unwrap_or_default();
                if status.success() {
                    return Ok(String::new());
                }
                let stderr = String::from_utf8_lossy(&stderr_buf);
                return Err(format!(
                    "FFmpeg error (exit {}): {}",
                    status.code().unwrap_or(-1),
                    &stderr[..stderr.len().min(600)]
                ));
            }
            Ok(None) => {
                if std::time::Instant::now() >= deadline {
                    let _ = child.kill();
                    let _ = child.wait();
                    if let Some(reader) = stderr_reader {
                        let _ = reader.join();
                    }
                    return Err(format!(
                        "FFmpeg operation timed out after {} seconds.",
                        timeout.as_secs()
                    ));
                }
                std::thread::sleep(std::time::Duration::from_millis(200));
            }
            Err(e) => {
                let _ = child.kill();
                if let Some(reader) = stderr_reader {
                    let _ = reader.join();
                }
                return Err(format!("FFmpeg wait error: {}", e));
            }
        }
    }
}

// ─── Cloud-aware FFmpeg via MCP microservice ──────────────────────────────

/// Returns true if the path looks like an HTTP/HTTPS URL.
pub fn is_cloud_url(path: &str) -> bool {
    path.starts_with("http://") || path.starts_with("https://")
}

/// Extract args from a TokioCommand using its Debug output.
fn extract_tokio_args(command: &TokioCommand) -> Vec<String> {
    let dbg = format!("{:?}", command);
    // Debug format: `"ffmpeg" "-y" "-i" "input" ...` (space-separated quoted strings)
    let mut args = Vec::new();
    let mut chars = dbg.chars().peekable();
    while let Some(&c) = chars.peek() {
        if c == '"' {
            chars.next();
            let mut s = String::new();
            while let Some(&c2) = chars.peek() {
                if c2 == '"' {
                    chars.next();
                    break;
                }
                s.push(c2);
                chars.next();
            }
            args.push(s);
        } else if c == ' ' {
            chars.next();
        } else {
            chars.next();
        }
    }
    args
}

/// Process media via the FFmpeg MCP microservice.
/// Input is an R2 presigned URL. Returns the output presigned URL.
/// Falls back to local FFmpeg if MCP is not configured.
pub async fn process_via_mcp(
    input_url: &str,
    ffmpeg_args: &[String],
    output_key: &str,
) -> Result<String, String> {
    let client = crate::ffmpeg_mcp_client::FfmpegMcpClient::from_env()
        .ok_or("FFmpeg MCP not configured, install by setting FFMPEG_MCP_URL".to_string())?;
    client.process(input_url, ffmpeg_args, output_key).await
}

/// Execute FFmpeg command asynchronously with timeout.
/// Routes through MCP when input is a cloud URL.
/// Timeout is 5 minutes (300 seconds) by default
pub async fn execute_ffmpeg_command_with_timeout(
    mut command: TokioCommand,
    timeout_secs: Option<u64>,
) -> Result<String, String> {
    // Try MCP first if input is a cloud URL
    // Extract args from the TokioCommand Debug representation
    let args: Vec<String> = extract_tokio_args(&command);
    let (is_cloud, input_url, ffmpeg_args, output_path) = parse_ffmpeg_args(&args);

    if is_cloud && input_url.is_some() && output_path.is_some() && std::env::var("FFMPEG_MCP_URL").is_ok() {
        let input_url = input_url.unwrap();
        let output_path = output_path.unwrap();
        let output_key = format!("processed/{}", output_path.split('/').last().unwrap_or("output"));

        match crate::ffmpeg_mcp_client::FfmpegMcpClient::from_env()
            .ok_or("MCP not configured")?
            .process(&input_url, &ffmpeg_args, &output_key)
            .await
        {
            Ok(output_url) => {
                write_marker_file(&output_path, &output_url)?;
                return Ok(String::new());
            }
            Err(mcp_err) => {
                tracing::warn!("FFmpeg MCP failed, falling back to local: {mcp_err}");
            }
        }
    }

    // Local FFmpeg path
    let timeout_duration = Duration::from_secs(timeout_secs.unwrap_or(300));
    tracing::debug!(
        "Executing FFmpeg with {}s timeout: {:?}",
        timeout_duration.as_secs(),
        command
    );

    let child = command
        .kill_on_drop(true)
        .spawn()
        .map_err(|e| format!("Failed to spawn FFmpeg process: {}", e))?;

    let output = match tokio::time::timeout(timeout_duration, child.wait_with_output()).await {
        Ok(Ok(output)) => output,
        Ok(Err(e)) => return Err(format!("FFmpeg process failed: {}", e)),
        Err(_) => {
            return Err(format!(
                "FFmpeg operation timed out after {} seconds. Process terminated.",
                timeout_duration.as_secs()
            ));
        }
    };

    if !output.status.success() {
        let stderr = String::from_utf8_lossy(&output.stderr);
        return Err(format!("FFmpeg error: {}", stderr));
    }

    Ok(String::from_utf8_lossy(&output.stdout).to_string())
}

/// Execute FFprobe for media analysis
pub fn execute_ffprobe_command(args: &[&str]) -> Result<String, String> {
    let output = Command::new("ffprobe")
        .args(args)
        .output()
        .map_err(|e| format!("Failed to execute FFprobe: {}", e))?;

    if !output.status.success() {
        let stderr = String::from_utf8_lossy(&output.stderr);
        return Err(format!("FFprobe error: {}", stderr));
    }

    Ok(String::from_utf8_lossy(&output.stdout).to_string())
}

/// Check if FFmpeg and FFprobe are available
pub fn check_ffmpeg_available() -> Result<(), String> {
    Command::new("ffmpeg")
        .args(&["-version"])
        .output()
        .map_err(|_| "FFmpeg not found. Please install FFmpeg.")?;

    Command::new("ffprobe")
        .args(&["-version"])
        .output()
        .map_err(|_| "FFprobe not found. Please install FFmpeg with FFprobe.")?;

    println!("✓ FFmpeg and FFprobe are available");
    Ok(())
}

/// Build FFmpeg video filter string
pub fn build_video_filter(filters: &[(&str, f64)]) -> String {
    if filters.is_empty() {
        return String::new();
    }

    let filter_strings: Vec<String> = filters.iter()
        .map(|(filter, intensity)| {
            match *filter {
                "brightness" => format!("eq=brightness={:.2}", intensity),
                "contrast" => format!("eq=contrast={:.2}", 1.0 + intensity),
                "saturation" => format!("eq=saturation={:.2}", 1.0 + intensity),
                "blur" => format!("boxblur={:.1}", intensity * 10.0),
                "sepia" => "colorchannelmixer=.393:.769:.189:0:.349:.686:.168:0:.272:.534:.131:0".to_string(),
                "grayscale" => "format=gray".to_string(),
                "edge" => "edgedetect".to_string(),
                "emboss" => "convolution='0 -1 0 -1 5 -1 0 -1 0:0 -1 0 -1 5 -1 0 -1 0:0 -1 0 -1 5 -1 0 -1 0:0 -1 0 -1 5 -1 0 -1 0'".to_string(),
                "negative" => "negate".to_string(),
                _ => filter.to_string(),
            }
        })
        .collect();

    filter_strings.join(",")
}

/// Build FFmpeg audio filter string
pub fn build_audio_filter(filters: &[(&str, f64)]) -> String {
    if filters.is_empty() {
        return String::new();
    }

    let filter_strings: Vec<String> = filters
        .iter()
        .map(|(filter, intensity)| match *filter {
            "volume" => format!("volume={:.2}", intensity),
            "echo" => format!("aecho=0.8:0.9:{}:0.3", (intensity * 1000.0) as u32),
            "reverb" => "aecho=0.8:0.88:60:0.4".to_string(),
            "chorus" => "chorus=0.5:0.9:50:0.4:0.25:2".to_string(),
            "lowpass" => format!("lowpass=f={:.0}", 1000.0 + intensity * 10000.0),
            "highpass" => format!("highpass=f={:.0}", intensity * 1000.0),
            _ => filter.to_string(),
        })
        .collect();

    filter_strings.join(",")
}

/// Generate FFmpeg quality settings based on quality level
pub fn get_quality_settings(quality: &str) -> Vec<String> {
    match quality.to_lowercase().as_str() {
        "low" => vec!["-crf".to_string(), "28".to_string()],
        "medium" => vec!["-crf".to_string(), "23".to_string()],
        "high" => vec!["-crf".to_string(), "18".to_string()],
        "ultra" => vec!["-crf".to_string(), "15".to_string()],
        _ => vec!["-crf".to_string(), "23".to_string()], // Default to medium
    }
}

/// Generate platform-specific encoding settings for FFmpeg
pub fn get_platform_settings(platform: &str) -> Vec<String> {
    match platform.to_lowercase().as_str() {
        "youtube" => vec![
            "-c:v".to_string(), "libx264".to_string(),
            "-preset".to_string(), "slow".to_string(),
            "-crf".to_string(), "18".to_string(),
            "-c:a".to_string(), "aac".to_string(),
            "-b:a".to_string(), "192k".to_string(),
            "-pix_fmt".to_string(), "yuv420p".to_string(),
            "-maxrate".to_string(), "8M".to_string(),
            "-bufsize".to_string(), "16M".to_string(),
        ],
        "instagram" => vec![
            "-c:v".to_string(), "libx264".to_string(),
            "-preset".to_string(), "medium".to_string(),
            "-crf".to_string(), "23".to_string(),
            "-c:a".to_string(), "aac".to_string(),
            "-b:a".to_string(), "128k".to_string(),
            "-pix_fmt".to_string(), "yuv420p".to_string(),
            "-vf".to_string(), "scale=1080:1080:force_original_aspect_ratio=decrease,pad=1080:1080:(ow-iw)/2:(oh-ih)/2".to_string(),
        ],
        "tiktok" => vec![
            "-c:v".to_string(), "libx264".to_string(),
            "-preset".to_string(), "medium".to_string(),
            "-crf".to_string(), "23".to_string(),
            "-c:a".to_string(), "aac".to_string(),
            "-b:a".to_string(), "128k".to_string(),
            "-pix_fmt".to_string(), "yuv420p".to_string(),
            "-vf".to_string(), "scale=1080:1920:force_original_aspect_ratio=decrease,pad=1080:1920:(ow-iw)/2:(oh-ih)/2".to_string(),
        ],
        "twitter" => vec![
            "-c:v".to_string(), "libx264".to_string(),
            "-preset".to_string(), "medium".to_string(),
            "-crf".to_string(), "23".to_string(),
            "-c:a".to_string(), "aac".to_string(),
            "-b:a".to_string(), "128k".to_string(),
            "-pix_fmt".to_string(), "yuv420p".to_string(),
            "-vf".to_string(), "scale=1280:720:force_original_aspect_ratio=decrease".to_string(),
        ],
        "facebook" => vec![
            "-c:v".to_string(), "libx264".to_string(),
            "-preset".to_string(), "medium".to_string(),
            "-crf".to_string(), "20".to_string(),
            "-c:a".to_string(), "aac".to_string(),
            "-b:a".to_string(), "128k".to_string(),
            "-pix_fmt".to_string(), "yuv420p".to_string(),
        ],
        "whatsapp" => vec![
            "-c:v".to_string(), "libx264".to_string(),
            "-preset".to_string(), "fast".to_string(),
            "-crf".to_string(), "28".to_string(),
            "-c:a".to_string(), "aac".to_string(),
            "-b:a".to_string(), "96k".to_string(),
            "-pix_fmt".to_string(), "yuv420p".to_string(),
            "-vf".to_string(), "scale=640:480:force_original_aspect_ratio=decrease".to_string(),
        ],
        "web-hd" => vec![
            "-c:v".to_string(), "libx264".to_string(),
            "-preset".to_string(), "medium".to_string(),
            "-crf".to_string(), "23".to_string(),
            "-c:a".to_string(), "aac".to_string(),
            "-b:a".to_string(), "128k".to_string(),
            "-pix_fmt".to_string(), "yuv420p".to_string(),
            "-vf".to_string(), "scale=1280:720:force_original_aspect_ratio=decrease".to_string(),
        ],
        "web-4k" => vec![
            "-c:v".to_string(), "libx264".to_string(),
            "-preset".to_string(), "slow".to_string(),
            "-crf".to_string(), "18".to_string(),
            "-c:a".to_string(), "aac".to_string(),
            "-b:a".to_string(), "192k".to_string(),
            "-pix_fmt".to_string(), "yuv420p".to_string(),
            "-vf".to_string(), "scale=3840:2160:force_original_aspect_ratio=decrease".to_string(),
        ],
        "dvd" => vec![
            "-c:v".to_string(), "mpeg2video".to_string(),
            "-b:v".to_string(), "6000k".to_string(),
            "-c:a".to_string(), "ac3".to_string(),
            "-b:a".to_string(), "192k".to_string(),
            "-vf".to_string(), "scale=720:480:force_original_aspect_ratio=decrease".to_string(),
        ],
        "mobile" => vec![
            "-c:v".to_string(), "libx264".to_string(),
            "-preset".to_string(), "fast".to_string(),
            "-crf".to_string(), "28".to_string(),
            "-c:a".to_string(), "aac".to_string(),
            "-b:a".to_string(), "96k".to_string(),
            "-pix_fmt".to_string(), "yuv420p".to_string(),
            "-vf".to_string(), "scale=640:360:force_original_aspect_ratio=decrease".to_string(),
        ],
        _ => vec![
            "-c:v".to_string(), "libx264".to_string(),
            "-c:a".to_string(), "aac".to_string(),
        ],
    }
}

/// Validate that all input files exist
pub fn validate_input_files(files: &[String]) -> Result<(), String> {
    for file in files {
        if !std::path::Path::new(file).exists() {
            return Err(format!("Input file does not exist: {}", file));
        }
    }
    Ok(())
}

/// Create output directory if it doesn't exist
pub fn ensure_output_directory(output_path: &str) -> Result<(), String> {
    if let Some(parent) = std::path::Path::new(output_path).parent() {
        if !parent.exists() {
            std::fs::create_dir_all(parent)
                .map_err(|e| format!("Failed to create output directory: {}", e))?;
        }
    }
    Ok(())
}

/// Get file extension from path
pub fn get_file_extension(path: &str) -> Option<String> {
    std::path::Path::new(path)
        .extension()
        .and_then(|ext| ext.to_str())
        .map(|ext| ext.to_lowercase())
}

/// Check if file is a supported video format
pub fn is_supported_video_format(path: &str) -> bool {
    match get_file_extension(path) {
        Some(ext) => matches!(
            ext.as_str(),
            "mp4" | "avi" | "mov" | "mkv" | "webm" | "flv" | "wmv" | "m4v" | "3gp" | "ogv"
        ),
        None => false,
    }
}

/// Check if file is a supported audio format
pub fn is_supported_audio_format(path: &str) -> bool {
    match get_file_extension(path) {
        Some(ext) => matches!(
            ext.as_str(),
            "mp3" | "wav" | "aac" | "ogg" | "flac" | "m4a" | "wma" | "opus"
        ),
        None => false,
    }
}

/// Create a temporary file path for intermediate processing
pub fn create_temp_file(prefix: &str, extension: &str) -> String {
    use std::time::{SystemTime, UNIX_EPOCH};

    let timestamp = SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .unwrap()
        .as_millis();

    format!(
        "/tmp/{}_{}_{}.{}",
        prefix,
        std::process::id(),
        timestamp,
        extension
    )
}

/// Clean up temporary files
pub fn cleanup_temp_files(files: &[String]) {
    for file in files {
        std::fs::remove_file(file).ok();
    }
}

/// Extract a single frame from a video at a specific timestamp (seconds).
///
/// Deterministic thumbnail generation — no AI needed. The timestamp comes from
/// Gemini's `thumbnail_sec` field in the viral moment analysis.
///
/// Returns the path to the extracted frame image (JPEG).
pub fn extract_frame_at_timestamp(
    video_path: &str,
    timestamp_sec: f64,
    output_path: &str,
) -> Result<String, String> {
    let timestamp_str = format!("{:.3}", timestamp_sec);
    let mut command = Command::new("ffmpeg");
    command
        .arg("-ss")
        .arg(&timestamp_str)
        .arg("-i")
        .arg(video_path)
        .arg("-vframes")
        .arg("1")
        .arg("-q:v")
        .arg("2") // High quality JPEG
        .arg("-y")
        .arg(output_path);

    execute_ffmpeg_command(command)?;

    if !std::path::Path::new(output_path).exists() {
        return Err(format!(
            "Frame extraction failed: output file not found at {}",
            output_path
        ));
    }

    Ok(output_path.to_string())
}

/// Convert time in seconds to FFmpeg time format (HH:MM:SS.mmm)
pub fn seconds_to_ffmpeg_time(seconds: f64) -> String {
    let hours = (seconds / 3600.0) as u32;
    let minutes = ((seconds % 3600.0) / 60.0) as u32;
    let secs = (seconds % 60.0) as u32;
    let millis = ((seconds % 1.0) * 1000.0) as u32;
    format!("{:02}:{:02}:{:02}.{:03}", hours, minutes, secs, millis)
}

/// Extract specific information from ffprobe output
pub fn get_media_info(file_path: &str, info_type: &str) -> Result<String, String> {
    let args = match info_type {
        "duration" => vec![
            "-v",
            "quiet",
            "-show_entries",
            "format=duration",
            "-of",
            "csv=p=0",
            file_path,
        ],
        "width" => vec![
            "-v",
            "quiet",
            "-select_streams",
            "v:0",
            "-show_entries",
            "stream=width",
            "-of",
            "csv=p=0",
            file_path,
        ],
        "height" => vec![
            "-v",
            "quiet",
            "-select_streams",
            "v:0",
            "-show_entries",
            "stream=height",
            "-of",
            "csv=p=0",
            file_path,
        ],
        "fps" => vec![
            "-v",
            "quiet",
            "-select_streams",
            "v:0",
            "-show_entries",
            "stream=r_frame_rate",
            "-of",
            "csv=p=0",
            file_path,
        ],
        _ => return Err("Unknown info type".to_string()),
    };

    execute_ffprobe_command(&args)
}

/// Execute FFmpeg with complex filter graphs
pub fn execute_ffmpeg_complex_filter(
    inputs: &[&str],
    filter_complex: &str,
    output: &str,
    additional_args: &[&str],
) -> Result<String, String> {
    let mut command = Command::new("ffmpeg");

    // Add input files
    for input in inputs {
        command.arg("-i").arg(input);
    }

    // Add filter complex
    command.arg("-filter_complex").arg(filter_complex);

    // Add additional arguments
    command.args(additional_args);

    // Add output
    command.arg("-y").arg(output);

    execute_ffmpeg_command(command)
}

/// Build FFmpeg resize filter with aspect ratio handling
pub fn build_resize_filter(width: u32, height: u32, maintain_aspect: bool) -> String {
    if maintain_aspect {
        format!(
            "scale={}:{}:force_original_aspect_ratio=decrease",
            width, height
        )
    } else {
        format!("scale={}:{}", width, height)
    }
}

/// Build FFmpeg crop filter
pub fn build_crop_filter(x: u32, y: u32, width: u32, height: u32) -> String {
    format!("crop={}:{}:{}:{}", width, height, x, y)
}

/// Build FFmpeg overlay filter for picture-in-picture
pub fn build_overlay_filter(x: u32, y: u32, opacity: f64) -> String {
    if opacity < 1.0 {
        format!(
            "overlay={}:{}:format=auto,colorchannelmixer=aa={:.2}",
            x, y, opacity
        )
    } else {
        format!("overlay={}:{}", x, y)
    }
}

/// Create a blank video with specified color, duration, and dimensions
pub fn create_blank_video(
    output_file: &str,
    duration: f64,
    width: u32,
    height: u32,
    color: &str,
) -> Result<String, String> {
    let mut command = Command::new("ffmpeg");
    command
        .arg("-f")
        .arg("lavfi")
        .arg("-i")
        .arg(format!(
            "color=c={}:s={}x{}:d={}",
            color, width, height, duration
        ))
        .arg("-c:v")
        .arg("libx264")
        .arg("-y")
        .arg(output_file);

    execute_ffmpeg_command(command)
}
