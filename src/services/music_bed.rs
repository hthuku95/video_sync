use serde_json::{json, Value};

/// Music library root. Bundled CC-BY beds (Kevin MacLeod / incompetech.com).
/// Attribution is added to delivery pages: "Music by Kevin MacLeod (incompetech.com), CC BY 4.0".
pub const MUSIC_LIBRARY_DIR: &str = "music_library";

/// Pick a bundled bed by prompt/scene keywords. Returns None when nothing fits.
pub fn pick_music_bed(prompt: &str) -> Option<&'static str> {
    let p = prompt.to_lowercase();
    // Ordered by specificity: first hit wins.
    if p.contains("corporate")
        || p.contains("business")
        || p.contains("saas")
        || p.contains("product")
    {
        return Some("bed_corporate_wallpaper.mp3");
    }
    if p.contains("electronic")
        || p.contains("tech")
        || p.contains("digital")
        || p.contains("futuristic")
        || p.contains("ai")
    {
        return Some("bed_electronic_electrodoodle.mp3");
    }
    if p.contains("ambient")
        || p.contains("calm")
        || p.contains("serious")
        || p.contains("documentary")
        || p.contains("explain")
    {
        return Some("bed_ambient_fretless.mp3");
    }
    if p.contains("upbeat")
        || p.contains("energetic")
        || p.contains("fun")
        || p.contains("friendly")
        || p.contains("casual")
    {
        return Some("bed_upbeat_carefree.mp3");
    }
    // Education default
    if p.contains("lesson") || p.contains("course") || p.contains("educat") {
        return Some("bed_ambient_fretless.mp3");
    }
    None
}

/// List available beds (for the agent to reference explicitly).
pub fn list_music_beds() -> Vec<String> {
    let dir = std::path::Path::new(MUSIC_LIBRARY_DIR);
    let mut out = Vec::new();
    if let Ok(entries) = std::fs::read_dir(dir) {
        for e in entries.flatten() {
            if e.path()
                .extension()
                .and_then(|x| x.to_str())
                .map(|x| x == "mp3")
                .unwrap_or(false)
            {
                out.push(e.file_name().to_string_lossy().to_string());
            }
        }
    }
    out.sort();
    out
}

/// Mix a music bed under an existing video (keeps narration, ducks music).
/// Music starts at `offset_seconds`, fades in/out, loops if shorter than the video.
pub fn add_background_music_impl(
    video_file: &str,
    music_file: &str,
    output_file: &str,
    volume: f32,
    offset_seconds: f64,
    fade_in: f32,
    fade_out: f32,
) -> Result<String, String> {
    let fade_in = fade_in as f64;
    let fade_out = fade_out as f64;
    let dur = crate::core::analyze_video(video_file)
        .map(|m| m.duration_seconds)
        .unwrap_or(0.0);
    if dur <= 0.0 {
        return Err("Unable to determine video duration".to_string());
    }
    // Loop the bed if it's shorter than the video
    let music_dur = std::process::Command::new("ffprobe")
        .args([
            "-v",
            "error",
            "-show_entries",
            "format=duration",
            "-of",
            "csv=p=0",
            music_file,
        ])
        .output()
        .ok()
        .and_then(|o| String::from_utf8(o.stdout).ok())
        .and_then(|s| s.trim().parse::<f64>().ok())
        .unwrap_or(0.0);

    let mut command = std::process::Command::new("ffmpeg");
    command
        .arg("-y")
        .arg("-i")
        .arg(video_file)
        .arg("-i")
        .arg(music_file);

    let filter = if music_dur > 0.0 && music_dur < dur {
        // Loop bed across the full duration
        format!(
            "[1:a]aloop=loop=-1:size=2^{}/{}:fade=t=in:st=0:d={}:fade=t=out:st={}:d={},volume={}[m];[0:a][m]amix=inputs=2:duration=first:dropout_transition=2:weights=1 {}[aout]",
            16, music_dur, fade_in, (dur - fade_out).max(0.0), fade_out, volume as f64, 1.0 - volume as f64
        )
    } else {
        format!(
            "[1:a]atrim=0:{dur}:fade=t=in:st=0:d={fi}:fade=t=out:st={fo}:d={fo},volume={vol}[m];[0:a][m]amix=inputs=2:duration=first:dropout_transition=2:weights=1 {w}[aout]",
            dur = dur,
            fi = fade_in,
            fo = (dur - fade_out).max(0.0),
            vol = volume as f64,
            w = 1.0 - volume
        )
    };

    command
        .arg("-filter_complex")
        .arg(&filter)
        .arg("-map")
        .arg("0:v")
        .arg("-map")
        .arg("[aout]")
        .arg("-c:v")
        .arg("copy")
        .arg("-c:a")
        .arg("aac")
        .arg("-b:a")
        .arg("192k")
        .arg("-y")
        .arg(output_file);

    let out = command
        .output()
        .map_err(|e| format!("ffmpeg failed: {e}"))?;
    if out.status.success() {
        Ok(output_file.to_string())
    } else {
        Err(String::from_utf8_lossy(&out.stderr)
            .lines()
            .last()
            .unwrap_or("ffmpeg error")
            .to_string())
    }
}

/// Demo helper for JSON responses
pub fn music_tool_json(_args: &Value) -> Value {
    json!({})
}
