// src/transform.rs


use crate::utils::execute_ffmpeg_command;
use std::process::Command;

pub fn resize_video(
    input_file: &str,
    output_file: &str,
    width: u32,
    height: u32,
) -> Result<String, String> {
    let filter = format!("scale={}:{}", width, height);

    let mut command = Command::new("ffmpeg");
    command
        .arg("-i")
        .arg(input_file)
        .arg("-vf")
        .arg(filter)
        .arg("-c:a")
        .arg("copy")
        .arg("-y")
        .arg(output_file);

    execute_ffmpeg_command(command)
}

pub fn crop_video(
    input_file: &str,
    output_file: &str,
    width: u32,
    height: u32,
    x: u32,
    y: u32,
) -> Result<String, String> {
    let filter = format!("crop={}:{}:{}:{}", width, height, x, y);

    let mut command = Command::new("ffmpeg");
    command
        .arg("-i")
        .arg(input_file)
        .arg("-vf")
        .arg(filter)
        .arg("-c:a")
        .arg("copy")
        .arg("-y")
        .arg(output_file);

    execute_ffmpeg_command(command)
}

pub fn rotate_video(input_file: &str, output_file: &str, angle: &str) -> Result<String, String> {
    let filter = match angle {
        "90" => "transpose=1",
        "180" => "transpose=2,transpose=2",
        "270" => "transpose=2",
        _ => return Err(format!("Unsupported angle: {}", angle)),
    };

    let mut command = Command::new("ffmpeg");
    command
        .arg("-i")
        .arg(input_file)
        .arg("-vf")
        .arg(filter)
        .arg("-c:a")
        .arg("copy")
        .arg("-y")
        .arg(output_file);

    execute_ffmpeg_command(command)
}

pub fn adjust_speed(
    input_file: &str,
    output_file: &str,
    speed_factor: f64,
) -> Result<String, String> {
    let video_filter = format!("setpts={}*PTS", 1.0 / speed_factor);
    let audio_filter = format!("atempo={}", speed_factor);

    let mut command = Command::new("ffmpeg");
    command
        .arg("-i")
        .arg(input_file)
        .arg("-filter:v")
        .arg(video_filter)
        .arg("-filter:a")
        .arg(audio_filter)
        .arg("-y")
        .arg(output_file);

    execute_ffmpeg_command(command)
}

pub fn flip_video(input_file: &str, output_file: &str, direction: &str) -> Result<String, String> {
    let filter = match direction {
        "horizontal" => "hflip",
        "vertical" => "vflip",
        _ => return Err(format!("Unsupported direction: {}", direction)),
    };

    let mut command = Command::new("ffmpeg");
    command
        .arg("-i")
        .arg(input_file)
        .arg("-vf")
        .arg(filter)
        .arg("-c:a")
        .arg("copy")
        .arg("-y")
        .arg(output_file);

    execute_ffmpeg_command(command)
}

pub fn scale_video(
    input_file: &str,
    output_file: &str,
    scale_factor: f64,
    algorithm: &str,
) -> Result<String, String> {
    let filter = format!("scale=iw*{}:ih*{}:flags={}", scale_factor, scale_factor, algorithm);

    let mut command = Command::new("ffmpeg");
    command
        .arg("-i")
        .arg(input_file)
        .arg("-vf")
        .arg(filter)
        .arg("-c:a")
        .arg("copy")
        .arg("-y")
        .arg(output_file);

    execute_ffmpeg_command(command)
}

pub fn stabilize_video(
    input_file: &str,
    output_file: &str,
    shakiness: u32,
) -> Result<String, String> {
    let detect_filter = format!("vidstabdetect=shakiness={}:result=transforms.trf", shakiness);
    let transform_filter = "vidstabtransform=input=transforms.trf";

    let mut detect_command = Command::new("ffmpeg");
    detect_command
        .arg("-i")
        .arg(input_file)
        .arg("-vf")
        .arg(detect_filter)
        .arg("-f")
        .arg("null")
        .arg("-");

    execute_ffmpeg_command(detect_command)?;

    let mut transform_command = Command::new("ffmpeg");
    transform_command
        .arg("-i")
        .arg(input_file)
        .arg("-vf")
        .arg(transform_filter)
        .arg("-c:a")
        .arg("copy")
        .arg("-y")
        .arg(output_file);

    execute_ffmpeg_command(transform_command)
}

pub fn create_thumbnail(
    input_file: &str,
    output_file: &str,
    timestamp: f64,
) -> Result<String, String> {
    let mut command = Command::new("ffmpeg");
    command
        .arg("-i")
        .arg(input_file)
        .arg("-ss")
        .arg(timestamp.to_string())
        .arg("-vframes")
        .arg("1")
        .arg("-y")
        .arg(output_file);

    execute_ffmpeg_command(command)
}

/// Create a thumbnail with custom resolution (for YouTube uploads)
///
/// YouTube recommends 1280x720 minimum resolution for thumbnails
///
/// # Arguments
/// * `input_file` - Path to input video
/// * `output_file` - Path to save thumbnail
/// * `timestamp` - Time in seconds to extract frame
/// * `width` - Target width in pixels
/// * `height` - Target height in pixels
pub fn create_thumbnail_scaled(
    input_file: &str,
    output_file: &str,
    timestamp: f64,
    width: u32,
    height: u32,
) -> Result<String, String> {
    let mut command = Command::new("ffmpeg");
    command
        .arg("-i")
        .arg(input_file)
        .arg("-ss")
        .arg(timestamp.to_string())
        .arg("-vframes")
        .arg("1")
        .arg("-vf")
        .arg(format!("scale={}:{}", width, height))
        .arg("-y")
        .arg(output_file);

    execute_ffmpeg_command(command)
}

pub fn deinterlace_video(
    input_file: &str,
    output_file: &str,
    mode: &str,
) -> Result<String, String> {
    let filter = format!("yadif=mode={}", mode);

    let mut command = Command::new("ffmpeg");
    command
        .arg("-i")
        .arg(input_file)
        .arg("-vf")
        .arg(filter)
        .arg("-c:a")
        .arg("copy")
        .arg("-y")
        .arg(output_file);

    execute_ffmpeg_command(command)
}