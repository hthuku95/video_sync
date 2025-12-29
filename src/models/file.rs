use serde::{Deserialize, Serialize};
use sqlx::FromRow;

#[derive(Debug, Serialize, Deserialize, FromRow)]
pub struct UploadedFile {
    pub id: String,
    pub session_id: Option<i32>,
    pub original_name: String,
    pub stored_name: String,
    pub file_path: String,
    pub file_size: i64,
    pub file_type: String,
    pub mime_type: Option<String>,
    pub upload_status: String,
    pub created_at: chrono::DateTime<chrono::Utc>,
    pub updated_at: chrono::DateTime<chrono::Utc>,
}

#[derive(Debug, Serialize, Deserialize)]
pub struct FileUploadResponse {
    pub id: String,
    pub original_name: String,
    pub stored_name: String,
    pub path: String,
    pub file_size: i64,  // Changed from 'size' to match frontend expectations
    pub file_type: String,
    pub status: String,
}

#[derive(Debug, Serialize, Deserialize)]
pub struct MultipleFileUploadResponse {
    pub success: bool,
    pub files: Vec<FileUploadResponse>,
    pub message: String,
}

#[derive(Debug, Serialize, Deserialize, FromRow)]
pub struct OutputVideo {
    pub id: i32,
    pub session_id: i32,
    pub user_id: i32,
    pub original_input_file_id: Option<String>,
    pub file_name: String,
    pub file_path: String,
    pub file_size: i64,
    pub mime_type: String,
    pub duration_seconds: Option<f64>,
    pub width: Option<i32>,
    pub height: Option<i32>,
    pub frame_rate: Option<f64>,
    pub operation_type: String,
    pub operation_params: Option<String>,
    pub processing_status: String,
    pub tool_used: String,
    pub ai_response_message: Option<String>,
    pub created_at: chrono::DateTime<chrono::Utc>,
    pub updated_at: chrono::DateTime<chrono::Utc>,
}

#[derive(Debug, Serialize, Deserialize)]
pub struct OutputVideoResponse {
    pub id: i32,
    pub file_name: String,
    pub file_size: i64,
    pub duration_seconds: Option<f64>,
    pub width: Option<i32>,
    pub height: Option<i32>,
    pub operation_type: String,
    pub tool_used: String,
    pub download_url: String,
    pub stream_url: String,
    pub created_at: String,
}