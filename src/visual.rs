// src/visual.rs


use crate::utils::{execute_ffmpeg_command, execute_ffmpeg_command_with_sync_timeout};
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
    // Validate input file exists and is readable
    if !std::path::Path::new(input_file).exists() {
        return Err(format!("❌ Input file does not exist: {}", input_file));
    }

    // Validate input file is a valid video (basic check using core::validate_video_file)
    match crate::core::validate_video_file(input_file) {
        Ok(is_valid) => {
            if !is_valid {
                return Err(format!("❌ Input video is corrupted or unreadable: {}", input_file));
            }
        }
        Err(e) => {
            tracing::warn!("⚠️ Could not validate input video: {}", e);
            // Continue anyway - validation might fail for other reasons
        }
    }

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

    // Use timeout version (10 minutes max for text overlay operations)
    tracing::info!("📝 Adding text overlay to video (with 10-minute timeout)");
    execute_ffmpeg_command_with_sync_timeout(command, Some(600))
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

// ============================================================================
// BATCH 2 — Color Grading
// ============================================================================

pub fn adjust_hue(input_file: &str, output_file: &str, hue_degrees: f64, saturation: f64) -> Result<String, String> {
    let filter = format!("hue=h={}:s={}", hue_degrees, saturation);
    let mut command = Command::new("ffmpeg");
    command.arg("-i").arg(input_file).arg("-vf").arg(filter).arg("-c:a").arg("copy").arg("-y").arg(output_file);
    execute_ffmpeg_command(command)
}

pub fn color_balance(input_file: &str, output_file: &str, shadows: (f64, f64, f64), midtones: (f64, f64, f64), highlights: (f64, f64, f64)) -> Result<String, String> {
    let filter = format!(
        "colorbalance=rs={}:gs={}:bs={}:rm={}:gm={}:bm={}:rh={}:gh={}:bh={}",
        shadows.0, shadows.1, shadows.2, midtones.0, midtones.1, midtones.2, highlights.0, highlights.1, highlights.2
    );
    let mut command = Command::new("ffmpeg");
    command.arg("-i").arg(input_file).arg("-vf").arg(filter).arg("-c:a").arg("copy").arg("-y").arg(output_file);
    execute_ffmpeg_command(command)
}

pub fn normalize_video(input_file: &str, output_file: &str, smoothing: u32) -> Result<String, String> {
    let filter = format!("normalize=smoothing={}", smoothing);
    let mut command = Command::new("ffmpeg");
    command.arg("-i").arg(input_file).arg("-vf").arg(filter).arg("-c:a").arg("copy").arg("-y").arg(output_file);
    execute_ffmpeg_command(command)
}

pub fn apply_lut(input_file: &str, output_file: &str, lut_file: &str, interp: &str) -> Result<String, String> {
    let filter = format!("lut3d=file='{}':interp={}", lut_file, interp);
    let mut command = Command::new("ffmpeg");
    command.arg("-i").arg(input_file).arg("-vf").arg(filter).arg("-c:a").arg("copy").arg("-y").arg(output_file);
    execute_ffmpeg_command(command)
}

// ============================================================================
// BATCH 3 — Denoising & Sharpening
// ============================================================================

pub fn denoise_video(input_file: &str, output_file: &str, luma_spatial: f64, luma_temporal: f64, chroma_spatial: f64, chroma_temporal: f64) -> Result<String, String> {
    let filter = format!("hqdn3d={}:{}:{}:{}", luma_spatial, chroma_spatial, luma_temporal, chroma_temporal);
    let mut command = Command::new("ffmpeg");
    command.arg("-i").arg(input_file).arg("-vf").arg(filter).arg("-c:a").arg("copy").arg("-y").arg(output_file);
    execute_ffmpeg_command(command)
}

pub fn unsharp_mask(input_file: &str, output_file: &str, luma_msize_x: u32, luma_msize_y: u32, luma_amount: f64) -> Result<String, String> {
    let filter = format!("unsharp=luma_msize_x={}:luma_msize_y={}:luma_amount={}", luma_msize_x, luma_msize_y, luma_amount);
    let mut command = Command::new("ffmpeg");
    command.arg("-i").arg(input_file).arg("-vf").arg(filter).arg("-c:a").arg("copy").arg("-y").arg(output_file);
    execute_ffmpeg_command(command)
}

pub fn reduce_noise(input_file: &str, output_file: &str, strength: f64, research_size: u32, patch_size: u32) -> Result<String, String> {
    let filter = format!("nlmeans=s={}:r={}:p={}", strength, research_size, patch_size);
    let mut command = Command::new("ffmpeg");
    command.arg("-i").arg(input_file).arg("-vf").arg(filter).arg("-c:a").arg("copy").arg("-y").arg(output_file);
    execute_ffmpeg_command(command)
}

// ============================================================================
// BATCH 5 — Video Composition & Layout
// ============================================================================

pub fn pad_video(input_file: &str, output_file: &str, width: u32, height: u32, x: u32, y: u32, color: &str) -> Result<String, String> {
    let filter = format!("pad={}:{}:{}:{}:{}", width, height, x, y, color);
    let mut command = Command::new("ffmpeg");
    command.arg("-i").arg(input_file).arg("-vf").arg(filter).arg("-c:a").arg("copy").arg("-y").arg(output_file);
    execute_ffmpeg_command(command)
}

pub fn blend_videos(input1: &str, input2: &str, output_file: &str, blend_mode: &str, opacity: f64) -> Result<String, String> {
    let filter = format!("[0:v][1:v]blend=all_mode={}:all_opacity={}", blend_mode, opacity);
    let mut command = Command::new("ffmpeg");
    command.arg("-i").arg(input1).arg("-i").arg(input2).arg("-filter_complex").arg(filter).arg("-y").arg(output_file);
    execute_ffmpeg_command(command)
}

pub fn stack_videos(input1: &str, input2: &str, output_file: &str, direction: &str) -> Result<String, String> {
    let filter = match direction {
        "vertical" => "[0:v][1:v]vstack=inputs=2".to_string(),
        _ => "[0:v][1:v]hstack=inputs=2".to_string(),
    };
    let mut command = Command::new("ffmpeg");
    command.arg("-i").arg(input1).arg("-i").arg(input2).arg("-filter_complex").arg(filter).arg("-y").arg(output_file);
    execute_ffmpeg_command(command)
}

pub fn add_vignette(input_file: &str, output_file: &str, angle: f64, mode: &str) -> Result<String, String> {
    let mode_val = if mode == "backward" { 1 } else { 0 };
    let filter = format!("vignette=angle={}:mode={}", angle, mode_val);
    let mut command = Command::new("ffmpeg");
    command.arg("-i").arg(input_file).arg("-vf").arg(filter).arg("-c:a").arg("copy").arg("-y").arg(output_file);
    execute_ffmpeg_command(command)
}

pub fn draw_box(input_file: &str, output_file: &str, x: u32, y: u32, width: u32, height: u32, color: &str, thickness: i32) -> Result<String, String> {
    let filter = format!("drawbox=x={}:y={}:w={}:h={}:color={}:t={}", x, y, width, height, color, thickness);
    let mut command = Command::new("ffmpeg");
    command.arg("-i").arg(input_file).arg("-vf").arg(filter).arg("-c:a").arg("copy").arg("-y").arg(output_file);
    execute_ffmpeg_command(command)
}

// ============================================================================
// BATCH 6 (partial) — Motion & Frame Effects
// ============================================================================

pub fn zoompan(input_file: &str, output_file: &str, zoom: f64, x_expr: &str, y_expr: &str, duration_frames: u32, fps: u32) -> Result<String, String> {
    let filter = format!("zoompan=z='{}':x='{}':y='{}':d={}:fps={}", zoom, x_expr, y_expr, duration_frames, fps);
    let mut command = Command::new("ffmpeg");
    command.arg("-i").arg(input_file).arg("-vf").arg(filter).arg("-c:a").arg("copy").arg("-y").arg(output_file);
    execute_ffmpeg_command(command)
}

pub fn minterpolate(input_file: &str, output_file: &str, target_fps: u32, mode: &str) -> Result<String, String> {
    let filter = format!("minterpolate=fps={}:mi_mode={}", target_fps, mode);
    let mut command = Command::new("ffmpeg");
    command.arg("-i").arg(input_file).arg("-vf").arg(filter).arg("-c:a").arg("copy").arg("-y").arg(output_file);
    execute_ffmpeg_command(command)
}
// ============================================================================
// BATCH 7 — Advanced Color Grading
// ============================================================================

pub fn adjust_curves(input_file: &str, output_file: &str, preset: &str, master: &str, red: &str, green: &str, blue: &str) -> Result<String, String> {
    let filter = if !preset.is_empty() {
        format!("curves=preset={}", preset)
    } else {
        let mut parts = Vec::new();
        if !master.is_empty() { parts.push(format!("master='{}'", master)); }
        if !red.is_empty()    { parts.push(format!("r='{}'", red)); }
        if !green.is_empty()  { parts.push(format!("g='{}'", green)); }
        if !blue.is_empty()   { parts.push(format!("b='{}'", blue)); }
        if parts.is_empty() { return Err("adjust_curves: provide preset or at least one channel curve".to_string()); }
        format!("curves={}", parts.join(":"))
    };
    let mut command = Command::new("ffmpeg");
    command.arg("-i").arg(input_file).arg("-vf").arg(filter).arg("-c:a").arg("copy").arg("-y").arg(output_file);
    execute_ffmpeg_command(command)
}

pub fn adjust_levels(input_file: &str, output_file: &str, rimin: f64, rimax: f64, gimin: f64, gimax: f64, bimin: f64, bimax: f64, romin: f64, romax: f64, gomin: f64, gomax: f64, bomin: f64, bomax: f64) -> Result<String, String> {
    let filter = format!(
        "colorlevels=rimin={}:rimax={}:gimin={}:gimax={}:bimin={}:bimax={}:romin={}:romax={}:gomin={}:gomax={}:bomin={}:bomax={}",
        rimin, rimax, gimin, gimax, bimin, bimax, romin, romax, gomin, gomax, bomin, bomax
    );
    let mut command = Command::new("ffmpeg");
    command.arg("-i").arg(input_file).arg("-vf").arg(filter).arg("-c:a").arg("copy").arg("-y").arg(output_file);
    execute_ffmpeg_command(command)
}

pub fn split_tone(input_file: &str, output_file: &str, shadow_hue: f64, shadow_saturation: f64, highlight_hue: f64, highlight_saturation: f64, _balance: f64) -> Result<String, String> {
    let s_rad = shadow_hue.to_radians();
    let h_rad = highlight_hue.to_radians();
    let sr = s_rad.cos() * shadow_saturation;
    let sg = (s_rad + 2.094).cos() * shadow_saturation;
    let sb = (s_rad + 4.189).cos() * shadow_saturation;
    let hr = h_rad.cos() * highlight_saturation;
    let hg = (h_rad + 2.094).cos() * highlight_saturation;
    let hb = (h_rad + 4.189).cos() * highlight_saturation;
    let filter = format!(
        "colorbalance=rs={:.4}:gs={:.4}:bs={:.4}:rh={:.4}:gh={:.4}:bh={:.4}",
        sr.clamp(-1.0, 1.0), sg.clamp(-1.0, 1.0), sb.clamp(-1.0, 1.0),
        hr.clamp(-1.0, 1.0), hg.clamp(-1.0, 1.0), hb.clamp(-1.0, 1.0)
    );
    let mut command = Command::new("ffmpeg");
    command.arg("-i").arg(input_file).arg("-vf").arg(filter).arg("-c:a").arg("copy").arg("-y").arg(output_file);
    execute_ffmpeg_command(command)
}

pub fn convert_colorspace(input_file: &str, output_file: &str, colorspace: &str, trc: &str, primaries: &str) -> Result<String, String> {
    let mut parts = vec![format!("all={}", colorspace)];
    if !trc.is_empty() { parts.push(format!("trc={}", trc)); }
    if !primaries.is_empty() { parts.push(format!("primaries={}", primaries)); }
    let filter = format!("colorspace={}", parts.join(":"));
    let mut command = Command::new("ffmpeg");
    command.arg("-i").arg(input_file).arg("-vf").arg(filter).arg("-c:a").arg("copy").arg("-y").arg(output_file);
    execute_ffmpeg_command(command)
}

pub fn apply_tonemap(input_file: &str, output_file: &str, algorithm: &str, peak: f64, desat: f64) -> Result<String, String> {
    let filter = format!("tonemap={}:peak={}:desat={}", algorithm, peak, desat);
    let mut command = Command::new("ffmpeg");
    command.arg("-i").arg(input_file).arg("-vf").arg(filter).arg("-c:a").arg("copy").arg("-y").arg(output_file);
    execute_ffmpeg_command(command)
}

// ============================================================================
// BATCH 8 — Geometric Transforms
// ============================================================================

pub fn correct_perspective(input_file: &str, output_file: &str, x0: f64, y0: f64, x1: f64, y1: f64, x2: f64, y2: f64, x3: f64, y3: f64, interpolation: &str) -> Result<String, String> {
    let filter = format!(
        "perspective=x0={}:y0={}:x1={}:y1={}:x2={}:y2={}:x3={}:y3={}:interpolation={}",
        x0, y0, x1, y1, x2, y2, x3, y3, interpolation
    );
    let mut command = Command::new("ffmpeg");
    command.arg("-i").arg(input_file).arg("-vf").arg(filter).arg("-c:a").arg("copy").arg("-y").arg(output_file);
    execute_ffmpeg_command(command)
}

pub fn correct_lens(input_file: &str, output_file: &str, k1: f64, k2: f64, cx: f64, cy: f64, i: &str) -> Result<String, String> {
    let filter = format!("lenscorrection=k1={}:k2={}:cx={}:cy={}:i={}", k1, k2, cx, cy, i);
    let mut command = Command::new("ffmpeg");
    command.arg("-i").arg(input_file).arg("-vf").arg(filter).arg("-c:a").arg("copy").arg("-y").arg(output_file);
    execute_ffmpeg_command(command)
}

pub fn apply_shear(input_file: &str, output_file: &str, shx: f64, shy: f64, fillcolor: &str, interp: &str) -> Result<String, String> {
    let filter = format!("shear=shx={}:shy={}:fillcolor={}:interp={}", shx, shy, fillcolor, interp);
    let mut command = Command::new("ffmpeg");
    command.arg("-i").arg(input_file).arg("-vf").arg(filter).arg("-c:a").arg("copy").arg("-y").arg(output_file);
    execute_ffmpeg_command(command)
}

// ============================================================================
// BATCH 9 — Temporal Frame Effects
// ============================================================================

pub fn blend_frames(input_file: &str, output_file: &str, blend_mode: &str, opacity: f64) -> Result<String, String> {
    let filter = format!("tblend=all_mode={}:all_opacity={}", blend_mode, opacity);
    let mut command = Command::new("ffmpeg");
    command.arg("-i").arg(input_file).arg("-vf").arg(filter).arg("-c:a").arg("copy").arg("-y").arg(output_file);
    execute_ffmpeg_command(command)
}

pub fn temporal_median(input_file: &str, output_file: &str, radius: u32) -> Result<String, String> {
    let filter = format!("tmedian=radius={}", radius);
    let mut command = Command::new("ffmpeg");
    command.arg("-i").arg(input_file).arg("-vf").arg(filter).arg("-c:a").arg("copy").arg("-y").arg(output_file);
    execute_ffmpeg_command(command)
}

pub fn convert_framerate(input_file: &str, output_file: &str, target_fps: f64, round: &str) -> Result<String, String> {
    let filter = format!("fps=fps={}:round={}", target_fps, round);
    let mut command = Command::new("ffmpeg");
    command.arg("-i").arg(input_file).arg("-vf").arg(filter).arg("-c:a").arg("copy").arg("-y").arg(output_file);
    execute_ffmpeg_command(command)
}

pub fn tile_frames(input_file: &str, output_file: &str, columns: u32, rows: u32, frame_gap: u32) -> Result<String, String> {
    let filter = format!("tile={}x{}:padding={}:margin={}", columns, rows, frame_gap, frame_gap);
    let mut command = Command::new("ffmpeg");
    command.arg("-i").arg(input_file).arg("-vf").arg(filter).arg("-y").arg(output_file);
    execute_ffmpeg_command(command)
}

// ============================================================================
// PHASE D — Professional Finishing: Visual Tools
// ============================================================================

pub fn adjust_color_temperature(input_file: &str, output_file: &str, temperature: f64, mix: f64) -> Result<String, String> {
    let filter = format!("colortemperature=temperature={}:mix={}", temperature, mix);
    let mut command = Command::new("ffmpeg");
    command.arg("-i").arg(input_file).arg("-vf").arg(filter).arg("-c:a").arg("copy").arg("-y").arg(output_file);
    execute_ffmpeg_command(command)
}

pub fn adjust_vibrance(input_file: &str, output_file: &str, intensity: f64, rbal: f64, gbal: f64, bbal: f64) -> Result<String, String> {
    let filter = format!("vibrance=intensity={}:rbal={}:gbal={}:bbal={}", intensity, rbal, gbal, bbal);
    let mut command = Command::new("ffmpeg");
    command.arg("-i").arg(input_file).arg("-vf").arg(filter).arg("-c:a").arg("copy").arg("-y").arg(output_file);
    execute_ffmpeg_command(command)
}

pub fn remove_flicker(input_file: &str, output_file: &str, size: u32, mode: &str) -> Result<String, String> {
    let filter = format!("deflicker=size={}:mode={}", size, mode);
    let mut command = Command::new("ffmpeg");
    command.arg("-i").arg(input_file).arg("-vf").arg(filter).arg("-c:a").arg("copy").arg("-y").arg(output_file);
    execute_ffmpeg_command(command)
}

pub fn denoise_video_bm3d(input_file: &str, output_file: &str, sigma: f64, block_size: u32, mode: &str) -> Result<String, String> {
    let filter = format!("bm3d=sigma={}:block={}:mode={}", sigma, block_size, mode);
    let mut command = Command::new("ffmpeg");
    command.arg("-i").arg(input_file).arg("-vf").arg(filter).arg("-c:a").arg("copy").arg("-y").arg(output_file);
    execute_ffmpeg_command(command)
}

// ============================================================================
// PHASE E — Vectorscope, Waveform, Grid, LumaKey
// ============================================================================

pub fn analyze_vectorscope(input_file: &str, output_file: &str, mode: &str) -> Result<String, String> {
    let filter = format!("vectorscope=mode={}", mode);
    let mut command = Command::new("ffmpeg");
    command.arg("-i").arg(input_file).arg("-vf").arg(filter).arg("-frames:v").arg("1").arg("-y").arg(output_file);
    execute_ffmpeg_command(command)
}

pub fn analyze_waveform(input_file: &str, output_file: &str, mode: &str, filter_type: &str) -> Result<String, String> {
    let vf = format!("waveform=mode={}:filter={}", mode, filter_type);
    let mut command = Command::new("ffmpeg");
    command.arg("-i").arg(input_file).arg("-vf").arg(vf).arg("-frames:v").arg("1").arg("-y").arg(output_file);
    execute_ffmpeg_command(command)
}

pub fn draw_grid(input_file: &str, output_file: &str, width: u32, height: u32, thickness: u32, color: &str) -> Result<String, String> {
    let filter = format!("drawgrid=width={}:height={}:thickness={}:color={}", width, height, thickness, color);
    let mut command = Command::new("ffmpeg");
    command.arg("-i").arg(input_file).arg("-vf").arg(filter).arg("-c:a").arg("copy").arg("-y").arg(output_file);
    execute_ffmpeg_command(command)
}

pub fn grid_stack_videos(input_files: &[String], output_file: &str, layout: &str) -> Result<String, String> {
    let n = input_files.len();
    if n < 2 {
        return Err("grid_stack_videos requires at least 2 input files".to_string());
    }
    let auto_layout;
    let layout_str = if layout.is_empty() {
        auto_layout = match n {
            2 => "0_0|w0_0".to_string(),
            4 => "0_0|w0_0|0_h0|w0_h0".to_string(),
            _ => {
                let mut l = "0_0".to_string();
                for i in 1..n {
                    l.push_str(&format!("|w{}_0", i - 1));
                }
                l
            }
        };
        auto_layout.as_str()
    } else {
        layout
    };
    let mut fc = String::new();
    for i in 0..n {
        fc.push_str(&format!("[{}:v]", i));
    }
    fc.push_str(&format!("xstack=inputs={}:layout={}", n, layout_str));
    let mut command = Command::new("ffmpeg");
    for f in input_files {
        command.arg("-i").arg(f);
    }
    command.arg("-filter_complex").arg(fc).arg("-y").arg(output_file);
    execute_ffmpeg_command(command)
}

pub fn luma_key(input_file: &str, output_file: &str, threshold: f64, tolerance: f64, softness: f64) -> Result<String, String> {
    let filter = format!("lumakey=threshold={}:tolerance={}:softness={}", threshold, tolerance, softness);
    let mut command = Command::new("ffmpeg");
    command.arg("-i").arg(input_file).arg("-vf").arg(filter).arg("-c:a").arg("copy").arg("-y").arg(output_file);
    execute_ffmpeg_command(command)
}

// ============================================================================
// PHASE F — Niche/Specialised Visual Tools
// ============================================================================

pub fn displace_video(input_file: &str, xmap_file: &str, ymap_file: &str, output_file: &str, edge: &str) -> Result<String, String> {
    let filter = format!("[0:v][1:v][2:v]displace=edge={}", edge);
    let mut command = Command::new("ffmpeg");
    command.arg("-i").arg(input_file)
           .arg("-i").arg(xmap_file)
           .arg("-i").arg(ymap_file)
           .arg("-filter_complex").arg(filter)
           .arg("-c:a").arg("copy")
           .arg("-y").arg(output_file);
    execute_ffmpeg_command(command)
}

pub fn decimate_frames(input_file: &str, output_file: &str, cycle: u32, dupthresh: f64, scthresh: f64) -> Result<String, String> {
    let filter = format!("decimate=cycle={}:dupthresh={}:scthresh={}", cycle, dupthresh, scthresh);
    let mut command = Command::new("ffmpeg");
    command.arg("-i").arg(input_file).arg("-vf").arg(filter).arg("-c:a").arg("copy").arg("-y").arg(output_file);
    execute_ffmpeg_command(command)
}

pub fn denoise_video_owden(input_file: &str, output_file: &str, luma_strength: f64, chroma_strength: f64) -> Result<String, String> {
    let filter = format!("owdenoise=luma_strength={}:chroma_strength={}", luma_strength, chroma_strength);
    let mut command = Command::new("ffmpeg");
    command.arg("-i").arg(input_file).arg("-vf").arg(filter).arg("-c:a").arg("copy").arg("-y").arg(output_file);
    execute_ffmpeg_command(command)
}

pub fn despill_video(input_file: &str, output_file: &str, spill_type: &str, mix: f64, expand: f64) -> Result<String, String> {
    let filter = format!("despill=type={}:mix={}:expand={}", spill_type, mix, expand);
    let mut command = Command::new("ffmpeg");
    command.arg("-i").arg(input_file).arg("-vf").arg(filter).arg("-c:a").arg("copy").arg("-y").arg(output_file);
    execute_ffmpeg_command(command)
}

pub fn remap_pixels(input_file: &str, xmap_file: &str, ymap_file: &str, output_file: &str, fill: &str) -> Result<String, String> {
    let filter = format!("[0:v][1:v][2:v]remap=fill={}", fill);
    let mut command = Command::new("ffmpeg");
    command.arg("-i").arg(input_file)
           .arg("-i").arg(xmap_file)
           .arg("-i").arg(ymap_file)
           .arg("-filter_complex").arg(filter)
           .arg("-c:a").arg("copy")
           .arg("-y").arg(output_file);
    execute_ffmpeg_command(command)
}

pub fn adjust_exposure(input_file: &str, output_file: &str, exposure: f64, black: f64) -> Result<String, String> {
    let filter = format!("exposure=exposure={}:black={}", exposure, black);
    let mut command = Command::new("ffmpeg");
    command.arg("-i").arg(input_file).arg("-vf").arg(filter).arg("-c:a").arg("copy").arg("-y").arg(output_file);
    execute_ffmpeg_command(command)
}

pub fn measure_vmaf(distorted_file: &str, reference_file: &str, model_path: &str) -> Result<String, String> {
    let filter = if model_path.is_empty() {
        "libvmaf".to_string()
    } else {
        format!("libvmaf=model_path={}", model_path)
    };
    let output = std::process::Command::new("ffmpeg")
        .arg("-i").arg(distorted_file)
        .arg("-i").arg(reference_file)
        .arg("-lavfi").arg(filter)
        .arg("-f").arg("null").arg("-")
        .output()
        .map_err(|e| format!("Failed to execute FFmpeg: {}", e))?;
    let stderr = String::from_utf8_lossy(&output.stderr);
    let mut result = Vec::new();
    for line in stderr.lines() {
        if line.contains("VMAF") || line.contains("vmaf") || line.contains("score") {
            result.push(line.trim().to_string());
        }
    }
    if result.is_empty() {
        Err("Could not measure VMAF — ensure both files exist and libvmaf is compiled into your FFmpeg build".to_string())
    } else {
        Ok(result.join("\n"))
    }
}

// ============================================================================
// PHASE G — AI/ML Filters
// ============================================================================

pub fn detect_objects_dnn(input_file: &str, output_file: &str, model: &str, backend: &str, confidence: f64, labels: &str) -> Result<String, String> {
    let filter = if labels.is_empty() {
        format!("dnn_detect=dnn_backend={}:model={}:confidence={}", backend, model, confidence)
    } else {
        format!("dnn_detect=dnn_backend={}:model={}:confidence={}:labels={}", backend, model, confidence, labels)
    };
    let mut command = Command::new("ffmpeg");
    command.arg("-i").arg(input_file).arg("-vf").arg(filter).arg("-c:a").arg("copy").arg("-y").arg(output_file);
    execute_ffmpeg_command(command)
}

pub fn classify_frames_dnn(input_file: &str, output_file: &str, model: &str, backend: &str, labels: &str) -> Result<String, String> {
    let filter = if labels.is_empty() {
        format!("dnn_classify=dnn_backend={}:model={}", backend, model)
    } else {
        format!("dnn_classify=dnn_backend={}:model={}:labels={}", backend, model, labels)
    };
    let mut command = Command::new("ffmpeg");
    command.arg("-i").arg(input_file).arg("-vf").arg(filter).arg("-c:a").arg("copy").arg("-y").arg(output_file);
    execute_ffmpeg_command(command)
}

pub fn upscale_super_resolution(input_file: &str, output_file: &str, scale_factor: u32, model: &str, backend: &str) -> Result<String, String> {
    let filter = if model.is_empty() {
        format!("sr=dnn_backend={}:scale_factor={}", backend, scale_factor)
    } else {
        format!("sr=dnn_backend={}:scale_factor={}:model={}", backend, scale_factor, model)
    };
    let mut command = Command::new("ffmpeg");
    command.arg("-i").arg(input_file).arg("-vf").arg(filter).arg("-c:a").arg("copy").arg("-y").arg(output_file);
    execute_ffmpeg_command(command)
}

pub fn remove_rain_ai(input_file: &str, output_file: &str, model: &str, backend: &str) -> Result<String, String> {
    let filter = format!("derain=dnn_backend={}:model={}", backend, model);
    let mut command = Command::new("ffmpeg");
    command.arg("-i").arg(input_file).arg("-vf").arg(filter).arg("-c:a").arg("copy").arg("-y").arg(output_file);
    execute_ffmpeg_command(command)
}

pub fn detect_frozen_frames(input_file: &str, noise_db: f64, duration: f64) -> Result<String, String> {
    let filter = format!("freezedetect=n={}dB:d={}", noise_db, duration);
    let output = std::process::Command::new("ffmpeg")
        .arg("-i").arg(input_file)
        .arg("-vf").arg(filter)
        .arg("-f").arg("null").arg("-")
        .output()
        .map_err(|e| format!("Failed to execute FFmpeg: {}", e))?;
    let stderr = String::from_utf8_lossy(&output.stderr);
    let mut result = Vec::new();
    for line in stderr.lines() {
        if line.contains("freeze") || line.contains("Freeze") || line.contains("lavfi.freezedetect") {
            result.push(line.trim().to_string());
        }
    }
    if result.is_empty() {
        Ok("No frozen frames detected in the video.".to_string())
    } else {
        Ok(result.join("\n"))
    }
}

pub fn apply_edgedetect(input_file: &str, output_file: &str, low: f64, high: f64, mode: &str) -> Result<String, String> {
    let filter = format!("edgedetect=low={}:high={}:mode={}", low, high, mode);
    let mut command = Command::new("ffmpeg");
    command.arg("-i").arg(input_file).arg("-vf").arg(filter).arg("-c:a").arg("copy").arg("-y").arg(output_file);
    execute_ffmpeg_command(command)
}

// ================================================================
// PHASE I — Long-tail sweep, Batch 1 (video filters)
// ================================================================

/// Ken Burns zoom-and-pan effect using FFmpeg zoompan filter.
pub fn zoom_pan(input_file: &str, output_file: &str, zoom: f64, x_expr: &str, y_expr: &str, duration_frames: u32, fps: f64) -> Result<String, String> {
    let z_val = if zoom <= 1.0 { 1.5_f64 } else { zoom };
    let x = if x_expr.is_empty() { "iw/2-(iw/zoom/2)".to_string() } else { x_expr.to_string() };
    let y = if y_expr.is_empty() { "ih/2-(ih/zoom/2)".to_string() } else { y_expr.to_string() };
    let d = if duration_frames == 0 { 125 } else { duration_frames };
    let fps_val = if fps <= 0.0 { 25.0 } else { fps };
    let filter = format!("zoompan=z='{}':x='{}':y='{}':d={}:fps={}", z_val, x, y, d, fps_val);
    let mut command = Command::new("ffmpeg");
    command.arg("-i").arg(input_file).arg("-vf").arg(filter).arg("-c:a").arg("copy").arg("-y").arg(output_file);
    execute_ffmpeg_command(command)
}

/// Chromatic aberration via FFmpeg rgbashift filter — shifts R/G/B channels independently.
pub fn chromatic_aberration(input_file: &str, output_file: &str, rh: i32, rv: i32, bh: i32, bv: i32) -> Result<String, String> {
    let filter = format!("rgbashift=rh={}:rv={}:gh=0:gv=0:bh={}:bv={}", rh, rv, bh, bv);
    let mut command = Command::new("ffmpeg");
    command.arg("-i").arg(input_file).arg("-vf").arg(filter).arg("-c:a").arg("copy").arg("-y").arg(output_file);
    execute_ffmpeg_command(command)
}

/// Temporal frame blending for motion blur simulation using FFmpeg tblend filter.
pub fn temporal_blend(input_file: &str, output_file: &str, mode: &str, opacity: f64) -> Result<String, String> {
    let blend_mode = if mode.is_empty() { "average" } else { mode };
    let op = if opacity <= 0.0 || opacity > 1.0 { 1.0 } else { opacity };
    let filter = format!("tblend=all_mode={}:all_opacity={}", blend_mode, op);
    let mut command = Command::new("ffmpeg");
    command.arg("-i").arg(input_file).arg("-vf").arg(filter).arg("-c:a").arg("copy").arg("-y").arg(output_file);
    execute_ffmpeg_command(command)
}

/// Motion-compensated frame interpolation (slow-motion / frame-rate boost) via minterpolate.
pub fn motion_interpolate(input_file: &str, output_file: &str, target_fps: f64, mi_mode: &str) -> Result<String, String> {
    let fps_val = if target_fps <= 0.0 { 60.0 } else { target_fps };
    let mode = if mi_mode.is_empty() { "mci" } else { mi_mode };
    let filter = format!("minterpolate=fps={}:mi_mode={}:mc_mode=aobmc:me_mode=bidir:vsbmc=1", fps_val, mode);
    let mut command = Command::new("ffmpeg");
    command.arg("-i").arg(input_file).arg("-vf").arg(filter).arg("-c:a").arg("copy").arg("-y").arg(output_file);
    execute_ffmpeg_command(command)
}

/// Barrel/pincushion lens distortion correction via FFmpeg lenscorrection filter (simple k1/k2).
pub fn correct_lens_simple(input_file: &str, output_file: &str, k1: f64, k2: f64) -> Result<String, String> {
    let filter = format!("lenscorrection=cx=0.5:cy=0.5:k1={}:k2={}", k1, k2);
    let mut command = Command::new("ffmpeg");
    command.arg("-i").arg(input_file).arg("-vf").arg(filter).arg("-c:a").arg("copy").arg("-y").arg(output_file);
    execute_ffmpeg_command(command)
}

/// Deinterlace video using FFmpeg yadif filter with full mode/parity control.
pub fn deinterlace_yadif(input_file: &str, output_file: &str, mode: u32, parity: i32) -> Result<String, String> {
    let filter = format!("yadif=mode={}:parity={}:deint=0", mode, parity);
    let mut command = Command::new("ffmpeg");
    command.arg("-i").arg(input_file).arg("-vf").arg(filter).arg("-c:a").arg("copy").arg("-y").arg(output_file);
    execute_ffmpeg_command(command)
}

/// Perspective correction / keystone fix using FFmpeg perspective filter (linear interpolation).
pub fn correct_perspective_linear(input_file: &str, output_file: &str, x0: f64, y0: f64, x1: f64, y1: f64, x2: f64, y2: f64, x3: f64, y3: f64) -> Result<String, String> {
    let filter = format!(
        "perspective=x0={}:y0={}:x1={}:y1={}:x2={}:y2={}:x3={}:y3={}:interpolation=linear",
        x0, y0, x1, y1, x2, y2, x3, y3
    );
    let mut command = Command::new("ffmpeg");
    command.arg("-i").arg(input_file).arg("-vf").arg(filter).arg("-c:a").arg("copy").arg("-y").arg(output_file);
    execute_ffmpeg_command(command)
}

/// Colorize a grayscale (or desaturated) video using FFmpeg colorize filter.
pub fn colorize_video(input_file: &str, output_file: &str, hue: f64, saturation: f64, lightness: f64) -> Result<String, String> {
    let filter = format!("colorize=hue={}:saturation={}:lightness={}", hue, saturation, lightness);
    let mut command = Command::new("ffmpeg");
    command.arg("-i").arg(input_file).arg("-vf").arg(filter).arg("-c:a").arg("copy").arg("-y").arg(output_file);
    execute_ffmpeg_command(command)
}

/// High-quality 3D denoiser using FFmpeg hqdn3d filter. Fast and effective luma+chroma noise reduction.
pub fn denoise_hqdn3d(input_file: &str, output_file: &str, luma_spatial: f64, chroma_spatial: f64, luma_tmp: f64, chroma_tmp: f64) -> Result<String, String> {
    let ls = if luma_spatial <= 0.0 { 4.0 } else { luma_spatial };
    let cs = if chroma_spatial <= 0.0 { 3.0 } else { chroma_spatial };
    let lt = if luma_tmp <= 0.0 { 6.0 } else { luma_tmp };
    let ct = if chroma_tmp <= 0.0 { 4.5 } else { chroma_tmp };
    let filter = format!("hqdn3d={}:{}:{}:{}", ls, cs, lt, ct);
    let mut command = Command::new("ffmpeg");
    command.arg("-i").arg(input_file).arg("-vf").arg(filter).arg("-c:a").arg("copy").arg("-y").arg(output_file);
    execute_ffmpeg_command(command)
}

// ============================================================================
// PHASE I BATCH 2 — Long-tail: morphology, histogram, select, convolution, etc.
// ============================================================================

/// Selects specific frames using FFmpeg select+setpts. expr: FFmpeg boolean expression e.g. "eq(pict_type\\,PICT_TYPE_I)" for keyframes.
pub fn select_frames(input_file: &str, output_file: &str, expr: &str, fps: f64) -> Result<String, String> {
    let e = if expr.is_empty() { "eq(pict_type\\,PICT_TYPE_I)" } else { expr };
    let vf = format!("select='{}',setpts=N/FRAME_RATE/TB", e);
    let af = "aselect='1',asetpts=N/SR/TB";
    let mut command = Command::new("ffmpeg");
    command.arg("-i").arg(input_file).arg("-vf").arg(&vf).arg("-af").arg(af);
    if fps > 0.0 {
        command.arg("-r").arg(fps.to_string());
    }
    command.arg("-y").arg(output_file);
    execute_ffmpeg_command(command)
}

/// Posterizes video to N colour levels using FFmpeg posterize filter. Creates a graphic-novel look.
pub fn posterize_video(input_file: &str, output_file: &str, levels: u32) -> Result<String, String> {
    let l = if levels == 0 { 5 } else { levels.min(64) };
    let filter = format!("posterize=levels={}", l);
    let mut command = Command::new("ffmpeg");
    command.arg("-i").arg(input_file).arg("-vf").arg(filter).arg("-c:a").arg("copy").arg("-y").arg(output_file);
    execute_ffmpeg_command(command)
}

/// Solarize effect: inverts pixels above threshold using FFmpeg solarize filter.
pub fn solarize_video(input_file: &str, output_file: &str, threshold: u32) -> Result<String, String> {
    let t = if threshold == 0 { 128 } else { threshold.min(255) };
    let filter = format!("solarize=threshold={}", t);
    let mut command = Command::new("ffmpeg");
    command.arg("-i").arg(input_file).arg("-vf").arg(filter).arg("-c:a").arg("copy").arg("-y").arg(output_file);
    execute_ffmpeg_command(command)
}

/// Morphological dilation: expands bright regions using FFmpeg dilation filter.
pub fn apply_dilation(input_file: &str, output_file: &str, threshold0: u32, threshold1: u32, threshold2: u32, threshold3: u32, coordinates: u32) -> Result<String, String> {
    let coords = if coordinates == 0 { 255 } else { coordinates };
    let filter = format!("dilation=threshold0={}:threshold1={}:threshold2={}:threshold3={}:coordinates={}",
        threshold0, threshold1, threshold2, threshold3, coords);
    let mut command = Command::new("ffmpeg");
    command.arg("-i").arg(input_file).arg("-vf").arg(filter).arg("-c:a").arg("copy").arg("-y").arg(output_file);
    execute_ffmpeg_command(command)
}

/// Morphological erosion: shrinks bright regions using FFmpeg erosion filter.
pub fn apply_erosion(input_file: &str, output_file: &str, threshold0: u32, threshold1: u32, threshold2: u32, threshold3: u32, coordinates: u32) -> Result<String, String> {
    let coords = if coordinates == 0 { 255 } else { coordinates };
    let filter = format!("erosion=threshold0={}:threshold1={}:threshold2={}:threshold3={}:coordinates={}",
        threshold0, threshold1, threshold2, threshold3, coords);
    let mut command = Command::new("ffmpeg");
    command.arg("-i").arg(input_file).arg("-vf").arg(filter).arg("-c:a").arg("copy").arg("-y").arg(output_file);
    execute_ffmpeg_command(command)
}

/// Median noise filter using FFmpeg median filter. Removes salt-and-pepper noise well.
pub fn apply_median_filter(input_file: &str, output_file: &str, radius: u32, planes: u32) -> Result<String, String> {
    let r = if radius == 0 { 1 } else { radius.min(127) };
    let p = if planes == 0 { 15 } else { planes };
    let filter = format!("median=radius={}:planes={}", r, p);
    let mut command = Command::new("ffmpeg");
    command.arg("-i").arg(input_file).arg("-vf").arg(filter).arg("-c:a").arg("copy").arg("-y").arg(output_file);
    execute_ffmpeg_command(command)
}

/// Global histogram equalisation via FFmpeg histeq. Stretches contrast across the luminance range.
pub fn apply_histogram_eq(input_file: &str, output_file: &str, strength: f64, intensity: f64, antibanding: &str) -> Result<String, String> {
    let s = if strength <= 0.0 { 0.2 } else { strength.min(1.0) };
    let i = if intensity <= 0.0 { 0.21 } else { intensity.min(1.0) };
    let ab = if antibanding.is_empty() { "none" } else { antibanding };
    let filter = format!("histeq=strength={}:intensity={}:antibanding={}", s, i, ab);
    let mut command = Command::new("ffmpeg");
    command.arg("-i").arg(input_file).arg("-vf").arg(filter).arg("-c:a").arg("copy").arg("-y").arg(output_file);
    execute_ffmpeg_command(command)
}

/// CLAHE (Contrast-Limited Adaptive Histogram Equalisation) via FFmpeg clahe. Better local contrast than global histeq.
pub fn apply_clahe(input_file: &str, output_file: &str, clip_limit: f64, nb_tiles_x: u32, nb_tiles_y: u32) -> Result<String, String> {
    let cl = if clip_limit <= 0.0 { 25.0 } else { clip_limit };
    let tx = if nb_tiles_x == 0 { 8 } else { nb_tiles_x };
    let ty = if nb_tiles_y == 0 { 8 } else { nb_tiles_y };
    let filter = format!("clahe=clip_limit={}:nb_tiles_x={}:nb_tiles_y={}", cl, tx, ty);
    let mut command = Command::new("ffmpeg");
    command.arg("-i").arg(input_file).arg("-vf").arg(filter).arg("-c:a").arg("copy").arg("-y").arg(output_file);
    execute_ffmpeg_command(command)
}

/// Removes block/DCT artefacts from compressed video via FFmpeg deblock filter.
pub fn apply_deblock(input_file: &str, output_file: &str, filter_type: u32, block_size: u32, strength: f64, planes: u32) -> Result<String, String> {
    let ft = if filter_type == 0 { 4 } else { filter_type.min(4) };
    let bs = if block_size == 0 { 8 } else { block_size };
    let s = if strength <= 0.0 { 0.5 } else { strength.min(1.0) };
    let p = if planes == 0 { 15 } else { planes };
    let filter = format!("deblock=filter={}:block={}:alpha={}:beta={}:gamma={}:delta={}:planes={}", ft, bs, s, s, s, s, p);
    let mut command = Command::new("ffmpeg");
    command.arg("-i").arg(input_file).arg("-vf").arg(filter).arg("-c:a").arg("copy").arg("-y").arg(output_file);
    execute_ffmpeg_command(command)
}

/// Precise hue + saturation control via FFmpeg huesaturation filter (added FFmpeg 5.1).
pub fn adjust_hue_saturation(input_file: &str, output_file: &str, hue: f64, saturation: f64, intensity: f64, lightness: f64) -> Result<String, String> {
    let filter = format!("huesaturation=hue={}:saturation={}:intensity={}:lightness={}", hue, saturation, intensity, lightness);
    let mut command = Command::new("ffmpeg");
    command.arg("-i").arg(input_file).arg("-vf").arg(filter).arg("-c:a").arg("copy").arg("-y").arg(output_file);
    execute_ffmpeg_command(command)
}

// ============================================================================
// PHASE I BATCH 3 — Long-tail: blur variants, grain, rotation, geq, CCM, denoisers, LUT3D, SITI, amplify
// ============================================================================

/// Gaussian blur via FFmpeg gblur filter. Smooth, natural-looking blur.
pub fn apply_gaussian_blur(input_file: &str, output_file: &str, sigma: f64, steps: u32, planes: u32) -> Result<String, String> {
    let s = if sigma <= 0.0 { 3.0 } else { sigma };
    let st = if steps == 0 { 1 } else { steps };
    let p = if planes == 0 { 15 } else { planes };
    let filter = format!("gblur=sigma={}:steps={}:planes={}", s, st, p);
    let mut command = Command::new("ffmpeg");
    command.arg("-i").arg(input_file).arg("-vf").arg(filter).arg("-c:a").arg("copy").arg("-y").arg(output_file);
    execute_ffmpeg_command(command)
}

/// Box (average) blur via FFmpeg avgblur filter. Fast rectangular blur.
pub fn apply_box_blur(input_file: &str, output_file: &str, size_x: u32, size_y: u32, planes: u32) -> Result<String, String> {
    let sx = if size_x == 0 { 3 } else { size_x };
    let sy = if size_y == 0 { sx } else { size_y };
    let p = if planes == 0 { 15 } else { planes };
    let filter = format!("avgblur=sizeX={}:sizeY={}:planes={}", sx, sy, p);
    let mut command = Command::new("ffmpeg");
    command.arg("-i").arg(input_file).arg("-vf").arg(filter).arg("-c:a").arg("copy").arg("-y").arg(output_file);
    execute_ffmpeg_command(command)
}

/// Smart blur via FFmpeg smartblur. Blurs flat regions while preserving edges.
pub fn apply_smart_blur(input_file: &str, output_file: &str, luma_radius: f64, luma_strength: f64, luma_threshold: i32) -> Result<String, String> {
    let lr = if luma_radius <= 0.0 { 1.0 } else { luma_radius };
    let ls = if luma_strength.abs() < f64::EPSILON { -0.3 } else { luma_strength };
    let lt = luma_threshold;
    let filter = format!("smartblur=luma_radius={}:luma_strength={}:luma_threshold={}", lr, ls, lt);
    let mut command = Command::new("ffmpeg");
    command.arg("-i").arg(input_file).arg("-vf").arg(filter).arg("-c:a").arg("copy").arg("-y").arg(output_file);
    execute_ffmpeg_command(command)
}

/// Adds film grain/analog noise via FFmpeg noise filter. Simulates film texture.
pub fn add_film_grain(input_file: &str, output_file: &str, all_strength: u32, flags: &str) -> Result<String, String> {
    let s = if all_strength == 0 { 8 } else { all_strength.min(100) };
    let f = if flags.is_empty() { "a" } else { flags }; // a=additive, u=uniform, p=temporal
    let filter = format!("noise=all_seed=12345:all_strength={}:all_flags={}", s, f);
    let mut command = Command::new("ffmpeg");
    command.arg("-i").arg(input_file).arg("-vf").arg(filter).arg("-c:a").arg("copy").arg("-y").arg(output_file);
    execute_ffmpeg_command(command)
}

/// Applies a 3D LUT (.cube file) via FFmpeg lut3d filter. More precise than haldclut for colour grading.
pub fn apply_lut3d(input_file: &str, output_file: &str, lut_file: &str, interp: &str) -> Result<String, String> {
    let i = if interp.is_empty() { "tetrahedral" } else { interp };
    let filter = format!("lut3d=file='{}':interp={}", lut_file, i);
    let mut command = Command::new("ffmpeg");
    command.arg("-i").arg(input_file).arg("-vf").arg(filter).arg("-c:a").arg("copy").arg("-y").arg(output_file);
    execute_ffmpeg_command(command)
}

/// Per-pixel generic equation via FFmpeg geq filter. Full luma/chroma formula control using FFmpeg expressions.
pub fn apply_geq(input_file: &str, output_file: &str, lum_expr: &str, cb_expr: &str, cr_expr: &str) -> Result<String, String> {
    let l = if lum_expr.is_empty() { "lum(X,Y)" } else { lum_expr };
    let cb = if cb_expr.is_empty() { "cb(X,Y)" } else { cb_expr };
    let cr = if cr_expr.is_empty() { "cr(X,Y)" } else { cr_expr };
    let filter = format!("geq=lum='{}':cb='{}':cr='{}'", l, cb, cr);
    let mut command = Command::new("ffmpeg");
    command.arg("-i").arg(input_file).arg("-vf").arg(filter).arg("-c:a").arg("copy").arg("-y").arg(output_file);
    execute_ffmpeg_command(command)
}

/// Colour channel matrix mixer via FFmpeg colorchannelmixer. Enables cross-channel colour grading, channel swap, and precise greyscale conversion.
pub fn apply_colorchannelmixer(input_file: &str, output_file: &str, rr: f64, rg: f64, rb: f64, gr: f64, gg: f64, gb: f64, br: f64, bg: f64, bb: f64) -> Result<String, String> {
    let filter = format!("colorchannelmixer=rr={}:rg={}:rb={}:gr={}:gg={}:gb={}:br={}:bg={}:bb={}", rr, rg, rb, gr, gg, gb, br, bg, bb);
    let mut command = Command::new("ffmpeg");
    command.arg("-i").arg(input_file).arg("-vf").arg(filter).arg("-c:a").arg("copy").arg("-y").arg(output_file);
    execute_ffmpeg_command(command)
}

/// Adaptive temporal averaging denoiser via FFmpeg atadenoise. Excellent for consistent temporal noise.
pub fn apply_atadenoise(input_file: &str, output_file: &str, window_size: u32, threshold_a: f64, threshold_b: f64, planes: u32) -> Result<String, String> {
    let w = if window_size == 0 { 9 } else { (window_size | 1).min(129) };
    let ta = if threshold_a <= 0.0 { 0.02 } else { threshold_a };
    let tb = if threshold_b <= 0.0 { 0.04 } else { threshold_b };
    let p = if planes == 0 { 7 } else { planes };
    let filter = format!("atadenoise=0a={}:0b={}:1a={}:1b={}:2a={}:2b={}:s={}:p={}", ta, tb, ta, tb, ta, tb, w, p);
    let mut command = Command::new("ffmpeg");
    command.arg("-i").arg(input_file).arg("-vf").arg(filter).arg("-c:a").arg("copy").arg("-y").arg(output_file);
    execute_ffmpeg_command(command)
}

/// Wavelet-based denoiser via FFmpeg vaguedenoiser. Good at preserving fine detail.
pub fn apply_vaguedenoiser(input_file: &str, output_file: &str, threshold: f64, method: u32, nsteps: u32, percent: f64, planes: u32) -> Result<String, String> {
    let t = if threshold <= 0.0 { 2.0 } else { threshold };
    let m = method.min(2); // 0=soft, 1=hard, 2=garrote
    let n = if nsteps == 0 { 6 } else { nsteps };
    let pc = if percent <= 0.0 { 85.0 } else { percent };
    let p = if planes == 0 { 7 } else { planes };
    let filter = format!("vaguedenoiser=threshold={}:method={}:nsteps={}:percent={}:planes={}", t, m, n, pc, p);
    let mut command = Command::new("ffmpeg");
    command.arg("-i").arg(input_file).arg("-vf").arg(filter).arg("-c:a").arg("copy").arg("-y").arg(output_file);
    execute_ffmpeg_command(command)
}

/// FFT-based video denoiser via FFmpeg fftdnoiz. Excellent for uniform additive noise.
pub fn apply_fftdnoiz(input_file: &str, output_file: &str, sigma: f64, amount: f64, block_size: u32, overlap: f64, planes: u32) -> Result<String, String> {
    let s = if sigma <= 0.0 { 1.0 } else { sigma };
    let a = if amount <= 0.0 { 0.96 } else { amount };
    let bs = if block_size == 0 { 32 } else { block_size };
    let ov = if overlap <= 0.0 { 0.5 } else { overlap };
    let p = if planes == 0 { 7 } else { planes };
    let filter = format!("fftdnoiz=sigma={}:amount={}:block={}:overlap={}:planes={}", s, a, bs, ov, p);
    let mut command = Command::new("ffmpeg");
    command.arg("-i").arg(input_file).arg("-vf").arg(filter).arg("-c:a").arg("copy").arg("-y").arg(output_file);
    execute_ffmpeg_command(command)
}

/// Measures Spatial Information (SI) and Temporal Information (TI) via FFmpeg siti filter. Industry standard for video complexity analysis and codec preset selection.
pub fn measure_siti(input_file: &str) -> Result<String, String> {
    let output = std::process::Command::new("ffmpeg")
        .arg("-i").arg(input_file)
        .arg("-vf").arg("siti=print_summary=1")
        .arg("-f").arg("null").arg("-")
        .output()
        .map_err(|e| format!("Failed to execute FFmpeg: {}", e))?;
    let stderr = String::from_utf8_lossy(&output.stderr);
    let mut lines = Vec::new();
    for line in stderr.lines() {
        if line.contains("SI") || line.contains("TI") || line.contains("siti") || line.contains("mean") {
            lines.push(line.trim().to_string());
        }
    }
    if lines.is_empty() {
        Err("Could not measure SI/TI — ensure file has a video stream".to_string())
    } else {
        Ok(lines.join("\n"))
    }
}

/// Amplifies pixel differences between consecutive frames via FFmpeg amplify filter. Makes subtle motion visible; creates dramatic temporal effects.
pub fn apply_amplify(input_file: &str, output_file: &str, radius: u32, factor: f64, threshold: f64, planes: u32) -> Result<String, String> {
    let r = if radius == 0 { 2 } else { radius };
    let f = if factor <= 0.0 { 2.0 } else { factor };
    let t = if threshold < 0.0 { 10.0 } else { threshold };
    let p = if planes == 0 { 7 } else { planes };
    let filter = format!("amplify=radius={}:factor={}:threshold={}:planes={}", r, f, t, p);
    let mut command = Command::new("ffmpeg");
    command.arg("-i").arg(input_file).arg("-vf").arg(filter).arg("-c:a").arg("copy").arg("-y").arg(output_file);
    execute_ffmpeg_command(command)
}

// ============================================================================
// PHASE I BATCH 4 — negate, pixelize, colorlevels, pseudocolor, colorhold,
//                   shuffleplanes, blackdetect, idet, vstack, hstack, setdar,
//                   stereo3d, telecine, pullup, thumbnail_select
// ============================================================================

/// Inverts video colours (negative) via FFmpeg negate filter.
pub fn apply_negate(input_file: &str, output_file: &str, components: u32) -> Result<String, String> {
    let c = if components == 0 { 7 } else { components }; // 1=R,2=G,4=B,8=A; 7=RGB
    let filter = format!("negate=components={}", c);
    let mut command = Command::new("ffmpeg");
    command.arg("-i").arg(input_file).arg("-vf").arg(filter).arg("-c:a").arg("copy").arg("-y").arg(output_file);
    execute_ffmpeg_command(command)
}

/// Pixelates video (mosaic effect) via FFmpeg pixelize filter.
pub fn apply_pixelize(input_file: &str, output_file: &str, width: u32, height: u32, mode: u32) -> Result<String, String> {
    let w = if width == 0 { 16 } else { width };
    let h = if height == 0 { w } else { height };
    let m = mode.min(1); // 0=avg (default), 1=blocks
    let filter = format!("pixelize=width={}:height={}:mode={}", w, h, m);
    let mut command = Command::new("ffmpeg");
    command.arg("-i").arg(input_file).arg("-vf").arg(filter).arg("-c:a").arg("copy").arg("-y").arg(output_file);
    execute_ffmpeg_command(command)
}

/// Clips and remaps input/output colour levels per channel via FFmpeg colorlevels. Like Levels in Photoshop.
pub fn apply_colorlevels(input_file: &str, output_file: &str, rimin: f64, rimax: f64, gimin: f64, gimax: f64, bimin: f64, bimax: f64, romin: f64, romax: f64) -> Result<String, String> {
    let filter = format!(
        "colorlevels=rimin={}:rimax={}:gimin={}:gimax={}:bimin={}:bimax={}:romin={}:romax={}:gomin={}:gomax={}:bomin={}:bomax={}",
        rimin, rimax, gimin, gimax, bimin, bimax, romin, romax, romin, romax, romin, romax
    );
    let mut command = Command::new("ffmpeg");
    command.arg("-i").arg(input_file).arg("-vf").arg(filter).arg("-c:a").arg("copy").arg("-y").arg(output_file);
    execute_ffmpeg_command(command)
}

/// False-colour scientific visualisation via FFmpeg pseudocolor filter. Maps luminance to a colour palette.
pub fn apply_pseudocolor(input_file: &str, output_file: &str, preset: i32, opacity: f64) -> Result<String, String> {
    // preset: -1=custom, 0=magma, 1=inferno, 2=plasma, 3=viridis, 4=turbo, etc.
    let p = if preset < 0 { 0 } else { preset };
    let op = if opacity <= 0.0 { 1.0 } else { opacity.min(1.0) };
    let filter = format!("pseudocolor=preset={}:opacity={}", p, op);
    let mut command = Command::new("ffmpeg");
    command.arg("-i").arg(input_file).arg("-vf").arg(filter).arg("-c:a").arg("copy").arg("-y").arg(output_file);
    execute_ffmpeg_command(command)
}

/// Keeps a specific colour and desaturates the rest via FFmpeg colorhold filter. Creates selective-colour effects.
pub fn apply_colorhold(input_file: &str, output_file: &str, color: &str, similarity: f64, blend: f64) -> Result<String, String> {
    let c = if color.is_empty() { "red" } else { color };
    let s = if similarity <= 0.0 { 0.1 } else { similarity.min(1.0) };
    let b = blend.min(1.0).max(0.0);
    let filter = format!("colorhold=color={}:similarity={}:blend={}", c, s, b);
    let mut command = Command::new("ffmpeg");
    command.arg("-i").arg(input_file).arg("-vf").arg(filter).arg("-c:a").arg("copy").arg("-y").arg(output_file);
    execute_ffmpeg_command(command)
}

/// Reorders or duplicates video planes via FFmpeg shuffleplanes filter. Enables channel-swap creative effects.
pub fn apply_shuffleplanes(input_file: &str, output_file: &str, map0: u32, map1: u32, map2: u32, map3: u32) -> Result<String, String> {
    let filter = format!("shuffleplanes=map0={}:map1={}:map2={}:map3={}", map0, map1, map2, map3);
    let mut command = Command::new("ffmpeg");
    command.arg("-i").arg(input_file).arg("-vf").arg(filter).arg("-c:a").arg("copy").arg("-y").arg(output_file);
    execute_ffmpeg_command(command)
}

/// Detects black/near-black frames via FFmpeg blackdetect filter. Returns timestamps of black segments.
pub fn detect_black_frames(input_file: &str, black_min_duration: f64, picture_black_ratio_th: f64, pixel_black_th: f64) -> Result<String, String> {
    let d = if black_min_duration <= 0.0 { 2.0 } else { black_min_duration };
    let pbt = if pixel_black_th <= 0.0 { 0.10 } else { pixel_black_th };
    let pbr = if picture_black_ratio_th <= 0.0 { 0.98 } else { picture_black_ratio_th };
    let filter = format!("blackdetect=d={}:pic_th={}:pix_th={}", d, pbr, pbt);
    let output = std::process::Command::new("ffmpeg")
        .arg("-i").arg(input_file)
        .arg("-vf").arg(filter)
        .arg("-f").arg("null").arg("-")
        .output()
        .map_err(|e| format!("Failed to execute FFmpeg: {}", e))?;
    let stderr = String::from_utf8_lossy(&output.stderr);
    let mut lines = Vec::new();
    for line in stderr.lines() {
        if line.contains("black_start") || line.contains("black_end") || line.contains("black_duration") {
            lines.push(line.trim().to_string());
        }
    }
    if lines.is_empty() {
        Ok("No black frames detected".to_string())
    } else {
        Ok(lines.join("\n"))
    }
}

/// Detects interlacing type via FFmpeg idet filter. Identifies progressive, top-field-first, or bottom-field-first content.
pub fn detect_interlace_type(input_file: &str) -> Result<String, String> {
    let output = std::process::Command::new("ffmpeg")
        .arg("-i").arg(input_file)
        .arg("-vf").arg("idet")
        .arg("-f").arg("null").arg("-")
        .output()
        .map_err(|e| format!("Failed to execute FFmpeg: {}", e))?;
    let stderr = String::from_utf8_lossy(&output.stderr);
    let mut lines = Vec::new();
    for line in stderr.lines() {
        if line.contains("idet") || line.contains("TFF") || line.contains("BFF") || line.contains("Progressive") || line.contains("Undetermined") {
            lines.push(line.trim().to_string());
        }
    }
    if lines.is_empty() {
        Ok("Could not determine interlace type — ensure file has a video stream".to_string())
    } else {
        Ok(lines.join("\n"))
    }
}

/// Stacks two videos vertically via FFmpeg vstack filter.
pub fn apply_vstack(input_file: &str, secondary_file: &str, output_file: &str, shortest: bool) -> Result<String, String> {
    let s = if shortest { 1 } else { 0 };
    let filter = format!("[0:v][1:v]vstack=inputs=2:shortest={}", s);
    let mut command = Command::new("ffmpeg");
    command.arg("-i").arg(input_file).arg("-i").arg(secondary_file)
        .arg("-filter_complex").arg(filter)
        .arg("-c:a").arg("copy").arg("-y").arg(output_file);
    execute_ffmpeg_command(command)
}

/// Stacks two videos horizontally via FFmpeg hstack filter.
pub fn apply_hstack(input_file: &str, secondary_file: &str, output_file: &str, shortest: bool) -> Result<String, String> {
    let s = if shortest { 1 } else { 0 };
    let filter = format!("[0:v][1:v]hstack=inputs=2:shortest={}", s);
    let mut command = Command::new("ffmpeg");
    command.arg("-i").arg(input_file).arg("-i").arg(secondary_file)
        .arg("-filter_complex").arg(filter)
        .arg("-c:a").arg("copy").arg("-y").arg(output_file);
    execute_ffmpeg_command(command)
}

/// Sets display aspect ratio via FFmpeg setdar filter without re-encoding pixels.
pub fn apply_setdar(input_file: &str, output_file: &str, dar: &str) -> Result<String, String> {
    let ratio = if dar.is_empty() { "16/9" } else { dar };
    let filter = format!("setdar=ratio={}", ratio);
    let mut command = Command::new("ffmpeg");
    command.arg("-i").arg(input_file).arg("-vf").arg(filter).arg("-c:a").arg("copy").arg("-y").arg(output_file);
    execute_ffmpeg_command(command)
}

/// Converts between stereoscopic 3D video formats via FFmpeg stereo3d filter. Supports SBS, over-under, anaglyph, and more.
pub fn apply_stereo3d(input_file: &str, output_file: &str, input_format: &str, output_format: &str) -> Result<String, String> {
    // Formats: sbsl/sbsr=side-by-side left/right, abl/abr=above-below, arcd=red-cyan anaglyph, ml=mono left, mr=mono right
    let inf = if input_format.is_empty() { "sbsl" } else { input_format };
    let outf = if output_format.is_empty() { "arcd" } else { output_format };
    let filter = format!("stereo3d={}:{}", inf, outf);
    let mut command = Command::new("ffmpeg");
    command.arg("-i").arg(input_file).arg("-vf").arg(filter).arg("-c:a").arg("copy").arg("-y").arg(output_file);
    execute_ffmpeg_command(command)
}

/// Applies 3:2 pulldown telecine (film-to-video) via FFmpeg telecine filter. Converts 24fps film to 29.97fps broadcast.
pub fn apply_telecine(input_file: &str, output_file: &str, pattern: &str, first_field: u32) -> Result<String, String> {
    let p = if pattern.is_empty() { "23" } else { pattern }; // "23" = 3:2 pulldown
    let ff = first_field.min(1); // 0=top, 1=bottom
    let filter = format!("telecine=pattern={}:first_field={}", p, ff);
    let mut command = Command::new("ffmpeg");
    command.arg("-i").arg(input_file).arg("-vf").arg(filter).arg("-c:a").arg("copy").arg("-y").arg(output_file);
    execute_ffmpeg_command(command)
}

/// Removes 3:2 pulldown (inverse telecine) via FFmpeg pullup filter. Recovers 24fps from 29.97fps telecined content.
pub fn apply_pullup(input_file: &str, output_file: &str) -> Result<String, String> {
    let mut command = Command::new("ffmpeg");
    command.arg("-i").arg(input_file)
        .arg("-vf").arg("pullup,fps=24000/1001")
        .arg("-c:a").arg("copy").arg("-y").arg(output_file);
    execute_ffmpeg_command(command)
}

/// Selects the best representative thumbnail frame from a video using FFmpeg thumbnail filter.
pub fn select_thumbnail_frame(input_file: &str, output_file: &str, n: u32) -> Result<String, String> {
    let frames = if n == 0 { 100 } else { n };
    // thumbnail selects the best frame from every N-frame batch; output as single image or short clip
    let filter = format!("thumbnail=n={}", frames);
    let mut command = Command::new("ffmpeg");
    command.arg("-i").arg(input_file).arg("-vf").arg(filter).arg("-frames:v").arg("1").arg("-y").arg(output_file);
    execute_ffmpeg_command(command)
}

/// Applies a pixel-value threshold — pixels below low become 0, above high become max.
pub fn apply_threshold(input_file: &str, output_file: &str, planes: u32) -> Result<String, String> {
    let p = if planes == 0 { 15 } else { planes };
    let filter = format!("threshold=planes={}", p);
    let mut command = Command::new("ffmpeg");
    command.arg("-i").arg(input_file).arg("-vf").arg(filter).arg("-c:a").arg("copy").arg("-y").arg(output_file);
    execute_ffmpeg_command(command)
}

/// Clamps pixel values between dark and bright reference streams using maskedclamp.
pub fn apply_maskedclamp(input_file: &str, output_file: &str, undershoot: u32, overshoot: u32, planes: u32) -> Result<String, String> {
    let p = if planes == 0 { 15 } else { planes };
    let filter = format!("maskedclamp=undershoot={}:overshoot={}:planes={}", undershoot, overshoot, p);
    let mut command = Command::new("ffmpeg");
    command.arg("-i").arg(input_file).arg("-vf").arg(filter).arg("-c:a").arg("copy").arg("-y").arg(output_file);
    execute_ffmpeg_command(command)
}

/// Applies Roberts cross edge detection operator.
pub fn apply_roberts(input_file: &str, output_file: &str, planes: u32, scale: f64, delta: f64) -> Result<String, String> {
    let p = if planes == 0 { 15 } else { planes };
    let filter = format!("roberts=planes={}:scale={}:delta={}", p, scale, delta);
    let mut command = Command::new("ffmpeg");
    command.arg("-i").arg(input_file).arg("-vf").arg(filter).arg("-c:a").arg("copy").arg("-y").arg(output_file);
    execute_ffmpeg_command(command)
}

/// Applies Sobel edge detection operator.
pub fn apply_sobel(input_file: &str, output_file: &str, planes: u32, scale: f64, delta: f64) -> Result<String, String> {
    let p = if planes == 0 { 15 } else { planes };
    let filter = format!("sobel=planes={}:scale={}:delta={}", p, scale, delta);
    let mut command = Command::new("ffmpeg");
    command.arg("-i").arg(input_file).arg("-vf").arg(filter).arg("-c:a").arg("copy").arg("-y").arg(output_file);
    execute_ffmpeg_command(command)
}

/// Applies Prewitt edge detection operator.
pub fn apply_prewitt(input_file: &str, output_file: &str, planes: u32, scale: f64, delta: f64) -> Result<String, String> {
    let p = if planes == 0 { 15 } else { planes };
    let filter = format!("prewitt=planes={}:scale={}:delta={}", p, scale, delta);
    let mut command = Command::new("ffmpeg");
    command.arg("-i").arg(input_file).arg("-vf").arg(filter).arg("-c:a").arg("copy").arg("-y").arg(output_file);
    execute_ffmpeg_command(command)
}

/// Applies Kirsch edge detection operator.
pub fn apply_kirsch(input_file: &str, output_file: &str, planes: u32, scale: f64, delta: f64) -> Result<String, String> {
    let p = if planes == 0 { 15 } else { planes };
    let filter = format!("kirsch=planes={}:scale={}:delta={}", p, scale, delta);
    let mut command = Command::new("ffmpeg");
    command.arg("-i").arg(input_file).arg("-vf").arg(filter).arg("-c:a").arg("copy").arg("-y").arg(output_file);
    execute_ffmpeg_command(command)
}

/// Limits video signal to a specified range, clamping pixel values (video limiter, not audio).
pub fn apply_video_limiter(input_file: &str, output_file: &str, min: u32, max: u32, planes: u32) -> Result<String, String> {
    let p = if planes == 0 { 15 } else { planes };
    let filter = format!("limiter=min={}:max={}:planes={}", min, max, p);
    let mut command = Command::new("ffmpeg");
    command.arg("-i").arg(input_file).arg("-vf").arg(filter).arg("-c:a").arg("copy").arg("-y").arg(output_file);
    execute_ffmpeg_command(command)
}

/// Applies bilateral filter — edge-preserving noise reduction.
pub fn apply_bilateral(input_file: &str, output_file: &str, sigmaS: f64, sigmaR: f64, planes: u32) -> Result<String, String> {
    let ss = if sigmaS <= 0.0 { 0.1 } else { sigmaS };
    let sr = if sigmaR <= 0.0 { 0.1 } else { sigmaR };
    let p = if planes == 0 { 1 } else { planes };
    let filter = format!("bilateral=sigmaS={}:sigmaR={}:planes={}", ss, sr, p);
    let mut command = Command::new("ffmpeg");
    command.arg("-i").arg(input_file).arg("-vf").arg(filter).arg("-c:a").arg("copy").arg("-y").arg(output_file);
    execute_ffmpeg_command(command)
}

/// Applies unsharp mask for sharpening or blurring luma/chroma independently.
pub fn apply_unsharp_mask(input_file: &str, output_file: &str, lx: u32, ly: u32, la: f64, cx: u32, cy: u32, ca: f64) -> Result<String, String> {
    let lmx = if lx == 0 { 5 } else { lx };
    let lmy = if ly == 0 { 5 } else { ly };
    let cmx = if cx == 0 { 5 } else { cx };
    let cmy = if cy == 0 { 5 } else { cy };
    let filter = format!("unsharp=lx={}:ly={}:la={}:cx={}:cy={}:ca={}", lmx, lmy, la, cmx, cmy, ca);
    let mut command = Command::new("ffmpeg");
    command.arg("-i").arg(input_file).arg("-vf").arg(filter).arg("-c:a").arg("copy").arg("-y").arg(output_file);
    execute_ffmpeg_command(command)
}

/// Applies lagfun — slow continuous EMA (exponential moving average) for motion trails/ghost effect.
pub fn apply_lagfun(input_file: &str, output_file: &str, decay: f64, planes: u32) -> Result<String, String> {
    let d = if decay <= 0.0 || decay > 1.0 { 0.95 } else { decay };
    let p = if planes == 0 { 15 } else { planes };
    let filter = format!("lagfun=decay={}:planes={}", d, p);
    let mut command = Command::new("ffmpeg");
    command.arg("-i").arg(input_file).arg("-vf").arg(filter).arg("-c:a").arg("copy").arg("-y").arg(output_file);
    execute_ffmpeg_command(command)
}

/// Applies tinterlace — temporal field interlacing modes for broadcast output.
pub fn apply_tinterlace(input_file: &str, output_file: &str, mode: u32, flags: &str) -> Result<String, String> {
    let f = if flags.is_empty() { "vlpf" } else { flags };
    let filter = format!("tinterlace=mode={}:flags={}", mode, f);
    let mut command = Command::new("ffmpeg");
    command.arg("-i").arg(input_file).arg("-vf").arg(filter).arg("-c:a").arg("copy").arg("-y").arg(output_file);
    execute_ffmpeg_command(command)
}

/// Renders a datascope (pixel data value visualizer) as an overlay/standalone output.
pub fn apply_datascope(input_file: &str, output_file: &str, size: &str, x: u32, y: u32, mode: u32, opacity: f64, axis: bool) -> Result<String, String> {
    let s = if size.is_empty() { "hd720" } else { size };
    let filter = format!("datascope=size={}:x={}:y={}:mode={}:opacity={}:axis={}", s, x, y, mode, opacity, if axis { 1 } else { 0 });
    let mut command = Command::new("ffmpeg");
    command.arg("-i").arg(input_file).arg("-vf").arg(filter).arg("-c:a").arg("copy").arg("-y").arg(output_file);
    execute_ffmpeg_command(command)
}

/// Applies Fast Super Pixel (fspp) frequency-domain denoising/smoothing.
pub fn apply_fspp(input_file: &str, output_file: &str, quality: u32, strength: f64, use_bframe_qp: bool) -> Result<String, String> {
    let q = if quality == 0 { 4 } else { quality.min(5) };
    let filter = format!("fspp=quality={}:strength={}:use_bframe_qp={}", q, strength, if use_bframe_qp { 1 } else { 0 });
    let mut command = Command::new("ffmpeg");
    command.arg("-i").arg(input_file).arg("-vf").arg(filter).arg("-c:a").arg("copy").arg("-y").arg(output_file);
    execute_ffmpeg_command(command)
}

/// Converts video between colour matrix standards (BT.601, BT.709, etc.) using colormatrix filter.
pub fn apply_colormatrix(input_file: &str, output_file: &str, src: &str, dst: &str) -> Result<String, String> {
    let s = if src.is_empty() { "bt601" } else { src };
    let d = if dst.is_empty() { "bt709" } else { dst };
    let filter = format!("colormatrix={}:{}", s, d);
    let mut command = Command::new("ffmpeg");
    command.arg("-i").arg(input_file).arg("-vf").arg(filter).arg("-c:a").arg("copy").arg("-y").arg(output_file);
    execute_ffmpeg_command(command)
}

/// Shifts chroma (colour) channels horizontally and vertically for chromatic aberration or colour-fringing effects.
pub fn apply_chromashift(input_file: &str, output_file: &str, cbh: i32, cbv: i32, crh: i32, crv: i32) -> Result<String, String> {
    let filter = format!("chromashift=cbh={}:cbv={}:crh={}:crv={}", cbh, cbv, crh, crv);
    let mut command = Command::new("ffmpeg");
    command.arg("-i").arg(input_file).arg("-vf").arg(filter).arg("-c:a").arg("copy").arg("-y").arg(output_file);
    execute_ffmpeg_command(command)
}

/// Applies Contrast Adaptive Sharpening (CAS) — AMD FidelityFX-style sharpening that boosts local contrast.
pub fn apply_cas(input_file: &str, output_file: &str, strength: f64, planes: u32) -> Result<String, String> {
    let p = if planes == 0 { 7 } else { planes };
    let filter = format!("cas=strength={}:planes={}", strength, p);
    let mut command = Command::new("ffmpeg");
    command.arg("-i").arg(input_file).arg("-vf").arg(filter).arg("-c:a").arg("copy").arg("-y").arg(output_file);
    execute_ffmpeg_command(command)
}

/// Applies Non-Local Means denoising (nlmeans) for high-quality video noise reduction.
pub fn apply_nlmeans_video(input_file: &str, output_file: &str, s: f64, p: u32, pc: u32, r: u32, rc: u32) -> Result<String, String> {
    let strength = if s <= 0.0 { 1.0 } else { s };
    let patch = if p == 0 { 3 } else { p };
    let patch_c = if pc == 0 { patch } else { pc };
    let research = if r == 0 { 7 } else { r };
    let research_c = if rc == 0 { research } else { rc };
    let filter = format!("nlmeans=s={}:p={}:pc={}:r={}:rc={}", strength, patch, patch_c, research, research_c);
    let mut command = Command::new("ffmpeg");
    command.arg("-i").arg(input_file).arg("-vf").arg(filter).arg("-c:a").arg("copy").arg("-y").arg(output_file);
    execute_ffmpeg_command(command)
}

/// Applies Simple Post-Processing (spp) — block-DCT-based denoising/deblocking.
pub fn apply_spp(input_file: &str, output_file: &str, quality: u32, qp: i32, mode: &str) -> Result<String, String> {
    let q = if quality == 0 { 3 } else { quality.min(6) };
    let m = if mode.is_empty() { "hard" } else { mode }; // hard or soft
    let filter = if qp == 0 {
        format!("spp=quality={}:mode={}", q, m)
    } else {
        format!("spp=quality={}:qp={}:mode={}", q, qp, m)
    };
    let mut command = Command::new("ffmpeg");
    command.arg("-i").arg(input_file).arg("-vf").arg(filter).arg("-c:a").arg("copy").arg("-y").arg(output_file);
    execute_ffmpeg_command(command)
}

/// Applies FFmpeg pp (postprocess) filter — collection of deblocking, deringing, and denoise subfilters.
pub fn apply_pp(input_file: &str, output_file: &str, subfilters: &str) -> Result<String, String> {
    let sf = if subfilters.is_empty() { "default" } else { subfilters };
    // e.g. "hb/vb/dr" for horiz/vert deblock + deringing
    let filter = format!("pp={}", sf);
    let mut command = Command::new("ffmpeg");
    command.arg("-i").arg(input_file).arg("-vf").arg(filter).arg("-c:a").arg("copy").arg("-y").arg(output_file);
    execute_ffmpeg_command(command)
}

/// Estimates motion vectors and renders them as an overlay for motion analysis.
pub fn apply_mestimate(input_file: &str, output_file: &str, method: &str, mb_size: u32, search_param: u32) -> Result<String, String> {
    let m = if method.is_empty() { "esa" } else { method }; // esa, tss, tdls, ntss, fss, ds, hexbs, epzs, umh
    let mb = if mb_size == 0 { 16 } else { mb_size };
    let sp = if search_param == 0 { 7 } else { search_param };
    let filter = format!("mestimate=method={}:mb_size={}:search_param={}", m, mb, sp);
    let mut command = Command::new("ffmpeg");
    command.arg("-i").arg(input_file).arg("-vf").arg(filter).arg("-c:a").arg("copy").arg("-y").arg(output_file);
    execute_ffmpeg_command(command)
}

/// Applies mid-equalizer between two video streams to match their midtone exposure.
pub fn apply_midequalizer(input_file: &str, secondary_file: &str, output_file: &str, planes: u32) -> Result<String, String> {
    let p = if planes == 0 { 15 } else { planes };
    let filter = format!("[0:v][1:v]midequalizer=planes={}[vout]", p);
    let mut command = Command::new("ffmpeg");
    command.arg("-i").arg(input_file).arg("-i").arg(secondary_file)
        .arg("-filter_complex").arg(filter)
        .arg("-map").arg("[vout]")
        .arg("-c:a").arg("copy").arg("-y").arg(output_file);
    execute_ffmpeg_command(command)
}

/// Applies a spatio-temporal median filter across multiple frames to remove outliers/noise.
pub fn apply_median_spatial(input_file: &str, output_file: &str, radius: u32, radiusV: u32, percentile: f64, planes: u32) -> Result<String, String> {
    let r = if radius == 0 { 1 } else { radius };
    let rv = if radiusV == 0 { r } else { radiusV };
    let p = if planes == 0 { 15 } else { planes };
    let filter = format!("median=radius={}:radiusV={}:percentile={}:planes={}", r, rv, percentile, p);
    let mut command = Command::new("ffmpeg");
    command.arg("-i").arg(input_file).arg("-vf").arg(filter).arg("-c:a").arg("copy").arg("-y").arg(output_file);
    execute_ffmpeg_command(command)
}

/// Applies a custom convolution kernel via FFmpeg convolution filter. Same matrix applied to all planes.
pub fn apply_xfade_transition(input_file: &str, secondary_file: &str, output_file: &str, transition: &str, duration: f64, offset: f64) -> Result<String, String> {
    let t = if transition.is_empty() { "fade" } else { transition };
    let filter = format!("[0:v][1:v]xfade=transition={}:duration={}:offset={}[vout]", t, duration, offset);
    let mut command = Command::new("ffmpeg");
    command.arg("-i").arg(input_file)
        .arg("-i").arg(secondary_file)
        .arg("-filter_complex").arg(&filter)
        .arg("-map").arg("[vout]")
        .arg("-y").arg(output_file);
    execute_ffmpeg_command(command)
}

pub fn apply_color_key(input_file: &str, output_file: &str, color: &str, similarity: f64, blend: f64) -> Result<String, String> {
    let c = if color.is_empty() { "0x00FF00" } else { color };
    let filter = format!("colorkey=color={}:similarity={}:blend={}", c, similarity, blend);
    let mut command = Command::new("ffmpeg");
    command.arg("-i").arg(input_file).arg("-vf").arg(filter).arg("-c:a").arg("copy").arg("-y").arg(output_file);
    execute_ffmpeg_command(command)
}

pub fn apply_monochrome(input_file: &str, output_file: &str, cb: f64, cr: f64, size: f64) -> Result<String, String> {
    let filter = format!("monochrome=cb={}:cr={}:size={}", cb, cr, size);
    let mut command = Command::new("ffmpeg");
    command.arg("-i").arg(input_file).arg("-vf").arg(filter).arg("-c:a").arg("copy").arg("-y").arg(output_file);
    execute_ffmpeg_command(command)
}

pub fn apply_maskedmerge(input_file: &str, overlay_file: &str, mask_file: &str, output_file: &str, planes: u32) -> Result<String, String> {
    let p = if planes == 0 { 15 } else { planes };
    let filter = format!("[0:v][1:v][2:v]maskedmerge=planes={}[vout]", p);
    let mut command = Command::new("ffmpeg");
    command.arg("-i").arg(input_file)
        .arg("-i").arg(overlay_file)
        .arg("-i").arg(mask_file)
        .arg("-filter_complex").arg(&filter)
        .arg("-map").arg("[vout]")
        .arg("-map").arg("0:a?")
        .arg("-y").arg(output_file);
    execute_ffmpeg_command(command)
}

pub fn convert_360_video(input_file: &str, output_file: &str, input_fmt: &str, output_fmt: &str, width: u32, height: u32, h_fov: f64, v_fov: f64) -> Result<String, String> {
    let inf = if input_fmt.is_empty() { "equirect" } else { input_fmt };
    let outf = if output_fmt.is_empty() { "flat" } else { output_fmt };
    let w = if width == 0 { 1920 } else { width };
    let h = if height == 0 { 1080 } else { height };
    let filter = format!("v360={}:{}:w={}:h={}:h_fov={}:v_fov={}", inf, outf, w, h, h_fov, v_fov);
    let mut command = Command::new("ffmpeg");
    command.arg("-i").arg(input_file).arg("-vf").arg(filter).arg("-c:a").arg("copy").arg("-y").arg(output_file);
    execute_ffmpeg_command(command)
}

pub fn fix_banding(input_file: &str, output_file: &str, strength: f64, radius: u32) -> Result<String, String> {
    let s = if strength <= 0.0 { 1.2 } else { strength };
    let r = if radius == 0 { 16 } else { radius };
    let filter = format!("gradfun=strength={}:radius={}", s, r);
    let mut command = Command::new("ffmpeg");
    command.arg("-i").arg(input_file).arg("-vf").arg(filter).arg("-c:a").arg("copy").arg("-y").arg(output_file);
    execute_ffmpeg_command(command)
}

pub fn apply_greyedge(input_file: &str, output_file: &str, difford: u32, minknorm: f64, sigma: f64) -> Result<String, String> {
    let filter = format!("greyedge=difford={}:minknorm={}:sigma={}", difford, minknorm, sigma);
    let mut command = Command::new("ffmpeg");
    command.arg("-i").arg(input_file).arg("-vf").arg(filter).arg("-c:a").arg("copy").arg("-y").arg(output_file);
    execute_ffmpeg_command(command)
}

pub fn apply_fade_video(input_file: &str, output_file: &str, fade_type: &str, start_time: f64, duration: f64, color: &str) -> Result<String, String> {
    let ft = if fade_type.is_empty() { "in" } else { fade_type };
    let c = if color.is_empty() { "black" } else { color };
    let filter = format!("fade=type={}:start_time={}:duration={}:color={}", ft, start_time, duration, c);
    let mut command = Command::new("ffmpeg");
    command.arg("-i").arg(input_file).arg("-vf").arg(filter).arg("-c:a").arg("copy").arg("-y").arg(output_file);
    execute_ffmpeg_command(command)
}

pub fn extract_alpha_channel(input_file: &str, output_file: &str) -> Result<String, String> {
    let mut command = Command::new("ffmpeg");
    command.arg("-i").arg(input_file)
        .arg("-vf").arg("alphaextract")
        .arg("-c:a").arg("copy")
        .arg("-y").arg(output_file);
    execute_ffmpeg_command(command)
}

pub fn merge_alpha_channel(input_file: &str, alpha_file: &str, output_file: &str) -> Result<String, String> {
    let filter = "[0:v][1:v]alphamerge[vout]";
    let mut command = Command::new("ffmpeg");
    command.arg("-i").arg(input_file)
        .arg("-i").arg(alpha_file)
        .arg("-filter_complex").arg(filter)
        .arg("-map").arg("[vout]")
        .arg("-y").arg(output_file);
    execute_ffmpeg_command(command)
}

pub fn apply_framestep(input_file: &str, output_file: &str, step: u32) -> Result<String, String> {
    let s = if step == 0 { 1 } else { step };
    let filter = format!("framestep={}", s);
    let mut command = Command::new("ffmpeg");
    command.arg("-i").arg(input_file).arg("-vf").arg(filter).arg("-c:a").arg("copy").arg("-y").arg(output_file);
    execute_ffmpeg_command(command)
}

pub fn apply_swaprect(input_file: &str, output_file: &str, x1: u32, y1: u32, x2: u32, y2: u32, w: u32, h: u32) -> Result<String, String> {
    let filter = format!("swaprect=w={}:h={}:x1={}:y1={}:x2={}:y2={}", w, h, x1, y1, x2, y2);
    let mut command = Command::new("ffmpeg");
    command.arg("-i").arg(input_file).arg("-vf").arg(filter).arg("-c:a").arg("copy").arg("-y").arg(output_file);
    execute_ffmpeg_command(command)
}

pub fn apply_fillborders(input_file: &str, output_file: &str, left: u32, right: u32, top: u32, bottom: u32, mode: &str, color: &str) -> Result<String, String> {
    let m = if mode.is_empty() { "smear" } else { mode };
    let filter = if m == "fixed" {
        format!("fillborders=left={}:right={}:top={}:bottom={}:mode={}:color={}", left, right, top, bottom, m, color)
    } else {
        format!("fillborders=left={}:right={}:top={}:bottom={}:mode={}", left, right, top, bottom, m)
    };
    let mut command = Command::new("ffmpeg");
    command.arg("-i").arg(input_file).arg("-vf").arg(filter).arg("-c:a").arg("copy").arg("-y").arg(output_file);
    execute_ffmpeg_command(command)
}

pub fn apply_chromanr(input_file: &str, output_file: &str, thres: f64, sizew: u32, sizeh: u32, stepw: u32, steph: u32) -> Result<String, String> {
    let t = if thres <= 0.0 { 30.0 } else { thres };
    let sw = if sizew == 0 { 5 } else { sizew };
    let sh = if sizeh == 0 { 5 } else { sizeh };
    let spw = if stepw == 0 { 1 } else { stepw };
    let sph = if steph == 0 { 1 } else { steph };
    let filter = format!("chromanr=thres={}:sizew={}:sizeh={}:stepw={}:steph={}", t, sw, sh, spw, sph);
    let mut command = Command::new("ffmpeg");
    command.arg("-i").arg(input_file).arg("-vf").arg(filter).arg("-c:a").arg("copy").arg("-y").arg(output_file);
    execute_ffmpeg_command(command)
}

pub fn apply_weave(input_file: &str, output_file: &str, first_field: &str) -> Result<String, String> {
    let ff = if first_field.is_empty() { "top" } else { first_field };
    let filter = format!("weave=first_field={}", ff);
    let mut command = Command::new("ffmpeg");
    command.arg("-i").arg(input_file).arg("-vf").arg(filter).arg("-c:a").arg("copy").arg("-y").arg(output_file);
    execute_ffmpeg_command(command)
}

pub fn apply_interlace(input_file: &str, output_file: &str, scan: &str, lowpass: u32) -> Result<String, String> {
    let s = if scan.is_empty() { "tff" } else { scan };
    let lp = if lowpass == 0 { 1 } else { lowpass };
    let filter = format!("interlace=scan={}:lowpass={}", s, lp);
    let mut command = Command::new("ffmpeg");
    command.arg("-i").arg(input_file).arg("-vf").arg(filter).arg("-c:a").arg("copy").arg("-y").arg(output_file);
    execute_ffmpeg_command(command)
}

pub fn scale_to_reference(input_file: &str, ref_file: &str, output_file: &str, flags: &str) -> Result<String, String> {
    let f = if flags.is_empty() { "bilinear" } else { flags };
    let filter = format!("[0:v][1:v]scale2ref=flags={}[scaled][ref]", f);
    let mut command = Command::new("ffmpeg");
    command.arg("-i").arg(input_file)
        .arg("-i").arg(ref_file)
        .arg("-filter_complex").arg(&filter)
        .arg("-map").arg("[scaled]")
        .arg("-map").arg("0:a?")
        .arg("-y").arg(output_file);
    execute_ffmpeg_command(command)
}

pub fn apply_fieldorder(input_file: &str, output_file: &str, order: &str) -> Result<String, String> {
    let o = if order.is_empty() { "tff" } else { order };
    let filter = format!("fieldorder={}", o);
    let mut command = Command::new("ffmpeg");
    command.arg("-i").arg(input_file).arg("-vf").arg(filter).arg("-c:a").arg("copy").arg("-y").arg(output_file);
    execute_ffmpeg_command(command)
}

pub fn optimize_gif_palette(input_file: &str, output_file: &str, width: u32, fps: f64, stats_mode: &str) -> Result<String, String> {
    let w = if width == 0 { 320 } else { width };
    let f = if fps <= 0.0 { 10.0 } else { fps };
    let sm = if stats_mode.is_empty() { "diff" } else { stats_mode };
    let filter = format!(
        "[0:v]fps={},scale={}:-1:flags=lanczos,split[s0][s1];[s0]palettegen=stats_mode={}[p];[s1][p]paletteuse=dither=bayer",
        f, w, sm
    );
    let mut command = Command::new("ffmpeg");
    command.arg("-i").arg(input_file)
        .arg("-filter_complex").arg(&filter)
        .arg("-y").arg(output_file);
    execute_ffmpeg_command(command)
}

pub fn apply_hsv_key(input_file: &str, output_file: &str, hue: f64, saturation: f64, value: f64, similarity: f64, blend: f64) -> Result<String, String> {
    let filter = format!("hsvkey=hue={}:sat={}:val={}:similarity={}:blend={}", hue, saturation, value, similarity, blend);
    let mut command = Command::new("ffmpeg");
    command.arg("-i").arg(input_file).arg("-vf").arg(filter).arg("-c:a").arg("copy").arg("-y").arg(output_file);
    execute_ffmpeg_command(command)
}

pub fn apply_lut_yuv(input_file: &str, output_file: &str, y_expr: &str, u_expr: &str, v_expr: &str) -> Result<String, String> {
    let y = if y_expr.is_empty() { "val" } else { y_expr };
    let u = if u_expr.is_empty() { "val" } else { u_expr };
    let v = if v_expr.is_empty() { "val" } else { v_expr };
    let filter = format!("lutyuv=y='{}':u='{}':v='{}'", y, u, v);
    let mut command = Command::new("ffmpeg");
    command.arg("-i").arg(input_file).arg("-vf").arg(filter).arg("-c:a").arg("copy").arg("-y").arg(output_file);
    execute_ffmpeg_command(command)
}

pub fn apply_freezeframes(input_file: &str, output_file: &str, first: u32, last: u32, replace: u32) -> Result<String, String> {
    let filter = format!("freezeframes=first={}:last={}:replace={}", first, last, replace);
    let mut command = Command::new("ffmpeg");
    command.arg("-i").arg(input_file).arg("-vf").arg(filter).arg("-c:a").arg("copy").arg("-y").arg(output_file);
    execute_ffmpeg_command(command)
}

pub fn draw_signal_graph(input_file: &str, output_file: &str, signal: &str, width: u32, height: u32) -> Result<String, String> {
    let s = if signal.is_empty() { "YAVG" } else { signal };
    let w = if width == 0 { 1280 } else { width };
    let h = if height == 0 { 256 } else { height };
    let filter = format!(
        "signalstats,drawgraph=m1=lavfi.signalstats.{}:fg1=0xffff00ff:bg=0x00000080:size={}x{}:slide=scroll",
        s, w, h
    );
    let mut command = Command::new("ffmpeg");
    command.arg("-i").arg(input_file).arg("-vf").arg(filter).arg("-c:a").arg("copy").arg("-y").arg(output_file);
    execute_ffmpeg_command(command)
}

pub fn measure_video_entropy(input_file: &str) -> Result<String, String> {
    let output = std::process::Command::new("ffmpeg")
        .arg("-i").arg(input_file)
        .arg("-vf").arg("entropy")
        .arg("-f").arg("null").arg("-")
        .output()
        .map_err(|e| format!("Failed to execute FFmpeg: {}", e))?;
    let stderr = String::from_utf8_lossy(&output.stderr);
    let mut result = Vec::new();
    for line in stderr.lines() {
        if line.contains("entropy") || line.contains("normal_entropy") {
            result.push(line.trim().to_string());
        }
    }
    if result.is_empty() {
        Ok(stderr.lines().filter(|l| !l.is_empty()).take(20).collect::<Vec<_>>().join("\n"))
    } else {
        Ok(result.join("\n"))
    }
}

// PHASE I BATCH 10

pub fn stabilize_video_2pass(input_file: &str, output_file: &str, shakiness: u32, accuracy: u32, smoothing: u32, zoom: f64) -> Result<String, String> {
    let shak = if shakiness == 0 { 5 } else { shakiness.min(10) };
    let acc = if accuracy == 0 { 15 } else { accuracy.min(15) };
    let smth = if smoothing == 0 { 10 } else { smoothing };
    let z = if zoom == 0.0 { 0.0 } else { zoom };
    let transform_file = format!("{}.trf", input_file);
    // Pass 1: detect
    let pass1_filter = format!("vidstabdetect=shakiness={}:accuracy={}:result='{}'", shak, acc, transform_file);
    let mut p1 = Command::new("ffmpeg");
    p1.arg("-i").arg(input_file)
      .arg("-vf").arg(&pass1_filter)
      .arg("-f").arg("null").arg("-");
    execute_ffmpeg_command(p1)?;
    // Pass 2: transform
    let pass2_filter = format!("vidstabtransform=smoothing={}:zoom={}:input='{}'", smth, z, transform_file);
    let mut p2 = Command::new("ffmpeg");
    p2.arg("-i").arg(input_file)
      .arg("-vf").arg(&pass2_filter)
      .arg("-c:a").arg("copy").arg("-y").arg(output_file);
    execute_ffmpeg_command(p2)
}

pub fn apply_lut_rgb(input_file: &str, output_file: &str, r_expr: &str, g_expr: &str, b_expr: &str) -> Result<String, String> {
    let r = if r_expr.is_empty() { "val" } else { r_expr };
    let g = if g_expr.is_empty() { "val" } else { g_expr };
    let b = if b_expr.is_empty() { "val" } else { b_expr };
    let filter = format!("lutrgb=r='{}':g='{}':b='{}'", r, g, b);
    let mut command = Command::new("ffmpeg");
    command.arg("-i").arg(input_file).arg("-vf").arg(filter).arg("-c:a").arg("copy").arg("-y").arg(output_file);
    execute_ffmpeg_command(command)
}

pub fn apply_hsvhold(input_file: &str, output_file: &str, hue: f64, white: f64, black: f64, similarity: f64, blend: f64) -> Result<String, String> {
    let filter = format!("hsvhold=hue={}:white={}:black={}:similarity={}:blend={}", hue, white, black, similarity, blend);
    let mut command = Command::new("ffmpeg");
    command.arg("-i").arg(input_file).arg("-vf").arg(filter).arg("-c:a").arg("copy").arg("-y").arg(output_file);
    execute_ffmpeg_command(command)
}

pub fn convert_pixel_format(input_file: &str, output_file: &str, pix_fmt: &str) -> Result<String, String> {
    let fmt = if pix_fmt.is_empty() { "yuv420p" } else { pix_fmt };
    let mut command = Command::new("ffmpeg");
    command.arg("-i").arg(input_file).arg("-vf").arg(format!("format={}", fmt)).arg("-c:a").arg("copy").arg("-y").arg(output_file);
    execute_ffmpeg_command(command)
}

pub fn apply_setsar(input_file: &str, output_file: &str, sar: &str) -> Result<String, String> {
    let r = if sar.is_empty() { "1/1" } else { sar };
    let filter = format!("setsar=ratio={}", r);
    let mut command = Command::new("ffmpeg");
    command.arg("-i").arg(input_file).arg("-vf").arg(filter).arg("-c:a").arg("copy").arg("-y").arg(output_file);
    execute_ffmpeg_command(command)
}

pub fn apply_random_frames(input_file: &str, output_file: &str, frames: u32, seed: i64) -> Result<String, String> {
    let f = if frames == 0 { 30 } else { frames };
    let filter = if seed < 0 {
        format!("random=frames={}", f)
    } else {
        format!("random=frames={}:seed={}", f, seed)
    };
    let mut command = Command::new("ffmpeg");
    command.arg("-i").arg(input_file).arg("-vf").arg(filter).arg("-c:a").arg("copy").arg("-y").arg(output_file);
    execute_ffmpeg_command(command)
}

pub fn apply_convolution(input_file: &str, output_file: &str, matrix: &str, rdiv: f64, bias: f64, planes: u32) -> Result<String, String> {
    let m = if matrix.is_empty() { "0 -1 0 -1 5 -1 0 -1 0" } else { matrix };
    let rd = if rdiv <= 0.0 { 1.0 } else { rdiv };
    let p = if planes == 0 { 15 } else { planes };
    let filter = format!(
        "convolution=0m='{}':1m='{}':2m='{}':3m='{}':rdiv0={rd}:rdiv1={rd}:rdiv2={rd}:rdiv3={rd}:bias0={bias}:bias1={bias}:bias2={bias}:bias3={bias}:planes={p}",
        m, m, m, m, rd=rd, bias=bias, p=p
    );
    let mut command = Command::new("ffmpeg");
    command.arg("-i").arg(input_file).arg("-vf").arg(filter).arg("-c:a").arg("copy").arg("-y").arg(output_file);
    execute_ffmpeg_command(command)
}
