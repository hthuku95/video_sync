use serde_json::Value;
use sqlx::PgPool;
use uuid::Uuid;

/// Artifact-level checkpointing for agentic workflows.
/// Each stage persists its output so retries can resume from the last
/// successful stage instead of regenerating from scratch.
pub struct WorkflowArtifact;

impl WorkflowArtifact {
    /// Check if an artifact exists for a given workflow + stage.
    pub async fn exists(pool: &PgPool, workflow_id: Uuid, stage: &str) -> bool {
        sqlx::query_scalar::<_, bool>(
            "SELECT EXISTS(SELECT 1 FROM workflow_artifacts WHERE workflow_id = $1 AND stage = $2)",
        )
        .bind(workflow_id)
        .bind(stage)
        .fetch_one(pool)
        .await
        .unwrap_or(false)
    }

    /// Load an artifact's content + storage_url + metadata.
    pub async fn load(pool: &PgPool, workflow_id: Uuid, stage: &str) -> Option<(Option<String>, Option<String>, Value)> {
        let row = sqlx::query(
            "SELECT content, storage_url, COALESCE(metadata, '{}'::jsonb) AS metadata FROM workflow_artifacts WHERE workflow_id = $1 AND stage = $2",
        )
        .bind(workflow_id)
        .bind(stage)
        .fetch_optional(pool)
        .await
        .ok()??;
        let content: Option<String> = row.get("content");
        let url: Option<String> = row.get("storage_url");
        let meta: Value = row.get("metadata");
        Some((content, url, meta))
    }

    /// Save (upsert) an artifact for a given workflow + stage.
    /// Tools call this after producing output. On retry, the tool overwrites.
    pub async fn save(
        pool: &PgPool,
        workflow_id: Uuid,
        stage: &str,
        artifact_type: &str,
        content: Option<&str>,
        storage_url: Option<&str>,
        storage_backend: Option<&str>,
        metadata: &Value,
    ) {
        let _ = sqlx::query(
            r#"INSERT INTO workflow_artifacts (workflow_id, stage, artifact_type, content, storage_url, storage_backend, metadata, updated_at)
               VALUES ($1, $2, $3, $4, $5, $6, $7, NOW())
               ON CONFLICT (workflow_id, stage) DO UPDATE SET
                   artifact_type = EXCLUDED.artifact_type,
                   content = EXCLUDED.content,
                   storage_url = EXCLUDED.storage_url,
                   storage_backend = EXCLUDED.storage_backend,
                   metadata = EXCLUDED.metadata,
                   updated_at = NOW()"#,
        )
        .bind(workflow_id)
        .bind(stage)
        .bind(artifact_type)
        .bind(content)
        .bind(storage_url)
        .bind(storage_backend)
        .bind(metadata)
        .execute(pool)
        .await;
    }

    /// Delete all artifacts for a workflow (on completion or cancellation cleanup).
    pub async fn delete_all(pool: &PgPool, workflow_id: Uuid) {
        let _ = sqlx::query("DELETE FROM workflow_artifacts WHERE workflow_id = $1")
            .bind(workflow_id)
            .execute(pool)
            .await;
    }
}
