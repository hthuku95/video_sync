// src/handlers/output.rs
use crate::AppState;
use axum::{
    extract::{Extension, Path},
    http::{header, StatusCode},
    response::Response,
    routing::get,
    Router,
};
use serde::Serialize;
use std::{path::PathBuf, sync::Arc};
use tokio_util::io::ReaderStream;

#[derive(Serialize)]
pub struct VideoOutputResponse {
    pub file_id: String,
    pub filename: String,
    pub size_bytes: u64,
    pub download_url: String,
    pub stream_url: String,
    pub created_at: String,
    pub content_type: String,
}

#[derive(Serialize)]
pub struct VideoOutputListResponse {
    pub success: bool,
    pub outputs: Vec<VideoOutputResponse>,
}

pub fn output_routes() -> Router {
    Router::new()
        .route("/api/outputs/list/:session_id", get(list_session_outputs))
        .route("/api/outputs/download/:file_id", get(download_video_output))
        .route("/api/outputs/stream/:file_id", get(stream_video_output))
        .route("/api/outputs/info/:file_id", get(get_output_info))
}

/// List all video outputs for a session
async fn list_session_outputs(
    Path(session_id): Path<String>,
    Extension(_state): Extension<Arc<AppState>>,
) -> Result<axum::Json<VideoOutputListResponse>, StatusCode> {
    // Get output directory for this session
    let session_output_dir = PathBuf::from("outputs").join(&session_id);

    if !session_output_dir.exists() {
        return Ok(axum::Json(VideoOutputListResponse {
            success: true,
            outputs: vec![],
        }));
    }

    let mut outputs = Vec::new();

    // Read directory and collect video files
    match tokio::fs::read_dir(&session_output_dir).await {
        Ok(mut entries) => {
            while let Some(entry) = entries.next_entry().await.unwrap_or(None) {
                let path = entry.path();
                if let Some(extension) = path.extension() {
                    let ext_str = extension.to_string_lossy().to_lowercase();
                    if matches!(ext_str.as_str(), "mp4" | "avi" | "mov" | "mkv" | "webm") {
                        if let Some(filename) = path.file_name() {
                            let filename_str = filename.to_string_lossy();

                            // Get file metadata
                            if let Ok(metadata) = entry.metadata().await {
                                let file_id = generate_file_id(&path);

                                outputs.push(VideoOutputResponse {
                                    file_id: file_id.clone(),
                                    filename: filename_str.to_string(),
                                    size_bytes: metadata.len(),
                                    download_url: format!("/api/outputs/download/{}", file_id),
                                    stream_url: format!("/api/outputs/stream/{}", file_id),
                                    created_at: format_system_time(
                                        metadata.created().unwrap_or(std::time::SystemTime::now()),
                                    ),
                                    content_type: get_content_type(&ext_str),
                                });
                            }
                        }
                    }
                }
            }
        }
        Err(e) => {
            tracing::error!("Failed to read output directory: {}", e);
            return Err(StatusCode::INTERNAL_SERVER_ERROR);
        }
    }

    // Sort by creation time (newest first)
    outputs.sort_by(|a, b| b.created_at.cmp(&a.created_at));

    Ok(axum::Json(VideoOutputListResponse {
        success: true,
        outputs,
    }))
}

/// Download a video output file
async fn download_video_output(
    Path(file_id): Path<String>,
    Extension(state): Extension<Arc<AppState>>,
) -> Result<Response, StatusCode> {
    // Check for cloud URL first
    if let Ok(Some(artifact)) =
        crate::services::GeneratedArtifactService::find_by_legacy_file_id(&state.db_pool, &file_id)
            .await
    {
        if let Some(ref url) = artifact.public_url {
            if !url.starts_with('/') {
                // External URL — redirect
                return Ok(Response::builder()
                    .status(StatusCode::FOUND)
                    .header(header::LOCATION, url.as_str())
                    .body(axum::body::Body::empty())
                    .map_err(|_| StatusCode::INTERNAL_SERVER_ERROR)?);
            }
        }
    }

    let file_path = resolve_file_path(&state, &file_id).await?;

    if !file_path.exists() {
        return Err(StatusCode::NOT_FOUND);
    }

    // Open the file for reading
    match tokio::fs::File::open(&file_path).await {
        Ok(file) => {
            let stream = ReaderStream::new(file);
            let filename = file_path
                .file_name()
                .and_then(|name| name.to_str())
                .unwrap_or("video.mp4");

            let content_type = get_content_type_from_path(&file_path);

            Response::builder()
                .status(StatusCode::OK)
                .header(header::CONTENT_TYPE, content_type)
                .header(
                    header::CONTENT_DISPOSITION,
                    format!("attachment; filename=\"{}\"", filename),
                )
                .header(header::CACHE_CONTROL, "private, max-age=3600")
                .body(axum::body::Body::from_stream(stream))
                .map_err(|_| StatusCode::INTERNAL_SERVER_ERROR)
        }
        Err(e) => {
            tracing::error!("Failed to open file for download: {}", e);
            Err(StatusCode::INTERNAL_SERVER_ERROR)
        }
    }
}

/// Stream a video output file (for browser playback)
async fn stream_video_output(
    Path(file_id): Path<String>,
    Extension(state): Extension<Arc<AppState>>,
) -> Result<Response, StatusCode> {
    // Check for cloud URL first
    if let Ok(Some(artifact)) =
        crate::services::GeneratedArtifactService::find_by_legacy_file_id(&state.db_pool, &file_id)
            .await
    {
        if let Some(ref url) = artifact.public_url {
            if !url.starts_with('/') {
                return Ok(Response::builder()
                    .status(StatusCode::FOUND)
                    .header(header::LOCATION, url.as_str())
                    .body(axum::body::Body::empty())
                    .map_err(|_| StatusCode::INTERNAL_SERVER_ERROR)?);
            }
        }
    }

    let file_path = resolve_file_path(&state, &file_id).await?;

    if !file_path.exists() {
        return Err(StatusCode::NOT_FOUND);
    }

    // Open the file for streaming
    match tokio::fs::File::open(&file_path).await {
        Ok(file) => {
            let stream = ReaderStream::new(file);
            let content_type = get_content_type_from_path(&file_path);

            Response::builder()
                .status(StatusCode::OK)
                .header(header::CONTENT_TYPE, content_type)
                .header(header::ACCEPT_RANGES, "bytes")
                .header(header::CACHE_CONTROL, "public, max-age=3600")
                .body(axum::body::Body::from_stream(stream))
                .map_err(|_| StatusCode::INTERNAL_SERVER_ERROR)
        }
        Err(e) => {
            tracing::error!("Failed to open file for streaming: {}", e);
            Err(StatusCode::INTERNAL_SERVER_ERROR)
        }
    }
}

/// Get information about a video output file
async fn get_output_info(
    Path(file_id): Path<String>,
    Extension(state): Extension<Arc<AppState>>,
) -> Result<axum::Json<VideoOutputResponse>, StatusCode> {
    // Check for cloud artifact first
    if let Ok(Some(artifact)) =
        crate::services::GeneratedArtifactService::find_by_legacy_file_id(&state.db_pool, &file_id)
            .await
    {
        if let Some(ref url) = artifact.public_url {
            if !url.starts_with('/') {
                let filename = std::path::Path::new(url)
                    .file_name()
                    .and_then(|n| n.to_str())
                    .unwrap_or("output")
                    .to_string();
                return Ok(axum::Json(VideoOutputResponse {
                    file_id,
                    filename,
                    size_bytes: artifact.bytes.unwrap_or(0) as u64,
                    download_url: url.clone(),
                    stream_url: url.clone(),
                    created_at: artifact.created_at.format("%Y-%m-%d %H:%M:%S UTC").to_string(),
                    content_type: artifact.mime_type.unwrap_or_else(|| "application/octet-stream".to_string()),
                }));
            }
        }
    }

    let file_path = resolve_file_path(&state, &file_id).await?;

    if !file_path.exists() {
        return Err(StatusCode::NOT_FOUND);
    }

    match tokio::fs::metadata(&file_path).await {
        Ok(metadata) => {
            let filename = file_path
                .file_name()
                .and_then(|name| name.to_str())
                .unwrap_or("unknown.mp4")
                .to_string();

            let content_type = get_content_type_from_path(&file_path);

            Ok(axum::Json(VideoOutputResponse {
                file_id: file_id.clone(),
                filename,
                size_bytes: metadata.len(),
                download_url: format!("/api/outputs/download/{}", file_id),
                stream_url: format!("/api/outputs/stream/{}", file_id),
                created_at: format_system_time(
                    metadata.created().unwrap_or(std::time::SystemTime::now()),
                ),
                content_type,
            }))
        }
        Err(e) => {
            tracing::error!("Failed to get file metadata: {}", e);
            Err(StatusCode::INTERNAL_SERVER_ERROR)
        }
    }
}

// Helper functions

fn generate_file_id(path: &PathBuf) -> String {
    use std::collections::hash_map::DefaultHasher;
    use std::hash::{Hash, Hasher};

    // Convert to string first to match tool_executor.rs behavior
    let path_str = path.to_string_lossy().to_string();
    let mut hasher = DefaultHasher::new();
    path_str.hash(&mut hasher);
    format!("{:x}", hasher.finish())
}

pub(crate) fn resolve_output_file_path_for_verification(file_id: &str) -> Option<PathBuf> {
    resolve_file_path_from_scan(file_id).ok()
}

async fn resolve_file_path(state: &Arc<AppState>, file_id: &str) -> Result<PathBuf, StatusCode> {
    if let Ok(Some(artifact)) =
        crate::services::GeneratedArtifactService::find_by_legacy_file_id(&state.db_pool, file_id)
            .await
    {
        if let Some(path) = artifact.file_path {
            let candidate = PathBuf::from(path);
            if candidate.exists() {
                return Ok(candidate);
            }
        }
    }

    resolve_file_path_from_scan(file_id)
}

fn resolve_file_path_from_scan(file_id: &str) -> Result<PathBuf, StatusCode> {
    // In a production system, you'd want to store file_id -> path mappings in a database
    // For now, we'll scan both project root and outputs directory

    // CRITICAL FIX: Check project root directory first (where FFmpeg saves videos)
    let root_dir = PathBuf::from(".");
    if let Ok(root_entries) = std::fs::read_dir(&root_dir) {
        for entry in root_entries.flatten() {
            let path = entry.path();
            if path.is_file() && path.extension().and_then(|s| s.to_str()) == Some("mp4") {
                // Try matching with full path
                if generate_file_id(&path) == file_id {
                    return Ok(path);
                }
                // Also try matching with just the filename (for backward compatibility)
                if let Some(filename) = path.file_name().and_then(|n| n.to_str()) {
                    if generate_file_id(&PathBuf::from(filename)) == file_id {
                        return Ok(path);
                    }
                }
            }
        }
    }

    // Check outputs/ directory
    let outputs_dir = PathBuf::from("outputs");
    if let Ok(entries) = std::fs::read_dir(&outputs_dir) {
        for entry in entries.flatten() {
            let path = entry.path();

            // Check files in outputs/ directory
            if path.is_file() {
                // Try matching with full path
                if generate_file_id(&path) == file_id {
                    return Ok(path);
                }
                // Also try matching with just the filename (for backward compatibility)
                if let Some(filename) = path.file_name().and_then(|n| n.to_str()) {
                    if generate_file_id(&PathBuf::from(filename)) == file_id {
                        return Ok(path);
                    }
                }
            }

            if path.is_dir() {
                // Check session subdirectories
                if let Ok(session_entries) = std::fs::read_dir(&path) {
                    for session_entry in session_entries.flatten() {
                        let session_path = session_entry.path();
                        // Try matching with full path
                        if generate_file_id(&session_path) == file_id {
                            return Ok(session_path);
                        }
                        // Also try matching with just the filename
                        if let Some(filename) = session_path.file_name().and_then(|n| n.to_str()) {
                            if generate_file_id(&PathBuf::from(filename)) == file_id {
                                return Ok(session_path);
                            }
                        }
                    }
                }
            }
        }
    }

    Err(StatusCode::NOT_FOUND)
}

fn get_content_type(extension: &str) -> String {
    match extension.to_lowercase().as_str() {
        "mp4" => "video/mp4",
        "avi" => "video/x-msvideo",
        "mov" => "video/quicktime",
        "mkv" => "video/x-matroska",
        "webm" => "video/webm",
        _ => "application/octet-stream",
    }
    .to_string()
}

fn get_content_type_from_path(path: &PathBuf) -> String {
    if let Some(extension) = path.extension() {
        get_content_type(&extension.to_string_lossy())
    } else {
        "application/octet-stream".to_string()
    }
}

fn format_system_time(time: std::time::SystemTime) -> String {
    match time.duration_since(std::time::UNIX_EPOCH) {
        Ok(duration) => {
            let datetime = chrono::DateTime::from_timestamp(duration.as_secs() as i64, 0)
                .unwrap_or_else(chrono::Utc::now);
            datetime.format("%Y-%m-%d %H:%M:%S UTC").to_string()
        }
        Err(_) => chrono::Utc::now()
            .format("%Y-%m-%d %H:%M:%S UTC")
            .to_string(),
    }
}
