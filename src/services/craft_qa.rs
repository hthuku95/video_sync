//! Craft QA gates — deterministic motion-design checks adapted from
//! Motionmaxxing (Apache-2.0, Tejas Makwana; owner-approved integration
//! Nov 13 2026). Complements the AV-sync QA (content) with CRAFT checks:
//!
//! - **G2 no dead frames**: a run of several seconds with literally nothing
//!   moving on screen is the #1 "AI slop" tell. Motionmaxxing fails >3 flat
//!   frames at ~30fps for 15-40s launch films; our narrated 60-90s videos
//!   breathe longer by contract (scenes hold after elements enter), so the
//!   hard threshold here is a >2.5s fully-flat run, warn at >1.0s.
//! - **G3 end hold**: the ending should resolve and stop within a beat.
//!   Motionmaxxing: ≤1.4s for shorts; for narrated videos: warn >2s, fail >4s.
//!
//! Method: decode the video to tiny grayscale frames at 4fps with ffmpeg
//! (rawvideo to stdout), compute mean absolute pixel difference between
//! consecutive frames. No external deps; runs on the API container.

use std::path::PathBuf;

pub struct CraftReport {
    pub skipped_reason: Option<String>,
    /// seconds of fully-static footage at the end of the video
    pub end_hold_seconds: f64,
    /// longest run of consecutive ~zero-motion frames, in seconds
    pub max_dead_run_seconds: f64,
    pub fps_sampled: f64,
}

impl CraftReport {
    pub fn skipped(reason: &str) -> Self {
        Self {
            skipped_reason: Some(reason.to_string()),
            end_hold_seconds: 0.0,
            max_dead_run_seconds: 0.0,
            fps_sampled: 0.0,
        }
    }

    /// A completed audit with a dead-frame run >2.5s or end hold >4s fails.
    pub fn hard_fail(&self) -> bool {
        self.skipped_reason.is_none()
            && (self.max_dead_run_seconds > 2.5 || self.end_hold_seconds > 4.0)
    }

    pub fn render_markdown(&self) -> String {
        if let Some(r) = &self.skipped_reason {
            return format!("**Craft QA (motion gates):** ⏭️ skipped — {r}\n");
        }
        let mut out = String::from("**Craft QA (motion gates):**\n");
        out.push_str(&format!(
            "  • Dead-frame check (G2): longest fully-static run {:.1}s — {}\n",
            self.max_dead_run_seconds,
            if self.max_dead_run_seconds > 2.5 {
                "❌ FAIL (slop tell: >2.5s of nothing moving)"
            } else if self.max_dead_run_seconds > 1.0 {
                "⚠️ long hold — keep background/camera alive"
            } else {
                "✅"
            }
        ));
        out.push_str(&format!(
            "  • End-hold check (G3): final hold {:.1}s — {}\n",
            self.end_hold_seconds,
            if self.end_hold_seconds > 4.0 {
                "❌ FAIL (end card overstays)"
            } else if self.end_hold_seconds > 2.0 {
                "⚠️ end card runs long"
            } else {
                "✅"
            }
        ));
        out
    }

    pub fn retry_hint(&self) -> Option<String> {
        if !self.hard_fail() {
            return None;
        }
        let mut items = Vec::new();
        if self.max_dead_run_seconds > 2.5 {
            items.push(format!(
                "a {:.1}s run of completely static frames exists — keep background/camera/text idle motion alive on every frame (breathing drift, grain, subtle parallax)",
                self.max_dead_run_seconds
            ));
        }
        if self.end_hold_seconds > 4.0 {
            items.push(format!(
                "the final hold runs {:.1}s — cut the end card to ≤2s",
                self.end_hold_seconds
            ));
        }
        Some(format!(
            "Craft QA failure: {}. Re-render only the affected scene(s).",
            items.join("; ")
        ))
    }
}

/// Entry point: `video` is a local path (already resolved by the caller).
pub async fn verify_craft(video_path: &PathBuf) -> CraftReport {
    let video_path = video_path.clone();
    let out = tokio::task::spawn_blocking(move || run_craft_qa(&video_path)).await;
    match out {
        Ok(Ok(report)) => report,
        Ok(Err(e)) => CraftReport::skipped(&e),
        Err(e) => CraftReport::skipped(&format!("craft QA spawn failed: {e}")),
    }
}

fn run_craft_qa(video: &PathBuf) -> Result<CraftReport, String> {
    use std::process::Command;
    const FPS: f64 = 4.0;
    const W: u32 = 64;
    const H: u32 = 36;

    let out = Command::new("ffmpeg")
        .args([
            "-v",
            "error",
            "-i",
            video.to_str().unwrap_or(""),
            "-vf",
            &format!("fps={FPS},scale={W}:{H}"),
            "-f",
            "rawvideo",
            "-pix_fmt",
            "gray",
            "pipe:1",
        ])
        .output()
        .map_err(|e| format!("ffmpeg spawn: {e}"))?;
    if !out.status.success() {
        return Err(format!(
            "ffmpeg decode failed: {}",
            String::from_utf8_lossy(&out.stderr)
                .chars()
                .take(160)
                .collect::<String>()
        ));
    }

    let frame_len = (W * H) as usize;
    let data = out.stdout;
    if data.len() < frame_len * 3 {
        return Err("video too short to audit".into());
    }
    let n_frames = data.len() / frame_len;
    let mut diffs = Vec::with_capacity(n_frames.saturating_sub(1));
    for f in 1..n_frames {
        let a = &data[(f - 1) * frame_len..f * frame_len];
        let b = &data[f * frame_len..(f + 1) * frame_len];
        let sum: u64 = a
            .iter()
            .zip(b.iter())
            .map(|(x, y)| (i16::from(*x) - i16::from(*y)).unsigned_abs() as u64)
            .sum();
        let mean = sum as f64 / frame_len as f64;
        diffs.push(mean);
    }
    // Motion threshold: mean abs diff below this ≈ visually static (noise only).
    const FLAT_THRESHOLD: f64 = 0.35;

    // G2: longest run of flat frames.
    let mut max_run = 0usize;
    let mut run = 0usize;
    for d in &diffs {
        if *d < FLAT_THRESHOLD {
            run += 1;
            max_run = max_run.max(run);
        } else {
            run = 0;
        }
    }
    // G3: end hold — flat frames at the very end.
    let end_flat = diffs
        .iter()
        .rev()
        .take_while(|d| **d < FLAT_THRESHOLD)
        .count();

    Ok(CraftReport {
        skipped_reason: None,
        end_hold_seconds: end_flat as f64 / FPS,
        max_dead_run_seconds: max_run as f64 / FPS,
        fps_sampled: FPS,
    })
}
