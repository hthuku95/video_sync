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
    let filter = format!(
        "scale=iw*{}:ih*{}:flags={}",
        scale_factor, scale_factor, algorithm
    );

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
    let detect_filter = format!(
        "vidstabdetect=shakiness={}:result=transforms.trf",
        shakiness
    );
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

// ============================================================================
// BATCH 6 — Motion, Time & Frame Effects
// ============================================================================

pub fn reverse_video(input_file: &str, output_file: &str) -> Result<String, String> {
    let mut command = Command::new("ffmpeg");
    command
        .arg("-i")
        .arg(input_file)
        .arg("-vf")
        .arg("reverse")
        .arg("-af")
        .arg("areverse")
        .arg("-y")
        .arg(output_file);
    execute_ffmpeg_command(command)
}

pub fn loop_video(
    input_file: &str,
    output_file: &str,
    loop_count: i32,
    duration: f64,
) -> Result<String, String> {
    let vf = format!("loop=loop={}:size=32767:start=0", loop_count);
    let af = format!("aloop=loop={}:size=2147483647:start=0", loop_count);
    let mut command = Command::new("ffmpeg");
    command
        .arg("-i")
        .arg(input_file)
        .arg("-vf")
        .arg(vf)
        .arg("-af")
        .arg(af)
        .arg("-t")
        .arg(duration.to_string())
        .arg("-y")
        .arg(output_file);
    execute_ffmpeg_command(command)
}

// ============================================================================
// PHASE D — Professional Finishing: Transform Tools
// ============================================================================

/// Arbitrary-angle rotation via FFmpeg rotate filter (angle in radians). Unlike rotate_video (90°/180°/270° only), this rotates by any angle with configurable fill colour.
pub fn apply_rotate_angle(
    input_file: &str,
    output_file: &str,
    angle_rad: f64,
    fillcolor: &str,
    expand: bool,
) -> Result<String, String> {
    let fc = if fillcolor.is_empty() {
        "black"
    } else {
        fillcolor
    };
    let filter = if expand {
        format!(
            "rotate=angle={}:fillcolor={}:ow=rotw({}):oh=roth({})",
            angle_rad, fc, angle_rad, angle_rad
        )
    } else {
        format!("rotate=angle={}:fillcolor={}:ow=iw:oh=ih", angle_rad, fc)
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

pub fn deshake_video(
    input_file: &str,
    output_file: &str,
    x: i32,
    y: i32,
    w: i32,
    h: i32,
    rx: u32,
    ry: u32,
) -> Result<String, String> {
    let filter = format!(
        "deshake=x={}:y={}:w={}:h={}:rx={}:ry={}",
        x, y, w, h, rx, ry
    );
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
