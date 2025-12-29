// src/services/output_video.rs
use crate::models::file::OutputVideo;
use chrono::Utc;
use sqlx::PgPool;
use std::path::Path;
use std::fs;

pub struct OutputVideoService;

impl OutputVideoService {
    /// Save output video metadata to database after tool execution
    pub async fn save_output_video(
        pool: &PgPool,
        session_id: i32,
        user_id: i32,
        original_input_file_id: Option<String>,
        file_path: &str,
        operation_type: &str,
        operation_params: Option<&str>,
        tool_used: &str,
        ai_response_message: Option<&str>,
    ) -> Result<OutputVideo, sqlx::Error> {
        let file_path_obj = Path::new(file_path);
        
        // Extract file metadata
        let file_name = file_path_obj.file_name()
            .and_then(|n| n.to_str())
            .unwrap_or("output.mp4")
            .to_string();
            
        let file_size = fs::metadata(file_path)
            .map(|m| m.len() as i64)
            .unwrap_or(0);
            
        let mime_type = Self::determine_mime_type(&file_name);

        // Analyze video to get metadata (if possible)
        let (duration, width, height, frame_rate) = Self::analyze_video_metadata(file_path).await;

        // Insert into database
        let result = sqlx::query_as::<_, OutputVideo>(
            r#"
            INSERT INTO output_videos (
                session_id, user_id, original_input_file_id, file_name, file_path, file_size, 
                mime_type, duration_seconds, width, height, frame_rate, operation_type, 
                operation_params, processing_status, tool_used, ai_response_message, 
                created_at, updated_at
            ) VALUES ($1, $2, $3, $4, $5, $6, $7, $8, $9, $10, $11, $12, $13, $14, $15, $16, $17, $17)
            RETURNING *
            "#,
        )
        .bind(session_id)
        .bind(user_id)
        .bind(original_input_file_id)
        .bind(file_name)
        .bind(file_path)
        .bind(file_size)
        .bind(mime_type)
        .bind(duration)
        .bind(width)
        .bind(height)
        .bind(frame_rate)
        .bind(operation_type)
        .bind(operation_params)
        .bind("completed") // processing_status
        .bind(tool_used)
        .bind(ai_response_message)
        .bind(Utc::now())
        .fetch_one(pool).await?;

        Ok(result)
    }

    /// Get all output videos for a session
    pub async fn get_session_output_videos(
        pool: &PgPool,
        session_id: i32,
    ) -> Result<Vec<OutputVideo>, sqlx::Error> {
        sqlx::query_as::<_, OutputVideo>(
            "SELECT * FROM output_videos WHERE session_id = $1 ORDER BY created_at DESC"
        )
        .bind(session_id)
        .fetch_all(pool)
        .await
    }

    /// Get output video by ID
    pub async fn get_output_video_by_id(
        pool: &PgPool,
        id: i32,
    ) -> Result<Option<OutputVideo>, sqlx::Error> {
        sqlx::query_as::<_, OutputVideo>(
            "SELECT * FROM output_videos WHERE id = $1"
        )
        .bind(id)
        .fetch_optional(pool)
        .await
    }

    /// Get output video by file path (for backward compatibility with existing file_id system)
    pub async fn get_output_video_by_path(
        pool: &PgPool,
        file_path: &str,
    ) -> Result<Option<OutputVideo>, sqlx::Error> {
        sqlx::query_as::<_, OutputVideo>(
            "SELECT * FROM output_videos WHERE file_path = $1"
        )
        .bind(file_path)
        .fetch_optional(pool)
        .await
    }

    /// Determine MIME type based on file extension
    fn determine_mime_type(filename: &str) -> String {
        match filename.split('.').last().unwrap_or("").to_lowercase().as_str() {
            "mp4" => "video/mp4",
            "avi" => "video/avi", 
            "mov" => "video/quicktime",
            "mkv" => "video/x-matroska",
            "webm" => "video/webm",
            "flv" => "video/x-flv",
            "wmv" => "video/x-ms-wmv",
            _ => "video/mp4",
        }.to_string()
    }

    /// Analyze video metadata using existing core video analysis
    async fn analyze_video_metadata(file_path: &str) -> (Option<f64>, Option<i32>, Option<i32>, Option<f64>) {
        // Use the existing analyze_video function from core module
        match crate::core::analyze_video(file_path) {
            Ok(metadata) => (
                Some(metadata.duration_seconds),
                Some(metadata.width as i32),
                Some(metadata.height as i32),
                Some(metadata.fps),
            ),
            Err(_) => (None, None, None, None),
        }
    }

    /// Build file context string for AI agent (similar to get_session_files but for output videos)
    pub async fn build_output_video_context(
        pool: &PgPool,
        session_id: i32,
    ) -> Result<String, sqlx::Error> {
        let output_videos = Self::get_session_output_videos(pool, session_id).await?;
        
        if output_videos.is_empty() {
            return Ok(String::new());
        }

        let mut context = String::from("PREVIOUS OUTPUT VIDEOS IN THIS CHAT SESSION:\n");
        
        for (index, video) in output_videos.iter().enumerate() {
            let created_at_formatted = video.created_at.format("%Y-%m-%d %H:%M:%S UTC");
            let file_size_mb = video.file_size as f64 / (1024.0 * 1024.0);
            
            context.push_str(&format!(
                "{}. Video: \"{}\" (ID: {})\n   - Operation: {} using {}\n   - Size: {:.2} MB\n   - Path: {}\n   - Created: {}\n",
                index + 1,
                video.file_name,
                video.id,
                video.operation_type,
                video.tool_used,
                file_size_mb,
                video.file_path,
                created_at_formatted
            ));
            
            if let Some(params) = &video.operation_params {
                context.push_str(&format!("   - Parameters: {}\n", params));
            }
            context.push('\n');
        }
        
        context.push_str("IMPORTANT: You can reference these previous output videos by their file names or IDs for further editing!\n\n");
        
        Ok(context)
    }
}