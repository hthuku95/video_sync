// src/handlers/output.rs
use axum::{
    extract::{Path, Extension},
    http::{header, StatusCode},
    response::{IntoResponse, Response},
    routing::get,
    Router,
};
use std::{path::PathBuf, sync::Arc};
use tokio_util::io::ReaderStream;
use crate::AppState;
use serde::{Deserialize, Serialize};

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
    Extension(state): Extension<Arc<AppState>>,
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
                                    created_at: format_system_time(metadata.created().unwrap_or(std::time::SystemTime::now())),
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
    Extension(_state): Extension<Arc<AppState>>,
) -> Result<Response, StatusCode> {
    let file_path = resolve_file_path(&file_id)?;
    
    if !file_path.exists() {
        return Err(StatusCode::NOT_FOUND);
    }

    // Open the file for reading
    match tokio::fs::File::open(&file_path).await {
        Ok(file) => {
            let stream = ReaderStream::new(file);
            let filename = file_path.file_name()
                .and_then(|name| name.to_str())
                .unwrap_or("video.mp4");
            
            let content_type = get_content_type_from_path(&file_path);
            
            Response::builder()
                .status(StatusCode::OK)
                .header(header::CONTENT_TYPE, content_type)
                .header(header::CONTENT_DISPOSITION, format!("attachment; filename=\"{}\"", filename))
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
    Extension(_state): Extension<Arc<AppState>>,
) -> Result<Response, StatusCode> {
    let file_path = resolve_file_path(&file_id)?;
    
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
    Extension(_state): Extension<Arc<AppState>>,
) -> Result<axum::Json<VideoOutputResponse>, StatusCode> {
    let file_path = resolve_file_path(&file_id)?;
    
    if !file_path.exists() {
        return Err(StatusCode::NOT_FOUND);
    }

    match tokio::fs::metadata(&file_path).await {
        Ok(metadata) => {
            let filename = file_path.file_name()
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
                created_at: format_system_time(metadata.created().unwrap_or(std::time::SystemTime::now())),
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
    
    let mut hasher = DefaultHasher::new();
    path.hash(&mut hasher);
    format!("{:x}", hasher.finish())
}

fn resolve_file_path(file_id: &str) -> Result<PathBuf, StatusCode> {
    // In a production system, you'd want to store file_id -> path mappings in a database
    // For now, we'll scan both project root and outputs directory

    // CRITICAL FIX: Check project root directory first (where FFmpeg saves videos)
    let root_dir = PathBuf::from(".");
    if let Ok(root_entries) = std::fs::read_dir(&root_dir) {
        for entry in root_entries.flatten() {
            let path = entry.path();
            if path.is_file() && path.extension().and_then(|s| s.to_str()) == Some("mp4") {
                if generate_file_id(&path) == file_id {
                    return Ok(path);
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
            if path.is_file() && generate_file_id(&path) == file_id {
                return Ok(path);
            }

            if path.is_dir() {
                // Check session subdirectories
                if let Ok(session_entries) = std::fs::read_dir(&path) {
                    for session_entry in session_entries.flatten() {
                        let session_path = session_entry.path();
                        if generate_file_id(&session_path) == file_id {
                            return Ok(session_path);
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
    }.to_string()
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
        Err(_) => chrono::Utc::now().format("%Y-%m-%d %H:%M:%S UTC").to_string(),
    }
}

/// Ensure output directory exists for a session
pub async fn ensure_session_output_directory(session_id: &str) -> Result<PathBuf, std::io::Error> {
    let output_dir = PathBuf::from("outputs").join(session_id);
    tokio::fs::create_dir_all(&output_dir).await?;
    Ok(output_dir)
}