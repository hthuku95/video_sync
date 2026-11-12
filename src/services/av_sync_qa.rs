//! Timestamped audio-visual sync verification (owner directive Nov 12 2026).
//!
//! Answers the question "at second 37, is what's on screen what the audio at
//! second 37 is describing?" — deterministically, per speech segment:
//!
//! 1. Transcribe the final video's own audio with word-level timestamps
//!    (VibeVoice `transcribe` mode on the droplet — already-deployed infra).
//! 2. Extract one frame at the midpoint of each speech segment with ffmpeg.
//! 3. Ask the vision model, per pair: "does the frame at t=X show what the
//!    narration at t=X says?" → match / mismatch / unclear + reason.
//! 4. Aggregate into a report with per-timestamp verdicts and a retry_hint
//!    naming ONLY the failing seconds, so the agent can re-render just the
//!    offending scene instead of the whole video.
//!
//! Every step soft-degrades: the report is never an Err. If transcription or
//! the vision model is unavailable the caller gets a `skipped` report and
//! `review_video` continues with its existing checks.

use serde_json::Value;
use std::path::PathBuf;
use std::sync::Arc;

use crate::AppState;

/// Hard cap on vision checks per review (cost + latency bound).
const MAX_SEGMENTS_CHECKED: usize = 10;
/// Segments shorter than this are merged into their neighbor before capping.
const MIN_SEGMENT_SECONDS: f64 = 2.0;
/// Skip AV-sync QA entirely for ultra-short clips (nothing meaningful to check).
const MIN_VIDEO_SECONDS: f64 = 5.0;

#[derive(Debug, Clone)]
pub struct SegmentVerdict {
    pub start: f64,
    pub end: f64,
    pub spoken: String,
    pub verdict: &'static str, // "match" | "mismatch" | "unclear"
    pub detail: String,
}

#[derive(Debug)]
pub struct AvSyncReport {
    pub skipped_reason: Option<String>,
    pub video_duration: f64,
    pub segments_checked: usize,
    pub matched: usize,
    pub mismatches: Vec<SegmentVerdict>,
    pub unclear: Vec<SegmentVerdict>,
    /// Total transcribed speech seconds and their share of the runtime.
    /// `None` when transcription was skipped. Monica lesson (Nov 10 2026):
    /// 43% speech density shipped silently — anything under 60% is a fail.
    pub speech_seconds: f64,
    pub speech_coverage: Option<f64>,
}

impl AvSyncReport {
    pub fn ok(&self) -> bool {
        self.skipped_reason.is_none() && self.mismatches.is_empty() && !self.density_fail()
    }

    /// Only a *completed* audit with real mismatches — or speech density below
    /// the 60% floor — fails the review. A skipped audit (no transcriber, too
    /// short, etc.) never blocks a video.
    pub fn hard_fail(&self) -> bool {
        (self.skipped_reason.is_none() && !self.mismatches.is_empty()) || self.density_fail()
    }

    /// Monica lesson enforcement: speech must cover ≥60% of the runtime.
    fn density_fail(&self) -> bool {
        matches!(self.speech_coverage, Some(c) if c < 0.60)
    }

    /// Human/agent-readable block appended to the review_video output.
    pub fn render_markdown(&self) -> String {
        if let Some(reason) = &self.skipped_reason {
            return format!("**AV-Sync Verification:** ⏭️ skipped — {reason}\n");
        }
        let mut out = String::from("**AV-Sync Verification (timestamped):**\n");
        out.push_str(&format!(
            "  • Segments checked: {} (matched {}, mismatched {}, unclear {})\n",
            self.segments_checked,
            self.matched,
            self.mismatches.len(),
            self.unclear.len()
        ));
        for m in &self.mismatches {
            out.push_str(&format!(
                "  • ❌ t={:.1}s–{:.1}s — narration says \"{}\" but the frame does not match: {}\n",
                m.start,
                m.end,
                truncate(&m.spoken, 90),
                truncate(&m.detail, 140)
            ));
        }
        for u in &self.unclear {
            out.push_str(&format!(
                "  • ⚠️ t={:.1}s–{:.1}s — could not confirm: {}\n",
                u.start,
                u.end,
                truncate(&u.detail, 120)
            ));
        }
        if let Some(cov) = self.speech_coverage {
            let flag = if cov < 0.60 {
                "❌ BELOW THE 60% FLOOR"
            } else {
                "✅"
            };
            out.push_str(&format!(
                "  • Speech density: {:.0}% of runtime ({:.1}s speech / {:.0}s video) {}\n",
                cov * 100.0,
                self.speech_seconds,
                self.video_duration,
                flag
            ));
        }
        if self.ok() {
            out.push_str(
                "  • ✅ Every checked timestamp shows what the narration is describing.\n",
            );
        }
        out
    }

    /// Structured note the agent can act on: only the failing timestamps.
    pub fn retry_hint(&self) -> Option<String> {
        let mut items: Vec<String> = self
            .mismatches
            .iter()
            .map(|m| {
                format!(
                    "t={:.1}s–{:.1}s: narration \"{}\" vs visual ({})",
                    m.start,
                    m.end,
                    truncate(&m.spoken, 70),
                    truncate(&m.detail, 90)
                )
            })
            .collect();
        if let Some(cov) = self.speech_coverage {
            if cov < 0.60 {
                items.push(format!(
                    "speech density {:.0}% is below the 60% floor ({:.1}s of {:.0}s) — rewrite the narration denser (≈30-38 words per 15s scene) and re-generate narration",
                    cov * 100.0,
                    self.speech_seconds,
                    self.video_duration
                ));
            }
        }
        if items.is_empty() {
            return None;
        }
        let lead = if self.mismatches.is_empty() {
            "Narration density failure"
        } else {
            "AV-sync mismatch"
        };
        Some(format!(
            "{} at {} issue(s): {}. Fix ONLY the affected scene(s): re-render the HyperFrames scene covering failing timestamps so visuals match the narration at those seconds.",
            lead,
            items.len(),
            items.join("; ")
        ))
    }
}

fn truncate(s: &str, max: usize) -> String {
    let s = s.trim().replace('\n', " ");
    if s.chars().count() <= max {
        s
    } else {
        let cut: String = s.chars().take(max.saturating_sub(3)).collect();
        format!("{cut}...")
    }
}

/// Entry point. `video` is a local path or an http(s) URL (downloaded to temp).
pub async fn verify_av_sync(state: &Arc<AppState>, video: &str) -> AvSyncReport {
    match verify_av_sync_inner(state, video).await {
        Ok(report) => report,
        Err(e) => AvSyncReport {
            skipped_reason: Some(truncate(&e, 160)),
            video_duration: 0.0,
            segments_checked: 0,
            matched: 0,
            mismatches: Vec::new(),
            unclear: Vec::new(),
            speech_seconds: 0.0,
            speech_coverage: None,
        },
    }
}

async fn verify_av_sync_inner(state: &Arc<AppState>, video: &str) -> Result<AvSyncReport, String> {
    // ---- 0. Precondition: a vision model + transcriber must exist. ----
    let vision = state
        .video_gemini_client
        .as_ref()
        .or(state.gemini_client.as_ref())
        .ok_or("no Gemini vision client configured")?;
    let transcriber = state
        .vibevoice_client
        .as_ref()
        .ok_or("no VibeVoice transcriber configured")?;

    // ---- 1. Resolve the video to a local temp file. ----
    let video_path = resolve_local_video(video).await?;

    // ---- 2. Probe duration; bail on ultra-short clips. ----
    let duration = probe_duration(&video_path).await?;
    if duration < MIN_VIDEO_SECONDS {
        return Ok(AvSyncReport {
            skipped_reason: Some(format!("video too short for AV-sync QA ({duration:.1}s)")),
            video_duration: duration,
            segments_checked: 0,
            matched: 0,
            mismatches: Vec::new(),
            unclear: Vec::new(),
            speech_seconds: 0.0,
            speech_coverage: None,
        });
    }

    // ---- 3. Extract mono 16 kHz WAV for transcription. ----
    let tmp_dir = std::env::temp_dir().join(format!("avsync-{}", uuid::Uuid::new_v4()));
    tokio::fs::create_dir_all(&tmp_dir)
        .await
        .map_err(|e| format!("temp dir: {e}"))?;
    let audio_path = tmp_dir.join("audio.wav");
    run_ffmpeg_spawn_blocking(&[
        "-y".into(),
        "-i".into(),
        video_path.to_string_lossy().into_owned(),
        "-vn".into(),
        "-ac".into(),
        "1".into(),
        "-ar".into(),
        "16000".into(),
        audio_path.to_string_lossy().into_owned(),
    ])
    .await?;

    // ---- 4. Publish audio to a fetchable URL, then transcribe. ----
    let audio_url = crate::cloud_storage::upload_local_file_to_cloud(
        audio_path.to_str().unwrap_or(""),
        "audio/wav",
        0,
        "av-sync-qa",
        None,
        state,
    )
    .await
    .map_err(|e| format!("audio upload: {e}"))?;

    let payload = tokio::time::timeout(
        std::time::Duration::from_secs(240),
        transcriber.transcribe_url(&audio_url, &[], None, None, None),
    )
    .await
    .map_err(|_| "transcription timed out (240s)".to_string())?
    .map_err(|e| format!("transcription failed: {}", truncate_err(&e)))?;

    let mut segments = parse_segments(&payload);
    if segments.is_empty() {
        // No speech detected: not a sync failure — nothing to align.
        return Ok(AvSyncReport {
            skipped_reason: Some("no speech segments detected in audio".into()),
            video_duration: duration,
            segments_checked: 0,
            matched: 0,
            mismatches: Vec::new(),
            unclear: Vec::new(),
            speech_seconds: 0.0,
            speech_coverage: None,
        });
    }

    // Real speech coverage of the final mix, measured from the transcript
    // (owner's Monica lesson: 43% density shipped silently — enforce ≥60%).
    let speech_seconds: f64 = segments.iter().map(|s| s.1 - s.0).sum();
    merge_short_segments(&mut segments, MIN_SEGMENT_SECONDS);
    let checked = select_even_subset(&segments, MAX_SEGMENTS_CHECKED);

    // ---- 5. Extract one frame per segment midpoint. ----
    let mut frames: Vec<(usize, PathBuf)> = Vec::new();
    for (idx, seg) in checked.iter().enumerate() {
        let mid = (seg.0 + seg.1) / 2.0;
        let frame_path = tmp_dir.join(format!("frame_{idx:02}.jpg"));
        let ok = run_ffmpeg_spawn_blocking(&[
            "-y".into(),
            "-ss".into(),
            format!("{mid:.3}"),
            "-i".into(),
            video_path.to_string_lossy().into_owned(),
            "-frames:v".into(),
            "1".into(),
            "-vf".into(),
            "scale=640:-2".into(),
            "-q:v".into(),
            "4".into(),
            frame_path.to_string_lossy().into_owned(),
        ])
        .await
        .is_ok()
            && tokio::fs::metadata(&frame_path).await.is_ok();
        if ok {
            frames.push((idx, frame_path));
        }
    }
    if frames.is_empty() {
        return Ok(AvSyncReport {
            skipped_reason: Some("could not extract any frames".into()),
            video_duration: duration,
            segments_checked: 0,
            matched: 0,
            mismatches: Vec::new(),
            unclear: Vec::new(),
            speech_seconds: 0.0,
            speech_coverage: None,
        });
    }

    // ---- 6. Vision-check each (frame, spoken-text) pair. ----
    let results = futures::future::join_all(frames.iter().map(|(idx, path)| {
        let seg = checked[*idx].clone();
        let path = path.clone();
        let vision = vision.clone();
        async move {
            let bytes = match tokio::fs::read(&path).await {
                Ok(b) => b,
                Err(e) => return seg_verdict(&seg, "unclear", &format!("frame read failed: {e}")),
            };
            let prompt = format!(
                "You are auditing audio-visual sync in a produced video.\n\
                 NARRATION spoken during t={:.1}s–{:.1}s (you see the frame from the MIDDLE of that window):\n\
                 \"{}\"\n\n\
                 QUESTION: Does what is visible in this frame match what the narration is describing at this moment?\n\
                 - match = the on-screen content directly supports/illustrates the spoken line\n\
                 - mismatch = the frame shows unrelated, contradictory, placeholder, or blank-filler content\n\
                 - unclear = the visual is generic (plain background) but not contradictory\n\n\
                 Reply with ONLY a JSON object: {{\"verdict\":\"match|mismatch|unclear\",\"reason\":\"<one sentence>\"}}",
                seg.0, seg.1, seg.2
            );
            match tokio::time::timeout(
                std::time::Duration::from_secs(45),
                vision.analyze_image_bytes(&bytes, &prompt),
            )
            .await
            {
                Ok(Ok(resp)) => {
                    let (v, r) = parse_verdict(&resp);
                    seg_verdict(&seg, v, &r)
                }
                Ok(Err(e)) => seg_verdict(&seg, "unclear", &format!("vision error: {e}")),
                Err(_) => seg_verdict(&seg, "unclear", "vision call timed out"),
            }
        }
    }))
    .await;

    // ---- 7. Aggregate. ----
    let segments_checked = results.len();
    let mut matched = 0usize;
    let mut mismatches = Vec::new();
    let mut unclear = Vec::new();
    for v in results {
        match v.verdict {
            "match" => matched += 1,
            "mismatch" => mismatches.push(v),
            _ => unclear.push(v),
        }
    }

    let report = AvSyncReport {
        skipped_reason: None,
        video_duration: duration,
        segments_checked,
        matched,
        mismatches,
        unclear,
        speech_seconds,
        speech_coverage: Some((speech_seconds / duration).min(1.0)),
    };

    // Best-effort cleanup of the temp dir.
    let _ = tokio::fs::remove_dir_all(&tmp_dir).await;
    Ok(report)
}

// ---------------------------------------------------------------------------
// helpers
// ---------------------------------------------------------------------------

/// Public re-entry so craft QA can audit the same video without duplicating
/// resolution logic. Local paths pass through; URLs download to temp (120s
/// cap, same discipline as render_review).
pub async fn resolve_video_for_craft(video: &str) -> Result<PathBuf, String> {
    resolve_local_video(video).await
}

type Segment = (f64, f64, String); // start, end, spoken text

fn seg_verdict(seg: &Segment, verdict: &'static str, detail: &str) -> SegmentVerdict {
    SegmentVerdict {
        start: seg.0,
        end: seg.1,
        spoken: seg.2.clone(),
        verdict,
        detail: detail.to_string(),
    }
}

async fn resolve_local_video(video: &str) -> Result<PathBuf, String> {
    if video.starts_with("http://") || video.starts_with("https://") {
        // Bounded download (same discipline as render_review: an unbounded GET
        // once wedged QA for 35+ minutes).
        let resp = tokio::time::timeout(std::time::Duration::from_secs(120), reqwest::get(video))
            .await
            .map_err(|_| "video download timed out (120s)".to_string())?
            .map_err(|e| format!("video download failed: {e}"))?;
        let bytes = resp
            .bytes()
            .await
            .map_err(|e| format!("video read failed: {e}"))?;
        if bytes.is_empty() {
            return Err("downloaded video is empty".into());
        }
        let ext = video
            .rsplit('.')
            .next()
            .filter(|s| s.len() <= 5 && !s.contains('/'))
            .unwrap_or("mp4");
        let path =
            std::env::temp_dir().join(format!("avsync_video_{}.{}", uuid::Uuid::new_v4(), ext));
        tokio::fs::write(&path, &bytes)
            .await
            .map_err(|e| format!("temp write: {e}"))?;
        Ok(path)
    } else {
        for candidate in [
            PathBuf::from(video),
            PathBuf::from(format!("uploads/{video}")),
            PathBuf::from(format!("outputs/{video}")),
        ] {
            if tokio::fs::metadata(&candidate).await.is_ok() {
                return Ok(candidate);
            }
        }
        Err(format!("video file not found: {video}"))
    }
}

async fn probe_duration(path: &PathBuf) -> Result<f64, String> {
    use std::process::Command;
    let p = path.clone();
    let out = tokio::task::spawn_blocking(move || {
        Command::new("ffprobe")
            .args([
                "-v",
                "error",
                "-show_entries",
                "format=duration",
                "-of",
                "json",
            ])
            .arg(&p)
            .output()
    })
    .await
    .map_err(|e| format!("ffprobe spawn: {e}"))?
    .map_err(|e| format!("ffprobe failed: {e}"))?;
    let text = String::from_utf8_lossy(&out.stdout);
    let v: Value = serde_json::from_str(text.trim())
        .map_err(|_| "ffprobe returned invalid JSON".to_string())?;
    v.pointer("/format/duration")
        .and_then(|d| d.as_str().and_then(|s| s.parse::<f64>().ok()))
        .ok_or_else(|| "ffprobe could not read duration".to_string())
}

/// ffmpeg on local temp files — spawn_blocking keeps the runtime free.
async fn run_ffmpeg_spawn_blocking(args: &[String]) -> Result<(), String> {
    use std::process::Command;
    let args = args.to_vec();
    let out = tokio::task::spawn_blocking(move || Command::new("ffmpeg").args(&args).output())
        .await
        .map_err(|e| format!("ffmpeg spawn: {e}"))?
        .map_err(|e| format!("ffmpeg failed: {e}"))?;
    if out.status.success() {
        Ok(())
    } else {
        let stderr = String::from_utf8_lossy(&out.stderr);
        Err(format!(
            "ffmpeg exited {}: {}",
            out.status,
            truncate(&stderr, 200)
        ))
    }
}

/// Whisper-style segments: accepts the field aliases the droplet has used
/// (start/begin/from, end/finish/to, text/content).
fn parse_segments(payload: &Value) -> Vec<Segment> {
    fn num(v: Option<&Value>) -> Option<f64> {
        v.and_then(|x| {
            x.as_f64()
                .or_else(|| x.as_i64().map(|i| i as f64))
                .or_else(|| x.as_str().and_then(|s| s.parse::<f64>().ok()))
        })
    }
    let mut out = Vec::new();
    if let Some(segs) = payload.get("segments").and_then(|v| v.as_array()) {
        for seg in segs {
            let start = num(seg
                .get("start")
                .or_else(|| seg.get("begin"))
                .or_else(|| seg.get("from")));
            let end = num(seg
                .get("end")
                .or_else(|| seg.get("finish"))
                .or_else(|| seg.get("to")));
            let text = seg
                .get("text")
                .or_else(|| seg.get("content"))
                .and_then(|v| v.as_str())
                .unwrap_or("")
                .trim()
                .to_string();
            if let (Some(s), Some(e)) = (start, end) {
                if !text.is_empty() && e > s {
                    out.push((s, e, text));
                }
            }
        }
    }
    // Deterministic order.
    out.sort_by(|a, b| a.0.partial_cmp(&b.0).unwrap_or(std::cmp::Ordering::Equal));
    out
}

/// Fold segments shorter than `min` into their previous neighbor so the frame
/// window is meaningful (and the count stays bounded).
fn merge_short_segments(segments: &mut Vec<Segment>, min: f64) {
    let mut merged: Vec<Segment> = Vec::with_capacity(segments.len());
    for seg in segments.drain(..) {
        if let Some(last) = merged.last_mut() {
            if seg.1 - seg.0 < min || seg.0 - last.1 < 0.05 {
                last.1 = last.1.max(seg.1);
                last.2 = format!("{} {}", last.2, seg.2);
                continue;
            }
        }
        merged.push(seg);
    }
    *segments = merged;
}

/// Evenly spread up to `max` segments across the whole timeline so the audit
/// samples the beginning, middle and end — not just the first 10 lines.
fn select_even_subset(segments: &[Segment], max: usize) -> Vec<Segment> {
    if segments.len() <= max {
        return segments.to_vec();
    }
    let step = segments.len() as f64 / max as f64;
    (0..max)
        .map(|i| segments[((i as f64 * step) as usize).min(segments.len() - 1)].clone())
        .collect()
}

/// Pull the first JSON object out of a possibly-verbose model reply.
fn parse_verdict(resp: &str) -> (&'static str, String) {
    let s = resp.trim();
    let obj_start = match (s.find('{'), s.rfind('}')) {
        (Some(a), Some(b)) if b > a => &s[a..=b],
        _ => return ("unclear", truncate(s, 140)),
    };
    match serde_json::from_str::<Value>(obj_start) {
        Ok(v) => {
            let verdict = match v.get("verdict").and_then(|x| x.as_str()) {
                Some("match") => "match",
                Some("mismatch") => "mismatch",
                _ => "unclear",
            };
            let reason = v
                .get("reason")
                .and_then(|x| x.as_str())
                .unwrap_or("no reason given")
                .to_string();
            (verdict, reason)
        }
        Err(_) => ("unclear", truncate(obj_start, 140)),
    }
}

fn truncate_err(e: &str) -> String {
    truncate(e, 160)
}
