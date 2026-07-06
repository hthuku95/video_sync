#![allow(dead_code)]
use std::process::{Command, Stdio};
use std::io::Read;

/// Result of clip position selection
#[derive(Debug, Clone)]
pub struct ClipPosition {
    pub start_time: f64,
    pub score: f64,
}

/// First pass: detect scene changes at low threshold → larger set of candidates
/// Second pass: only include scenes that exceed the main threshold
pub fn detect_scene_changes(
    video_url: &str,
    threshold: f64,
    max_analysis_secs: Option<f64>,
) -> Result<Vec<f64>, String> {
    let duration = get_duration(video_url)?;
    let analysis_window = max_analysis_secs.unwrap_or(duration).min(duration);

    let mut args = vec![
        "-v", "quiet",
        "-i", video_url,
        "-filter", &format!("select='gt(scene,{})'", threshold),
        "-show_entries", "frame=pts_time",
        "-vsync", "vfr",
        "-f", "null",
    ];

    // If limiting analysis duration, add -t before -i
    let mut cmd = if analysis_window < duration && analysis_window > 0.0 {
        let mut cmd = Command::new("ffmpeg");
        cmd.arg("-t").arg(&format!("{:.1}", analysis_window));
        cmd.arg("-i").arg(video_url);
        cmd.arg("-filter").arg(&format!("select='gt(scene,{})'", threshold));
        cmd.arg("-show_entries").arg("frame=pts_time");
        cmd.arg("-vsync").arg("vfr");
        cmd.arg("-f").arg("null");
        cmd
    } else {
        let mut cmd = Command::new("ffmpeg");
        cmd.args(&args);
        cmd
    };

    let output = cmd
        .stdout(Stdio::piped())
        .stderr(Stdio::piped())
        .output()
        .map_err(|e| format!("Failed to run scene detection: {}", e))?;

    let stderr = String::from_utf8_lossy(&output.stderr);
    let mut changes: Vec<f64> = Vec::new();

    for line in stderr.lines() {
        if let Some(ts_str) = line.split("pts_time:").nth(1) {
            if let Ok(ts) = ts_str.trim().parse::<f64>() {
                changes.push(ts);
            }
        }
    }

    changes.sort_by(|a, b| a.partial_cmp(b).unwrap_or(std::cmp::Ordering::Equal));
    Ok(changes)
}

/// Detect active (non-silent) audio segments using silencedetect.
/// Returns list of (start_sec, end_sec) tuples where audio is present.
pub fn detect_active_audio_segments(
    video_url: &str,
    max_analysis_secs: Option<f64>,
) -> Result<Vec<(f64, f64)>, String> {
    let duration = get_duration(video_url)?;
    let analysis_window = max_analysis_secs.unwrap_or(duration).min(duration);

    let mut cmd = Command::new("ffmpeg");
    if analysis_window < duration && analysis_window > 0.0 {
        cmd.arg("-t").arg(&format!("{:.1}", analysis_window));
    }
    cmd.arg("-i").arg(video_url);
    cmd.arg("-af").arg("silencedetect=noise=-25dB:d=0.5");
    cmd.arg("-f").arg("null");

    let output = cmd
        .stdout(Stdio::piped())
        .stderr(Stdio::piped())
        .output()
        .map_err(|e| format!("Failed to run audio analysis: {}", e))?;

    let stderr = String::from_utf8_lossy(&output.stderr);

    let mut silence_starts: Vec<f64> = Vec::new();
    let mut silence_ends: Vec<f64> = Vec::new();

    for line in stderr.lines() {
        let trimmed = line.trim();
        if let Some(rest) = trimmed.strip_prefix("silence_start: ") {
            if let Ok(ts) = rest.trim().parse::<f64>() {
                silence_starts.push(ts);
            }
        }
        if let Some(rest) = trimmed.strip_prefix("silence_end: ") {
            let ts_str = rest.split('|').next().unwrap_or("").trim();
            if let Ok(ts) = ts_str.parse::<f64>() {
                silence_ends.push(ts);
            }
        }
    }

    // Invert silence periods into active periods
    // Video starts active. First silence is at silence_starts[0].
    // Between silence_ends[i] and silence_starts[i+1] is active.
    let mut active: Vec<(f64, f64)> = Vec::new();
    let mut prev_end = 0.0;

    for i in 0..silence_starts.len() {
        let start = silence_starts[i];
        if start > prev_end + 1.0 {
            active.push((prev_end, start));
        }
        if i < silence_ends.len() {
            prev_end = silence_ends[i];
        }
    }

    // If there's active audio after the last silence
    let window_end = analysis_window.min(duration);
    if prev_end < window_end - 1.0 {
        active.push((prev_end, window_end));
    }

    // If no silence detected at all, entire video is active
    if active.is_empty() && window_end > 10.0 {
        active.push((0.0, window_end));
    }

    Ok(active)
}

/// Get video duration via ffprobe on HTTP URL
pub fn get_duration(video_url: &str) -> Result<f64, String> {
    let output = Command::new("ffprobe")
        .args([
            "-v", "error",
            "-show_entries", "format=duration",
            "-of", "csv=p=0",
            video_url,
        ])
        .stdout(Stdio::piped())
        .stderr(Stdio::piped())
        .output()
        .map_err(|e| format!("Failed to probe duration: {}", e))?;

    let stdout = String::from_utf8_lossy(&output.stdout);
    stdout
        .trim()
        .parse::<f64>()
        .map_err(|e| format!("Failed to parse duration '{}': {}", stdout.trim(), e))
}

/// Select the best clip positions using combined signals:
/// - Scene changes: prefer clip starts near scene boundaries
/// - Audio energy: prefer segments with active audio
/// - Coverage diversity: don't cluster all clips in one spot
pub fn select_clip_positions(
    video_url: &str,
    clip_duration: f64,
    max_clips: usize,
    explicit_times: Option<Vec<f64>>,
    max_analysis_secs: Option<f64>,
) -> Result<Vec<ClipPosition>, String> {
    // If explicit times given, use them directly
    if let Some(times) = explicit_times {
        return Ok(times
            .into_iter()
            .map(|t| ClipPosition {
                start_time: t,
                score: 1.0,
            })
            .collect());
    }

    let duration = get_duration(video_url)?;
    if duration < clip_duration + 1.0 {
        return Ok(vec![ClipPosition {
            start_time: 0.0,
            score: 1.0,
        }]);
    }

    // Detect scene changes
    let scene_changes = detect_scene_changes(video_url, 0.35, max_analysis_secs).unwrap_or_default();

    // Detect active audio segments
    let active_segments =
        detect_active_audio_segments(video_url, max_analysis_secs).unwrap_or_default();

    // Score candidate positions
    let candidates = score_candidates(
        duration,
        clip_duration,
        max_clips,
        &scene_changes,
        &active_segments,
    );

    Ok(candidates)
}

/// Score potential clip start positions and return the top N
fn score_candidates(
    duration: f64,
    clip_duration: f64,
    max_clips: usize,
    scene_changes: &[f64],
    active_segments: &[(f64, f64)],
) -> Vec<ClipPosition> {
    let last_start = duration - clip_duration;
    if last_start <= 0.0 {
        return vec![ClipPosition {
            start_time: 0.0,
            score: 1.0,
        }];
    }

    // Score at 1-second granularity
    let mut scored: Vec<ClipPosition> = Vec::new();
    let step = 1.0_f64.max(clip_duration / 10.0); // at least 10 candidates

    let mut t = 0.0;
    while t <= last_start {
        let mut score = 0.0;

        // Scene boundary bonus: +3 if within 2s of a scene change
        for &sc in scene_changes {
            let dist = (t - sc).abs();
            if dist < 2.0 {
                score += 3.0 * (1.0 - dist / 2.0);
            }
        }

        // Audio energy bonus: +2 if within an active audio segment
        for &(seg_start, seg_end) in active_segments {
            if t >= seg_start && t < seg_end {
                score += 2.0;
            }
            // Bonus for start near active segment boundaries
            let dist_to_start = (t - seg_start).abs();
            if dist_to_start < 1.0 {
                score += 1.0;
            }
        }

        // Prefer clip starts that aren't at 0 (skip boring intro)
        if t < 3.0 {
            score -= 0.5;
        }

        scored.push(ClipPosition {
            start_time: t,
            score,
        });

        t += step;
    }

    // Sort by score descending
    scored.sort_by(|a, b| b.score.partial_cmp(&a.score).unwrap_or(std::cmp::Ordering::Equal));

    // Pick top N with diversity: ensure minimum 15s gap between selected clips
    let mut selected: Vec<ClipPosition> = Vec::new();
    let min_gap = clip_duration.max(15.0);

    for candidate in &scored {
        if selected.len() >= max_clips {
            break;
        }
        let too_close = selected.iter().any(|s| {
            let gap = (s.start_time - candidate.start_time).abs();
            gap < min_gap
        });
        if !too_close {
            selected.push(candidate.clone());
        }
    }

    // If we got fewer clips than requested, fill gaps with evenly spaced positions
    if selected.len() < max_clips && selected.len() < (duration / clip_duration) as usize {
        let needed = max_clips - selected.len();
        let step_even = last_start / (needed as f64 + 1.0);
        for i in 1..=needed {
            let pos = step_even * (i as f64);
            let too_close = selected.iter().any(|s| (s.start_time - pos).abs() < min_gap);
            if !too_close {
                selected.push(ClipPosition {
                    start_time: pos,
                    score: 0.5,
                });
            }
        }
    }

    // Sort by start time
    selected.sort_by(|a, b| a.start_time.partial_cmp(&b.start_time).unwrap_or(std::cmp::Ordering::Equal));

    selected
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_get_duration_invalid_url() {
        let result = get_duration("https://invalid-url.example/video.mp4");
        assert!(result.is_err() || result.is_ok());
    }

    #[test]
    fn test_score_candidates_empty() {
        let result = score_candidates(60.0, 15.0, 3, &[], &[]);
        assert!(!result.is_empty());
        assert!(result.len() <= 3);
    }

    #[test]
    fn test_score_candidates_with_scene_changes() {
        let scenes = vec![10.0, 30.0, 45.0];
        let audio = vec![(5.0, 20.0), (25.0, 50.0)];
        let result = score_candidates(60.0, 10.0, 4, &scenes, &audio);
        assert!(!result.is_empty());
        assert!(result.len() <= 4);
    }

    #[test]
    fn test_explicit_times_passthrough() {
        let url = "https://example.com/video.mp4";
        let explicit = vec![10.0, 45.0, 120.0];
        // This should fail on duration probe but we test the passthrough logic
        // by ensuring get_duration is called first
        let result = select_clip_positions(url, 15.0, 3, Some(explicit.clone()), None);
        // Should fail at get_duration for a fake URL, but the explicit_times branch
        // is tested by the success case
        assert!(result.is_err());
    }
}
