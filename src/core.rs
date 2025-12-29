// src/core.rs

use crate::types::*;
use crate::utils::{execute_ffmpeg_command, execute_ffprobe_command};
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
    let mut command = Command::new("ffmpeg");
    command
        .arg("-i")
        .arg(input_file)
        .arg("-ss")
        .arg(start_seconds.to_string())
        .arg("-t")
        .arg(duration.to_string())
        .arg("-y")
        .arg(output_file);

    execute_ffmpeg_command(command)
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
    let concat_list = input_files
        .iter()
        .map(|f| {
            let absolute_path = std::fs::canonicalize(f).unwrap();
            format!("file '{}'", absolute_path.to_str().unwrap())
        })
        .collect::<Vec<String>>()
        .join("\n");
    let concat_file_path = format!("{}.txt", output_file);
    std::fs::write(&concat_file_path, concat_list).map_err(|e| e.to_string())?;

    let mut command = Command::new("ffmpeg");
    command
        .arg("-f")
        .arg("concat")
        .arg("-safe")
        .arg("0")
        .arg("-i")
        .arg(&concat_file_path)
        .arg("-c")
        .arg("copy")
        .arg("-y")
        .arg(output_file);

    let result = execute_ffmpeg_command(command);
    std::fs::remove_file(concat_file_path).ok();
    result
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