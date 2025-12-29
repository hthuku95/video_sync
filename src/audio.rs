// src/audio.rs


use crate::utils::execute_ffmpeg_command;
use std::process::Command;

pub fn extract_audio(
    input_file: &str,
    output_file: &str,
    format: &str,
) -> Result<String, String> {
    let mut command = Command::new("ffmpeg");
    command
        .arg("-i")
        .arg(input_file)
        .arg("-vn")
        .arg("-acodec")
        .arg(match format {
            "mp3" => "libmp3lame",
            "aac" => "aac",
        "wav" => "pcm_s16le",
        "flac" => "flac",
        "ogg" => "libvorbis",
        _ => return Err(format!("Unsupported format: {}", format)),
    })
        .arg("-y")
        .arg(output_file);

    execute_ffmpeg_command(command)
}

pub fn add_audio(
    video_file: &str,
    audio_file: &str,
    output_file: &str,
) -> Result<String, String> {
    let mut command = Command::new("ffmpeg");
    command
        .arg("-i")
        .arg(video_file)
        .arg("-i")
        .arg(audio_file)
        .arg("-c:v")
        .arg("copy")
        .arg("-c:a")
        .arg("aac")
        .arg("-shortest")
        .arg("-y")
        .arg(output_file);

    execute_ffmpeg_command(command)
}

pub fn adjust_volume(
    input_file: &str,
    output_file: &str,
    volume_level: f64,
) -> Result<String, String> {
    let filter = format!("volume={}", volume_level);

    let mut command = Command::new("ffmpeg");
    command
        .arg("-i")
        .arg(input_file)
        .arg("-af")
        .arg(filter)
        .arg("-c:v")
        .arg("copy")
        .arg("-y")
        .arg(output_file);

    execute_ffmpeg_command(command)
}

pub fn fade_audio(
    input_file: &str,
    output_file: &str,
    fade_in_duration: f64,
    fade_out_duration: f64,
    duration: f64,
) -> Result<String, String> {
    let filter = format!(
        "afade=t=in:st=0:d={},afade=t=out:st={}:d={}",
        fade_in_duration,
        duration - fade_out_duration,
        fade_out_duration
    );

    let mut command = Command::new("ffmpeg");
    command
        .arg("-i")
        .arg(input_file)
        .arg("-af")
        .arg(filter)
        .arg("-c:v")
        .arg("copy")
        .arg("-y")
        .arg(output_file);

    execute_ffmpeg_command(command)
}

pub fn apply_audio_effect(
    input_file: &str,
    output_file: &str,
    effect: &str,
    intensity: f64,
) -> Result<String, String> {
    let filter = match effect {
        "echo" => format!("aecho=0.8:0.9:{}:0.3", 1000.0 * intensity),
        "reverb" => "aecho=0.8:0.88:60:0.4".to_string(),
        "chorus" => "chorus=0.5:0.9:50|60:0.4|0.3:0.25|0.4:2|1.5".to_string(),
        _ => return Err(format!("Unsupported effect: {}", effect)),
    };

    let mut command = Command::new("ffmpeg");
    command
        .arg("-i")
        .arg(input_file)
        .arg("-af")
        .arg(filter)
        .arg("-c:v")
        .arg("copy")
        .arg("-y")
        .arg(output_file);

    execute_ffmpeg_command(command)
}