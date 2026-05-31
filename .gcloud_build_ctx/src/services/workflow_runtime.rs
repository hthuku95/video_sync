use chrono::Utc;
use serde_json::{json, Value};
use sqlx::{PgPool, Row};
use uuid::Uuid;

#[allow(dead_code)]
#[derive(Debug, Clone, Copy)]
pub enum WorkflowStatus {
    Queued,
    Planning,
    Running,
    WaitingForInput,
    WaitingForExternalService,
    Retrying,
    Completed,
    Failed,
    Cancelled,
}

impl WorkflowStatus {
    pub fn as_str(self) -> &'static str {
        match self {
            WorkflowStatus::Queued => "queued",
            WorkflowStatus::Planning => "planning",
            WorkflowStatus::Running => "running",
            WorkflowStatus::WaitingForInput => "waiting_for_input",
            WorkflowStatus::WaitingForExternalService => "waiting_for_external_service",
            WorkflowStatus::Retrying => "retrying",
            WorkflowStatus::Completed => "completed",
            WorkflowStatus::Failed => "failed",
            WorkflowStatus::Cancelled => "cancelled",
        }
    }
}

#[derive(Debug, Clone)]
pub struct NewWorkflow {
    pub workflow_type: String,
    pub idempotency_key: Option<String>,
    pub status: WorkflowStatus,
    pub session_uuid: Option<String>,
    pub user_id: Option<i32>,
    pub source_table: Option<String>,
    pub source_record_id: Option<Uuid>,
    pub request_summary: String,
    pub current_step: Option<String>,
    pub metadata: Value,
    pub artifact_requirements: Value,
}

pub struct WorkflowRuntime {
    pool: PgPool,
}

#[derive(Debug, Clone)]
pub struct WorkflowNode {
    pub node_key: String,
    pub node_type: String,
    pub status: String,
    pub attempt_count: i32,
    pub max_attempts: i32,
    pub input: Value,
    pub output: Value,
    pub error_message: Option<String>,
}

#[derive(Debug, Clone)]
pub struct WorkflowNodeProgress {
    pub nodes: Vec<WorkflowNode>,
    pub total_nodes: usize,
    pub completed_nodes: usize,
    pub failed_nodes: usize,
    pub running_node: Option<WorkflowNode>,
    pub waiting_node: Option<WorkflowNode>,
    pub next_node: Option<WorkflowNode>,
    pub blocked_reason: Option<String>,
    pub progress_percent: i32,
}

impl WorkflowRuntime {
    pub fn new(pool: PgPool) -> Self {
        Self { pool }
    }

    pub async fn create_workflow(&self, workflow: NewWorkflow) -> Result<Uuid, String> {
        let workflow_type = workflow.workflow_type.clone();
        let status = workflow.status.as_str().to_string();
        let session_uuid = workflow.session_uuid.clone();
        let user_id = workflow.user_id;
        let idempotency_key = workflow.idempotency_key.clone();
        let current_step = workflow.current_step.clone();

        let created_id = sqlx::query_scalar::<_, Uuid>(
            r#"
            INSERT INTO app_workflows (
                idempotency_key,
                workflow_type,
                status,
                session_uuid,
                user_id,
                source_table,
                source_record_id,
                request_summary,
                current_step,
                metadata,
                artifact_requirements,
                last_heartbeat_at
            )
            VALUES ($1,$2,$3,$4,$5,$6,$7,$8,$9,$10,$11,NOW())
            RETURNING id
            "#,
        )
        .bind(workflow.idempotency_key)
        .bind(&workflow.workflow_type)
        .bind(workflow.status.as_str())
        .bind(workflow.session_uuid.as_deref())
        .bind(workflow.user_id)
        .bind(workflow.source_table.as_deref())
        .bind(workflow.source_record_id)
        .bind(&workflow.request_summary)
        .bind(workflow.current_step.as_deref())
        .bind(workflow.metadata)
        .bind(workflow.artifact_requirements)
        .fetch_one(&self.pool)
        .await
        .map_err(|e| format!("Failed to create workflow: {}", e))?;

        tracing::info!(
            workflow_id = %created_id,
            workflow_type = %workflow_type,
            status = %status,
            session_uuid = session_uuid.as_deref().unwrap_or(""),
            user_id = user_id.unwrap_or_default(),
            idempotency_key = idempotency_key.as_deref().unwrap_or(""),
            current_step = current_step.as_deref().unwrap_or(""),
            "Created durable workflow"
        );

        Ok(created_id)
    }

    pub async fn create_or_reuse_workflow(&self, workflow: NewWorkflow) -> Result<Uuid, String> {
        if let Some(idempotency_key) = workflow.idempotency_key.as_deref() {
            if let Some(existing_id) = self.find_reusable_workflow(idempotency_key).await? {
                tracing::info!(
                    workflow_id = %existing_id,
                    idempotency_key = %idempotency_key,
                    workflow_type = %workflow.workflow_type,
                    "Reusing active workflow for idempotent request"
                );
                return Ok(existing_id);
            }
        }

        self.create_workflow(workflow).await
    }

    pub async fn find_reusable_workflow(
        &self,
        idempotency_key: &str,
    ) -> Result<Option<Uuid>, String> {
        sqlx::query_scalar::<_, Uuid>(
            r#"
            SELECT id
              FROM app_workflows
             WHERE idempotency_key = $1
               AND status IN ('queued', 'planning', 'running', 'waiting_for_input', 'waiting_for_external_service', 'retrying')
             ORDER BY created_at DESC
             LIMIT 1
            "#,
        )
        .bind(idempotency_key)
        .fetch_optional(&self.pool)
        .await
        .map_err(|e| format!("Failed to lookup reusable workflow: {}", e))
    }

    pub async fn append_event(
        &self,
        workflow_id: Uuid,
        event_type: &str,
        node_name: Option<&str>,
        message: &str,
        details: Value,
    ) -> Result<(), String> {
        sqlx::query(
            r#"
            INSERT INTO app_workflow_events (workflow_id, event_type, node_name, message, details)
            VALUES ($1, $2, $3, $4, $5)
            "#,
        )
        .bind(workflow_id)
        .bind(event_type)
        .bind(node_name)
        .bind(message)
        .bind(details)
        .execute(&self.pool)
        .await
        .map_err(|e| format!("Failed to append workflow event: {}", e))?;

        Ok(())
    }

    pub async fn heartbeat(
        &self,
        workflow_id: Uuid,
        status: WorkflowStatus,
        current_step: Option<&str>,
        message: &str,
        details: Value,
    ) -> Result<(), String> {
        sqlx::query(
            r#"
            UPDATE app_workflows
               SET status = $2,
                   current_step = COALESCE($3, current_step),
                   last_heartbeat_at = NOW(),
                   updated_at = NOW()
             WHERE id = $1
            "#,
        )
        .bind(workflow_id)
        .bind(status.as_str())
        .bind(current_step)
        .execute(&self.pool)
        .await
        .map_err(|e| format!("Failed to heartbeat workflow: {}", e))?;

        self.append_event(workflow_id, "progress", current_step, message, details)
            .await
    }

    pub async fn mark_completed(
        &self,
        workflow_id: Uuid,
        current_step: Option<&str>,
        result_summary: &str,
        artifact_status: Value,
    ) -> Result<(), String> {
        let artifact_status_for_update = artifact_status.clone();
        sqlx::query(
            r#"
            UPDATE app_workflows
               SET status = 'completed',
                   current_step = COALESCE($2, current_step),
                   result_summary = $3,
                   artifact_status = $4,
                   last_heartbeat_at = NOW(),
                   completed_at = NOW(),
                   updated_at = NOW()
             WHERE id = $1
            "#,
        )
        .bind(workflow_id)
        .bind(current_step)
        .bind(result_summary)
        .bind(artifact_status_for_update)
        .execute(&self.pool)
        .await
        .map_err(|e| format!("Failed to complete workflow: {}", e))?;

        self.append_event(
            workflow_id,
            "completed",
            current_step,
            result_summary,
            json!({ "artifact_status": artifact_status }),
        )
        .await
    }

    pub async fn mark_failed(
        &self,
        workflow_id: Uuid,
        current_step: Option<&str>,
        error_message: &str,
        retry_count: Option<i32>,
    ) -> Result<(), String> {
        sqlx::query(
            r#"
            UPDATE app_workflows
               SET status = 'failed',
                   current_step = COALESCE($2, current_step),
                   error_message = $3,
                   retry_count = COALESCE($4, retry_count),
                   last_heartbeat_at = NOW(),
                   completed_at = NOW(),
                   updated_at = NOW()
             WHERE id = $1
            "#,
        )
        .bind(workflow_id)
        .bind(current_step)
        .bind(error_message)
        .bind(retry_count)
        .execute(&self.pool)
        .await
        .map_err(|e| format!("Failed to fail workflow: {}", e))?;

        self.append_event(
            workflow_id,
            "failed",
            current_step,
            error_message,
            json!({ "retry_count": retry_count }),
        )
        .await
    }

    pub async fn mark_cancelled(
        &self,
        workflow_id: Uuid,
        current_step: Option<&str>,
        message: &str,
    ) -> Result<(), String> {
        sqlx::query(
            r#"
            UPDATE app_workflows
               SET status = 'cancelled',
                   current_step = COALESCE($2, current_step),
                   result_summary = $3,
                   last_heartbeat_at = NOW(),
                   completed_at = NOW(),
                   updated_at = NOW()
             WHERE id = $1
            "#,
        )
        .bind(workflow_id)
        .bind(current_step)
        .bind(message)
        .execute(&self.pool)
        .await
        .map_err(|e| format!("Failed to cancel workflow: {}", e))?;

        self.append_event(
            workflow_id,
            "cancelled",
            current_step,
            message,
            json!({ "ts": Utc::now().to_rfc3339() }),
        )
        .await
    }

    pub async fn mark_retrying(
        &self,
        workflow_id: Uuid,
        current_step: Option<&str>,
        retry_count: i32,
        message: &str,
    ) -> Result<(), String> {
        sqlx::query(
            r#"
            UPDATE app_workflows
               SET status = 'retrying',
                   current_step = COALESCE($2, current_step),
                   retry_count = $3,
                   last_heartbeat_at = NOW(),
                   updated_at = NOW()
             WHERE id = $1
            "#,
        )
        .bind(workflow_id)
        .bind(current_step)
        .bind(retry_count)
        .execute(&self.pool)
        .await
        .map_err(|e| format!("Failed to mark workflow retrying: {}", e))?;

        self.append_event(
            workflow_id,
            "retrying",
            current_step,
            message,
            json!({ "retry_count": retry_count, "ts": Utc::now().to_rfc3339() }),
        )
        .await
    }

    pub async fn ensure_node(
        &self,
        workflow_id: Uuid,
        node_key: &str,
        node_type: &str,
        input: Value,
        max_attempts: i32,
    ) -> Result<WorkflowNode, String> {
        let row = sqlx::query(
            r#"
            INSERT INTO app_workflow_nodes (
                workflow_id, node_key, node_type, input, max_attempts
            )
            VALUES ($1, $2, $3, $4, $5)
            ON CONFLICT (workflow_id, node_key)
            DO UPDATE SET
                node_type = EXCLUDED.node_type,
                input = CASE
                    WHEN app_workflow_nodes.status = 'completed'
                    THEN app_workflow_nodes.input
                    ELSE EXCLUDED.input
                END,
                max_attempts = GREATEST(app_workflow_nodes.max_attempts, EXCLUDED.max_attempts)
            RETURNING node_key, node_type, status, attempt_count, max_attempts, input, output, error_message
            "#,
        )
        .bind(workflow_id)
        .bind(node_key)
        .bind(node_type)
        .bind(input)
        .bind(max_attempts.max(1))
        .fetch_one(&self.pool)
        .await
        .map_err(|e| format!("Failed to ensure workflow node {node_key}: {e}"))?;

        Ok(row_to_workflow_node(row))
    }

    #[allow(dead_code)]
    pub async fn get_node(
        &self,
        workflow_id: Uuid,
        node_key: &str,
    ) -> Result<Option<WorkflowNode>, String> {
        let row = sqlx::query(
            r#"
            SELECT node_key, node_type, status, attempt_count, max_attempts, input, output, error_message
              FROM app_workflow_nodes
             WHERE workflow_id = $1
               AND node_key = $2
               AND status NOT IN ('completed', 'skipped')
            "#,
        )
        .bind(workflow_id)
        .bind(node_key)
        .fetch_optional(&self.pool)
        .await
        .map_err(|e| format!("Failed to load workflow node {node_key}: {e}"))?;

        Ok(row.map(row_to_workflow_node))
    }

    pub async fn list_nodes(&self, workflow_id: Uuid) -> Result<Vec<WorkflowNode>, String> {
        let rows = sqlx::query(
            r#"
            SELECT node_key, node_type, status, attempt_count, max_attempts, input, output, error_message
              FROM app_workflow_nodes
             WHERE workflow_id = $1
             ORDER BY created_at ASC, node_key ASC
            "#,
        )
        .bind(workflow_id)
        .fetch_all(&self.pool)
        .await
        .map_err(|e| format!("Failed to list workflow nodes: {e}"))?;

        Ok(rows.into_iter().map(row_to_workflow_node).collect())
    }

    pub async fn node_progress(&self, workflow_id: Uuid) -> Result<WorkflowNodeProgress, String> {
        let nodes = self.list_nodes(workflow_id).await?;
        Ok(summarize_nodes(nodes))
    }

    pub async fn start_node(
        &self,
        workflow_id: Uuid,
        node_key: &str,
        message: &str,
        details: Value,
    ) -> Result<(), String> {
        sqlx::query(
            r#"
            UPDATE app_workflow_nodes
               SET status = 'running',
                   attempt_count = attempt_count + 1,
                   started_at = COALESCE(started_at, NOW()),
                   last_heartbeat_at = NOW(),
                   error_message = NULL
             WHERE workflow_id = $1
               AND node_key = $2
               AND status NOT IN ('completed', 'skipped')
            "#,
        )
        .bind(workflow_id)
        .bind(node_key)
        .execute(&self.pool)
        .await
        .map_err(|e| format!("Failed to start workflow node {node_key}: {e}"))?;

        self.heartbeat(
            workflow_id,
            WorkflowStatus::Running,
            Some(node_key),
            message,
            details,
        )
        .await
    }

    pub async fn complete_node(
        &self,
        workflow_id: Uuid,
        node_key: &str,
        output: Value,
        message: &str,
    ) -> Result<(), String> {
        let output_for_update = output.clone();
        sqlx::query(
            r#"
            UPDATE app_workflow_nodes
               SET status = 'completed',
                   output = $3,
                   last_heartbeat_at = NOW(),
                   completed_at = NOW(),
                   error_message = NULL
             WHERE workflow_id = $1
               AND node_key = $2
            "#,
        )
        .bind(workflow_id)
        .bind(node_key)
        .bind(output_for_update)
        .execute(&self.pool)
        .await
        .map_err(|e| format!("Failed to complete workflow node {node_key}: {e}"))?;

        self.append_event(
            workflow_id,
            "node_completed",
            Some(node_key),
            message,
            json!({ "output": output }),
        )
        .await
    }

    pub async fn fail_node(
        &self,
        workflow_id: Uuid,
        node_key: &str,
        error_message: &str,
        details: Value,
    ) -> Result<(), String> {
        sqlx::query(
            r#"
            UPDATE app_workflow_nodes
               SET status = 'failed',
                   error_message = $3,
                   last_heartbeat_at = NOW(),
                   completed_at = NOW()
             WHERE workflow_id = $1
               AND node_key = $2
            "#,
        )
        .bind(workflow_id)
        .bind(node_key)
        .bind(error_message)
        .execute(&self.pool)
        .await
        .map_err(|e| format!("Failed to fail workflow node {node_key}: {e}"))?;

        self.append_event(
            workflow_id,
            "node_failed",
            Some(node_key),
            error_message,
            details,
        )
        .await
    }

    pub async fn skip_node(
        &self,
        workflow_id: Uuid,
        node_key: &str,
        reason: &str,
        details: Value,
    ) -> Result<(), String> {
        sqlx::query(
            r#"
            UPDATE app_workflow_nodes
               SET status = 'skipped',
                   output = $3,
                   error_message = NULL,
                   last_heartbeat_at = NOW(),
                   completed_at = NOW()
             WHERE workflow_id = $1
               AND node_key = $2
               AND status IN ('pending', 'waiting')
            "#,
        )
        .bind(workflow_id)
        .bind(node_key)
        .bind(details.clone())
        .execute(&self.pool)
        .await
        .map_err(|e| format!("Failed to skip workflow node {node_key}: {e}"))?;

        self.append_event(
            workflow_id,
            "node_skipped",
            Some(node_key),
            reason,
            details,
        )
        .await
    }
}

fn row_to_workflow_node(row: sqlx::postgres::PgRow) -> WorkflowNode {
    WorkflowNode {
        node_key: row.get("node_key"),
        node_type: row.get("node_type"),
        status: row.get("status"),
        attempt_count: row.get("attempt_count"),
        max_attempts: row.get("max_attempts"),
        input: row.get("input"),
        output: row.get("output"),
        error_message: row.get("error_message"),
    }
}

fn summarize_nodes(nodes: Vec<WorkflowNode>) -> WorkflowNodeProgress {
    let total_nodes = nodes.len();
    let completed_nodes = nodes
        .iter()
        .filter(|node| node.status == "completed" || node.status == "skipped")
        .count();
    let failed_nodes = nodes.iter().filter(|node| node.status == "failed").count();
    let running_node = nodes.iter().find(|node| node.status == "running").cloned();
    let waiting_node = nodes.iter().find(|node| node.status == "waiting").cloned();
    let next_node = nodes.iter().find(|node| node.status == "pending").cloned();
    let blocked_reason = nodes
        .iter()
        .find(|node| node.status == "failed")
        .map(|node| {
            node.error_message.clone().unwrap_or_else(|| {
                format!(
                    "Workflow node '{}' failed after {}/{} attempt(s).",
                    node.node_key, node.attempt_count, node.max_attempts
                )
            })
        })
        .or_else(|| {
            waiting_node.as_ref().map(|node| {
                format!(
                    "Workflow node '{}' is waiting for an external dependency or capacity.",
                    node.node_key
                )
            })
        });

    let progress_percent = if total_nodes == 0 {
        0
    } else {
        ((completed_nodes as f64 / total_nodes as f64) * 100.0).round() as i32
    };

    WorkflowNodeProgress {
        nodes,
        total_nodes,
        completed_nodes,
        failed_nodes,
        running_node,
        waiting_node,
        next_node,
        blocked_reason,
        progress_percent,
    }
}
