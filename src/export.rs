// src/export.rs


use crate::utils::execute_ffmpeg_command;
use std::process::Command;

pub fn convert_format(
    input_file: &str,
    output_file: &str,
    format: &str,
) -> Result<String, String> {
    let mut command = Command::new("ffmpeg");
    command
        .arg("-i")
        .arg(input_file)
        .arg("-f")
        .arg(format)
        .arg("-c:v")
        .arg("libx264")
        .arg("-c:a")
        .arg("aac")
        .arg("-y")
        .arg(output_file);

    execute_ffmpeg_command(command)
}

pub fn export_custom_quality(
    input_file: &str,
    output_file: &str,
    quality: &str,
    resolution: Option<(u32, u32)>,
    bitrate: Option<u32>,
) -> Result<String, String> {
    let mut command = Command::new("ffmpeg");
    command.arg("-i").arg(input_file);

    if let Some((width, height)) = resolution {
        command.arg("-vf").arg(format!("scale={}:{}", width, height));
    }

    if let Some(b) = bitrate {
        command.arg("-b:v").arg(format!("{}k", b));
    } else {
        let crf = match quality {
            "low" => "28",
            "medium" => "23",
            "high" => "18",
            "ultra" => "14",
            _ => "23",
        };
        command.arg("-crf").arg(crf);
    }

    command.arg("-c:a").arg("aac").arg("-b:a").arg("192k");
    command.arg("-y").arg(output_file);

    execute_ffmpeg_command(command)
}

pub fn export_for_platform(
    input_file: &str,
    output_file: &str,
    platform: &str,
) -> Result<String, String> {
    let (resolution, bitrate, fps) = match platform {
        "youtube" => ((1920, 1080), 8000, 30),
        "youtube-4k" => ((3840, 2160), 35000, 30),
        "instagram" => ((1080, 1080), 3500, 30),
        "tiktok" => ((1080, 1920), 4000, 30),
        "twitter" => ((1280, 720), 5000, 30),
        "facebook" => ((1920, 1080), 6000, 30),
        _ => return Err(format!("Unsupported platform: {}", platform)),
    };

    let mut command = Command::new("ffmpeg");
    command
        .arg("-i")
        .arg(input_file)
        .arg("-vf")
        .arg(format!("scale={}:{}", resolution.0, resolution.1))
        .arg("-r")
        .arg(fps.to_string())
        .arg("-b:v")
        .arg(format!("{}k", bitrate))
        .arg("-c:a")
        .arg("aac")
        .arg("-b:a")
        .arg("192k")
        .arg("-y")
        .arg(output_file);

    execute_ffmpeg_command(command)
}

pub fn compress_video(
    input_file: &str,
    output_file: &str,
    preset: &str,
) -> Result<String, String> {
    let crf = match preset {
        "light" => "24",
        "medium" => "28",
        "heavy" => "32",
        "extreme" => "36",
        _ => "28",
    };

    let mut command = Command::new("ffmpeg");
    command
        .arg("-i")
        .arg(input_file)
        .arg("-vcodec")
        .arg("libx264")
        .arg("-crf")
        .arg(crf)
        .arg("-preset")
        .arg("slow")
        .arg("-c:a")
        .arg("copy")
        .arg("-y")
        .arg(output_file);

    execute_ffmpeg_command(command)
}

pub fn extract_frames(
    input_file: &str,
    output_dir: &str,
    fps: f64,
    format: &str,
) -> Result<String, String> {
    let output_pattern = format!("{}/frame_%04d.{}", output_dir, format);
    let mut command = Command::new("ffmpeg");
    command
        .arg("-i")
        .arg(input_file)
        .arg("-vf")
        .arg(format!("fps={}", fps))
        .arg("-y")
        .arg(output_pattern);

    execute_ffmpeg_command(command)
}