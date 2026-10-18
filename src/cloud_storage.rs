use crate::AppState;
use std::sync::Arc;
use uuid::Uuid;

/// Upload bytes directly to R2, register in GeneratedArtifactService,
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

    // ── Empty-artifact backstop (§CRIT) ──────────────────────────────────────
    // Never store a provably-empty/corrupt video. Mirrors the publish_output
    // guard so the tool-executor "upload to cloud" path can't persist 0-byte
    // MP4s (Oct 19: education agent uploaded a 0-byte output.mp4 + fallback).
    if content_type.starts_with("video/") {
        let min_bytes: usize = std::env::var("ARTIFACT_MIN_BYTES")
            .ok()
            .and_then(|v| v.parse().ok())
            .unwrap_or(50_000);
        if bytes.len() < min_bytes {
            return Err(format!(
                "ARTIFACT_INVALID: refusing to upload {}-byte video (minimum {min_bytes}) — {storage_key}",
                bytes.len()
            ));
        }
    }

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

    // 2. Upload to object storage (R2 primary, S3 fallback)
    let mut public_url: Option<String> = None;
    let mut storage_backend: &str = "r2";
    if let Some(storage) = app_state.r2_client.as_ref() {
        match storage.upload_tracked(tmp.to_str().unwrap_or(""), &storage_key).await {
            Ok(backend) => {
                storage_backend = backend.as_str();
                match storage
                    .presign_get_backend(&storage_key, backend, 7 * 24 * 3600)
                    .await
                {
                    Ok(url) => {
                        public_url = Some(url.clone());
                        tracing::info!("☁️ uploaded → {storage_key} ({storage_backend}) → {url}");
                    }
                    Err(e) => tracing::warn!("presign failed: {e}"),
                }
            }
            Err(e) => tracing::warn!("object storage upload failed: {e}"),
        }
    }

    // 3. Clean up temp file
    let _ = tokio::fs::remove_file(&tmp).await;

    if public_url.is_none() {
        return Err(format!("No cloud storage available (R2/S3 unavailable or failed)"));
    }

    let public_url = public_url.unwrap();

    // 4. Register in GeneratedArtifactService

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
        ON CONFLICT (source_table, source_record_key)
            WHERE source_table IS NOT NULL AND source_record_key IS NOT NULL
        DO UPDATE SET
            storage_backend = EXCLUDED.storage_backend,
            storage_key     = EXCLUDED.storage_key,
            public_url      = EXCLUDED.public_url,
            preview_url     = EXCLUDED.preview_url,
            bytes           = EXCLUDED.bytes,
            mime_type       = EXCLUDED.mime_type,
            updated_at      = NOW()
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
    // Check if this is an MCP marker file — if so, return the presigned URL directly
    if let Some(presigned_url) = crate::utils::read_marker_file(local_path) {
        tracing::info!(
            "📎 MCP marker detected for {local_path}, using presigned URL directly (no re-upload)"
        );
        return Ok(presigned_url);
    }

    let bytes = tokio::fs::read(local_path)
        .await
        .map_err(|e| format!("Failed to read local file {local_path}: {e}"))?;

    let file_name = std::path::Path::new(local_path)
        .file_name()
        .and_then(|n| n.to_str())
        .unwrap_or("unknown");

    // Namespace by session: concurrent deliveries all upload as user 0 with
    // identical filenames (clip_01.mp4), and colliding multipart uploads to
    // the same key 500 on complete (Oct 13: "service error" on every clip).
    let session_slug: String = session_id
        .chars()
        .map(|c| {
            if c.is_ascii_alphanumeric() || c == '-' || c == '_' {
                c
            } else {
                '_'
            }
        })
        .collect::<String>()
        .trim_matches('_')
        .to_string();
    let object_key = if session_slug.is_empty() {
        format!("generated/{user_id}/{file_name}")
    } else {
        format!("generated/{user_id}/{session_slug}/{file_name}")
    };

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
