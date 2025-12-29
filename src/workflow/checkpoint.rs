// Checkpointing - Persist and resume workflows (LangGraph-inspired)
use super::state::WorkflowState;
use sqlx::PgPool;
use chrono::{DateTime, Utc};
use serde::{Deserialize, Serialize};
use tracing::{error, info};

/// Checkpoint - Snapshot of workflow state
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct Checkpoint {
    pub checkpoint_id: String,
    pub workflow_id: String,
    pub thread_id: String,
    pub state: WorkflowState,
    pub version: i32,
    pub created_at: DateTime<Utc>,
}

/// Checkpointer - Saves and loads workflow state
pub struct WorkflowCheckpointer {
    pool: PgPool,
}

impl WorkflowCheckpointer {
    pub fn new(pool: PgPool) -> Self {
        Self { pool }
    }

    /// Setup checkpoint table
    pub async fn setup(&self) -> Result<(), sqlx::Error> {
        // Create table
        sqlx::query(
            r#"
            CREATE TABLE IF NOT EXISTS workflow_checkpoints (
                checkpoint_id VARCHAR(255) PRIMARY KEY,
                workflow_id VARCHAR(255) NOT NULL,
                thread_id VARCHAR(255) NOT NULL,
                state JSONB NOT NULL,
                version INTEGER NOT NULL,
                created_at TIMESTAMPTZ NOT NULL DEFAULT NOW()
            )
            "#,
        )
        .execute(&self.pool)
        .await?;

        // Create indexes separately
        sqlx::query(
            r#"
            CREATE INDEX IF NOT EXISTS idx_workflow_checkpoints_workflow_id
            ON workflow_checkpoints(workflow_id)
            "#,
        )
        .execute(&self.pool)
        .await?;

        sqlx::query(
            r#"
            CREATE INDEX IF NOT EXISTS idx_workflow_checkpoints_thread_id
            ON workflow_checkpoints(thread_id)
            "#,
        )
        .execute(&self.pool)
        .await?;

        sqlx::query(
            r#"
            CREATE INDEX IF NOT EXISTS idx_workflow_checkpoints_created_at
            ON workflow_checkpoints(created_at)
            "#,
        )
        .execute(&self.pool)
        .await?;

        info!("✅ Workflow checkpoint table setup complete");
        Ok(())
    }

    /// Save checkpoint (convenience method that extracts IDs from state)
    pub async fn save_checkpoint(&self, state: &WorkflowState) -> Result<String, String> {
        self.save(&state.workflow_id, &state.thread_id, state).await
    }

    /// Save checkpoint
    pub async fn save(
        &self,
        workflow_id: &str,
        thread_id: &str,
        state: &WorkflowState,
    ) -> Result<String, String> {
        let checkpoint_id = format!("{}::{}", workflow_id, Utc::now().timestamp_millis());

        // Get current version
        let current_version: Option<i32> = sqlx::query_scalar(
            "SELECT MAX(version) FROM workflow_checkpoints WHERE workflow_id = $1"
        )
        .bind(workflow_id)
        .fetch_one(&self.pool)
        .await
        .map_err(|e| format!("Failed to get version: {}", e))?;

        let version: i32 = current_version.unwrap_or(0) + 1;

        let state_json = serde_json::to_value(state)
            .map_err(|e| format!("Failed to serialize state: {}", e))?;

        sqlx::query(
            r#"
            INSERT INTO workflow_checkpoints
            (checkpoint_id, workflow_id, thread_id, state, version, created_at)
            VALUES ($1, $2, $3, $4, $5, $6)
            "#,
        )
        .bind(&checkpoint_id)
        .bind(workflow_id)
        .bind(thread_id)
        .bind(state_json)
        .bind(version)
        .bind(Utc::now())
        .execute(&self.pool)
        .await
        .map_err(|e| {
            error!("Failed to save checkpoint: {}", e);
            format!("Failed to save checkpoint: {}", e)
        })?;

        info!("💾 Checkpoint saved: {} (version: {})", checkpoint_id, version);
        Ok(checkpoint_id)
    }

    /// Load latest checkpoint for workflow
    pub async fn load_latest(
        &self,
        workflow_id: &str,
    ) -> Result<Option<Checkpoint>, String> {
        let result = sqlx::query_as::<_, CheckpointRow>(
            r#"
            SELECT checkpoint_id, workflow_id, thread_id, state, version, created_at
            FROM workflow_checkpoints
            WHERE workflow_id = $1
            ORDER BY version DESC
            LIMIT 1
            "#,
        )
        .bind(workflow_id)
        .fetch_optional(&self.pool)
        .await
        .map_err(|e| format!("Failed to load checkpoint: {}", e))?;

        if let Some(row) = result {
            let state: WorkflowState = serde_json::from_value(row.state)
                .map_err(|e| format!("Failed to deserialize state: {}", e))?;

            Ok(Some(Checkpoint {
                checkpoint_id: row.checkpoint_id,
                workflow_id: row.workflow_id,
                thread_id: row.thread_id,
                state,
                version: row.version,
                created_at: row.created_at,
            }))
        } else {
            Ok(None)
        }
    }

    /// Load specific checkpoint by ID
    pub async fn load_by_id(
        &self,
        checkpoint_id: &str,
    ) -> Result<Option<Checkpoint>, String> {
        let result = sqlx::query_as::<_, CheckpointRow>(
            r#"
            SELECT checkpoint_id, workflow_id, thread_id, state, version, created_at
            FROM workflow_checkpoints
            WHERE checkpoint_id = $1
            "#,
        )
        .bind(checkpoint_id)
        .fetch_optional(&self.pool)
        .await
        .map_err(|e| format!("Failed to load checkpoint: {}", e))?;

        if let Some(row) = result {
            let state: WorkflowState = serde_json::from_value(row.state)
                .map_err(|e| format!("Failed to deserialize state: {}", e))?;

            Ok(Some(Checkpoint {
                checkpoint_id: row.checkpoint_id,
                workflow_id: row.workflow_id,
                thread_id: row.thread_id,
                state,
                version: row.version,
                created_at: row.created_at,
            }))
        } else {
            Ok(None)
        }
    }

    /// Load checkpoint by thread_id (for session-based workflows)
    pub async fn load_by_thread(
        &self,
        thread_id: &str,
    ) -> Result<Option<Checkpoint>, String> {
        let result = sqlx::query_as::<_, CheckpointRow>(
            r#"
            SELECT checkpoint_id, workflow_id, thread_id, state, version, created_at
            FROM workflow_checkpoints
            WHERE thread_id = $1
            ORDER BY version DESC
            LIMIT 1
            "#,
        )
        .bind(thread_id)
        .fetch_optional(&self.pool)
        .await
        .map_err(|e| format!("Failed to load checkpoint: {}", e))?;

        if let Some(row) = result {
            let state: WorkflowState = serde_json::from_value(row.state)
                .map_err(|e| format!("Failed to deserialize state: {}", e))?;

            Ok(Some(Checkpoint {
                checkpoint_id: row.checkpoint_id,
                workflow_id: row.workflow_id,
                thread_id: row.thread_id,
                state,
                version: row.version,
                created_at: row.created_at,
            }))
        } else {
            Ok(None)
        }
    }

    /// List all checkpoints for a workflow (time-travel debugging)
    pub async fn list_checkpoints(
        &self,
        workflow_id: &str,
    ) -> Result<Vec<Checkpoint>, String> {
        let rows = sqlx::query_as::<_, CheckpointRow>(
            r#"
            SELECT checkpoint_id, workflow_id, thread_id, state, version, created_at
            FROM workflow_checkpoints
            WHERE workflow_id = $1
            ORDER BY version ASC
            "#,
        )
        .bind(workflow_id)
        .fetch_all(&self.pool)
        .await
        .map_err(|e| format!("Failed to list checkpoints: {}", e))?;

        rows.into_iter()
            .map(|row| {
                let state: WorkflowState = serde_json::from_value(row.state)
                    .map_err(|e| format!("Failed to deserialize state: {}", e))?;

                Ok(Checkpoint {
                    checkpoint_id: row.checkpoint_id,
                    workflow_id: row.workflow_id,
                    thread_id: row.thread_id,
                    state,
                    version: row.version,
                    created_at: row.created_at,
                })
            })
            .collect()
    }

    /// Delete old checkpoints (cleanup)
    pub async fn cleanup_old_checkpoints(
        &self,
        older_than_days: i64,
    ) -> Result<u64, String> {
        let cutoff_date = Utc::now() - chrono::Duration::days(older_than_days);

        let result = sqlx::query(
            "DELETE FROM workflow_checkpoints WHERE created_at < $1"
        )
        .bind(cutoff_date)
        .execute(&self.pool)
        .await
        .map_err(|e| format!("Failed to cleanup checkpoints: {}", e))?;

        info!("🧹 Cleaned up {} old checkpoints", result.rows_affected());
        Ok(result.rows_affected())
    }
}

#[derive(sqlx::FromRow)]
struct CheckpointRow {
    checkpoint_id: String,
    workflow_id: String,
    thread_id: String,
    state: serde_json::Value,
    version: i32,
    created_at: DateTime<Utc>,
}
