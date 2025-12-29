// src/advanced.rs


use crate::utils::execute_ffmpeg_command;
use std::process::Command;

pub fn picture_in_picture(
    main_video: &str,
    overlay_video: &str,
    output_file: &str,
    x: &str,
    y: &str,
) -> Result<String, String> {
    let filter = format!("[1:v]scale=iw/4:ih/4 [pip]; [0:v][pip]overlay=x={}:y={}", x, y);

    let mut command = Command::new("ffmpeg");
    command
        .arg("-i")
        .arg(main_video)
        .arg("-i")
        .arg(overlay_video)
        .arg("-filter_complex")
        .arg(&filter)
        .arg("-c:a")
        .arg("copy")
        .arg("-y")
        .arg(output_file);

    execute_ffmpeg_command(command)
}

pub fn chroma_key(
    input_file: &str,
    background_file: &str,
    output_file: &str,
    color: &str,
    similarity: f32,
    blend: f32,
) -> Result<String, String> {
    let filter = format!(
        "[1:v]colorkey=color={}:similarity={}:blend={}[ckout];[0:v][ckout]overlay[out]",
        color, similarity, blend
    );

    let mut command = Command::new("ffmpeg");
    command
        .arg("-i")
        .arg(background_file)
        .arg("-i")
        .arg(input_file)
        .arg("-filter_complex")
        .arg(&filter)
        .arg("-map")
        .arg("[out]")
        .arg("-map")
        .arg("0:a?")
        .arg("-c:a")
        .arg("copy")
        .arg("-y")
        .arg(output_file);

    execute_ffmpeg_command(command)
}

pub fn split_screen(
    video1: &str,
    video2: &str,
    output_file: &str,
    layout: &str,
) -> Result<String, String> {
    let filter = match layout {
        "horizontal" => "[0:v][1:v]hstack=inputs=2[v]",
        "vertical" => "[0:v][1:v]vstack=inputs=2[v]",
        _ => return Err(format!("Unsupported layout: {}", layout)),
    };

    let mut command = Command::new("ffmpeg");
    command
        .arg("-i")
        .arg(video1)
        .arg("-i")
        .arg(video2)
        .arg("-filter_complex")
        .arg(filter)
        .arg("-map")
        .arg("[v]")
        .arg("-map")
        .arg("0:a?")
        .arg("-c:a")
        .arg("copy")
        .arg("-y")
        .arg(output_file);

    execute_ffmpeg_command(command)
}