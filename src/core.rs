// src/core.rs

use crate::types::*;
use crate::utils::{execute_ffmpeg_command, execute_ffmpeg_command_with_sync_timeout, execute_ffprobe_command};
use serde_json::Value;
use std::process::Command;

pub fn analyze_video(file_path: &str) -> Result<VideoMetadata, String> {
    let args = &[
        "-v",
        "quiet",
        "-print_format",
        "json",
        "-show_format",
        "-show_streams",
        file_path,
    ];
    let ffprobe_output = execute_ffprobe_command(args)?;
    let json: Value = serde_json::from_str(&ffprobe_output)
        .map_err(|e| format!("Failed to parse ffprobe output: {}", e))?;

    let format = &json["format"];
    let duration_seconds = format["duration"]
        .as_str()
        .unwrap_or("0")
        .parse::<f64>()
        .unwrap_or(0.0);
    let file_size_mb = format["size"]
        .as_str()
        .unwrap_or("0")
        .parse::<f64>()
        .unwrap_or(0.0)
        / (1024.0 * 1024.0);

    let mut metadata = VideoMetadata {
        file_path: file_path.to_string(),
        duration_seconds,
        width: 0,
        height: 0,
        fps: 0.0,
        has_audio: false,
        has_video: false,
        format: format["format_name"].as_str().unwrap_or("unknown").to_string(),
        file_size_mb,
    };

    if let Some(streams) = json["streams"].as_array() {
        for stream in streams {
            if stream["codec_type"] == "video" {
                metadata.has_video = true;
                metadata.width = stream["width"].as_u64().unwrap_or(0) as u32;
                metadata.height = stream["height"].as_u64().unwrap_or(0) as u32;
                let fps_str = stream["r_frame_rate"].as_str().unwrap_or("0/1");
                let parts: Vec<&str> = fps_str.split('/').collect();
                if parts.len() == 2 {
                    let num = parts[0].parse::<f64>().unwrap_or(0.0);
                    let den = parts[1].parse::<f64>().unwrap_or(1.0);
                    if den != 0.0 {
                        metadata.fps = num / den;
                    }
                }
            } else if stream["codec_type"] == "audio" {
                metadata.has_audio = true;
            }
        }
    }

    Ok(metadata)
}

pub fn trim_video(
    input_file: &str,
    output_file: &str,
    start_seconds: f64,
    end_seconds: f64,
) -> Result<String, String> {
    let duration = end_seconds - start_seconds;

    // Check if input is already H.264/AAC (can use stream copy)
    let can_copy = match analyze_video(input_file) {
        Ok(meta) => {
            // Check if format contains h264/avc (video codec indicators)
            let is_h264 = meta.format.to_lowercase().contains("h264")
                || meta.format.to_lowercase().contains("avc")
                || meta.format.to_lowercase().contains("mp4");
            tracing::info!("Input format: {}, can use stream copy: {}", meta.format, is_h264);
            is_h264
        }
        Err(e) => {
            tracing::warn!("Unable to analyze input video, will re-encode for safety: {}", e);
            false  // If analysis fails, re-encode to be safe
        }
    };

    let mut command = Command::new("ffmpeg");

    if can_copy {
        // Use stream copy (fast) - move -ss BEFORE -i for faster seeking
        tracing::info!("🚀 Using stream copy for fast extraction");
        command
            .arg("-ss")
            .arg(start_seconds.to_string())
            .arg("-i")
            .arg(input_file)
            .arg("-t")
            .arg(duration.to_string())
            .arg("-c:v")
            .arg("copy")
            .arg("-c:a")
            .arg("copy")
            .arg("-avoid_negative_ts")
            .arg("make_zero")
            .arg("-y")
            .arg(output_file);
    } else {
        // Re-encode (safe and compatible)
        tracing::info!("🔄 Re-encoding for compatibility and quality");
        command
            .arg("-ss")
            .arg(start_seconds.to_string())
            .arg("-i")
            .arg(input_file)
            .arg("-t")
            .arg(duration.to_string())
            .arg("-c:v")
            .arg("libx264")
            .arg("-preset")
            .arg("fast")
            .arg("-crf")
            .arg("23")
            .arg("-c:a")
            .arg("aac")
            .arg("-b:a")
            .arg("192k")
            .arg("-movflags")
            .arg("faststart")
            .arg("-y")
            .arg(output_file);
    }

    // Use timeout-protected execution (10-minute timeout for clips)
    execute_ffmpeg_command_with_sync_timeout(command, Some(600))
}

pub fn extract_video_segment(
    input_file: &str,
    output_file: &str,
    start_seconds: f64,
    end_seconds: f64,
) -> Result<String, String> {
    trim_video(input_file, output_file, start_seconds, end_seconds)
}

pub fn merge_videos(input_files: &[String], output_file: &str) -> Result<String, String> {
    // Log video properties before merge for debugging
    tracing::info!("🎞️ Merging {} video clips", input_files.len());
    for (i, file) in input_files.iter().enumerate() {
        match analyze_video(file) {
            Ok(meta) => {
                tracing::info!(
                    "  Clip {}: {}x{} @ {:.1}fps, format: {}, duration: {:.1}s",
                    i + 1, meta.width, meta.height, meta.fps, meta.format, meta.duration_seconds
                );
            }
            Err(e) => {
                tracing::warn!("  Clip {}: Unable to analyze - {}", i + 1, e);
            }
        }
    }

    // Build concat filter for re-encoding (handles mixed properties)
    // Format: [0:v][0:a][1:v][1:a]...concat=n=N:v=1:a=1[outv][outa]
    let mut filter_parts = Vec::new();
    for i in 0..input_files.len() {
        filter_parts.push(format!("[{}:v][{}:a]", i, i));
    }
    let filter_complex = format!(
        "{}concat=n={}:v=1:a=1[outv][outa]",
        filter_parts.join(""),
        input_files.len()
    );

    tracing::info!("🔧 Using concat filter with re-encoding for compatibility");

    let mut command = Command::new("ffmpeg");

    // Add all input files
    for file in input_files {
        command.arg("-i").arg(file);
    }

    command
        .arg("-filter_complex")
        .arg(&filter_complex)
        .arg("-map")
        .arg("[outv]")
        .arg("-map")
        .arg("[outa]")
        // Video encoding settings (H.264 with good quality/speed balance)
        .arg("-c:v")
        .arg("libx264")
        .arg("-preset")
        .arg("medium") // medium preset: good balance between speed and quality
        .arg("-crf")
        .arg("23") // Constant Rate Factor: 23 is good quality (lower = better, 18-28 range)
        .arg("-pix_fmt")
        .arg("yuv420p") // Maximum compatibility pixel format
        // Audio encoding settings
        .arg("-c:a")
        .arg("aac")
        .arg("-b:a")
        .arg("192k") // Audio bitrate
        // Ensure proper moov atom placement for streaming
        .arg("-movflags")
        .arg("faststart")
        // Overwrite output file
        .arg("-y")
        .arg(output_file);

    tracing::info!("⏳ Re-encoding and merging (this may take 30-60 seconds)...");
    // Use timeout version (10 minutes max for complex merges)
    let result = execute_ffmpeg_command_with_sync_timeout(command, Some(600))?;

    // Validate the merged output
    tracing::info!("🔍 Validating merged output...");
    if !validate_video_file(output_file)? {
        return Err(format!("❌ Merged video is corrupted or unreadable: {}", output_file));
    }

    // Check that duration is reasonable (at least 1 second)
    match analyze_video(output_file) {
        Ok(metadata) => {
            if metadata.duration_seconds < 1.0 {
                return Err(format!(
                    "❌ Merged video is too short ({:.1}s), likely corrupted",
                    metadata.duration_seconds
                ));
            }
            tracing::info!(
                "✅ Merge successful: {}x{} @ {:.1}fps, duration: {:.1}s",
                metadata.width, metadata.height, metadata.fps, metadata.duration_seconds
            );
        }
        Err(e) => {
            tracing::warn!("⚠️ Unable to analyze merged output: {}", e);
        }
    }

    Ok(result)
}

pub fn split_video(
    input_file: &str,
    output_prefix: &str,
    segment_duration: f64,
) -> Result<String, String> {
    let mut command = Command::new("ffmpeg");
    command
        .arg("-i")
        .arg(input_file)
        .arg("-c")
        .arg("copy")
        .arg("-map")
        .arg("0")
        .arg("-segment_time")
        .arg(segment_duration.to_string())
        .arg("-f")
        .arg("segment")
        .arg("-reset_timestamps")
        .arg("1")
        .arg(format!("{}_%03d.mp4", output_prefix));

    execute_ffmpeg_command(command)
}

pub fn get_video_duration(file_path: &str) -> Result<f64, String> {
    let metadata = analyze_video(file_path)?;
    Ok(metadata.duration_seconds)
}

pub fn validate_video_file(file_path: &str) -> Result<bool, String> {
    match analyze_video(file_path) {
        Ok(_) => Ok(true),
        Err(_) => Ok(false),
    }
}