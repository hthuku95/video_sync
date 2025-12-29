use crate::models::file::{FileUploadResponse, MultipleFileUploadResponse};
use crate::middleware::auth::auth_middleware;
use crate::services::VideoVectorizationService;
use crate::AppState;
use sqlx::Row;
use axum::{
    extract::{multipart::Multipart, Extension, DefaultBodyLimit},
    http::StatusCode,
    response::Json,
    routing::post,
    Router,
};
use serde_json::{json, Value};
use std::path::Path;
use std::sync::Arc;
use tokio::fs;
use tokio::io::AsyncWriteExt;
use uuid::Uuid;

pub fn upload_routes() -> Router {
    let public_routes = Router::new()
        .route("/upload", post(upload_files))
        .route("/upload/form", axum::routing::get(upload_form))
        .route("/upload/status/:file_id", axum::routing::get(get_upload_status))
        .route("/upload/session/:session_uuid", post(upload_files_for_session))
        .layer(DefaultBodyLimit::max(100 * 1024 * 1024)); // 100MB limit for file uploads
    
    let protected_routes = Router::new()
        .route("/files/session/:session_uuid", axum::routing::get(get_session_files))
        .layer(axum::middleware::from_fn(auth_middleware));
    
    public_routes.merge(protected_routes)
}

pub async fn upload_form() -> axum::response::Html<String> {
    let html = r#"
    <!DOCTYPE html>
    <html>
    <head>
        <title>Video Editor - File Upload</title>
        <style>
            body { font-family: Arial, sans-serif; max-width: 800px; margin: 50px auto; padding: 20px; }
            .upload-area { border: 2px dashed #ccc; border-radius: 10px; padding: 40px; text-align: center; margin: 20px 0; }
            .upload-area:hover { border-color: #007bff; background-color: #f8f9fa; }
            button { background-color: #007bff; color: white; padding: 10px 20px; border: none; border-radius: 5px; cursor: pointer; }
            button:hover { background-color: #0056b3; }
            .file-list { margin-top: 20px; }
            .file-item { padding: 10px; border: 1px solid #ddd; margin: 5px 0; border-radius: 5px; }
            .success { color: green; }
            .error { color: red; }
        </style>
    </head>
    <body>
        <h1>🎬 VideoSync</h1>
        <p>Upload your videos, images, audio files, and documents to start editing with AI assistance.</p>
        
        <div class="upload-area" id="uploadArea">
            <p>📁 Drag and drop files here or click to select</p>
            <input type="file" id="fileInput" multiple accept="video/*,audio/*,image/*,.pdf,.doc,.docx,.txt" style="display: none;">
            <button onclick="document.getElementById('fileInput').click()">Choose Files</button>
        </div>
        
        <div id="fileList" class="file-list"></div>
        <div id="uploadStatus"></div>
        
        <script>
            const uploadArea = document.getElementById('uploadArea');
            const fileInput = document.getElementById('fileInput');
            const fileList = document.getElementById('fileList');
            const uploadStatus = document.getElementById('uploadStatus');
            
            // Handle drag and drop
            uploadArea.addEventListener('dragover', (e) => {
                e.preventDefault();
                uploadArea.style.backgroundColor = '#f8f9fa';
            });
            
            uploadArea.addEventListener('dragleave', () => {
                uploadArea.style.backgroundColor = '';
            });
            
            uploadArea.addEventListener('drop', (e) => {
                e.preventDefault();
                uploadArea.style.backgroundColor = '';
                const files = e.dataTransfer.files;
                handleFiles(files);
            });
            
            fileInput.addEventListener('change', (e) => {
                handleFiles(e.target.files);
            });
            
            function handleFiles(files) {
                fileList.innerHTML = '';
                for (let file of files) {
                    const fileItem = document.createElement('div');
                    fileItem.className = 'file-item';
                    fileItem.innerHTML = `📄 ${file.name} (${(file.size / 1024 / 1024).toFixed(2)} MB)`;
                    fileList.appendChild(fileItem);
                }
                
                if (files.length > 0) {
                    uploadFiles(files);
                }
            }
            
            async function uploadFiles(files) {
                const formData = new FormData();
                for (let file of files) {
                    formData.append('files', file);
                }
                
                uploadStatus.innerHTML = '<p>⏳ Uploading files...</p>';
                
                // Generate a session UUID for this upload
                const sessionUuid = crypto.randomUUID();
                
                try {
                    const response = await fetch(`/upload/session/${sessionUuid}`, {
                        method: 'POST',
                        body: formData
                    });
                    
                    const result = await response.json();
                    
                    if (result.success) {
                        uploadStatus.innerHTML = `<p class="success">✅ ${result.message}</p>`;
                        
                        // Update file list with upload results
                        fileList.innerHTML = '';
                        result.files.forEach(file => {
                            const fileItem = document.createElement('div');
                            fileItem.className = 'file-item';
                            fileItem.innerHTML = `
                                ✅ <strong>${file.original_name}</strong><br>
                                📁 ${file.path}<br>
                                📊 ${(file.size / 1024 / 1024).toFixed(2)} MB | ${file.file_type}
                            `;
                            fileList.appendChild(fileItem);
                        });
                    } else {
                        uploadStatus.innerHTML = '<p class="error">❌ Upload failed</p>';
                    }
                } catch (error) {
                    uploadStatus.innerHTML = '<p class="error">❌ Error uploading files: ' + error.message + '</p>';
                }
            }
        </script>
    </body>
    </html>
    "#;
    
    axum::response::Html(html.to_string())
}

pub async fn upload_files(
    Extension(state): Extension<Arc<AppState>>,
    mut multipart: Multipart,
) -> Result<Json<MultipleFileUploadResponse>, StatusCode> {
    let mut uploaded_files = Vec::new();
    let upload_dir = "uploads";
    
    // Ensure upload directory exists
    if let Err(_) = fs::create_dir_all(&upload_dir).await {
        return Err(StatusCode::INTERNAL_SERVER_ERROR);
    }

    while let Some(field) = multipart.next_field().await.map_err(|_| StatusCode::BAD_REQUEST)? {
        let name = field.name().unwrap_or("unknown").to_string();
        let filename = field.file_name().unwrap_or("unknown").to_string();
        
        // Generate unique filename
        let file_extension = Path::new(&filename)
            .extension()
            .and_then(|ext| ext.to_str())
            .unwrap_or("");
        let unique_filename = format!("{}_{}.{}", Uuid::new_v4(), name, file_extension);
        let file_path = format!("{}/{}", upload_dir, unique_filename);
        
        let data = field.bytes().await.map_err(|_| StatusCode::BAD_REQUEST)?;
        
        // Validate file type
        let file_type = detect_file_type(&filename, &data);
        if !is_supported_file_type(&file_type) {
            tracing::warn!("Rejected file '{}' with unsupported file type: {}", filename, file_type);
            continue;
        }
        
        // Write file to disk
        match fs::File::create(&file_path).await {
            Ok(mut file) => {
                if let Err(_) = file.write_all(&data).await {
                    return Err(StatusCode::INTERNAL_SERVER_ERROR);
                }
            }
            Err(_) => return Err(StatusCode::INTERNAL_SERVER_ERROR),
        }
        
        let file_id = Uuid::new_v4().to_string();
        let mime_type = detect_mime_type(&filename);
        
        // Save to database (simplified query to avoid compile-time checks)
        let insert_result = sqlx::query(
            "INSERT INTO uploaded_files (id, original_name, stored_name, file_path, file_size, file_type, mime_type, upload_status) VALUES ($1, $2, $3, $4, $5, $6, $7, $8)"
        )
        .bind(&file_id)
        .bind(&filename)
        .bind(&unique_filename)
        .bind(&file_path)
        .bind(data.len() as i64)
        .bind(&file_type)
        .bind(&mime_type)
        .bind("uploaded")
        .execute(&state.db_pool)
        .await;
        
        match insert_result {
            Ok(_) => {
                uploaded_files.push(FileUploadResponse {
                    id: file_id,
                    original_name: filename.clone(),
                    stored_name: unique_filename.clone(),
                    path: file_path.clone(),
                    file_size: data.len() as i64,
                    file_type,
                    status: "uploaded".to_string(),
                });
                
                tracing::info!("Uploaded and stored file: {} -> {}", filename, file_path);
            }
            Err(e) => {
                tracing::error!("Failed to save file to database: {}", e);
                // Clean up the file if database save failed
                let _ = fs::remove_file(&file_path).await;
                continue;
            }
        }
    }

    let file_count = uploaded_files.len();
    Ok(Json(MultipleFileUploadResponse {
        success: true,
        files: uploaded_files,
        message: format!("Successfully uploaded {} files", file_count),
    }))
}

pub async fn get_upload_status(
    axum::extract::Path(file_id): axum::extract::Path<String>,
    Extension(state): Extension<Arc<AppState>>,
) -> Result<Json<Value>, StatusCode> {
    match sqlx::query_as::<_, crate::models::file::UploadedFile>(
        "SELECT id, session_id, original_name, stored_name, file_path, file_size, file_type, mime_type, upload_status, created_at, updated_at FROM uploaded_files WHERE id = $1"
    )
    .bind(&file_id)
    .fetch_optional(&state.db_pool)
    .await
    {
        Ok(Some(file)) => Ok(Json(json!({
            "file_id": file.id,
            "original_name": file.original_name,
            "file_type": file.file_type,
            "file_size": file.file_size,
            "status": file.upload_status,
            "created_at": file.created_at,
            "message": "File found"
        }))),
        Ok(None) => Err(StatusCode::NOT_FOUND),
        Err(e) => {
            tracing::error!("Database error checking file status: {}", e);
            Err(StatusCode::INTERNAL_SERVER_ERROR)
        }
    }
}

fn detect_file_type(filename: &str, _data: &[u8]) -> String {
    let extension = Path::new(filename)
        .extension()
        .and_then(|ext| ext.to_str())
        .unwrap_or("")
        .to_lowercase();
    
    match extension.as_str() {
        // Video formats
        "mp4" | "avi" | "mov" | "wmv" | "flv" | "webm" | "mkv" => "video".to_string(),
        // Audio formats  
        "mp3" | "wav" | "aac" | "flac" | "ogg" | "m4a" => "audio".to_string(),
        // Image formats
        "jpg" | "jpeg" | "png" | "gif" | "bmp" | "tiff" | "webp" => "image".to_string(),
        // Document formats
        "pdf" | "doc" | "docx" | "txt" | "rtf" => "document".to_string(),
        _ => "unknown".to_string(),
    }
}

fn is_supported_file_type(file_type: &str) -> bool {
    matches!(file_type, "video" | "audio" | "image" | "document")
}

fn detect_mime_type(filename: &str) -> Option<String> {
    let extension = std::path::Path::new(filename)
        .extension()
        .and_then(|ext| ext.to_str())
        .unwrap_or("")
        .to_lowercase();
    
    let mime_type = match extension.as_str() {
        // Video MIME types
        "mp4" => "video/mp4",
        "avi" => "video/x-msvideo",
        "mov" => "video/quicktime",
        "wmv" => "video/x-ms-wmv",
        "flv" => "video/x-flv",
        "webm" => "video/webm",
        "mkv" => "video/x-matroska",
        
        // Audio MIME types
        "mp3" => "audio/mpeg",
        "wav" => "audio/wav",
        "aac" => "audio/aac",
        "flac" => "audio/flac",
        "ogg" => "audio/ogg",
        "m4a" => "audio/mp4",
        
        // Image MIME types
        "jpg" | "jpeg" => "image/jpeg",
        "png" => "image/png",
        "gif" => "image/gif",
        "bmp" => "image/bmp",
        "tiff" => "image/tiff",
        "webp" => "image/webp",
        
        // Document MIME types
        "pdf" => "application/pdf",
        "doc" => "application/msword",
        "docx" => "application/vnd.openxmlformats-officedocument.wordprocessingml.document",
        "txt" => "text/plain",
        "rtf" => "application/rtf",
        
        _ => return None,
    };
    
    Some(mime_type.to_string())
}

// Upload files and associate them with a specific chat session
pub async fn upload_files_for_session(
    axum::extract::Path(session_uuid): axum::extract::Path<String>,
    Extension(state): Extension<Arc<AppState>>,
    mut multipart: Multipart,
) -> Result<Json<MultipleFileUploadResponse>, StatusCode> {
    tracing::info!("Starting file upload for session: {}", session_uuid);
    let mut uploaded_files = Vec::new();
    let upload_dir = "uploads";
    
    // Ensure upload directory exists
    if let Err(_) = fs::create_dir_all(&upload_dir).await {
        tracing::error!("Failed to create upload directory: {}", upload_dir);
        return Err(StatusCode::INTERNAL_SERVER_ERROR);
    }

    // Get or create chat session
    let session_id = match get_or_create_session(&state, &session_uuid).await {
        Ok(id) => Some(id),
        Err(e) => {
            tracing::warn!("Failed to get/create session {}: {}", session_uuid, e);
            None // Files will be uploaded without session association
        }
    };

    while let Some(field) = multipart.next_field().await.map_err(|e| {
        tracing::error!("Failed to parse multipart field: {}", e);
        StatusCode::BAD_REQUEST
    })? {
        // Skip empty fields
        if field.name().is_none() {
            tracing::warn!("Skipping field with no name");
            continue;
        }
        
        let name = field.name().unwrap().to_string();
        tracing::debug!("Processing field: {}", name);
        
        // Only process fields named 'files'
        if name != "files" {
            tracing::debug!("Skipping non-file field: {}", name);
            continue;
        }
        
        let filename = match field.file_name() {
            Some(name) => name.to_string(),
            None => {
                tracing::warn!("Field '{}' has no filename, skipping", name);
                continue;
            }
        };
        
        tracing::info!("Processing file upload: {} for session {}", filename, session_uuid);
        
        // Generate unique filename
        let file_extension = Path::new(&filename)
            .extension()
            .and_then(|ext| ext.to_str())
            .unwrap_or("");
        let unique_filename = format!("{}_{}.{}", Uuid::new_v4(), name, file_extension);
        let file_path = format!("{}/{}", upload_dir, unique_filename);
        
        let data = field.bytes().await.map_err(|e| {
            tracing::error!("Failed to read field bytes for file '{}': {}", filename, e);
            StatusCode::BAD_REQUEST
        })?;
        
        // Validate file type
        let file_type = detect_file_type(&filename, &data);
        if !is_supported_file_type(&file_type) {
            tracing::warn!("Rejected file '{}' with unsupported file type: {} for session {}", filename, file_type, session_uuid);
            continue;
        }
        
        // Write file to disk
        match fs::File::create(&file_path).await {
            Ok(mut file) => {
                if let Err(_) = file.write_all(&data).await {
                    return Err(StatusCode::INTERNAL_SERVER_ERROR);
                }
            }
            Err(_) => return Err(StatusCode::INTERNAL_SERVER_ERROR),
        }
        
        let file_id = Uuid::new_v4().to_string();
        let mime_type = detect_mime_type(&filename);
        
        // Save to database with session association
        let insert_result = sqlx::query(
            "INSERT INTO uploaded_files (id, session_id, original_name, stored_name, file_path, file_size, file_type, mime_type, upload_status) VALUES ($1, $2, $3, $4, $5, $6, $7, $8, $9)"
        )
        .bind(&file_id)
        .bind(session_id)
        .bind(&filename)
        .bind(&unique_filename)
        .bind(&file_path)
        .bind(data.len() as i64)
        .bind(&file_type)
        .bind(&mime_type)
        .bind("uploaded")
        .execute(&state.db_pool)
        .await;
        
        match insert_result {
            Ok(_) => {
                uploaded_files.push(FileUploadResponse {
                    id: file_id.clone(),
                    original_name: filename.clone(),
                    stored_name: unique_filename.clone(),
                    path: file_path.clone(),
                    file_size: data.len() as i64,
                    file_type: file_type.clone(),
                    status: "uploaded".to_string(),
                });
                
                tracing::info!("Uploaded file for session {}: {} -> {}", session_uuid, filename, file_path);
                
                // Process video files for vectorization
                if file_type == "video" {
                    let state_clone = state.clone();
                    let file_id_clone = file_id.clone();
                    let session_uuid_clone = session_uuid.clone();
                    let file_path_clone = file_path.clone();
                    
                    tokio::spawn(async move {
                        tracing::info!("Starting background video vectorization for file: {}", file_id_clone);
                        match VideoVectorizationService::process_video_for_vectorization(
                            &file_path_clone,
                            &file_id_clone,
                            &session_uuid_clone,
                            None, // user_id - will be extracted from session in the service
                            &state_clone,
                        ).await {
                            Ok(_) => {
                                tracing::info!("Successfully vectorized video: {}", file_id_clone);
                            },
                            Err(e) => {
                                tracing::error!("Failed to vectorize video {}: {}", file_id_clone, e);
                            }
                        }
                    });
                }
            }
            Err(e) => {
                tracing::error!("Failed to save file to database: {}", e);
                // Clean up the file if database save failed
                let _ = fs::remove_file(&file_path).await;
                continue;
            }
        }
    }

    let file_count = uploaded_files.len();
    
    if file_count == 0 {
        return Err(StatusCode::BAD_REQUEST);
    }
    
    Ok(Json(MultipleFileUploadResponse {
        success: true,
        files: uploaded_files,
        message: format!("Successfully uploaded {} files for session {}", file_count, session_uuid),
    }))
}

// Get all files associated with a chat session
pub async fn get_session_files(
    axum::extract::Path(session_uuid): axum::extract::Path<String>,
    Extension(state): Extension<Arc<AppState>>,
) -> Result<Json<Value>, StatusCode> {
    match sqlx::query_as::<_, crate::models::file::UploadedFile>(
        "SELECT uf.id, uf.session_id, uf.original_name, uf.stored_name, uf.file_path, uf.file_size, uf.file_type, uf.mime_type, uf.upload_status, uf.created_at, uf.updated_at FROM uploaded_files uf JOIN chat_sessions cs ON uf.session_id = cs.id WHERE cs.session_uuid = $1 ORDER BY uf.created_at DESC"
    )
    .bind(&session_uuid)
    .fetch_all(&state.db_pool)
    .await
    {
        Ok(files) => {
            let file_responses: Vec<Value> = files
                .into_iter()
                .map(|file| json!({
                    "id": file.id,
                    "original_name": file.original_name,
                    "stored_name": file.stored_name,
                    "file_type": file.file_type,
                    "file_size": file.file_size,
                    "mime_type": file.mime_type,
                    "upload_status": file.upload_status,
                    "created_at": file.created_at
                }))
                .collect();
            
            Ok(Json(json!({
                "success": true,
                "session_uuid": session_uuid,
                "files": file_responses,
                "message": format!("Found {} files for session", file_responses.len())
            })))
        }
        Err(e) => {
            tracing::error!("Database error getting session files: {}", e);
            Err(StatusCode::INTERNAL_SERVER_ERROR)
        }
    }
}

// Helper function to get or create a chat session
pub async fn get_or_create_session(state: &AppState, session_uuid: &str) -> Result<i32, sqlx::Error> {
    // First try to find existing session
    let session_row = sqlx::query("SELECT id FROM chat_sessions WHERE session_uuid = $1")
        .bind(session_uuid)
        .fetch_optional(&state.db_pool)
        .await?;
    
    if let Some(session) = session_row {
        let id: i32 = session.get("id");
        return Ok(id);
    }
    
    // If no session exists, create a new one with a default user (for now)
    // In a real app, you'd get the user_id from authentication
    let default_user_id = 1; // This should come from auth context
    
    let new_session = sqlx::query("INSERT INTO chat_sessions (user_id, session_uuid, title) VALUES ($1, $2, $3) RETURNING id")
        .bind(default_user_id)
        .bind(session_uuid)
        .bind("New Chat Session")
        .fetch_one(&state.db_pool)
        .await?;
    
    let new_id: i32 = new_session.get("id");
    tracing::info!("Created new chat session: {} (id: {})", session_uuid, new_id);
    Ok(new_id)
}