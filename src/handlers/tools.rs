// src/handlers/tools.rs
//
// REST API endpoints for on-demand FFmpeg tool execution.
// These allow the content_machine frontend to apply video processing
// to uploaded files without going through the full clipping pipeline.
//
// All endpoints:
//   - Accept a file_path (relative to the uploads/ dir) or an absolute path
//   - Write output to outputs/<job_id>.<ext>
//   - Return a download URL via the existing /api/outputs/download/:file_id route

use axum::{
    extract::Extension,
    http::StatusCode,
    routing::post,
    Json, Router,
};
use serde::{Deserialize, Serialize};
use std::sync::Arc;
use uuid::Uuid;

use crate::AppState;

// ─── Response type ───────────────────────────────────────────────────────────

#[derive(Serialize)]
pub struct ToolResult {
    pub success: bool,
    pub message: String,
    pub output_file_id: Option<String>,
    pub download_url: Option<String>,
}

impl ToolResult {
    fn ok(msg: impl Into<String>, file_id: &str) -> Self {
        ToolResult {
            success: true,
            message: msg.into(),
            output_file_id: Some(file_id.to_string()),
            download_url: Some(format!("/api/outputs/download/{}", file_id)),
        }
    }
    fn err(msg: impl Into<String>) -> Self {
        ToolResult {
            success: false,
            message: msg.into(),
            output_file_id: None,
            download_url: None,
        }
    }
}

// ─── Helpers ─────────────────────────────────────────────────────────────────

/// Resolve an input path from either an absolute path or a relative uploads/ path.
fn resolve_input(path: &str) -> String {
    if path.starts_with('/') {
        path.to_string()
    } else {
        format!("uploads/{}", path)
    }
}

/// Create a unique output path in the outputs/ directory.
fn output_path(ext: &str) -> (String, String) {
    let file_id = Uuid::new_v4().to_string();
    let path = format!("outputs/{}.{}", file_id, ext);
    (file_id, path)
}

// ─── Stabilize ───────────────────────────────────────────────────────────────

#[derive(Deserialize)]
pub struct StabilizeRequest {
    /// Relative path under uploads/ or absolute path on the server
    pub input_file: String,
    /// Shakiness detection strength: 1 (weakest) – 10 (strongest). Default 5.
    #[serde(default = "default_shakiness")]
    pub shakiness: u32,
    /// Accuracy of detection: 1–15. Default 10.
    #[serde(default = "default_accuracy")]
    pub accuracy: u32,
    /// Smoothing strength (larger = more stable but more crop). Default 10.
    #[serde(default = "default_smoothing")]
    pub smoothing: u32,
    /// Zoom percentage to apply (positive = zoom in to hide borders). Default 0.
    #[serde(default)]
    pub zoom: f64,
}

fn default_shakiness() -> u32 { 5 }
fn default_accuracy() -> u32 { 10 }
fn default_smoothing() -> u32 { 10 }

pub async fn stabilize_video(
    Extension(_state): Extension<Arc<AppState>>,
    Json(req): Json<StabilizeRequest>,
) -> Result<Json<ToolResult>, StatusCode> {
    let input = resolve_input(&req.input_file);
    let (file_id, output) = output_path("mp4");

    let shakiness = req.shakiness.clamp(1, 10);
    let accuracy = req.accuracy.clamp(1, 15);
    let smoothing = req.smoothing.clamp(1, 100);

    match crate::visual::stabilize_video_2pass(&input, &output, shakiness, accuracy, smoothing, req.zoom) {
        Ok(msg) => Ok(Json(ToolResult::ok(msg, &file_id))),
        Err(e) => Ok(Json(ToolResult::err(format!("Stabilization failed: {}", e)))),
    }
}

// ─── Format conversion ────────────────────────────────────────────────────────

#[derive(Deserialize)]
pub struct ConvertFormatRequest {
    pub input_file: String,
    /// Target format: mp4, mkv, webm, mov, avi, ts, mp3, aac, flac, wav, m4a
    pub format: String,
}

pub async fn convert_format(
    Extension(_state): Extension<Arc<AppState>>,
    Json(req): Json<ConvertFormatRequest>,
) -> Result<Json<ToolResult>, StatusCode> {
    let input = resolve_input(&req.input_file);
    let ext = match req.format.as_str() {
        "matroska" | "mkv" => "mkv",
        "webm" => "webm",
        "mov" => "mov",
        "avi" => "avi",
        "ts" | "mpegts" => "ts",
        "mp3" => "mp3",
        "aac" => "aac",
        "flac" => "flac",
        "wav" => "wav",
        "m4a" | "ipod" => "m4a",
        _ => "mp4",
    };
    let (file_id, output) = output_path(ext);

    match crate::export::convert_format(&input, &output, &req.format) {
        Ok(msg) => Ok(Json(ToolResult::ok(msg, &file_id))),
        Err(e) => Ok(Json(ToolResult::err(format!("Conversion failed: {}", e)))),
    }
}

// ─── Audio visualization (waveform PNG) ──────────────────────────────────────

#[derive(Deserialize)]
pub struct AudioVisualizeRequest {
    pub input_file: String,
    /// Visualization mode: "waveform" or "spectrum". Default "waveform".
    #[serde(default = "default_viz_mode")]
    pub mode: String,
    /// Width in pixels. Default 1280.
    #[serde(default = "default_viz_width")]
    pub width: u32,
    /// Height in pixels. Default 400.
    #[serde(default = "default_viz_height")]
    pub height: u32,
}

fn default_viz_mode() -> String { "waveform".into() }
fn default_viz_width() -> u32 { 1280 }
fn default_viz_height() -> u32 { 400 }

pub async fn visualize_audio(
    Extension(_state): Extension<Arc<AppState>>,
    Json(req): Json<AudioVisualizeRequest>,
) -> Result<Json<ToolResult>, StatusCode> {
    let input = resolve_input(&req.input_file);
    let (file_id, output) = output_path("mp4");

    let result = if req.mode == "spectrum" {
        crate::audio::measure_audio_spectrum(&input, &output, req.width, req.height, "combined", "intensity")
    } else if req.mode == "cqt" {
        crate::audio::visualize_cqt(&input, &output, req.width, req.height, 20, 30)
    } else {
        // default: waveform video
        crate::audio::generate_waveform_video(&input, &output, req.width, req.height, "cwave", "0xFFFFFF")
    };

    match result {
        Ok(msg) => Ok(Json(ToolResult::ok(msg, &file_id))),
        Err(e) => Ok(Json(ToolResult::err(format!("Visualization failed: {}", e)))),
    }
}

// ─── Workflow runner ──────────────────────────────────────────────────────────

#[derive(Deserialize)]
pub struct WorkflowRequest {
    pub input_file: String,
    /// One of: youtube_ready, podcast_cleanup, cinematic_grade, talking_head_cleanup
    pub workflow: String,
    // create_gif params (only used when workflow == "create_gif")
    #[serde(default)]
    pub start_seconds: f64,
    #[serde(default = "default_duration")]
    pub duration_seconds: f64,
    #[serde(default = "default_gif_width")]
    pub gif_width: u32,
    #[serde(default = "default_gif_fps")]
    pub gif_fps: f64,
}

fn default_duration() -> f64 { 5.0 }
fn default_gif_width() -> u32 { 480 }
fn default_gif_fps() -> f64 { 15.0 }

pub async fn run_workflow(
    Extension(_state): Extension<Arc<AppState>>,
    Json(req): Json<WorkflowRequest>,
) -> Result<Json<ToolResult>, StatusCode> {
    let input = resolve_input(&req.input_file);

    let (ext, file_id, output) = match req.workflow.as_str() {
        "create_gif" => {
            let (id, path) = output_path("gif");
            ("gif", id, path)
        }
        "podcast_cleanup" => {
            let (id, path) = output_path("wav");
            ("wav", id, path)
        }
        _ => {
            let (id, path) = output_path("mp4");
            ("mp4", id, path)
        }
    };
    let _ = ext; // used above to pick extension

    let result = match req.workflow.as_str() {
        "youtube_ready" => crate::workflows::youtube_ready_export(&input, &output),
        "podcast_cleanup" => crate::workflows::podcast_cleanup(&input, &output),
        "cinematic_grade" => crate::workflows::cinematic_grade(&input, &output),
        "talking_head_cleanup" => crate::workflows::talking_head_cleanup(&input, &output),
        "create_gif" => crate::workflows::create_gif(
            &input,
            &output,
            req.start_seconds,
            req.duration_seconds,
            req.gif_width,
            req.gif_fps,
        ),
        other => Err(format!("Unknown workflow: {}", other)),
    };

    match result {
        Ok(msg) => Ok(Json(ToolResult::ok(msg, &file_id))),
        Err(e) => Ok(Json(ToolResult::err(e))),
    }
}

// ─── Router ───────────────────────────────────────────────────────────────────

pub fn tools_routes() -> Router {
    Router::new()
        .route("/api/tools/stabilize", post(stabilize_video))
        .route("/api/tools/convert", post(convert_format))
        .route("/api/tools/visualize-audio", post(visualize_audio))
        .route("/api/tools/workflow", post(run_workflow))
}
