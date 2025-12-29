// src/visual.rs


use crate::utils::execute_ffmpeg_command;
use serde_json::Value;
use std::process::Command;

pub fn apply_filter(
    input_file: &str,
    output_file: &str,
    filter_type: &str,
    intensity: f64,
) -> Result<String, String> {
    let filter = match filter_type {
        "grayscale" => "format=gray".to_string(),
        "sepia" => format!(
            "colorchannelmixer=.393:.769:.189:0:.349:.686:.168:0:.272:.534:.131"
        ),
        "blur" => format!("gblur=sigma={}", intensity * 5.0),
        "sharpen" => format!("unsharp=5:5:1.0:5:5:0.0"),
        "edge" => "edgedetect".to_string(),
        "emboss" => "convolution=-2 -1 0 -1 1 1 0 1 2:-2 -1 0 -1 1 1 0 1 2:-2 -1 0 -1 1 1 0 1 2:-2 -1 0 -1 1 1 0 1 2".to_string(),
        "negative" => "negate".to_string(),
        _ => return Err(format!("Unsupported filter type: {}", filter_type)),
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

pub fn adjust_color(
    input_file: &str,
    output_file: &str,
    brightness: f64,
    contrast: f64,
    saturation: f64,
) -> Result<String, String> {
    let filter = format!(
        "eq=brightness={}:contrast={}:saturation={}",
        brightness, contrast, saturation
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

pub fn add_overlay(
    input_file: &str,
    overlay_file: &str,
    output_file: &str,
    x: u32,
    y: u32,
) -> Result<String, String> {
    let filter = format!("overlay={}:{}", x, y);

    let mut command = Command::new("ffmpeg");
    command
        .arg("-i")
        .arg(input_file)
        .arg("-i")
        .arg(overlay_file)
        .arg("-filter_complex")
        .arg(filter)
        .arg("-c:a")
        .arg("copy")
        .arg("-y")
        .arg(output_file);

    execute_ffmpeg_command(command)
}

pub fn add_subtitles(
    input_file: &str,
    subtitle_file: &str,
    output_file: &str,
) -> Result<String, String> {
    let filter = format!("subtitles={}", subtitle_file);

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

pub fn add_transition(
    input1: &str,
    input2: &str,
    output_file: &str,
    transition_type: &str,
    duration: f64,
    offset: f64,
) -> Result<String, String> {
    let filter = format!(
        "[0:v]settb=AVTB[v0];[1:v]settb=AVTB[v1];[v0][v1]xfade=transition={}:duration={}:offset={}",
        transition_type, duration, offset
    );

    let mut command = Command::new("ffmpeg");
    command
        .arg("-i")
        .arg(input1)
        .arg("-i")
        .arg(input2)
        .arg("-filter_complex")
        .arg(filter)
        .arg("-c:a")
        .arg("copy")
        .arg("-y")
        .arg(output_file);

    execute_ffmpeg_command(command)
}

pub fn add_text_overlay(
    input_file: &str,
    output_file: &str,
    text: &str,
    x: &str,
    y: &str,
    font_file: &str,
    font_size: u32,
    font_color: &str,
    start_time: f64,
    end_time: f64,
) -> Result<String, String> {
    let filter = format!(
        "drawtext=text='{}':x={}:y={}:fontfile={}:fontsize={}:fontcolor={}:enable='between(t,{},{})'",
        text, x, y, font_file, font_size, font_color, start_time, end_time
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

pub fn add_animated_text(
    input_file: &str,
    output_file: &str,
    text: &str,
    animation_type: &str,
    start_time: f64,
    duration: f64,
) -> Result<String, String> {
    let filter = match animation_type {
        "fade_in" => format!(
            "drawtext=text='{}':fontfile=/path/to/font.ttf:fontsize=24:fontcolor=white:x=(w-text_w)/2:y=(h-text_h)/2:alpha='if(lt(t,{}),0,if(lt(t,{}),(t-{})/{},1))'",
            text, start_time, start_time + duration, start_time, duration
        ),
        "slide_in" => format!(
            "drawtext=text='{}':fontfile=/path/to/font.ttf:fontsize=24:fontcolor=white:x='if(lt(t,{}),-w+(t-{})*w/{},w/2-text_w/2)':y=(h-text_h)/2",
            text, start_time, start_time, duration
        ),
        "typewriter" => format!(
            "drawtext=text='{}':fontfile=/path/to/font.ttf:fontsize=24:fontcolor=white:x=(w-text_w)/2:y=(h-text_h)/2:text_shaping=1:alpha='if(lt(t,{}),0,1)':text='{}'",
            text, start_time, text
        ),
        _ => return Err(format!("Unsupported animation type: {}", animation_type)),
    };

    let mut command = Command::new("ffmpeg");
    command
        .arg("-i")
        .arg(input_file)
        .arg("-vf")
        .arg(&filter)
        .arg("-c:a")
        .arg("copy")
        .arg("-y")
        .arg(output_file);

    execute_ffmpeg_command(command)
}

pub fn apply_filter_chain(
    input_file: &str,
    output_file: &str,
    filters: &[(String, Value)],
) -> Result<String, String> {
    let filter_str = filters
        .iter()
        .map(|(name, value)| match name.as_str() {
            "brightness" => format!("eq=brightness={}", value.as_f64().unwrap_or(0.0)),
            "contrast" => format!("eq=contrast={}", value.as_f64().unwrap_or(1.0)),
            "saturation" => format!("eq=saturation={}", value.as_f64().unwrap_or(1.0)),
            "blur" => format!("gblur=sigma={}", value.as_f64().unwrap_or(0.0)),
            _ => "".to_string(),
        })
        .collect::<Vec<String>>()
        .join(",");

    let mut command = Command::new("ffmpeg");
    command
        .arg("-i")
        .arg(input_file)
        .arg("-vf")
        .arg(filter_str)
        .arg("-c:a")
        .arg("copy")
        .arg("-y")
        .arg(output_file);

    execute_ffmpeg_command(command)
}