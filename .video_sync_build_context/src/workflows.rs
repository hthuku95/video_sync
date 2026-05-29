/// Named multi-step workflow recipes that chain existing FFmpeg tool functions.
///
/// Each recipe takes an input file and output file, applies a fixed series of
/// transforms using temp files, and returns the path to the final result.
///
/// The agent can invoke these as high-level intents (e.g. "make it YouTube-ready")
/// rather than manually chaining individual tools.
use std::path::Path;

fn temp_path(base: &str, suffix: u32, ext: &str) -> String {
    let stem = Path::new(base)
        .file_stem()
        .and_then(|s| s.to_str())
        .unwrap_or("tmp");
    format!("/tmp/{}_wf_{}.{}", stem, suffix, ext)
}

fn cleanup(path: &str) {
    let _ = std::fs::remove_file(path);
}

/// YouTube-ready export: stabilize → adjust color → loudnorm → yuv420p
///
/// Produces a file that passes YouTube's quality checks:
/// stabilized video, broadcast-safe color, -14 LUFS loudness, yuv420p pixel format.
pub fn youtube_ready_export(input_file: &str, output_file: &str) -> Result<String, String> {
    let t1 = temp_path(input_file, 1, "mp4");
    let t2 = temp_path(input_file, 2, "mp4");
    let t3 = temp_path(input_file, 3, "mp4");

    // Pass 1: stabilize (2-pass: shakiness=5, accuracy=10, smoothing=10, zoom=0.0)
    crate::visual::stabilize_video_2pass(input_file, &t1, 5, 10, 10, 0.0).inspect_err(|_| {
        cleanup(&t1);
    })?;

    // Pass 2: color grading (normalize)
    crate::visual::normalize_video(&t1, &t2, 0u32).inspect_err(|_| {
        cleanup(&t1);
        cleanup(&t2);
    })?;
    cleanup(&t1);

    // Pass 3: loudnorm to -14 LUFS (YouTube target)
    crate::audio::normalize_loudness(&t2, &t3, -14.0, 11.0, -1.0).inspect_err(|_| {
        cleanup(&t2);
        cleanup(&t3);
    })?;
    cleanup(&t2);

    // Pass 4: convert to yuv420p for maximum compatibility
    crate::visual::convert_pixel_format(&t3, output_file, "yuv420p").inspect_err(|_| {
        cleanup(&t3);
    })?;
    cleanup(&t3);

    Ok(format!("✅ YouTube-ready export complete: {}", output_file))
}

/// Podcast audio cleanup: denoise → reduce sibilance → limiter → loudnorm
///
/// Cleans up speech audio: removes background noise, de-esses harsh S sounds,
/// limits peaks, and normalises to podcast standard (-16 LUFS).
pub fn podcast_cleanup(input_file: &str, output_file: &str) -> Result<String, String> {
    let t1 = temp_path(input_file, 1, "wav");
    let t2 = temp_path(input_file, 2, "wav");
    let t3 = temp_path(input_file, 3, "wav");

    // Pass 1: remove background noise (afftdn — no external model needed)
    crate::audio::denoise_audio(input_file, &t1, -30.0, 12.0, false).inspect_err(|_| {
        cleanup(&t1);
    })?;

    // Pass 2: reduce sibilance (de-esser at 8.5 kHz)
    crate::audio::reduce_sibilance(&t1, &t2, 8500.0, 0.3, "split").inspect_err(|_| {
        cleanup(&t1);
        cleanup(&t2);
    })?;
    cleanup(&t1);

    // Pass 3: limit peaks to -1 dB
    crate::audio::audio_limiter(&t2, &t3, -1.0, 5.0, 50.0, false).inspect_err(|_| {
        cleanup(&t2);
        cleanup(&t3);
    })?;
    cleanup(&t2);

    // Pass 4: loudnorm to -16 LUFS (podcast standard)
    crate::audio::normalize_loudness(&t3, output_file, -16.0, 11.0, -1.5).inspect_err(|_| {
        cleanup(&t3);
    })?;
    cleanup(&t3);

    Ok(format!("✅ Podcast cleanup complete: {}", output_file))
}

/// Cinematic grade: vintage curves → vibrance → vignette → film grain
///
/// Applies a cinematic look suitable for trailers and highlight reels.
pub fn cinematic_grade(input_file: &str, output_file: &str) -> Result<String, String> {
    let t1 = temp_path(input_file, 1, "mp4");
    let t2 = temp_path(input_file, 2, "mp4");
    let t3 = temp_path(input_file, 3, "mp4");

    // Pass 1: vintage color curves
    crate::visual::adjust_curves(input_file, &t1, "vintage", "", "", "", "").inspect_err(|_| {
        cleanup(&t1);
    })?;

    // Pass 2: boost vibrance (intensity=0.4, neutral RGB balance)
    crate::visual::adjust_vibrance(&t1, &t2, 0.4, 0.0, 0.0, 0.0).inspect_err(|_| {
        cleanup(&t1);
        cleanup(&t2);
    })?;
    cleanup(&t1);

    // Pass 3: add vignette (angle=PI/5, forward mode)
    crate::visual::add_vignette(&t2, &t3, std::f64::consts::PI / 5.0, "forward").inspect_err(
        |_| {
            cleanup(&t2);
            cleanup(&t3);
        },
    )?;
    cleanup(&t2);

    // Pass 4: film grain
    crate::visual::add_film_grain(&t3, output_file, 15, "").inspect_err(|_| {
        cleanup(&t3);
    })?;
    cleanup(&t3);

    Ok(format!("✅ Cinematic grade complete: {}", output_file))
}

/// GIF creation: trim → scale → optimize palette
///
/// Creates a high-quality, compact GIF optimised with palette generation.
pub fn create_gif(
    input_file: &str,
    output_file: &str,
    start_seconds: f64,
    duration_seconds: f64,
    width: u32,
    fps: f64,
) -> Result<String, String> {
    let t1 = temp_path(input_file, 1, "mp4");

    // Pass 1: trim to segment
    let end = start_seconds + duration_seconds;
    crate::core::trim_video(input_file, &t1, start_seconds, end).inspect_err(|_| {
        cleanup(&t1);
    })?;

    // Pass 2: scale + optimize palette → direct GIF output
    crate::visual::optimize_gif_palette(&t1, output_file, width, fps, "diff").inspect_err(
        |_| {
            cleanup(&t1);
        },
    )?;
    cleanup(&t1);

    Ok(format!("✅ GIF created: {}", output_file))
}

/// Talking head cleanup: deshake → denoise speech → reduce sibilance → loudnorm
///
/// Optimises talking-head footage for YouTube/podcast: stabilises camera shake,
/// cleans up audio noise, removes harsh sibilance, and normalises loudness.
pub fn talking_head_cleanup(input_file: &str, output_file: &str) -> Result<String, String> {
    let t1 = temp_path(input_file, 1, "mp4");
    let t2 = temp_path(input_file, 2, "mp4");
    let t3 = temp_path(input_file, 3, "mp4");

    // Pass 1: stabilize video
    crate::visual::stabilize_video_2pass(input_file, &t1, 5, 10, 10, 0.0).inspect_err(|_| {
        cleanup(&t1);
    })?;

    // Pass 2: denoise audio
    crate::audio::denoise_audio(&t1, &t2, -30.0, 12.0, false).inspect_err(|_| {
        cleanup(&t1);
        cleanup(&t2);
    })?;
    cleanup(&t1);

    // Pass 3: reduce sibilance
    crate::audio::reduce_sibilance(&t2, &t3, 8500.0, 0.3, "split").inspect_err(|_| {
        cleanup(&t2);
        cleanup(&t3);
    })?;
    cleanup(&t2);

    // Pass 4: loudnorm
    crate::audio::normalize_loudness(&t3, output_file, -16.0, 11.0, -1.5).inspect_err(|_| {
        cleanup(&t3);
    })?;
    cleanup(&t3);

    Ok(format!("✅ Talking head cleanup complete: {}", output_file))
}
