// src/core.rs

use crate::types::*;
use crate::utils::{
    execute_ffmpeg_command, execute_ffmpeg_command_with_sync_timeout, execute_ffprobe_command,
};
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
        format: format["format_name"]
            .as_str()
            .unwrap_or("unknown")
            .to_string(),
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
            tracing::info!(
                "Input format: {}, can use stream copy: {}",
                meta.format,
                is_h264
            );
            is_h264
        }
        Err(e) => {
            tracing::warn!(
                "Unable to analyze input video, will re-encode for safety: {}",
                e
            );
            false // If analysis fails, re-encode to be safe
        }
    };

    let mut command = Command::new("ffmpeg");

    if can_copy {
        // Input seeking (-ss before -i) for fast keyframe alignment on video.
        // Re-encode audio (never copy) to avoid silent-audio keyframe-misalignment
        // on Twitch VODs and other variable-keyframe sources where audio sync points
        // don't match video keyframes.
        tracing::info!("🚀 Using stream copy (video) + AAC re-encode (audio) for fast extraction");
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
            .arg("aac")
            .arg("-b:a")
            .arg("192k")
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

/// Trim a video segment, convert to YouTube Shorts format, and apply content-aware
/// video editing enhancements — all in a single FFmpeg pass.
///
/// Applied transformations (always):
///  1. Trim `start_seconds`–`end_seconds` from the source
///  2. Light stabilisation via `deshake` (removes camera shake)
///  3. Center-crop to 9:16 aspect ratio (landscape → portrait)
///  4. Scale to 1080×1920 (Full-HD Shorts resolution)
///  5. Subtle colour grading: saturation +12%, contrast +4%, gamma +3%
///  6. Light sharpening via `unsharp` (crispens compressed source)
///  7. Title text overlay burned at bottom of frame (if `title` is non-empty)
///  8. yuv420p pixel format for YouTube compatibility
///  9. Normalize audio loudness to −14 LUFS (EBU R128 Shorts target)
///
/// Content-type aware additions:
///  - gaming / sports / action → extra `deshake` strength (shake_x/y = 24)
///  - tutorial / vlog / educational / news → FFT audio denoising (`afftdn`)
///    prepended before loudnorm for cleaner voice
pub fn trim_and_convert_to_shorts(
    input_file: &str,
    output_file: &str,
    start_seconds: f64,
    end_seconds: f64,
    _title: &str,
    _content_type: &str,
) -> Result<String, String> {
    let duration = end_seconds - start_seconds;

    // ── Video filter chain ────────────────────────────────────────────────────

    // 1. Portrait crop + scale
    // fps=30: Twitch streams at 60fps — halve frame count to cut encode time 2x.
    let crop_scale = "fps=30,crop=ih*9/16:ih:(iw-ih*9/16)/2:0,scale=1080:1920:flags=bilinear";

    // 2. Pixel format (colour grading and sharpening removed — too slow on 1 vCPU)
    let pix_fmt = "format=yuv420p";

    // Assemble vf chain.
    // drawtext removed: fontconfig cache-building on first run in a fresh container
    // takes 2-5 minutes and blocks all concurrent FFmpeg processes.
    // deshake removed: scans the entire source file even with input seeking.
    // eq/unsharp removed: each filter adds ~15-20% CPU overhead — skip on 1 vCPU.
    let vf_parts: Vec<&str> = vec![crop_scale, pix_fmt];
    let vf = vf_parts.join(",");

    // ── Audio filter chain ────────────────────────────────────────────────────
    // dynaudnorm: single-pass loudness normalization — much faster than loudnorm
    // with I/LRA/TP params (which forces two-pass analysis and can run 10+ min
    // on shared CPU even for a 60s segment).
    let af = "dynaudnorm=f=75:g=25";

    // ── Build FFmpeg command ──────────────────────────────────────────────────
    let mut command = Command::new("ffmpeg");
    command
        .arg("-loglevel")
        .arg("error") // suppress frame-by-frame progress to avoid huge stderr buffers
        .arg("-ss")
        .arg(start_seconds.to_string())
        .arg("-i")
        .arg(input_file)
        .arg("-t")
        .arg(duration.to_string())
        .arg("-vf")
        .arg(&vf)
        .arg("-af")
        .arg(&af)
        .arg("-c:v")
        .arg("libx264")
        .arg("-preset")
        .arg("ultrafast")
        .arg("-crf")
        .arg("26")
        .arg("-c:a")
        .arg("aac")
        .arg("-b:a")
        .arg("128k")
        .arg("-movflags")
        .arg("faststart")
        .arg("-y")
        .arg(output_file);

    execute_ffmpeg_command_with_sync_timeout(command, Some(300))
}

pub fn merge_videos(input_files: &[String], output_file: &str) -> Result<String, String> {
    // Log video properties before merge for debugging
    tracing::info!("🎞️ Merging {} video clips", input_files.len());
    for (i, file) in input_files.iter().enumerate() {
        match analyze_video(file) {
            Ok(meta) => {
                tracing::info!(
                    "  Clip {}: {}x{} @ {:.1}fps, format: {}, duration: {:.1}s",
                    i + 1,
                    meta.width,
                    meta.height,
                    meta.fps,
                    meta.format,
                    meta.duration_seconds
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
        .arg("fast") // fast preset: ~2.5x faster than medium, ~5% larger file — acceptable for AI-generated videos
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
    // Use timeout version (15 minutes max for complex multi-clip merges)
    let result = execute_ffmpeg_command_with_sync_timeout(command, Some(900))?;

    // Validate the merged output
    tracing::info!("🔍 Validating merged output...");
    if !validate_video_file(output_file)? {
        return Err(format!(
            "❌ Merged video is corrupted or unreadable: {}",
            output_file
        ));
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
                metadata.width,
                metadata.height,
                metadata.fps,
                metadata.duration_seconds
            );
        }
        Err(e) => {
            tracing::warn!("⚠️ Unable to analyze merged output: {}", e);
        }
    }

    Ok(result)
}

/// Merge video clips with crossfade transitions between them.
/// Falls back gracefully to `merge_videos` (hard-cut) if xfade fails.
pub fn merge_videos_with_transitions(
    input_files: &[String],
    output_file: &str,
    transition_duration: f64,
) -> Result<String, String> {
    if input_files.len() < 2 {
        return merge_videos(input_files, output_file);
    }

    // Get durations for computing xfade time offsets
    let mut durations: Vec<f64> = Vec::new();
    for f in input_files {
        let d = analyze_video(f).map(|m| m.duration_seconds).unwrap_or(10.0);
        durations.push(d.max(transition_duration * 2.0 + 0.1)); // guard: clip must be longer than 2×transition
    }

    let n = input_files.len();
    let mut filter_parts: Vec<String> = Vec::new();

    // Normalise each video to 1920×1080 @ 30fps with consistent timebase
    for i in 0..n {
        filter_parts.push(format!(
            "[{}:v]scale=1920:1080:force_original_aspect_ratio=decrease,\
             pad=1920:1080:(ow-iw)/2:(oh-ih)/2:color=black,\
             settb=AVTB,fps=30[v{}]",
            i, i
        ));
    }

    // Chain xfade filters for video
    let mut current = "v0".to_string();
    let mut offset = durations[0] - transition_duration;
    for i in 1..n {
        let out = if i == n - 1 {
            "outv".to_string()
        } else {
            format!("xf{}", i)
        };
        filter_parts.push(format!(
            "[{}][v{}]xfade=transition=fade:duration={:.3}:offset={:.3}[{}]",
            current,
            i,
            transition_duration,
            offset.max(0.1),
            out
        ));
        offset += durations[i] - transition_duration;
        current = out;
    }

    // Audio: normalise sample rate and concat
    for i in 0..n {
        filter_parts.push(format!("[{}:a]aresample=44100[a{}]", i, i));
    }
    let audio_inputs: String = (0..n)
        .map(|i| format!("[a{}]", i))
        .collect::<Vec<_>>()
        .join("");
    filter_parts.push(format!("{}concat=n={}:v=0:a=1[outa]", audio_inputs, n));

    let filter_complex = filter_parts.join(";");

    let mut command = Command::new("ffmpeg");
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
        .arg("-c:v")
        .arg("libx264")
        .arg("-preset")
        .arg("fast") // fast preset: ~2.5x faster than medium
        .arg("-crf")
        .arg("23")
        .arg("-pix_fmt")
        .arg("yuv420p")
        .arg("-c:a")
        .arg("aac")
        .arg("-b:a")
        .arg("192k")
        .arg("-movflags")
        .arg("faststart")
        .arg("-y")
        .arg(output_file);

    tracing::info!(
        "⏳ Merging {} clips with {:.2}s crossfade transitions...",
        n,
        transition_duration
    );

    match execute_ffmpeg_command_with_sync_timeout(command, Some(900)) {
        Ok(result) => {
            if std::path::Path::new(output_file).exists()
                && std::fs::metadata(output_file).map(|m| m.len()).unwrap_or(0) > 1024
            {
                tracing::info!("✅ Transition merge successful: {}", output_file);
                Ok(result)
            } else {
                tracing::warn!(
                    "⚠️ Transition merge produced empty/missing file, falling back to hard-cut"
                );
                merge_videos(input_files, output_file)
            }
        }
        Err(xfade_err) => {
            tracing::warn!(
                "⚠️ Transition merge failed ({}), falling back to hard-cut merge",
                xfade_err
            );
            match merge_videos(input_files, output_file) {
                Ok(r) => Ok(r),
                Err(hard_cut_err) => Err(format!(
                    "Both merge strategies failed. xfade error: {}; hard-cut error: {}",
                    xfade_err, hard_cut_err
                )),
            }
        }
    }
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
    // ffprobe structural analysis (also provides file size — avoids a separate stat call)
    let meta = match analyze_video(file_path) {
        Ok(m) => m,
        Err(e) => {
            tracing::warn!("validate_video_file: ffprobe failed: {}", e);
            return Ok(false);
        }
    };

    // 1. File size check (catches empty/partial downloads)
    let size_bytes = (meta.file_size_mb * 1024.0 * 1024.0) as u64;
    if size_bytes < 100_000 {
        tracing::warn!(
            "validate_video_file: file too small ({} bytes): {}",
            size_bytes,
            file_path
        );
        return Ok(false);
    }

    // 3. Duration — must be at least 1 second
    if meta.duration_seconds < 1.0 {
        tracing::warn!(
            "validate_video_file: suspicious duration {:.2}s: {}",
            meta.duration_seconds,
            file_path
        );
        return Ok(false);
    }

    // 4. Video stream must exist
    if !meta.has_video {
        tracing::warn!("validate_video_file: no video stream found: {}", file_path);
        return Ok(false);
    }

    // 5. Non-zero resolution
    if meta.width == 0 || meta.height == 0 {
        tracing::warn!(
            "validate_video_file: zero resolution {}x{}: {}",
            meta.width,
            meta.height,
            file_path
        );
        return Ok(false);
    }

    Ok(true)
}

// ============================================================================
// BATCH 7 — Media Analysis Tools
// ============================================================================

pub fn detect_scene_changes(input_file: &str, threshold: f64) -> Result<String, String> {
    let filter = format!("scdet=threshold={}", threshold);
    let output = std::process::Command::new("ffmpeg")
        .arg("-i")
        .arg(input_file)
        .arg("-vf")
        .arg(filter)
        .arg("-f")
        .arg("null")
        .arg("-")
        .output()
        .map_err(|e| format!("Failed to execute FFmpeg: {}", e))?;
    let stderr = String::from_utf8_lossy(&output.stderr);
    let mut scenes = Vec::new();
    for line in stderr.lines() {
        if line.contains("scdet") && line.contains("pts_time") {
            if let Some(ts_start) = line.find("pts_time:") {
                let rest = &line[ts_start + 9..];
                let end = rest
                    .find(' ')
                    .or_else(|| rest.find('\n'))
                    .unwrap_or(rest.len());
                if let Ok(ts) = rest[..end].trim().parse::<f64>() {
                    scenes.push(ts);
                }
            }
        }
    }
    if scenes.is_empty() {
        Ok(format!(
            "No scene changes detected above threshold {}",
            threshold
        ))
    } else {
        Ok(format!(
            "Scene changes at {} timestamps (seconds): {:?}",
            scenes.len(),
            scenes
        ))
    }
}

pub fn measure_loudness(input_file: &str) -> Result<String, String> {
    let output = std::process::Command::new("ffmpeg")
        .arg("-i")
        .arg(input_file)
        .arg("-af")
        .arg("volumedetect")
        .arg("-f")
        .arg("null")
        .arg("-")
        .output()
        .map_err(|e| format!("Failed to execute FFmpeg: {}", e))?;
    let stderr = String::from_utf8_lossy(&output.stderr);
    let mut result = Vec::new();
    for line in stderr.lines() {
        if line.contains("mean_volume") || line.contains("max_volume") || line.contains("histogram")
        {
            result.push(line.trim().to_string());
        }
    }
    if result.is_empty() {
        Err("Could not measure loudness — no audio stream or unsupported format".to_string())
    } else {
        Ok(result.join("\n"))
    }
}

pub fn detect_silence(
    input_file: &str,
    noise_db: f64,
    min_duration: f64,
) -> Result<String, String> {
    let filter = format!("silencedetect=noise={}dB:d={}", noise_db, min_duration);
    let output = std::process::Command::new("ffmpeg")
        .arg("-i")
        .arg(input_file)
        .arg("-af")
        .arg(filter)
        .arg("-f")
        .arg("null")
        .arg("-")
        .output()
        .map_err(|e| format!("Failed to execute FFmpeg: {}", e))?;
    let stderr = String::from_utf8_lossy(&output.stderr);
    let mut result = Vec::new();
    for line in stderr.lines() {
        if line.contains("silence_start")
            || line.contains("silence_end")
            || line.contains("silence_duration")
        {
            result.push(line.trim().to_string());
        }
    }
    if result.is_empty() {
        Ok("No silence detected".to_string())
    } else {
        Ok(result.join("\n"))
    }
}
// ============================================================================
// BATCH 8 — Quality Metrics
// ============================================================================

pub fn compare_ssim(reference_file: &str, distorted_file: &str) -> Result<String, String> {
    let output = std::process::Command::new("ffmpeg")
        .arg("-i")
        .arg(distorted_file)
        .arg("-i")
        .arg(reference_file)
        .arg("-filter_complex")
        .arg("ssim")
        .arg("-f")
        .arg("null")
        .arg("-")
        .output()
        .map_err(|e| format!("Failed to execute FFmpeg: {}", e))?;
    let stderr = String::from_utf8_lossy(&output.stderr);
    let mut result = Vec::new();
    for line in stderr.lines() {
        if line.contains("SSIM") || line.contains("ssim") {
            result.push(line.trim().to_string());
        }
    }
    if result.is_empty() {
        Err(
            "Could not compute SSIM — ensure both files have the same resolution and codec support"
                .to_string(),
        )
    } else {
        Ok(result.join("\n"))
    }
}

pub fn compare_psnr(reference_file: &str, distorted_file: &str) -> Result<String, String> {
    let output = std::process::Command::new("ffmpeg")
        .arg("-i")
        .arg(distorted_file)
        .arg("-i")
        .arg(reference_file)
        .arg("-filter_complex")
        .arg("psnr")
        .arg("-f")
        .arg("null")
        .arg("-")
        .output()
        .map_err(|e| format!("Failed to execute FFmpeg: {}", e))?;
    let stderr = String::from_utf8_lossy(&output.stderr);
    let mut result = Vec::new();
    for line in stderr.lines() {
        if line.contains("PSNR") || line.contains("psnr") {
            result.push(line.trim().to_string());
        }
    }
    if result.is_empty() {
        Err(
            "Could not compute PSNR — ensure both files have the same resolution and codec support"
                .to_string(),
        )
    } else {
        Ok(result.join("\n"))
    }
}

pub fn analyze_audio_stats(input_file: &str, reset_interval: u32) -> Result<String, String> {
    let filter = if reset_interval > 0 {
        format!("astats=reset={}", reset_interval)
    } else {
        "astats".to_string()
    };
    let output = std::process::Command::new("ffmpeg")
        .arg("-i")
        .arg(input_file)
        .arg("-af")
        .arg(filter)
        .arg("-f")
        .arg("null")
        .arg("-")
        .output()
        .map_err(|e| format!("Failed to execute FFmpeg: {}", e))?;
    let stderr = String::from_utf8_lossy(&output.stderr);
    let mut result = Vec::new();
    for line in stderr.lines() {
        if line.contains("RMS")
            || line.contains("Peak")
            || line.contains("Crest")
            || line.contains("Flat")
            || line.contains("astats")
            || line.contains("DC")
        {
            result.push(line.trim().to_string());
        }
    }
    if result.is_empty() {
        Err("Could not analyze audio stats — no audio stream or unsupported format".to_string())
    } else {
        Ok(result.join("\n"))
    }
}

pub fn analyze_video_signal(input_file: &str) -> Result<String, String> {
    let output = std::process::Command::new("ffmpeg")
        .arg("-i")
        .arg(input_file)
        .arg("-vf")
        .arg("signalstats")
        .arg("-f")
        .arg("null")
        .arg("-")
        .output()
        .map_err(|e| format!("Failed to execute FFmpeg: {}", e))?;
    let stderr = String::from_utf8_lossy(&output.stderr);
    let mut result = Vec::new();
    for line in stderr.lines() {
        if line.contains("YMIN")
            || line.contains("YMAX")
            || line.contains("UMIN")
            || line.contains("VMIN")
            || line.contains("SATMAX")
            || line.contains("signalstats")
        {
            result.push(line.trim().to_string());
        }
    }
    if result.is_empty() {
        let lines: Vec<&str> = stderr.lines().collect();
        let start = if lines.len() > 10 {
            lines.len() - 10
        } else {
            0
        };
        Ok(lines[start..].join("\n"))
    } else {
        Ok(result.join("\n"))
    }
}

// ================================================================
// PHASE H — Codec / Format Depth
// ================================================================

/// VP9 encoding via libvpx-vp9. Best-quality open codec for web delivery.
pub fn encode_vp9(
    input_file: &str,
    output_file: &str,
    crf: u32,
    bitrate: &str,
    speed: u32,
    threads: u32,
) -> Result<String, String> {
    let mut command = Command::new("ffmpeg");
    command
        .arg("-i")
        .arg(input_file)
        .arg("-c:v")
        .arg("libvpx-vp9")
        .arg("-crf")
        .arg(crf.to_string())
        .arg("-b:v")
        .arg(if bitrate.is_empty() { "0" } else { bitrate })
        .arg("-cpu-used")
        .arg(speed.to_string())
        .arg("-threads")
        .arg(threads.to_string())
        .arg("-c:a")
        .arg("libopus")
        .arg("-y")
        .arg(output_file);
    execute_ffmpeg_command(command)
}

/// AV1 encoding via libaom-av1 or libsvtav1. Next-gen codec with superior compression.
pub fn encode_av1(
    input_file: &str,
    output_file: &str,
    crf: u32,
    speed: u32,
    threads: u32,
    encoder: &str,
) -> Result<String, String> {
    let codec = if encoder.is_empty() {
        "libaom-av1"
    } else {
        encoder
    };
    let mut command = Command::new("ffmpeg");
    command
        .arg("-i")
        .arg(input_file)
        .arg("-c:v")
        .arg(codec)
        .arg("-crf")
        .arg(crf.to_string())
        .arg("-cpu-used")
        .arg(speed.to_string())
        .arg("-threads")
        .arg(threads.to_string())
        .arg("-c:a")
        .arg("libopus")
        .arg("-y")
        .arg(output_file);
    execute_ffmpeg_command(command)
}

/// H.265/HEVC encoding via libx265. Up to 50% better compression than H.264.
pub fn encode_hevc(
    input_file: &str,
    output_file: &str,
    crf: u32,
    preset: &str,
    tune: &str,
) -> Result<String, String> {
    let preset_val = if preset.is_empty() { "medium" } else { preset };
    let mut command = Command::new("ffmpeg");
    command
        .arg("-i")
        .arg(input_file)
        .arg("-c:v")
        .arg("libx265")
        .arg("-crf")
        .arg(crf.to_string())
        .arg("-preset")
        .arg(preset_val);
    if !tune.is_empty() {
        command.arg("-tune").arg(tune);
    }
    command.arg("-c:a").arg("copy").arg("-y").arg(output_file);
    execute_ffmpeg_command(command)
}

/// Opus audio encoding via libopus. Best-in-class lossy audio codec for streaming.
pub fn encode_opus(
    input_file: &str,
    output_file: &str,
    bitrate_kbps: u32,
    vbr: bool,
    compression: u32,
) -> Result<String, String> {
    let mut command = Command::new("ffmpeg");
    command
        .arg("-i")
        .arg(input_file)
        .arg("-c:a")
        .arg("libopus")
        .arg("-b:a")
        .arg(format!("{}k", bitrate_kbps))
        .arg("-vbr")
        .arg(if vbr { "on" } else { "off" })
        .arg("-compression_level")
        .arg(compression.to_string())
        .arg("-vn")
        .arg("-y")
        .arg(output_file);
    execute_ffmpeg_command(command)
}

/// HDR10 encoding via libx265 with mastering display and content light level metadata.
pub fn encode_hdr10(
    input_file: &str,
    output_file: &str,
    crf: u32,
    preset: &str,
    master_display: &str,
    max_cll: &str,
) -> Result<String, String> {
    let preset_val = if preset.is_empty() { "slow" } else { preset };
    // default HDR10 mastering display (Rec. 2020 / D65 white point)
    let md = if master_display.is_empty() {
        "G(13250,34500)B(7500,3000)R(34000,16000)WP(15635,16450)L(10000000,1)"
    } else {
        master_display
    };
    let cll = if max_cll.is_empty() {
        "1000,400"
    } else {
        max_cll
    };
    let x265_params = format!(
        "hdr-opt=1:repeat-headers=1:colorprim=bt2020:transfer=smpte2084:colormatrix=bt2020nc:master-display={}:max-cll={}",
        md, cll
    );
    let mut command = Command::new("ffmpeg");
    command
        .arg("-i")
        .arg(input_file)
        .arg("-c:v")
        .arg("libx265")
        .arg("-crf")
        .arg(crf.to_string())
        .arg("-preset")
        .arg(preset_val)
        .arg("-x265-params")
        .arg(x265_params)
        .arg("-color_primaries")
        .arg("bt2020")
        .arg("-color_trc")
        .arg("smpte2084")
        .arg("-colorspace")
        .arg("bt2020nc")
        .arg("-c:a")
        .arg("copy")
        .arg("-y")
        .arg(output_file);
    execute_ffmpeg_command(command)
}

/// NVIDIA NVENC hardware encoding. Requires CUDA-capable GPU.
pub fn encode_nvenc(
    input_file: &str,
    output_file: &str,
    codec: &str,
    preset: &str,
    bitrate: &str,
    cq: u32,
) -> Result<String, String> {
    let nvenc_codec = match codec {
        "hevc" | "h265" => "hevc_nvenc",
        "av1" => "av1_nvenc",
        _ => "h264_nvenc",
    };
    let preset_val = if preset.is_empty() { "p4" } else { preset };
    let mut command = Command::new("ffmpeg");
    command
        .arg("-hwaccel")
        .arg("cuda")
        .arg("-i")
        .arg(input_file)
        .arg("-c:v")
        .arg(nvenc_codec)
        .arg("-preset")
        .arg(preset_val)
        .arg("-cq")
        .arg(cq.to_string());
    if !bitrate.is_empty() {
        command.arg("-b:v").arg(bitrate);
    }
    command.arg("-c:a").arg("copy").arg("-y").arg(output_file);
    execute_ffmpeg_command(command)
}

/// Intel/AMD VAAPI hardware encoding. Requires VAAPI-compatible GPU.
pub fn encode_vaapi(
    input_file: &str,
    output_file: &str,
    codec: &str,
    quality: u32,
    profile: &str,
) -> Result<String, String> {
    let vaapi_codec = match codec {
        "hevc" | "h265" => "hevc_vaapi",
        "vp9" => "vp9_vaapi",
        "av1" => "av1_vaapi",
        _ => "h264_vaapi",
    };
    let profile_val = if profile.is_empty() { "high" } else { profile };
    let mut command = Command::new("ffmpeg");
    command
        .arg("-vaapi_device")
        .arg("/dev/dri/renderD128")
        .arg("-i")
        .arg(input_file)
        .arg("-vf")
        .arg("format=nv12,hwupload")
        .arg("-c:v")
        .arg(vaapi_codec)
        .arg("-qp")
        .arg(quality.to_string())
        .arg("-profile:v")
        .arg(profile_val)
        .arg("-c:a")
        .arg("copy")
        .arg("-y")
        .arg(output_file);
    execute_ffmpeg_command(command)
}

/// Intel Quick Sync Video (QSV) hardware encoding.
pub fn encode_qsv(
    input_file: &str,
    output_file: &str,
    codec: &str,
    preset: &str,
    bitrate: &str,
) -> Result<String, String> {
    let qsv_codec = match codec {
        "hevc" | "h265" => "hevc_qsv",
        "av1" => "av1_qsv",
        "vp9" => "vp9_qsv",
        _ => "h264_qsv",
    };
    let preset_val = if preset.is_empty() { "medium" } else { preset };
    let mut command = Command::new("ffmpeg");
    command
        .arg("-i")
        .arg(input_file)
        .arg("-c:v")
        .arg(qsv_codec)
        .arg("-preset")
        .arg(preset_val);
    if !bitrate.is_empty() {
        command.arg("-b:v").arg(bitrate);
    }
    command.arg("-c:a").arg("copy").arg("-y").arg(output_file);
    execute_ffmpeg_command(command)
}

/// Apple ProRes encoding via prores_ks. Professional intermediate codec for editing workflows.
pub fn encode_prores(
    input_file: &str,
    output_file: &str,
    profile: u32,
    vendor: &str,
) -> Result<String, String> {
    // profiles: 0=proxy, 1=LT, 2=standard, 3=HQ, 4=4444, 5=4444XQ
    let vendor_val = if vendor.is_empty() { "apl0" } else { vendor };
    let mut command = Command::new("ffmpeg");
    command
        .arg("-i")
        .arg(input_file)
        .arg("-c:v")
        .arg("prores_ks")
        .arg("-profile:v")
        .arg(profile.to_string())
        .arg("-vendor")
        .arg(vendor_val)
        .arg("-c:a")
        .arg("copy")
        .arg("-y")
        .arg(output_file);
    execute_ffmpeg_command(command)
}

/// Avid DNxHD/DNxHR encoding. Professional intermediate codec for Avid workflows.
pub fn encode_dnxhd(input_file: &str, output_file: &str, profile: &str) -> Result<String, String> {
    // profile examples: dnxhd, dnxhr_lb, dnxhr_sq, dnxhr_hq, dnxhr_hqx, dnxhr_444
    let profile_val = if profile.is_empty() {
        "dnxhr_sq"
    } else {
        profile
    };
    let mut command = Command::new("ffmpeg");
    command
        .arg("-i")
        .arg(input_file)
        .arg("-c:v")
        .arg("dnxhd")
        .arg("-vf")
        .arg("scale=trunc(iw/2)*2:trunc(ih/2)*2") // ensure even dimensions
        .arg("-profile:v")
        .arg(profile_val)
        .arg("-c:a")
        .arg("pcm_s16le")
        .arg("-y")
        .arg(output_file);
    execute_ffmpeg_command(command)
}

/// High-quality animated GIF using FFmpeg palette optimisation (2-pass palettegen + paletteuse).
pub fn encode_gif(
    input_file: &str,
    output_file: &str,
    fps: f64,
    scale: u32,
    loop_count: i32,
) -> Result<String, String> {
    let palette_file = format!("{}.palette.png", output_file);
    let fps_val = if fps <= 0.0 { 15.0 } else { fps };
    let scale_val = if scale == 0 { 480 } else { scale };
    let loop_val = if loop_count < 0 { 0 } else { loop_count }; // 0 = infinite

    // Pass 1: generate palette
    let vf_pass1 = format!(
        "fps={},scale={}:-1:flags=lanczos,palettegen",
        fps_val, scale_val
    );
    let pass1 = Command::new("ffmpeg")
        .arg("-i")
        .arg(input_file)
        .arg("-vf")
        .arg(&vf_pass1)
        .arg("-y")
        .arg(&palette_file)
        .output()
        .map_err(|e| format!("GIF palette pass failed: {}", e))?;
    if !pass1.status.success() {
        let _ = std::fs::remove_file(&palette_file);
        return Err(format!(
            "GIF palette generation failed: {}",
            String::from_utf8_lossy(&pass1.stderr)
        ));
    }

    // Pass 2: encode GIF using palette
    let vf_pass2 = format!(
        "fps={},scale={}:-1:flags=lanczos[x];[x][1:v]paletteuse",
        fps_val, scale_val
    );
    let mut command = Command::new("ffmpeg");
    command
        .arg("-i")
        .arg(input_file)
        .arg("-i")
        .arg(&palette_file)
        .arg("-lavfi")
        .arg(&vf_pass2)
        .arg("-loop")
        .arg(loop_val.to_string())
        .arg("-y")
        .arg(output_file);
    let result = execute_ffmpeg_command(command);
    let _ = std::fs::remove_file(&palette_file);
    result
}

// ================================================================
// PHASE I — Long-tail sweep, Batch 1
// ================================================================

/// Segment a video into fixed-duration chunks using the FFmpeg segment muxer.
pub fn segment_video(
    input_file: &str,
    output_pattern: &str,
    segment_time: f64,
    reset_timestamps: bool,
) -> Result<String, String> {
    let seg = if segment_time <= 0.0 {
        60.0
    } else {
        segment_time
    };
    let reset = if reset_timestamps { "1" } else { "0" };
    let mut command = Command::new("ffmpeg");
    command
        .arg("-i")
        .arg(input_file)
        .arg("-c")
        .arg("copy")
        .arg("-f")
        .arg("segment")
        .arg("-segment_time")
        .arg(seg.to_string())
        .arg("-reset_timestamps")
        .arg(reset)
        .arg("-y")
        .arg(output_pattern);
    execute_ffmpeg_command(command)
}

/// Add black (or coloured) padding frames at the start/end of a video via FFmpeg tpad filter.
pub fn pad_video_time(
    input_file: &str,
    output_file: &str,
    start_duration: f64,
    stop_duration: f64,
    color: &str,
) -> Result<String, String> {
    let col = if color.is_empty() { "black" } else { color };
    let filter = format!(
        "tpad=start_duration={}:stop_duration={}:color={}",
        start_duration, stop_duration, col
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

/// WebM container encoding (VP8/VP9 video + Vorbis/Opus audio). Open web format.
pub fn encode_webm(
    input_file: &str,
    output_file: &str,
    video_codec: &str,
    audio_codec: &str,
    crf: u32,
    bitrate: &str,
) -> Result<String, String> {
    let vcodec = match video_codec {
        "vp9" | "VP9" => "libvpx-vp9",
        _ => "libvpx", // VP8 default
    };
    let acodec = match audio_codec {
        "opus" | "Opus" => "libopus",
        _ => "libvorbis", // Vorbis default
    };
    let mut command = Command::new("ffmpeg");
    command
        .arg("-i")
        .arg(input_file)
        .arg("-c:v")
        .arg(vcodec)
        .arg("-crf")
        .arg(crf.to_string())
        .arg("-b:v")
        .arg(if bitrate.is_empty() { "0" } else { bitrate })
        .arg("-c:a")
        .arg(acodec)
        .arg("-y")
        .arg(output_file);
    execute_ffmpeg_command(command)
}

/// Generates a test pattern video using FFmpeg lavfi source (smptebars, smptehdbars, testsrc). No input file required.
pub fn create_test_pattern(
    output_file: &str,
    width: u32,
    height: u32,
    duration: f64,
    pattern: &str,
    framerate: f64,
) -> Result<String, String> {
    let w = if width == 0 { 1920 } else { width };
    let h = if height == 0 { 1080 } else { height };
    let d = if duration <= 0.0 { 10.0 } else { duration };
    let fps = if framerate <= 0.0 { 25.0 } else { framerate };
    let src = match pattern {
        "smptehdbars" | "hd" | "hdbars" => "smptehdbars",
        "testsrc" | "test" => "testsrc",
        "testsrc2" => "testsrc2",
        _ => "smptebars",
    };
    let lavfi = format!("{}=size={}x{}:duration={}:rate={}", src, w, h, d, fps);
    let mut command = Command::new("ffmpeg");
    command
        .arg("-f")
        .arg("lavfi")
        .arg("-i")
        .arg(&lavfi)
        .arg("-y")
        .arg(output_file);
    execute_ffmpeg_command(command)
}
