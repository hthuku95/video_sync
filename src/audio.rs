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

// ============================================================================
// BATCH 4 — Audio Processing
// ============================================================================

pub fn compress_audio(input_file: &str, output_file: &str, threshold: f64, ratio: f64, attack: f64, release: f64, makeup: f64) -> Result<String, String> {
    let filter = format!("acompressor=threshold={}dB:ratio={}:attack={}:release={}:makeup={}dB", threshold, ratio, attack, release, makeup);
    let mut command = Command::new("ffmpeg");
    command.arg("-i").arg(input_file).arg("-af").arg(filter).arg("-c:v").arg("copy").arg("-y").arg(output_file);
    execute_ffmpeg_command(command)
}

pub fn normalize_audio(input_file: &str, output_file: &str, target_lufs: f64, loudness_range: f64, true_peak: f64) -> Result<String, String> {
    let filter = format!("loudnorm=I={}:LRA={}:TP={}", target_lufs, loudness_range, true_peak);
    let mut command = Command::new("ffmpeg");
    command.arg("-i").arg(input_file).arg("-af").arg(filter).arg("-c:v").arg("copy").arg("-y").arg(output_file);
    execute_ffmpeg_command(command)
}

pub fn equalize_audio(input_file: &str, output_file: &str, frequency: f64, gain: f64, bandwidth: f64, eq_type: &str) -> Result<String, String> {
    // Valid eq_type/t values: h (Hz), q (Q-Factor), o (octave), s (slope), k (kHz)
    let t = match eq_type { "h" | "q" | "o" | "s" | "k" => eq_type, _ => "q" };
    let f = if frequency <= 0.0 { 1000.0 } else { frequency };
    let w = if bandwidth <= 0.0 { 1.0 } else { bandwidth };
    let filter = format!("equalizer=f={}:g={}:w={}:t={}", f, gain, w, t);
    let mut command = Command::new("ffmpeg");
    command.arg("-i").arg(input_file).arg("-af").arg(filter).arg("-c:v").arg("copy").arg("-y").arg(output_file);
    execute_ffmpeg_command(command)
}

pub fn gate_audio(input_file: &str, output_file: &str, threshold: f64, ratio: f64, attack: f64, release: f64) -> Result<String, String> {
    let filter = format!("agate=threshold={}dB:ratio={}:attack={}:release={}", threshold, ratio, attack, release);
    let mut command = Command::new("ffmpeg");
    command.arg("-i").arg(input_file).arg("-af").arg(filter).arg("-c:v").arg("copy").arg("-y").arg(output_file);
    execute_ffmpeg_command(command)
}

pub fn denoise_audio(input_file: &str, output_file: &str, noise_floor: f64, reduction: f64, track_noise: bool) -> Result<String, String> {
    let filter = format!("afftdn=noise_floor={}:noise_reduction={}:track_noise={}", noise_floor, reduction, if track_noise { 1 } else { 0 });
    let mut command = Command::new("ffmpeg");
    command.arg("-i").arg(input_file).arg("-af").arg(filter).arg("-c:v").arg("copy").arg("-y").arg(output_file);
    execute_ffmpeg_command(command)
}
// ============================================================================
// BATCH 5 — Audio Tone Shaping
// ============================================================================

pub fn filter_highpass(input_file: &str, output_file: &str, frequency: f64, poles: u32, width: f64) -> Result<String, String> {
    let filter = format!("highpass=f={}:p={}:w={}", frequency, poles, width);
    let mut command = Command::new("ffmpeg");
    command.arg("-i").arg(input_file).arg("-af").arg(filter).arg("-c:v").arg("copy").arg("-y").arg(output_file);
    execute_ffmpeg_command(command)
}

pub fn filter_lowpass(input_file: &str, output_file: &str, frequency: f64, poles: u32, width: f64) -> Result<String, String> {
    let filter = format!("lowpass=f={}:p={}:w={}", frequency, poles, width);
    let mut command = Command::new("ffmpeg");
    command.arg("-i").arg(input_file).arg("-af").arg(filter).arg("-c:v").arg("copy").arg("-y").arg(output_file);
    execute_ffmpeg_command(command)
}

pub fn adjust_bass(input_file: &str, output_file: &str, gain_db: f64, frequency: f64, width: f64) -> Result<String, String> {
    let filter = format!("bass=g={}:f={}:w={}", gain_db, frequency, width);
    let mut command = Command::new("ffmpeg");
    command.arg("-i").arg(input_file).arg("-af").arg(filter).arg("-c:v").arg("copy").arg("-y").arg(output_file);
    execute_ffmpeg_command(command)
}

pub fn adjust_treble(input_file: &str, output_file: &str, gain_db: f64, frequency: f64, width: f64) -> Result<String, String> {
    let filter = format!("treble=g={}:f={}:w={}", gain_db, frequency, width);
    let mut command = Command::new("ffmpeg");
    command.arg("-i").arg(input_file).arg("-af").arg(filter).arg("-c:v").arg("copy").arg("-y").arg(output_file);
    execute_ffmpeg_command(command)
}

pub fn audio_compand(input_file: &str, output_file: &str, attacks: &str, decays: &str, points: &str, soft_knee: f64, gain: f64) -> Result<String, String> {
    let filter = format!("compand=attacks={}:decays={}:points={}:soft-knee={}:gain={}", attacks, decays, points, soft_knee, gain);
    let mut command = Command::new("ffmpeg");
    command.arg("-i").arg(input_file).arg("-af").arg(filter).arg("-c:v").arg("copy").arg("-y").arg(output_file);
    execute_ffmpeg_command(command)
}

pub fn add_audio_delay(input_file: &str, output_file: &str, delays_ms: &str, all_channels: bool) -> Result<String, String> {
    let filter = if all_channels {
        format!("adelay=delays={}:all=1", delays_ms)
    } else {
        format!("adelay=delays={}", delays_ms)
    };
    let mut command = Command::new("ffmpeg");
    command.arg("-i").arg(input_file).arg("-af").arg(filter).arg("-c:v").arg("copy").arg("-y").arg(output_file);
    execute_ffmpeg_command(command)
}

pub fn add_phaser(input_file: &str, output_file: &str, in_gain: f64, out_gain: f64, delay: f64, decay: f64, speed: f64, phaser_type: &str) -> Result<String, String> {
    let filter = format!("aphaser=in_gain={}:out_gain={}:delay={}:decay={}:speed={}:type={}", in_gain, out_gain, delay, decay, speed, phaser_type);
    let mut command = Command::new("ffmpeg");
    command.arg("-i").arg(input_file).arg("-af").arg(filter).arg("-c:v").arg("copy").arg("-y").arg(output_file);
    execute_ffmpeg_command(command)
}

// ============================================================================
// BATCH 6 — Audio Restoration
// ============================================================================

pub fn remove_clicks(input_file: &str, output_file: &str, window: f64, overlap: f64, arorder: u32, threshold: f64) -> Result<String, String> {
    let filter = format!("adeclick=w={}:o={}:a={}:t={}", window, overlap, arorder, threshold);
    let mut command = Command::new("ffmpeg");
    command.arg("-i").arg(input_file).arg("-af").arg(filter).arg("-c:v").arg("copy").arg("-y").arg(output_file);
    execute_ffmpeg_command(command)
}

pub fn restore_clipping(input_file: &str, output_file: &str, window: f64, overlap: f64, arorder: u32, threshold: f64) -> Result<String, String> {
    let filter = format!("adeclip=w={}:o={}:a={}:t={}", window, overlap, arorder, threshold);
    let mut command = Command::new("ffmpeg");
    command.arg("-i").arg(input_file).arg("-af").arg(filter).arg("-c:v").arg("copy").arg("-y").arg(output_file);
    execute_ffmpeg_command(command)
}

pub fn remove_silence(input_file: &str, output_file: &str, start_periods: u32, start_threshold_db: f64, stop_periods: i32, stop_threshold_db: f64, stop_duration: f64) -> Result<String, String> {
    let filter = format!(
        "silenceremove=start_periods={}:start_threshold={}dB:stop_periods={}:stop_threshold={}dB:stop_duration={}",
        start_periods, start_threshold_db, stop_periods, stop_threshold_db, stop_duration
    );
    let mut command = Command::new("ffmpeg");
    command.arg("-i").arg(input_file).arg("-af").arg(filter).arg("-c:v").arg("copy").arg("-y").arg(output_file);
    execute_ffmpeg_command(command)
}

// ============================================================================
// BATCH 7 — Spatial Audio
// ============================================================================

pub fn adjust_stereo_width(input_file: &str, output_file: &str, width: f64, balance: f64, mode: &str) -> Result<String, String> {
    let filter = format!("stereotools=mlev={}:sbal={}:mode={}", width, balance, mode);
    let mut command = Command::new("ffmpeg");
    command.arg("-i").arg(input_file).arg("-af").arg(filter).arg("-c:v").arg("copy").arg("-y").arg(output_file);
    execute_ffmpeg_command(command)
}

pub fn apply_stereo_widen(input_file: &str, output_file: &str, delay_ms: f64, feedback: f64, crossfeed: f64, drymix: f64) -> Result<String, String> {
    let filter = format!("stereowiden=delay={}:feedback={}:crossfeed={}:drymix={}", delay_ms, feedback, crossfeed, drymix);
    let mut command = Command::new("ffmpeg");
    command.arg("-i").arg(input_file).arg("-af").arg(filter).arg("-c:v").arg("copy").arg("-y").arg(output_file);
    execute_ffmpeg_command(command)
}

pub fn mix_audio_channels(input_file: &str, output_file: &str, channel_layout: &str, channel_expressions: &[String]) -> Result<String, String> {
    let exprs = channel_expressions.join("|");
    let filter = format!("pan={}|{}", channel_layout, exprs);
    let mut command = Command::new("ffmpeg");
    command.arg("-i").arg(input_file).arg("-af").arg(filter).arg("-c:v").arg("copy").arg("-y").arg(output_file);
    execute_ffmpeg_command(command)
}

// ============================================================================
// PHASE D — Professional Finishing: Audio Tools
// ============================================================================

pub fn measure_lufs(input_file: &str, target_lufs: f64) -> Result<String, String> {
    let filter = format!("ebur128=target={}:metadata=1:peak=true", target_lufs as i32);
    let output = std::process::Command::new("ffmpeg")
        .arg("-i").arg(input_file)
        .arg("-af").arg(filter)
        .arg("-f").arg("null").arg("-")
        .output()
        .map_err(|e| format!("Failed to execute FFmpeg: {}", e))?;
    let stderr = String::from_utf8_lossy(&output.stderr);
    let mut result = Vec::new();
    for line in stderr.lines() {
        if line.contains("Integrated loudness") || line.contains("I:") || line.contains("LRA")
            || line.contains("True peak") || line.contains("Summary") || line.contains("LUFS")
            || line.contains("LU") || line.contains("dBFS") {
            result.push(line.trim().to_string());
        }
    }
    if result.is_empty() {
        Err("Could not measure LUFS — no audio stream or unsupported format".to_string())
    } else {
        Ok(result.join("\n"))
    }
}

pub fn parametric_eq(input_file: &str, output_file: &str, params: &str) -> Result<String, String> {
    let filter = format!("anequalizer={}", params);
    let mut command = Command::new("ffmpeg");
    command.arg("-i").arg(input_file).arg("-af").arg(filter).arg("-c:v").arg("copy").arg("-y").arg(output_file);
    execute_ffmpeg_command(command)
}

pub fn audio_limiter(input_file: &str, output_file: &str, limit: f64, attack: f64, release: f64, asc: bool) -> Result<String, String> {
    // Convert dB to linear amplitude (alimiter expects 0.0625–1.0 linear, not dB)
    let limit_linear = (10.0_f64.powf(limit / 20.0)).clamp(0.0625, 1.0);
    let filter = format!("alimiter=level_in=1:level_out=1:limit={}:attack={}:release={}:asc={}", limit_linear, attack, release, if asc { 1 } else { 0 });
    let mut command = Command::new("ffmpeg");
    command.arg("-i").arg(input_file).arg("-af").arg(filter).arg("-c:v").arg("copy").arg("-y").arg(output_file);
    execute_ffmpeg_command(command)
}

pub fn reduce_sibilance(input_file: &str, output_file: &str, split: f64, threshold: f64, omode: &str) -> Result<String, String> {
    // deesser params: f (0-1 normalized frequency), i (intensity 0-1), m (max deessing 0-1), s (output mode: i/o/e)
    let f_normalized = (split / 22050.0).clamp(0.0, 1.0);
    let m = threshold.clamp(0.0, 1.0);
    let s = match omode { "e" | "ess" => "e", "i" | "input" => "i", _ => "o" };
    let filter = format!("deesser=i=0.5:f={}:m={}:s={}", f_normalized, m, s);
    let mut command = Command::new("ffmpeg");
    command.arg("-i").arg(input_file).arg("-af").arg(filter).arg("-c:v").arg("copy").arg("-y").arg(output_file);
    execute_ffmpeg_command(command)
}

pub fn denoise_speech_rnn(input_file: &str, output_file: &str, model: &str, mix: f64) -> Result<String, String> {
    let filter = if model.is_empty() {
        format!("arnndn=mix={}", mix)
    } else {
        format!("arnndn=model='{}':mix={}", model, mix)
    };
    let mut command = Command::new("ffmpeg");
    command.arg("-i").arg(input_file).arg("-af").arg(filter).arg("-c:v").arg("copy").arg("-y").arg(output_file);
    execute_ffmpeg_command(command)
}

// ============================================================================
// PHASE E — Binaural, Modulation Effects, NLM Denoising
// ============================================================================

pub fn render_binaural(input_file: &str, output_file: &str, hrir_type: &str) -> Result<String, String> {
    let filter = if hrir_type.is_empty() || hrir_type == "stereo" {
        "headphone".to_string()
    } else {
        format!("headphone=hrir={}", hrir_type)
    };
    let mut command = Command::new("ffmpeg");
    command.arg("-i").arg(input_file).arg("-af").arg(filter).arg("-c:v").arg("copy").arg("-y").arg(output_file);
    execute_ffmpeg_command(command)
}

pub fn add_vibrato(input_file: &str, output_file: &str, frequency: f64, depth: f64) -> Result<String, String> {
    let filter = format!("vibrato=f={}:d={}", frequency, depth);
    let mut command = Command::new("ffmpeg");
    command.arg("-i").arg(input_file).arg("-af").arg(filter).arg("-c:v").arg("copy").arg("-y").arg(output_file);
    execute_ffmpeg_command(command)
}

pub fn add_tremolo(input_file: &str, output_file: &str, frequency: f64, depth: f64) -> Result<String, String> {
    let filter = format!("tremolo=f={}:d={}", frequency, depth);
    let mut command = Command::new("ffmpeg");
    command.arg("-i").arg(input_file).arg("-af").arg(filter).arg("-c:v").arg("copy").arg("-y").arg(output_file);
    execute_ffmpeg_command(command)
}

pub fn add_flanger(input_file: &str, output_file: &str, delay: f64, depth: f64, speed: f64, shape: &str) -> Result<String, String> {
    let filter = format!("flanger=delay={}:depth={}:speed={}:shape={}", delay, depth, speed, shape);
    let mut command = Command::new("ffmpeg");
    command.arg("-i").arg(input_file).arg("-af").arg(filter).arg("-c:v").arg("copy").arg("-y").arg(output_file);
    execute_ffmpeg_command(command)
}

pub fn denoise_audio_nlm(input_file: &str, output_file: &str, strength: f64, patch_size: f64, research_size: f64) -> Result<String, String> {
    let filter = format!("anlmdn=s={}:p={}:r={}", strength, patch_size, research_size);
    let mut command = Command::new("ffmpeg");
    command.arg("-i").arg(input_file).arg("-af").arg(filter).arg("-c:v").arg("copy").arg("-y").arg(output_file);
    execute_ffmpeg_command(command)
}

// ============================================================================
// PHASE F — Niche/Specialised Audio Tools
// ============================================================================

pub fn shift_audio_frequency(input_file: &str, output_file: &str, shift: f64) -> Result<String, String> {
    let filter = format!("afreqshift=shift={}", shift);
    let mut command = Command::new("ffmpeg");
    command.arg("-i").arg(input_file).arg("-af").arg(filter).arg("-c:v").arg("copy").arg("-y").arg(output_file);
    execute_ffmpeg_command(command)
}

pub fn apply_audio_pulsator(input_file: &str, output_file: &str, hz: f64, amount: f64, offset_l: f64, offset_r: f64, mode: &str) -> Result<String, String> {
    let filter = format!("apulsator=hz={}:amount={}:offset_l={}:offset_r={}:mode={}", hz, amount, offset_l, offset_r, mode);
    let mut command = Command::new("ffmpeg");
    command.arg("-i").arg(input_file).arg("-af").arg(filter).arg("-c:v").arg("copy").arg("-y").arg(output_file);
    execute_ffmpeg_command(command)
}

pub fn enhance_dialogue(input_file: &str, output_file: &str, original: f64, expand: f64) -> Result<String, String> {
    let filter = format!("dialoguenhance=original={}:expand={}", original, expand);
    let mut command = Command::new("ffmpeg");
    command.arg("-i").arg(input_file).arg("-af").arg(filter).arg("-c:v").arg("copy").arg("-y").arg(output_file);
    execute_ffmpeg_command(command)
}

pub fn split_audio_channels(input_file: &str, output_file: &str, channel_layout: &str, channel: &str) -> Result<String, String> {
    let filter = format!("channelsplit=channel_layout={}[{ch}]", channel_layout, ch = channel);
    let mut command = Command::new("ffmpeg");
    command.arg("-i").arg(input_file)
           .arg("-filter_complex").arg(filter)
           .arg("-map").arg(format!("[{}]", channel))
           .arg("-y").arg(output_file);
    execute_ffmpeg_command(command)
}

pub fn map_audio_channels(input_file: &str, output_file: &str, channel_map: &str, channel_layout: &str) -> Result<String, String> {
    let filter = format!("channelmap=map={}:channel_layout={}", channel_map, channel_layout);
    let mut command = Command::new("ffmpeg");
    command.arg("-i").arg(input_file).arg("-af").arg(filter).arg("-c:v").arg("copy").arg("-y").arg(output_file);
    execute_ffmpeg_command(command)
}

pub fn merge_audio_inputs(input_files: &[String], output_file: &str) -> Result<String, String> {
    let n = input_files.len();
    if n < 2 {
        return Err("merge_audio_inputs requires at least 2 input files".to_string());
    }
    let mut fc = String::new();
    for i in 0..n {
        fc.push_str(&format!("[{}:a]", i));
    }
    fc.push_str(&format!("amerge=inputs={}", n));
    let mut command = Command::new("ffmpeg");
    for f in input_files {
        command.arg("-i").arg(f);
    }
    command.arg("-filter_complex").arg(fc).arg("-y").arg(output_file);
    execute_ffmpeg_command(command)
}

pub fn apply_crossfeed(input_file: &str, output_file: &str, strength: f64, slope: f64, level_in: f64, level_out: f64) -> Result<String, String> {
    let filter = format!("crossfeed=strength={}:slope={}:level_in={}:level_out={}", strength, slope, level_in, level_out);
    let mut command = Command::new("ffmpeg");
    command.arg("-i").arg(input_file).arg("-af").arg(filter).arg("-c:v").arg("copy").arg("-y").arg(output_file);
    execute_ffmpeg_command(command)
}

pub fn apply_extrastereo(input_file: &str, output_file: &str, multiplier: f64, clipping: bool) -> Result<String, String> {
    let filter = format!("extrastereo=m={}:c={}", multiplier, if clipping { 1 } else { 0 });
    let mut command = Command::new("ffmpeg");
    command.arg("-i").arg(input_file).arg("-af").arg(filter).arg("-c:v").arg("copy").arg("-y").arg(output_file);
    execute_ffmpeg_command(command)
}

pub fn apply_firequalizer(input_file: &str, output_file: &str, gain_entry: &str) -> Result<String, String> {
    let filter = format!("firequalizer=gain_entry='{}'", gain_entry);
    let mut command = Command::new("ffmpeg");
    command.arg("-i").arg(input_file).arg("-af").arg(filter).arg("-c:v").arg("copy").arg("-y").arg(output_file);
    execute_ffmpeg_command(command)
}

pub fn apply_biquad(input_file: &str, output_file: &str, b0: f64, b1: f64, b2: f64, a0: f64, a1: f64, a2: f64) -> Result<String, String> {
    let filter = format!("biquad=b0={}:b1={}:b2={}:a0={}:a1={}:a2={}", b0, b1, b2, a0, a1, a2);
    let mut command = Command::new("ffmpeg");
    command.arg("-i").arg(input_file).arg("-af").arg(filter).arg("-c:v").arg("copy").arg("-y").arg(output_file);
    execute_ffmpeg_command(command)
}

pub fn filter_bandpass(input_file: &str, output_file: &str, frequency: f64, width: f64, width_type: &str) -> Result<String, String> {
    let filter = format!("bandpass=frequency={}:width={}:width_type={}", frequency, width, width_type);
    let mut command = Command::new("ffmpeg");
    command.arg("-i").arg(input_file).arg("-af").arg(filter).arg("-c:v").arg("copy").arg("-y").arg(output_file);
    execute_ffmpeg_command(command)
}

pub fn filter_bandreject(input_file: &str, output_file: &str, frequency: f64, width: f64, width_type: &str) -> Result<String, String> {
    let filter = format!("bandreject=frequency={}:width={}:width_type={}", frequency, width, width_type);
    let mut command = Command::new("ffmpeg");
    command.arg("-i").arg(input_file).arg("-af").arg(filter).arg("-c:v").arg("copy").arg("-y").arg(output_file);
    execute_ffmpeg_command(command)
}

pub fn boost_sub_bass(input_file: &str, output_file: &str, dry: f64, wet: f64, freq: f64, decay: f64) -> Result<String, String> {
    // asubboost uses cutoff (50-900 Hz), not freq; dry/wet are 0-1
    let cutoff = freq.clamp(50.0, 900.0);
    let d = dry.clamp(0.0, 1.0);
    let w = wet.clamp(0.0, 1.0);
    let dc = decay.clamp(0.0, 1.0);
    let filter = format!("asubboost=dry={}:wet={}:cutoff={}:decay={}", d, w, cutoff, dc);
    let mut command = Command::new("ffmpeg");
    command.arg("-i").arg(input_file).arg("-af").arg(filter).arg("-c:v").arg("copy").arg("-y").arg(output_file);
    execute_ffmpeg_command(command)
}

// ================================================================
// PHASE I — Long-tail sweep, Batch 1 (audio filters)
// ================================================================

/// Echo / delay effect using FFmpeg aecho filter.
pub fn add_echo(input_file: &str, output_file: &str, in_gain: f64, out_gain: f64, delays: &str, decays: &str) -> Result<String, String> {
    let ig = if in_gain <= 0.0 { 0.6 } else { in_gain };
    let og = if out_gain <= 0.0 { 0.3 } else { out_gain };
    let d = if delays.is_empty() { "1000" } else { delays };
    let dc = if decays.is_empty() { "0.5" } else { decays };
    let filter = format!("aecho={}:{}:{}:{}", ig, og, d, dc);
    let mut command = Command::new("ffmpeg");
    command.arg("-i").arg(input_file).arg("-af").arg(filter).arg("-c:v").arg("copy").arg("-y").arg(output_file);
    execute_ffmpeg_command(command)
}

/// Noise gate using FFmpeg agate filter. Mutes audio below a threshold to eliminate background noise.
pub fn noise_gate(input_file: &str, output_file: &str, threshold: f64, range: f64, attack: f64, release: f64) -> Result<String, String> {
    let th = if threshold <= 0.0 { 0.01 } else { threshold };
    let ra = if range <= 0.0 { 0.06125 } else { range };
    let at = if attack <= 0.0 { 20.0 } else { attack };
    let re = if release <= 0.0 { 250.0 } else { release };
    let filter = format!("agate=threshold={}:range={}:attack={}:release={}", th, ra, at, re);
    let mut command = Command::new("ffmpeg");
    command.arg("-i").arg(input_file).arg("-af").arg(filter).arg("-c:v").arg("copy").arg("-y").arg(output_file);
    execute_ffmpeg_command(command)
}

/// Dynamic range compression using FFmpeg acompressor filter.
pub fn compress_dynamics(input_file: &str, output_file: &str, threshold: f64, ratio: f64, attack: f64, release: f64, makeup: f64) -> Result<String, String> {
    let th = if threshold <= 0.0 { 0.125 } else { threshold };
    let ra = if ratio <= 0.0 { 4.0 } else { ratio };
    let at = if attack <= 0.0 { 20.0 } else { attack };
    let re = if release <= 0.0 { 250.0 } else { release };
    let mk = if makeup <= 0.0 { 1.0 } else { makeup };
    let filter = format!("acompressor=threshold={}:ratio={}:attack={}:release={}:makeup={}", th, ra, at, re, mk);
    let mut command = Command::new("ffmpeg");
    command.arg("-i").arg(input_file).arg("-af").arg(filter).arg("-c:v").arg("copy").arg("-y").arg(output_file);
    execute_ffmpeg_command(command)
}

/// Chorus effect using FFmpeg chorus filter. Adds shimmer and width to vocals/instruments.
pub fn add_chorus(input_file: &str, output_file: &str, in_gain: f64, out_gain: f64, delays: &str, decays: &str, speeds: &str, depths: &str) -> Result<String, String> {
    let ig = if in_gain <= 0.0 { 0.4 } else { in_gain };
    let og = if out_gain <= 0.0 { 0.4 } else { out_gain };
    let d = if delays.is_empty() { "55" } else { delays };
    let dc = if decays.is_empty() { "0.4" } else { decays };
    let sp = if speeds.is_empty() { "0.25" } else { speeds };
    let dp = if depths.is_empty() { "2" } else { depths };
    let filter = format!("chorus={}:{}:{}:{}:{}:{}", ig, og, d, dc, sp, dp);
    let mut command = Command::new("ffmpeg");
    command.arg("-i").arg(input_file).arg("-af").arg(filter).arg("-c:v").arg("copy").arg("-y").arg(output_file);
    execute_ffmpeg_command(command)
}

/// Stereo widening using FFmpeg stereowiden filter.
pub fn widen_stereo(input_file: &str, output_file: &str, delay: f64, feedback: f64, crossfeed: f64, drymix: f64) -> Result<String, String> {
    let dl = if delay <= 0.0 { 20.0 } else { delay };
    let fb = if feedback < 0.0 { 0.0 } else { feedback };
    let cf = if crossfeed < 0.0 { 0.0 } else { crossfeed };
    let dm = if drymix <= 0.0 { 0.8 } else { drymix };
    let filter = format!("stereowiden=delay={}:feedback={}:crossfeed={}:drymix={}", dl, fb, cf, dm);
    let mut command = Command::new("ffmpeg");
    command.arg("-i").arg(input_file).arg("-af").arg(filter).arg("-c:v").arg("copy").arg("-y").arg(output_file);
    execute_ffmpeg_command(command)
}

/// Speech volume normalisation using FFmpeg speechnorm filter.
pub fn normalize_speech(input_file: &str, output_file: &str, peak: f64, _strength: f64) -> Result<String, String> {
    // speechnorm: peak (0-1), expansion (e, min 1), raise rate (r).
    // strength was misnamed — speechnorm doesn't have that param.
    let p = if peak <= 0.0 || peak > 1.0 { 0.95 } else { peak };
    let filter = format!("speechnorm=p={}", p);
    let mut command = Command::new("ffmpeg");
    command.arg("-i").arg(input_file).arg("-af").arg(filter).arg("-c:v").arg("copy").arg("-y").arg(output_file);
    execute_ffmpeg_command(command)
}

/// Remove silence using FFmpeg silenceremove filter (simple threshold/duration interface).
pub fn remove_silence_simple(input_file: &str, output_file: &str, threshold: f64, duration: f64, periods: i32) -> Result<String, String> {
    let th = if threshold <= 0.0 { 0.02 } else { threshold };
    let dur = if duration <= 0.0 { 0.5 } else { duration };
    let p = if periods == 0 { 1 } else { periods };
    let filter = format!("silenceremove=start_periods={}:start_duration={}:start_threshold={}", p, dur, th);
    let mut command = Command::new("ffmpeg");
    command.arg("-i").arg(input_file).arg("-af").arg(filter).arg("-c:v").arg("copy").arg("-y").arg(output_file);
    execute_ffmpeg_command(command)
}

/// Soft audio clipping using FFmpeg asoftclip filter. Prevents harsh digital clipping.
pub fn soft_clip_audio(input_file: &str, output_file: &str, clip_type: &str, param: f64) -> Result<String, String> {
    let ct = if clip_type.is_empty() { "tanh" } else { clip_type };
    let p = if param <= 0.0 { 1.0 } else { param };
    let filter = format!("asoftclip=type={}:param={}", ct, p);
    let mut command = Command::new("ffmpeg");
    command.arg("-i").arg(input_file).arg("-af").arg(filter).arg("-c:v").arg("copy").arg("-y").arg(output_file);
    execute_ffmpeg_command(command)
}

// ============================================================================
// PHASE I BATCH 2 — Long-tail: reverse audio, amix, silencedetect, showspectrum
// ============================================================================

/// Reverses the audio stream using FFmpeg areverse filter.
pub fn reverse_audio(input_file: &str, output_file: &str) -> Result<String, String> {
    let mut command = Command::new("ffmpeg");
    command.arg("-i").arg(input_file).arg("-af").arg("areverse").arg("-c:v").arg("copy").arg("-y").arg(output_file);
    execute_ffmpeg_command(command)
}

/// Mixes two audio inputs using FFmpeg amix filter. Blends a secondary audio file into the primary.
pub fn blend_audio_streams(input_file: &str, secondary_file: &str, output_file: &str, duration: &str, dropout_transition: f64) -> Result<String, String> {
    let dur = if duration.is_empty() { "longest" } else { duration };
    let dt = if dropout_transition < 0.0 { 2.0 } else { dropout_transition };
    let filter = format!("amix=inputs=2:duration={}:dropout_transition={}", dur, dt);
    let mut command = Command::new("ffmpeg");
    command.arg("-i").arg(input_file).arg("-i").arg(secondary_file)
        .arg("-filter_complex").arg(filter).arg("-y").arg(output_file);
    execute_ffmpeg_command(command)
}

/// Detects silence using FFmpeg silencedetect. Returns timestamps of silent regions (analysis only).
pub fn measure_silence(input_file: &str, noise_db: f64, duration_s: f64) -> Result<String, String> {
    let noise = if noise_db >= 0.0 { -30.0 } else { noise_db };
    let dur = if duration_s <= 0.0 { 0.5 } else { duration_s };
    let filter = format!("silencedetect=noise={}dB:duration={}", noise, dur);
    let output = std::process::Command::new("ffmpeg")
        .arg("-i").arg(input_file)
        .arg("-af").arg(filter)
        .arg("-f").arg("null").arg("-")
        .output()
        .map_err(|e| format!("Failed to execute FFmpeg: {}", e))?;
    let stderr = String::from_utf8_lossy(&output.stderr);
    let mut lines = Vec::new();
    for line in stderr.lines() {
        if line.contains("silence_start") || line.contains("silence_end") || line.contains("silence_duration") {
            lines.push(line.trim().to_string());
        }
    }
    if lines.is_empty() {
        Ok("No silence detected (or no audio stream found)".to_string())
    } else {
        Ok(lines.join("\n"))
    }
}

/// Renders audio amplitude waveform as a video using FFmpeg showwaves filter. Shows waveform shape over time.
pub fn generate_waveform_video(input_file: &str, output_file: &str, width: u32, height: u32, mode: &str, color: &str) -> Result<String, String> {
    let w = if width == 0 { 1280 } else { width };
    let h = if height == 0 { 240 } else { height };
    let m = if mode.is_empty() { "line" } else { mode }; // line, point, p2p, cline
    let c = if color.is_empty() { "white" } else { color };
    let filter = format!("[0:a]showwaves=size={}x{}:mode={}:colors={}[v]", w, h, m, c);
    let mut command = Command::new("ffmpeg");
    command.arg("-i").arg(input_file)
        .arg("-filter_complex").arg(filter)
        .arg("-map").arg("[v]")
        .arg("-y").arg(output_file);
    execute_ffmpeg_command(command)
}

/// Applies Haas stereo effect — delays one channel slightly for spatial widening.
pub fn apply_haas(input_file: &str, output_file: &str, level_in: f64, level_out: f64, side_gain: f64, middle_source: &str, middle_phase: bool, left_delay: f64, left_balance: f64, right_delay: f64, right_balance: f64) -> Result<String, String> {
    let ms = if middle_source.is_empty() { "mid" } else { middle_source };
    let filter = format!(
        "haas=level_in={}:level_out={}:side_gain={}:middle_source={}:middle_phase={}:left_delay={}:left_balance={}:right_delay={}:right_balance={}",
        level_in, level_out, side_gain, ms,
        if middle_phase { 1 } else { 0 },
        left_delay, left_balance, right_delay, right_balance
    );
    let mut command = Command::new("ffmpeg");
    command.arg("-i").arg(input_file).arg("-af").arg(filter).arg("-c:v").arg("copy").arg("-y").arg(output_file);
    execute_ffmpeg_command(command)
}

/// Applies audio emphasis/de-emphasis curves (aemphasis) for pre/de-emphasis processing.
pub fn apply_aemphasis(input_file: &str, output_file: &str, level_in: f64, level_out: f64, mode: &str, emph_type: &str) -> Result<String, String> {
    let m = if mode.is_empty() { "reproduction" } else { mode }; // production or reproduction
    let t = if emph_type.is_empty() { "cd" } else { emph_type }; // riaa, cd, 50fm, 75fm, 50kf, 75kf
    let filter = format!("aemphasis=level_in={}:level_out={}:mode={}:type={}", level_in, level_out, m, t);
    let mut command = Command::new("ffmpeg");
    command.arg("-i").arg(input_file).arg("-af").arg(filter).arg("-c:v").arg("copy").arg("-y").arg(output_file);
    execute_ffmpeg_command(command)
}

/// Applies bit-crusher/lo-fi distortion effect using FFmpeg acrusher filter.
pub fn apply_acrusher(input_file: &str, output_file: &str, level_in: f64, level_out: f64, bits: f64, mix: f64, mode: &str, dc: f64, aa: f64, samples: f64, lfo: bool, lforange: f64, lforate: f64) -> Result<String, String> {
    let m = if mode.is_empty() { "log" } else { mode }; // lin or log
    let filter = format!(
        "acrusher=level_in={}:level_out={}:bits={}:mix={}:mode={}:dc={}:aa={}:samples={}:lfo={}:lforange={}:lforate={}",
        level_in, level_out, bits, mix, m, dc, aa, samples,
        if lfo { 1 } else { 0 }, lforange, lforate
    );
    let mut command = Command::new("ffmpeg");
    command.arg("-i").arg(input_file).arg("-af").arg(filter).arg("-c:v").arg("copy").arg("-y").arg(output_file);
    execute_ffmpeg_command(command)
}

/// Changes audio tempo without pitch shift using FFmpeg atempo filter (0.5–100x speed, chained for extremes).
pub fn apply_atempo(input_file: &str, output_file: &str, tempo: f64) -> Result<String, String> {
    let t = tempo.clamp(0.5, 100.0);
    // For tempos outside 0.5-2.0, chain multiple atempo filters
    let filter = if t <= 2.0 && t >= 0.5 {
        format!("atempo={}", t)
    } else if t > 2.0 {
        // chain: atempo=2.0,atempo=remaining
        let remaining = t / 2.0;
        if remaining <= 2.0 {
            format!("atempo=2.0,atempo={}", remaining)
        } else {
            format!("atempo=2.0,atempo=2.0,atempo={}", t / 4.0)
        }
    } else {
        // t < 0.5
        let remaining = t / 0.5;
        format!("atempo=0.5,atempo={}", remaining)
    };
    let mut command = Command::new("ffmpeg");
    command.arg("-i").arg(input_file).arg("-af").arg(filter).arg("-c:v").arg("copy").arg("-y").arg(output_file);
    execute_ffmpeg_command(command)
}

/// Sets a fixed number of samples per audio frame using asetnsamples — useful for downstream processing alignment.
pub fn apply_asetnsamples(input_file: &str, output_file: &str, nb_samples: u32, pad: bool) -> Result<String, String> {
    let n = if nb_samples == 0 { 1024 } else { nb_samples };
    let filter = format!("asetnsamples=nb_out_samples={}:pad={}", n, if pad { 1 } else { 0 });
    let mut command = Command::new("ffmpeg");
    command.arg("-i").arg(input_file).arg("-af").arg(filter).arg("-c:v").arg("copy").arg("-y").arg(output_file);
    execute_ffmpeg_command(command)
}

/// Pads audio with silence at the end using apad — useful for ensuring minimum duration.
pub fn apply_apad(input_file: &str, output_file: &str, packet_size: u32, pad_len: i64, whole_len: i64, pad_dur: f64, whole_dur: f64) -> Result<String, String> {
    let mut parts = vec![format!("packet_size={}", if packet_size == 0 { 4096 } else { packet_size })];
    if pad_len > 0 { parts.push(format!("pad_len={}", pad_len)); }
    if whole_len > 0 { parts.push(format!("whole_len={}", whole_len)); }
    if pad_dur > 0.0 { parts.push(format!("pad_dur={}", pad_dur)); }
    if whole_dur > 0.0 { parts.push(format!("whole_dur={}", whole_dur)); }
    let filter = format!("apad={}", parts.join(":"));
    let mut command = Command::new("ffmpeg");
    command.arg("-i").arg(input_file).arg("-af").arg(filter).arg("-c:v").arg("copy").arg("-y").arg(output_file);
    execute_ffmpeg_command(command)
}

/// Cuts low sub-bass frequencies below a cutoff using asubcut (high-pass for sub frequencies).
pub fn apply_asubcut(input_file: &str, output_file: &str, cutoff: f64, order: u32, level: f64) -> Result<String, String> {
    let c = if cutoff <= 0.0 { 20.0 } else { cutoff };
    let o = if order == 0 { 10 } else { order };
    let filter = format!("asubcut=cutoff={}:order={}:level={}", c, o, level);
    let mut command = Command::new("ffmpeg");
    command.arg("-i").arg(input_file).arg("-af").arg(filter).arg("-c:v").arg("copy").arg("-y").arg(output_file);
    execute_ffmpeg_command(command)
}

/// Cuts high super-treble frequencies above a cutoff using asupercut (low-pass for super-treble frequencies).
pub fn apply_asupercut(input_file: &str, output_file: &str, cutoff: f64, order: u32, level: f64) -> Result<String, String> {
    let c = if cutoff <= 0.0 { 20000.0 } else { cutoff };
    let o = if order == 0 { 10 } else { order };
    let filter = format!("asupercut=cutoff={}:order={}:level={}", c, o, level);
    let mut command = Command::new("ffmpeg");
    command.arg("-i").arg(input_file).arg("-af").arg(filter).arg("-c:v").arg("copy").arg("-y").arg(output_file);
    execute_ffmpeg_command(command)
}

/// Renders audio frequency spectrum as a video file using FFmpeg showspectrum filter.
pub fn normalize_loudness(input_file: &str, output_file: &str, i: f64, lra: f64, tp: f64) -> Result<String, String> {
    let integrated = if i == 0.0 { -23.0 } else { i };
    let filter = format!("loudnorm=I={}:LRA={}:TP={}", integrated, lra, tp);
    let mut command = Command::new("ffmpeg");
    command.arg("-i").arg(input_file).arg("-af").arg(filter).arg("-c:v").arg("copy").arg("-y").arg(output_file);
    execute_ffmpeg_command(command)
}

pub fn dynamic_audio_normalize(input_file: &str, output_file: &str, frame_len: u32, gausssize: u32, peak: f64, max_gain: f64, rms: f64, coupling: bool) -> Result<String, String> {
    let fl = if frame_len == 0 { 500 } else { frame_len };
    let gs = if gausssize == 0 { 31 } else { gausssize };
    let p = if peak <= 0.0 { 0.95 } else { peak };
    let mg = if max_gain <= 0.0 { 10.0 } else { max_gain };
    let filter = format!("dynaudnorm=f={}:g={}:p={}:m={}:r={}:n={}", fl, gs, p, mg, rms, if coupling { 1 } else { 0 });
    let mut command = Command::new("ffmpeg");
    command.arg("-i").arg(input_file).arg("-af").arg(filter).arg("-c:v").arg("copy").arg("-y").arg(output_file);
    execute_ffmpeg_command(command)
}

pub fn resample_audio(input_file: &str, output_file: &str, sample_rate: u32, resampler: &str) -> Result<String, String> {
    let sr = if sample_rate == 0 { 44100 } else { sample_rate };
    let r = if resampler.is_empty() { "swr" } else { resampler };
    let filter = format!("aresample={}:resampler={}", sr, r);
    let mut command = Command::new("ffmpeg");
    command.arg("-i").arg(input_file).arg("-af").arg(filter).arg("-c:v").arg("copy").arg("-y").arg(output_file);
    execute_ffmpeg_command(command)
}

pub fn trim_audio(input_file: &str, output_file: &str, start: f64, end: f64, duration: f64) -> Result<String, String> {
    let filter = if duration > 0.0 {
        format!("atrim=start={}:duration={},asetpts=PTS-STARTPTS", start, duration)
    } else if end > 0.0 {
        format!("atrim=start={}:end={},asetpts=PTS-STARTPTS", start, end)
    } else {
        format!("atrim=start={},asetpts=PTS-STARTPTS", start)
    };
    let mut command = Command::new("ffmpeg");
    command.arg("-i").arg(input_file).arg("-af").arg(&filter).arg("-c:v").arg("copy").arg("-y").arg(output_file);
    execute_ffmpeg_command(command)
}

pub fn apply_crystalizer(input_file: &str, output_file: &str, i: f64, clip: bool) -> Result<String, String> {
    let intensity = if i == 0.0 { 2.0 } else { i };
    let filter = format!("crystalizer=i={}:c={}", intensity, if clip { 1 } else { 0 });
    let mut command = Command::new("ffmpeg");
    command.arg("-i").arg(input_file).arg("-af").arg(filter).arg("-c:v").arg("copy").arg("-y").arg(output_file);
    execute_ffmpeg_command(command)
}

pub fn multiband_compress(input_file: &str, output_file: &str, params: &str) -> Result<String, String> {
    // params: mcompand band spec, e.g. "0.005 0.1 -47/-40 -34/-34 -17/-17 0/0 2 500 ..."
    let default_params = "0.005 0.1 -47/-40 -34/-34 -17/-17 0/0 2 500 0.003 0.05 -47/-40 -34/-34 -17/-17 0/0 2";
    let p = if params.is_empty() { default_params } else { params };
    let filter = format!("mcompand={}", p);
    let mut command = Command::new("ffmpeg");
    command.arg("-i").arg(input_file).arg("-af").arg(filter).arg("-c:v").arg("copy").arg("-y").arg(output_file);
    execute_ffmpeg_command(command)
}

pub fn apply_super_equalizer(input_file: &str, output_file: &str, bands: &str) -> Result<String, String> {
    // 18-band eq: 65Hz to 18kHz, each band 0.0–20.0, default 10.0 (unity)
    let default_bands = "1b=10.0:2b=10.0:3b=10.0:4b=10.0:5b=10.0:6b=10.0:7b=10.0:8b=10.0:9b=10.0:10b=10.0:11b=10.0:12b=10.0:13b=10.0:14b=10.0:15b=10.0:16b=10.0:17b=10.0:18b=10.0";
    let b = if bands.is_empty() { default_bands } else { bands };
    let filter = format!("superequalizer={}", b);
    let mut command = Command::new("ffmpeg");
    command.arg("-i").arg(input_file).arg("-af").arg(filter).arg("-c:v").arg("copy").arg("-y").arg(output_file);
    execute_ffmpeg_command(command)
}

pub fn denoise_audio_fft(input_file: &str, output_file: &str, noise_floor: f64, noise_reduction: f64, track_noise: bool) -> Result<String, String> {
    let nf = if noise_floor == 0.0 { -25.0 } else { noise_floor };
    let nr = if noise_reduction == 0.0 { 12.0 } else { noise_reduction };
    let filter = format!("afftdn=nf={}:nr={}:tn={}", nf, nr, if track_noise { 1 } else { 0 });
    let mut command = Command::new("ffmpeg");
    command.arg("-i").arg(input_file).arg("-af").arg(filter).arg("-c:v").arg("copy").arg("-y").arg(output_file);
    execute_ffmpeg_command(command)
}

pub fn loop_audio(input_file: &str, output_file: &str, loop_count: i32, size: u32, start: u32) -> Result<String, String> {
    let filter = format!("aloop=loop={}:size={}:start={}", loop_count, size, start);
    let mut command = Command::new("ffmpeg");
    command.arg("-i").arg(input_file).arg("-af").arg(filter).arg("-c:v").arg("copy").arg("-y").arg(output_file);
    execute_ffmpeg_command(command)
}

pub fn apply_dc_shift(input_file: &str, output_file: &str, shift: f64, limitergain: f64) -> Result<String, String> {
    let filter = if limitergain > 0.0 {
        format!("dcshift=shift={}:limitergain={}", shift, limitergain)
    } else {
        format!("dcshift=shift={}", shift)
    };
    let mut command = Command::new("ffmpeg");
    command.arg("-i").arg(input_file).arg("-af").arg(filter).arg("-c:v").arg("copy").arg("-y").arg(output_file);
    execute_ffmpeg_command(command)
}

pub fn measure_dynamic_range(input_file: &str) -> Result<String, String> {
    let output = std::process::Command::new("ffmpeg")
        .arg("-i").arg(input_file)
        .arg("-af").arg("drmeter")
        .arg("-f").arg("null").arg("-")
        .output()
        .map_err(|e| format!("Failed to execute FFmpeg: {}", e))?;
    let stderr = String::from_utf8_lossy(&output.stderr);
    let mut result = Vec::new();
    for line in stderr.lines() {
        if line.contains("DR") || line.contains("Peak") || line.contains("RMS") || line.contains("drmeter") {
            result.push(line.trim().to_string());
        }
    }
    if result.is_empty() {
        Ok(stderr.lines().filter(|l| !l.is_empty()).take(20).collect::<Vec<_>>().join("\n"))
    } else {
        Ok(result.join("\n"))
    }
}

pub fn apply_single_eq_band(input_file: &str, output_file: &str, frequency: f64, width: f64, gain: f64, width_type: &str) -> Result<String, String> {
    let wt = if width_type.is_empty() { "o" } else { width_type };
    let filter = format!("equalizer=f={}:t={}:w={}:g={}", frequency, wt, width, gain);
    let mut command = Command::new("ffmpeg");
    command.arg("-i").arg(input_file).arg("-af").arg(filter).arg("-c:v").arg("copy").arg("-y").arg(output_file);
    execute_ffmpeg_command(command)
}

pub fn apply_stereotools(input_file: &str, output_file: &str, level_in: f64, level_out: f64, balance_in: f64, balance_out: f64, softclip: bool, mutel: bool, muter: bool, phasel: bool, phaser: bool, mode: &str) -> Result<String, String> {
    let m = if mode.is_empty() { "lr>lr" } else { mode };
    let filter = format!(
        "stereotools=level_in={}:level_out={}:balance_in={}:balance_out={}:softclip={}:mutel={}:muter={}:phasel={}:phaser={}:mode={}",
        level_in, level_out, balance_in, balance_out,
        if softclip { 1 } else { 0 },
        if mutel { 1 } else { 0 },
        if muter { 1 } else { 0 },
        if phasel { 1 } else { 0 },
        if phaser { 1 } else { 0 },
        m
    );
    let mut command = Command::new("ffmpeg");
    command.arg("-i").arg(input_file).arg("-af").arg(filter).arg("-c:v").arg("copy").arg("-y").arg(output_file);
    execute_ffmpeg_command(command)
}

pub fn apply_asetrate(input_file: &str, output_file: &str, sample_rate: u32) -> Result<String, String> {
    // asetrate changes sample rate metadata without resampling, shifting pitch+speed together
    let sr = if sample_rate == 0 { 44100 } else { sample_rate };
    let filter = format!("asetrate={}", sr);
    let mut command = Command::new("ffmpeg");
    command.arg("-i").arg(input_file).arg("-af").arg(filter).arg("-c:v").arg("copy").arg("-y").arg(output_file);
    execute_ffmpeg_command(command)
}

pub fn apply_compensation_delay(input_file: &str, output_file: &str, mm: f64, cm: f64, m: f64, dry: f64, wet: f64, temp: f64) -> Result<String, String> {
    let temperature = if temp == 0.0 { 20.0 } else { temp };
    let filter = format!("compensationdelay=mm={}:cm={}:m={}:dry={}:wet={}:temp={}", mm, cm, m, dry, wet, temperature);
    let mut command = Command::new("ffmpeg");
    command.arg("-i").arg(input_file).arg("-af").arg(filter).arg("-c:v").arg("copy").arg("-y").arg(output_file);
    execute_ffmpeg_command(command)
}

pub fn apply_earwax(input_file: &str, output_file: &str) -> Result<String, String> {
    let mut command = Command::new("ffmpeg");
    command.arg("-i").arg(input_file).arg("-af").arg("earwax").arg("-c:v").arg("copy").arg("-y").arg(output_file);
    execute_ffmpeg_command(command)
}

pub fn apply_allpass_filter(input_file: &str, output_file: &str, frequency: f64, width: f64, width_type: &str, mix: f64) -> Result<String, String> {
    let wt = if width_type.is_empty() { "q" } else { width_type };
    let filter = format!("allpass=f={}:t={}:w={}:m={}", frequency, wt, width, mix);
    let mut command = Command::new("ffmpeg");
    command.arg("-i").arg(input_file).arg("-af").arg(filter).arg("-c:v").arg("copy").arg("-y").arg(output_file);
    execute_ffmpeg_command(command)
}

pub fn apply_highshelf(input_file: &str, output_file: &str, frequency: f64, gain: f64, width: f64, width_type: &str, poles: u32) -> Result<String, String> {
    let wt = if width_type.is_empty() { "s" } else { width_type };
    let p = if poles == 0 { 2 } else { poles };
    let filter = format!("highshelf=f={}:g={}:t={}:w={}:p={}", frequency, gain, wt, width, p);
    let mut command = Command::new("ffmpeg");
    command.arg("-i").arg(input_file).arg("-af").arg(filter).arg("-c:v").arg("copy").arg("-y").arg(output_file);
    execute_ffmpeg_command(command)
}

pub fn apply_lowshelf(input_file: &str, output_file: &str, frequency: f64, gain: f64, width: f64, width_type: &str, poles: u32) -> Result<String, String> {
    let wt = if width_type.is_empty() { "s" } else { width_type };
    let p = if poles == 0 { 2 } else { poles };
    let filter = format!("lowshelf=f={}:g={}:t={}:w={}:p={}", frequency, gain, wt, width, p);
    let mut command = Command::new("ffmpeg");
    command.arg("-i").arg(input_file).arg("-af").arg(filter).arg("-c:v").arg("copy").arg("-y").arg(output_file);
    execute_ffmpeg_command(command)
}

pub fn apply_surround_upmix(input_file: &str, output_file: &str, chl_out: &str, chl_in: &str, level_in: f64, level_out: f64) -> Result<String, String> {
    let out_layout = if chl_out.is_empty() { "5.1" } else { chl_out };
    let in_layout = if chl_in.is_empty() { "stereo" } else { chl_in };
    let li = if level_in <= 0.0 { 1.0 } else { level_in };
    let lo = if level_out <= 0.0 { 1.0 } else { level_out };
    let filter = format!("surround=chl_out={}:chl_in={}:level_in={}:level_out={}", out_layout, in_layout, li, lo);
    let mut command = Command::new("ffmpeg");
    command.arg("-i").arg(input_file).arg("-af").arg(filter).arg("-c:v").arg("copy").arg("-y").arg(output_file);
    execute_ffmpeg_command(command)
}

pub fn detect_volume_levels(input_file: &str) -> Result<String, String> {
    let output = std::process::Command::new("ffmpeg")
        .arg("-i").arg(input_file)
        .arg("-af").arg("volumedetect")
        .arg("-f").arg("null").arg("-")
        .output()
        .map_err(|e| format!("Failed to execute FFmpeg: {}", e))?;
    let stderr = String::from_utf8_lossy(&output.stderr);
    let mut result = Vec::new();
    for line in stderr.lines() {
        if line.contains("max_volume") || line.contains("mean_volume") || line.contains("histogram") {
            result.push(line.trim().to_string());
        }
    }
    if result.is_empty() {
        Err("Could not detect volume — no audio stream or unsupported format".to_string())
    } else {
        Ok(result.join("\n"))
    }
}

// PHASE I BATCH 10

pub fn visualize_cqt(input_file: &str, output_file: &str, width: u32, height: u32, bar_h: u32, axis_h: u32) -> Result<String, String> {
    let w = if width == 0 { 1920 } else { width };
    let h = if height == 0 { 1080 } else { height };
    let bh = if bar_h == 0 { 20 } else { bar_h };
    let ah = if axis_h == 0 { 30 } else { axis_h };
    let filter = format!("[0:a]showcqt=size={}x{}:bar_h={}:axis_h={}[v]", w, h, bh, ah);
    let mut command = Command::new("ffmpeg");
    command.arg("-i").arg(input_file)
        .arg("-filter_complex").arg(&filter)
        .arg("-map").arg("[v]")
        .arg("-y").arg(output_file);
    execute_ffmpeg_command(command)
}

pub fn visualize_frequencies(input_file: &str, output_file: &str, width: u32, height: u32, mode: &str, ascale: &str) -> Result<String, String> {
    let w = if width == 0 { 1024 } else { width };
    let h = if height == 0 { 512 } else { height };
    let m = if mode.is_empty() { "line" } else { mode };
    let a = if ascale.is_empty() { "log" } else { ascale };
    let filter = format!("[0:a]showfreqs=size={}x{}:mode={}:ascale={}[v]", w, h, m, a);
    let mut command = Command::new("ffmpeg");
    command.arg("-i").arg(input_file)
        .arg("-filter_complex").arg(&filter)
        .arg("-map").arg("[v]")
        .arg("-y").arg(output_file);
    execute_ffmpeg_command(command)
}

pub fn apply_audio_iir(input_file: &str, output_file: &str, zeros: &str, poles: &str, gains: &str) -> Result<String, String> {
    let z = if zeros.is_empty() { "1" } else { zeros };
    let p = if poles.is_empty() { "1" } else { poles };
    let g = if gains.is_empty() { "1" } else { gains };
    let filter = format!("aiir=zeros='{}':poles='{}':gains='{}'", z, p, g);
    let mut command = Command::new("ffmpeg");
    command.arg("-i").arg(input_file).arg("-af").arg(filter).arg("-c:v").arg("copy").arg("-y").arg(output_file);
    execute_ffmpeg_command(command)
}

pub fn apply_audio_expression(input_file: &str, output_file: &str, exprs: &str) -> Result<String, String> {
    // aeval expressions use val(ch) syntax, not bare "val". Default: pass-through.
    let e = if exprs.is_empty() || exprs == "val" { "val(ch)" } else { exprs };
    // No surrounding quotes — process::Command passes the string directly to FFmpeg
    let filter = format!("aeval=exprs={}", e);
    let mut command = Command::new("ffmpeg");
    command.arg("-i").arg(input_file).arg("-af").arg(filter).arg("-c:v").arg("copy").arg("-y").arg(output_file);
    execute_ffmpeg_command(command)
}

pub fn convert_audio_format(input_file: &str, output_file: &str, sample_fmts: &str, sample_rates: &str, channel_layouts: &str) -> Result<String, String> {
    let mut parts = Vec::new();
    if !sample_fmts.is_empty() { parts.push(format!("sample_fmts={}", sample_fmts)); }
    if !sample_rates.is_empty() { parts.push(format!("sample_rates={}", sample_rates)); }
    if !channel_layouts.is_empty() { parts.push(format!("channel_layouts={}", channel_layouts)); }
    let filter = if parts.is_empty() { "aformat=sample_fmts=s16".to_string() } else { format!("aformat={}", parts.join(":")) };
    let mut command = Command::new("ffmpeg");
    command.arg("-i").arg(input_file).arg("-af").arg(filter).arg("-c:v").arg("copy").arg("-y").arg(output_file);
    execute_ffmpeg_command(command)
}

pub fn apply_cross_correlate(input_file: &str, secondary_file: &str, output_file: &str, size: u32, algo: &str) -> Result<String, String> {
    let s = if size == 0 { 256 } else { size };
    let a = if algo.is_empty() { "fast" } else { algo };
    let filter = format!("[0:a][1:a]axcorrelate=size={}:algo={}[aout]", s, a);
    let mut command = Command::new("ffmpeg");
    command.arg("-i").arg(input_file).arg("-i").arg(secondary_file)
        .arg("-filter_complex").arg(&filter)
        .arg("-map").arg("[aout]").arg("-c:v").arg("copy").arg("-y").arg(output_file);
    execute_ffmpeg_command(command)
}

pub fn apply_audio_multiply(input_file: &str, secondary_file: &str, output_file: &str) -> Result<String, String> {
    let filter = "[0:a][1:a]amultiply[aout]";
    let mut command = Command::new("ffmpeg");
    command.arg("-i").arg(input_file).arg("-i").arg(secondary_file)
        .arg("-filter_complex").arg(filter)
        .arg("-map").arg("[aout]").arg("-c:v").arg("copy").arg("-y").arg(output_file);
    execute_ffmpeg_command(command)
}

pub fn apply_audio_contrast(input_file: &str, output_file: &str, contrast: f64) -> Result<String, String> {
    let c = if contrast <= 0.0 { 33.0 } else { contrast.min(100.0) };
    let filter = format!("acontrast=contrast={}", c);
    let mut command = Command::new("ffmpeg");
    command.arg("-i").arg(input_file).arg("-af").arg(filter).arg("-c:v").arg("copy").arg("-y").arg(output_file);
    execute_ffmpeg_command(command)
}

pub fn decode_hdcd(input_file: &str, output_file: &str, disable_autoconvert: bool, process_stereo: bool, force_pe: bool) -> Result<String, String> {
    let mut params = Vec::new();
    if disable_autoconvert { params.push("disable_autoconvert=1".to_string()); }
    if process_stereo { params.push("process_stereo=1".to_string()); }
    if force_pe { params.push("force_pe=1".to_string()); }
    let filter = if params.is_empty() { "hdcd".to_string() } else { format!("hdcd={}", params.join(":")) };
    let mut command = Command::new("ffmpeg");
    command.arg("-i").arg(input_file).arg("-af").arg(filter).arg("-c:v").arg("copy").arg("-y").arg(output_file);
    execute_ffmpeg_command(command)
}

pub fn measure_audio_spectrum(input_file: &str, output_file: &str, width: u32, height: u32, mode: &str, color: &str) -> Result<String, String> {
    let w = if width == 0 { 1024 } else { width };
    let h = if height == 0 { 512 } else { height };
    let m = if mode.is_empty() { "combined" } else { mode };
    let c = if color.is_empty() { "intensity" } else { color };
    let filter = format!("[0:a]showspectrum=size={}x{}:mode={}:color={}[v]", w, h, m, c);
    let mut command = Command::new("ffmpeg");
    command.arg("-i").arg(input_file)
        .arg("-filter_complex").arg(filter)
        .arg("-map").arg("[v]")
        .arg("-y").arg(output_file);
    execute_ffmpeg_command(command)
}
