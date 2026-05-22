use crate::models::file::{GeneratedArtifact, OutputVideo};
use chrono::Utc;
use sqlx::PgPool;
use uuid::Uuid;

pub struct GeneratedArtifactService;

impl GeneratedArtifactService {
    pub async fn register_local_artifact(
        pool: &PgPool,
        session_uuid: Option<&str>,
        workflow_id: Option<Uuid>,
        kind: &str,
        file_path: &str,
        mime_type: Option<&str>,
        bytes: Option<i64>,
        source_table: &str,
        source_record_key: &str,
    ) -> Result<GeneratedArtifact, sqlx::Error> {
        let workflow_id = match workflow_id {
            Some(value) => Some(value),
            None => {
                if let Some(session_uuid) = session_uuid {
                    Self::latest_workflow_for_session(pool, session_uuid).await?
                } else {
                    None
                }
            }
        };

        let legacy_file_id = Some(Self::legacy_file_id(file_path));
        let now = Utc::now();
        let artifact = sqlx::query_as::<_, GeneratedArtifact>(
            r#"
            INSERT INTO generated_artifacts (
                workflow_id,
                session_uuid,
                kind,
                storage_backend,
                storage_key,
                file_path,
                legacy_file_id,
                public_url,
                preview_url,
                mime_type,
                bytes,
                checksum,
                source_table,
                source_record_key,
                created_at,
                updated_at
            ) VALUES (
                $1, $2, $3, 'local', $4, $5, $6, NULL, NULL, $7, $8, NULL, $9, $10, $11, $11
            )
            ON CONFLICT (source_table, source_record_key)
            DO UPDATE SET
                workflow_id = COALESCE(EXCLUDED.workflow_id, generated_artifacts.workflow_id),
                session_uuid = COALESCE(EXCLUDED.session_uuid, generated_artifacts.session_uuid),
                kind = EXCLUDED.kind,
                storage_backend = EXCLUDED.storage_backend,
                storage_key = EXCLUDED.storage_key,
                file_path = EXCLUDED.file_path,
                legacy_file_id = EXCLUDED.legacy_file_id,
                mime_type = EXCLUDED.mime_type,
                bytes = EXCLUDED.bytes,
                updated_at = EXCLUDED.updated_at
            RETURNING *
            "#,
        )
        .bind(workflow_id)
        .bind(session_uuid)
        .bind(kind)
        .bind(file_path)
        .bind(file_path)
        .bind(legacy_file_id)
        .bind(mime_type)
        .bind(bytes)
        .bind(source_table)
        .bind(source_record_key)
        .bind(now)
        .fetch_one(pool)
        .await?;

        tracing::info!(
            artifact_id = %artifact.artifact_id,
            workflow_id = ?artifact.workflow_id,
            kind = %artifact.kind,
            file_path = artifact.file_path.as_deref().unwrap_or(""),
            source_table = artifact.source_table.as_deref().unwrap_or(""),
            source_record_key = artifact.source_record_key.as_deref().unwrap_or(""),
            "Registered local generated artifact"
        );

        Ok(artifact)
    }

    pub async fn register_output_video(
        pool: &PgPool,
        output_video: &OutputVideo,
        session_uuid: Option<&str>,
        workflow_id: Option<Uuid>,
    ) -> Result<GeneratedArtifact, sqlx::Error> {
        let workflow_id = match workflow_id {
            Some(value) => Some(value),
            None => {
                if let Some(session_uuid) = session_uuid {
                    Self::latest_workflow_for_session(pool, session_uuid).await?
                } else {
                    None
                }
            }
        };

        let legacy_file_id = Some(Self::legacy_file_id(&output_video.file_path));
        let now = Utc::now();
        let artifact = sqlx::query_as::<_, GeneratedArtifact>(
            r#"
            INSERT INTO generated_artifacts (
                workflow_id,
                session_uuid,
                kind,
                storage_backend,
                storage_key,
                file_path,
                legacy_file_id,
                public_url,
                preview_url,
                mime_type,
                bytes,
                checksum,
                source_table,
                source_record_key,
                created_at,
                updated_at
            ) VALUES (
                $1, $2, 'output_video', 'local', $3, $4, $5, $6, $7, $8, $9, NULL, 'output_videos', $10, $11, $11
            )
            ON CONFLICT (source_table, source_record_key)
            DO UPDATE SET
                workflow_id = COALESCE(EXCLUDED.workflow_id, generated_artifacts.workflow_id),
                session_uuid = COALESCE(EXCLUDED.session_uuid, generated_artifacts.session_uuid),
                storage_backend = EXCLUDED.storage_backend,
                storage_key = EXCLUDED.storage_key,
                file_path = EXCLUDED.file_path,
                legacy_file_id = EXCLUDED.legacy_file_id,
                public_url = EXCLUDED.public_url,
                preview_url = EXCLUDED.preview_url,
                mime_type = EXCLUDED.mime_type,
                bytes = EXCLUDED.bytes,
                updated_at = EXCLUDED.updated_at
            RETURNING *
            "#,
        )
        .bind(workflow_id)
        .bind(session_uuid)
        .bind(&output_video.file_path)
        .bind(&output_video.file_path)
        .bind(legacy_file_id.clone())
        .bind(
            legacy_file_id
                .as_ref()
                .map(|file_id| format!("/api/outputs/download/{file_id}")),
        )
        .bind(
            legacy_file_id
                .as_ref()
                .map(|file_id| format!("/api/outputs/stream/{file_id}")),
        )
        .bind(&output_video.mime_type)
        .bind(output_video.file_size)
        .bind(output_video.id.to_string())
        .bind(now)
        .fetch_one(pool)
        .await?;

        tracing::info!(
            artifact_id = %artifact.artifact_id,
            workflow_id = ?artifact.workflow_id,
            session_uuid = artifact.session_uuid.as_deref().unwrap_or(""),
            legacy_file_id = artifact.legacy_file_id.as_deref().unwrap_or(""),
            source_record_key = artifact.source_record_key.as_deref().unwrap_or(""),
            storage_backend = %artifact.storage_backend,
            file_path = artifact.file_path.as_deref().unwrap_or(""),
            "Registered generated artifact"
        );

        Ok(artifact)
    }

    pub async fn find_by_legacy_file_id(
        pool: &PgPool,
        legacy_file_id: &str,
    ) -> Result<Option<GeneratedArtifact>, sqlx::Error> {
        sqlx::query_as::<_, GeneratedArtifact>(
            "SELECT * FROM generated_artifacts WHERE legacy_file_id = $1 ORDER BY created_at DESC LIMIT 1",
        )
        .bind(legacy_file_id)
        .fetch_optional(pool)
        .await
    }

    pub async fn find_for_workflow(
        pool: &PgPool,
        workflow_id: Uuid,
    ) -> Result<Vec<GeneratedArtifact>, sqlx::Error> {
        sqlx::query_as::<_, GeneratedArtifact>(
            "SELECT * FROM generated_artifacts WHERE workflow_id = $1 ORDER BY created_at DESC",
        )
        .bind(workflow_id)
        .fetch_all(pool)
        .await
    }

    pub async fn latest_workflow_for_session(
        pool: &PgPool,
        session_uuid: &str,
    ) -> Result<Option<Uuid>, sqlx::Error> {
        sqlx::query_scalar::<_, Uuid>(
            "SELECT id FROM app_workflows WHERE session_uuid = $1 ORDER BY created_at DESC LIMIT 1",
        )
        .bind(session_uuid)
        .fetch_optional(pool)
        .await
    }

    pub fn legacy_file_id(file_path: &str) -> String {
        use std::collections::hash_map::DefaultHasher;
        use std::hash::{Hash, Hasher};

        let mut hasher = DefaultHasher::new();
        file_path.hash(&mut hasher);
        format!("{:x}", hasher.finish())
    }
}
