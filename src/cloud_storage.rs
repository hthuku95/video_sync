use crate::AppState;
use std::sync::Arc;
use uuid::Uuid;

/// Upload bytes directly to R2 + GCS, register in GeneratedArtifactService,
/// and return a presigned download URL. No local disk write.
pub async fn upload_bytes_to_cloud(
    bytes: Vec<u8>,
    object_key: &str,
    content_type: &str,
    user_id: i32,
    session_id: &str,
    workflow_id: Option<Uuid>,
    app_state: &Arc<AppState>,
) -> Result<String, String> {
    let now = chrono::Utc::now();
    let storage_key = object_key.to_string();

    // 1. Write to a temp file for R2 upload (R2 SDK requires a file path)
    let tmp = std::env::temp_dir().join(&storage_key.replace('/', "_"));
    if let Some(parent) = tmp.parent() {
        tokio::fs::create_dir_all(parent)
            .await
            .map_err(|e| format!("Failed to create temp dir: {e}"))?;
    }
    tokio::fs::write(&tmp, &bytes)
        .await
        .map_err(|e| format!("Failed to write temp file: {e}"))?;

    // 2. Upload to R2
    let mut public_url: Option<String> = None;
    if let Some(r2) = app_state.r2_client.as_ref() {
        match r2.upload(tmp.to_str().unwrap_or(""), &storage_key).await {
            Ok(()) => match r2.presign_get(&storage_key, 7 * 24 * 3600).await {
                Ok(url) => {
                    public_url = Some(url.clone());
                    tracing::info!("R2: uploaded → {storage_key} → {url}");
                }
                Err(e) => tracing::warn!("R2 presign failed: {e}"),
            },
            Err(e) => tracing::warn!("R2 upload failed: {e}"),
        }
    }

    // 3. Upload to GCS
    if let Some(gcs) = app_state.gcs_client.as_ref() {
        match gcs.upload_bytes(&bytes, &storage_key, content_type).await {
            Ok(url) => {
                if public_url.is_none() {
                    public_url = Some(url.clone());
                }
                tracing::info!("GCS: uploaded → {storage_key}");
            }
            Err(e) => tracing::warn!("GCS upload failed: {e}"),
        }
    }

    // 4. Clean up temp file
    let _ = tokio::fs::remove_file(&tmp).await;

    if public_url.is_none() {
        return Err(format!(
            "No cloud storage available (R2 and GCS both unavailable or failed)"
        ));
    }

    let public_url = public_url.unwrap();

    // 5. Register in GeneratedArtifactService
    let storage_backend = if app_state.gcs_client.is_some() && app_state.r2_client.is_some() {
        "r2_gcs"
    } else if app_state.r2_client.is_some() {
        "r2"
    } else {
        "gcs"
    };

    let file_name = std::path::Path::new(object_key)
        .file_name()
        .and_then(|n| n.to_str())
        .unwrap_or(object_key);

    let legacy_file_id = crate::services::GeneratedArtifactService::legacy_file_id(file_name);

    let session_uuid = if session_id.is_empty() {
        None
    } else {
        Some(session_id.to_string())
    };

    let sql_result = sqlx::query_as::<_, crate::models::file::GeneratedArtifact>(
        r#"
        INSERT INTO generated_artifacts (
            workflow_id, session_uuid, kind, storage_backend, storage_key,
            file_path, legacy_file_id, public_url, preview_url,
            mime_type, bytes, checksum, source_table, source_record_key,
            created_at, updated_at
        ) VALUES (
            $1, $2, $3, $4, $5,
            $6, $7, $8, $9,
            $10, $11, NULL, 'cloud_uploads', $12, $13, $13
        )
        RETURNING *
        "#,
    )
    .bind(workflow_id)
    .bind(session_uuid)
    .bind(infer_kind_from_mime(content_type))
    .bind(storage_backend)
    .bind(&storage_key)
    .bind(Some(storage_key.clone()))
    .bind(Some(legacy_file_id))
    .bind(Some(&public_url))
    .bind(Some(&public_url))
    .bind(Some(content_type))
    .bind(Some(bytes.len() as i64))
    .bind(format!("{}:{}", session_id, file_name))
    .bind(now)
    .fetch_optional(&app_state.db_pool)
    .await;

    match sql_result {
        Ok(Some(artifact)) => {
            tracing::info!(
                artifact_id = %artifact.artifact_id,
                storage_backend = %artifact.storage_backend,
                "Registered cloud artifact"
            );
        }
        Ok(None) => {
            tracing::warn!("No artifact returned from INSERT (conflict?)");
        }
        Err(e) => {
            tracing::warn!("Failed to register cloud artifact in DB: {e}");
        }
    }

    Ok(public_url)
}

pub async fn upload_local_file_to_cloud(
    local_path: &str,
    content_type: &str,
    user_id: i32,
    session_id: &str,
    workflow_id: Option<Uuid>,
    app_state: &Arc<AppState>,
) -> Result<String, String> {
    let bytes = tokio::fs::read(local_path)
        .await
        .map_err(|e| format!("Failed to read local file {local_path}: {e}"))?;

    let file_name = std::path::Path::new(local_path)
        .file_name()
        .and_then(|n| n.to_str())
        .unwrap_or("unknown");

    let object_key = format!("generated/{user_id}/{file_name}");

    upload_bytes_to_cloud(
        bytes, &object_key, content_type, user_id, session_id, workflow_id, app_state,
    )
    .await
}

fn infer_kind_from_mime(mime: &str) -> &'static str {
    if mime.starts_with("video/") {
        "video"
    } else if mime.starts_with("image/") {
        "image"
    } else if mime.starts_with("audio/") {
        "audio"
    } else {
        "file"
    }
}
