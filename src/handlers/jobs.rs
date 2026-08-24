// src/handlers/jobs.rs
//! Job control endpoints - pause, resume, cancel, status

use crate::jobs::{JobControl, JobId};
use crate::AppState;
use axum::{
    extract::{Extension, Path, State},
    http::StatusCode,
    response::{IntoResponse, Json},
    routing::{get, post},
    Router,
};
use serde::{Deserialize, Serialize};
use sqlx::Row;
use std::collections::BTreeMap;
use std::sync::Arc;

#[derive(Deserialize)]
pub struct JobControlRequest {
    pub action: String, // "pause", "resume", "cancel"
}

#[derive(Serialize)]
pub struct JobStatusResponse {
    pub job_id: String,
    pub status: crate::jobs::JobStatus,
    pub message: String,
}

#[derive(Serialize)]
pub struct WorkflowStatusResponse {
    pub workflow_id: String,
    pub workflow_type: String,
    pub status: String,
    pub current_step: Option<String>,
    pub request_summary: String,
    pub error_message: Option<String>,
    pub retry_count: i32,
    pub last_heartbeat_at: String,
    pub created_at: String,
    pub completed_at: Option<String>,
    pub metadata: serde_json::Value,
    pub artifact_requirements: serde_json::Value,
    pub artifact_status: serde_json::Value,
    pub node_summary: serde_json::Value,
    pub workflow_nodes: serde_json::Value,
    pub generated_artifacts: serde_json::Value,
}

type WorkflowStatusRow = (
    uuid::Uuid,
    String,
    String,
    Option<String>,
    Option<i32>,
    Option<String>,
    String,
    Option<String>,
    i32,
    chrono::DateTime<chrono::Utc>,
    chrono::DateTime<chrono::Utc>,
    Option<chrono::DateTime<chrono::Utc>>,
    serde_json::Value,
    serde_json::Value,
    serde_json::Value,
);

/// GET /api/jobs/:job_id/status - Get job status
pub async fn get_job_status(
    Path(job_id): Path<JobId>,
    Extension(state): Extension<Arc<AppState>>,
) -> impl IntoResponse {
    match state.job_manager.get_job_status(&job_id).await {
        Some(status) => {
            let response = JobStatusResponse {
                job_id: job_id.clone(),
                status,
                message: "Job status retrieved".to_string(),
            };
            (StatusCode::OK, Json(response)).into_response()
        }
        None => (StatusCode::NOT_FOUND, "Job not found").into_response(),
    }
}

/// POST /api/jobs/:job_id/control - Control job (pause/resume/cancel)
pub async fn control_job(
    Path(job_id): Path<JobId>,
    Extension(state): Extension<Arc<AppState>>,
    Json(request): Json<JobControlRequest>,
) -> impl IntoResponse {
    let command = match request.action.as_str() {
        "pause" => JobControl::Pause,
        "resume" => JobControl::Resume,
        "cancel" => JobControl::Cancel,
        _ => {
            return (StatusCode::BAD_REQUEST, "Invalid action").into_response();
        }
    };

    match state.job_manager.send_control(&job_id, command).await {
        Ok(_) => {
            let message = format!(
                "Job {} action '{}' sent successfully",
                job_id, request.action
            );
            tracing::info!("{}", message);
            (StatusCode::OK, message).into_response()
        }
        Err(e) => {
            tracing::error!("Failed to control job {}: {}", job_id, e);
            (StatusCode::INTERNAL_SERVER_ERROR, e).into_response()
        }
    }
}

/// GET /api/jobs/session/:session_id - Get all jobs for a session
pub async fn get_session_jobs(
    Path(session_id): Path<String>,
    Extension(state): Extension<Arc<AppState>>,
) -> impl IntoResponse {
    let jobs = state.job_manager.get_session_jobs(&session_id).await;
    // Convert jobs to JSON-friendly format
    let job_count = jobs.len();
    let response = serde_json::json!({
        "session_id": session_id,
        "job_count": job_count,
        "jobs": jobs.iter().map(|job| serde_json::json!({
            "id": job.id,
            "job_type": job.job_type,
            "status": job.status,
            "created_at": job.created_at,
            "started_at": job.started_at,
            "completed_at": job.completed_at,
        })).collect::<Vec<_>>()
    });
    (StatusCode::OK, Json(response)).into_response()
}

/// GET /api/workflows/:workflow_id/status - Get canonical workflow status
pub async fn get_workflow_status(
    Path(workflow_id): Path<String>,
    Extension(state): Extension<Arc<AppState>>,
    Extension(claims): Extension<crate::models::auth::Claims>,
) -> impl IntoResponse {
    let workflow_uuid = match uuid::Uuid::parse_str(&workflow_id) {
        Ok(id) => id,
        Err(_) => {
            return (StatusCode::BAD_REQUEST, "Invalid workflow id").into_response();
        }
    };

    let row = sqlx::query_as::<_, WorkflowStatusRow>(
        r#"
        SELECT id, workflow_type, status, session_uuid, user_id, current_step, request_summary, error_message, retry_count,
               last_heartbeat_at, created_at, completed_at, metadata, artifact_requirements, artifact_status
          FROM app_workflows
         WHERE id = $1
        "#,
    )
    .bind(workflow_uuid)
    .fetch_optional(&state.db_pool)
    .await;

    match row {
        Ok(Some((
            id,
            workflow_type,
            status,
            session_uuid,
            workflow_user_id,
            current_step,
            request_summary,
            error_message,
            retry_count,
            last_heartbeat_at,
            created_at,
            completed_at,
            metadata,
            artifact_requirements,
            artifact_status,
        ))) => {
            let requesting_user_id = claims.sub.parse::<i32>().unwrap_or(0);
            let session_owner = if let Some(session_uuid) = session_uuid.as_deref() {
                sqlx::query_scalar::<_, Option<i32>>(
                    "SELECT user_id FROM chat_sessions WHERE session_uuid = $1 LIMIT 1",
                )
                .bind(session_uuid)
                .fetch_optional(&state.db_pool)
                .await
                .ok()
                .flatten()
                .flatten()
            } else {
                None
            };

            let authorized = workflow_user_id == Some(requesting_user_id)
                || session_owner == Some(requesting_user_id)
                || (workflow_user_id.is_none() && session_owner.is_none());

            if !authorized {
                return (StatusCode::FORBIDDEN, "Forbidden").into_response();
            }

            let node_progress = crate::services::WorkflowRuntime::new(state.db_pool.clone())
                .node_progress(id)
                .await
                .ok();
            let node_summary = node_progress
                .as_ref()
                .map(|progress| {
                    let active_node = progress
                        .running_node
                        .as_ref()
                        .or(progress.waiting_node.as_ref())
                        .or(progress.next_node.as_ref());
                    let mut policy_counts = BTreeMap::<String, usize>::new();
                    for node in &progress.nodes {
                        if let Some(policy) =
                            node.input.get("durable_policy").and_then(|value| value.as_str())
                        {
                            *policy_counts.entry(policy.to_string()).or_insert(0) += 1;
                        }
                    }

                    serde_json::json!({
                        "available": true,
                        "progress_percent": progress.progress_percent,
                        "total_nodes": progress.total_nodes,
                        "completed_nodes": progress.completed_nodes,
                        "failed_nodes": progress.failed_nodes,
                        "blocked_reason": progress.blocked_reason.as_deref(),
                        "durable_policy_counts": policy_counts,
                        "active_node": active_node.map(|node| serde_json::json!({
                            "node_key": node.node_key.as_str(),
                            "node_type": node.node_type.as_str(),
                            "status": node.status.as_str(),
                            "attempt_count": node.attempt_count,
                            "max_attempts": node.max_attempts,
                            "tool_name": node.input.get("tool_name").and_then(|value| value.as_str()),
                            "durable_policy": node.input.get("durable_policy").and_then(|value| value.as_str()),
                            "requires_durable_node": node.input.get("requires_durable_node").and_then(|value| value.as_bool()),
                            "timeout_hint_seconds": node.input.get("timeout_hint_seconds").and_then(|value| value.as_i64()),
                            "error_message": node.error_message.as_deref(),
                        })),
                    })
                })
                .unwrap_or_else(|| serde_json::json!({ "available": false }));
            let workflow_nodes = node_progress
                .as_ref()
                .map(|progress| {
                    let nodes = progress
                        .nodes
                        .iter()
                        .map(|node| {
                            serde_json::json!({
                                "node_key": node.node_key.as_str(),
                                "node_type": node.node_type.as_str(),
                                "status": node.status.as_str(),
                                "attempt_count": node.attempt_count,
                                "max_attempts": node.max_attempts,
                                "tool_name": node.input.get("tool_name").and_then(|value| value.as_str()),
                                "durable_policy": node.input.get("durable_policy").and_then(|value| value.as_str()),
                                "requires_durable_node": node.input.get("requires_durable_node").and_then(|value| value.as_bool()),
                                "timeout_hint_seconds": node.input.get("timeout_hint_seconds").and_then(|value| value.as_i64()),
                                "error_message": node.error_message.as_deref(),
                                "output": &node.output,
                            })
                        })
                        .collect::<Vec<_>>();
                    serde_json::json!(nodes)
                })
                .unwrap_or_else(|| serde_json::json!([]));

            let response = WorkflowStatusResponse {
                workflow_id: id.to_string(),
                workflow_type,
                status,
                current_step,
                request_summary,
                error_message,
                retry_count,
                last_heartbeat_at: last_heartbeat_at.to_rfc3339(),
                created_at: created_at.to_rfc3339(),
                completed_at: completed_at.map(|ts| ts.to_rfc3339()),
                metadata,
                artifact_requirements,
                artifact_status,
                node_summary,
                workflow_nodes,
                generated_artifacts:
                    match crate::services::GeneratedArtifactService::find_for_workflow(
                        &state.db_pool,
                        id,
                    )
                    .await
                    {
                        Ok(artifacts) => serde_json::json!(artifacts),
                        Err(_) => serde_json::json!([]),
                    },
            };
            (StatusCode::OK, Json(response)).into_response()
        }
        Ok(None) => (StatusCode::NOT_FOUND, "Workflow not found").into_response(),
        Err(e) => {
            tracing::error!("Failed to fetch workflow {}: {}", workflow_id, e);
            (
                StatusCode::INTERNAL_SERVER_ERROR,
                "Failed to fetch workflow",
            )
                .into_response()
        }
    }
}

/// GET /api/workflows/:workflow_id/debug - Rich workflow debug surface
pub async fn get_workflow_debug(
    Path(workflow_id): Path<String>,
    Extension(state): Extension<Arc<AppState>>,
    Extension(claims): Extension<crate::models::auth::Claims>,
) -> impl IntoResponse {
    let workflow_uuid = match uuid::Uuid::parse_str(&workflow_id) {
        Ok(id) => id,
        Err(_) => {
            return (StatusCode::BAD_REQUEST, "Invalid workflow id").into_response();
        }
    };

    let workflow_row = sqlx::query(
        r#"
        SELECT id, idempotency_key, workflow_type, status, session_uuid, user_id, source_table, source_record_id,
               request_summary, error_message, retry_count, last_heartbeat_at, created_at, completed_at,
               current_step, metadata, artifact_requirements, artifact_status
          FROM app_workflows
         WHERE id = $1
        "#,
    )
    .bind(workflow_uuid)
    .fetch_optional(&state.db_pool)
    .await;

    let workflow_row = match workflow_row {
        Ok(Some(row)) => row,
        Ok(None) => return (StatusCode::NOT_FOUND, "Workflow not found").into_response(),
        Err(e) => {
            tracing::error!("Failed to fetch workflow debug {}: {}", workflow_id, e);
            return (
                StatusCode::INTERNAL_SERVER_ERROR,
                "Failed to fetch workflow",
            )
                .into_response();
        }
    };

    let id: uuid::Uuid = match workflow_row.try_get("id") {
        Ok(value) => value,
        Err(e) => {
            tracing::error!("Failed to decode workflow debug {} id: {}", workflow_id, e);
            return (
                StatusCode::INTERNAL_SERVER_ERROR,
                "Failed to decode workflow",
            )
                .into_response();
        }
    };
    let idempotency_key: Option<String> = workflow_row.try_get("idempotency_key").ok();
    let workflow_type: String = workflow_row.try_get("workflow_type").unwrap_or_default();
    let status: String = workflow_row.try_get("status").unwrap_or_default();
    let session_uuid: Option<String> = workflow_row.try_get("session_uuid").ok();
    let workflow_user_id: Option<i32> = workflow_row.try_get("user_id").ok();
    let source_table: Option<String> = workflow_row.try_get("source_table").ok();
    let source_record_id: Option<uuid::Uuid> = workflow_row.try_get("source_record_id").ok();
    let request_summary: String = workflow_row.try_get("request_summary").unwrap_or_default();
    let error_message: Option<String> = workflow_row.try_get("error_message").ok();
    let retry_count: i32 = workflow_row.try_get("retry_count").unwrap_or_default();
    let last_heartbeat_at: chrono::DateTime<chrono::Utc> =
        workflow_row.try_get("last_heartbeat_at").unwrap();
    let created_at: chrono::DateTime<chrono::Utc> = workflow_row.try_get("created_at").unwrap();
    let completed_at: Option<chrono::DateTime<chrono::Utc>> =
        workflow_row.try_get("completed_at").ok();
    let current_step: Option<String> = workflow_row.try_get("current_step").ok();
    let metadata: serde_json::Value = workflow_row.try_get("metadata").unwrap_or_default();
    let artifact_requirements: serde_json::Value = workflow_row
        .try_get("artifact_requirements")
        .unwrap_or_default();
    let artifact_status: serde_json::Value =
        workflow_row.try_get("artifact_status").unwrap_or_default();

    let requesting_user_id = claims.sub.parse::<i32>().unwrap_or(0);
    let session_owner = if let Some(session_uuid) = session_uuid.as_deref() {
        sqlx::query_scalar::<_, Option<i32>>(
            "SELECT user_id FROM chat_sessions WHERE session_uuid = $1 LIMIT 1",
        )
        .bind(session_uuid)
        .fetch_optional(&state.db_pool)
        .await
        .ok()
        .flatten()
        .flatten()
    } else {
        None
    };

    let authorized = workflow_user_id == Some(requesting_user_id)
        || session_owner == Some(requesting_user_id)
        || (workflow_user_id.is_none() && session_owner.is_none());

    if !authorized {
        return (StatusCode::FORBIDDEN, "Forbidden").into_response();
    }

    let artifacts =
        crate::services::GeneratedArtifactService::find_for_workflow(&state.db_pool, id)
            .await
            .map(|rows| serde_json::json!(rows))
            .unwrap_or_else(|_| serde_json::json!([]));

    let events = sqlx::query_as::<
        _,
        (
            String,
            Option<String>,
            String,
            serde_json::Value,
            chrono::DateTime<chrono::Utc>,
        ),
    >(
        r#"
        SELECT event_type, node_name, message, details, created_at
          FROM app_workflow_events
         WHERE workflow_id = $1
         ORDER BY created_at ASC
        "#,
    )
    .bind(id)
    .fetch_all(&state.db_pool)
    .await
    .map(|rows| {
        rows.into_iter()
            .map(|(event_type, node_name, message, details, created_at)| {
                serde_json::json!({
                    "event_type": event_type,
                    "node_name": node_name,
                    "message": message,
                    "details": details,
                    "created_at": created_at.to_rfc3339(),
                })
            })
            .collect::<Vec<_>>()
    })
    .unwrap_or_default();

    let nodes = sqlx::query(
        r#"
        SELECT node_key, node_type, status, attempt_count, max_attempts, input, output,
               error_message, started_at, completed_at, last_heartbeat_at, created_at, updated_at
          FROM app_workflow_nodes
         WHERE workflow_id = $1
         ORDER BY created_at ASC, node_key ASC
        "#,
    )
    .bind(id)
    .fetch_all(&state.db_pool)
    .await
    .map(|rows| {
        rows.into_iter()
            .map(|row| {
                let started_at: Option<chrono::DateTime<chrono::Utc>> =
                    row.try_get("started_at").ok();
                let completed_at: Option<chrono::DateTime<chrono::Utc>> =
                    row.try_get("completed_at").ok();
                let last_heartbeat_at: Option<chrono::DateTime<chrono::Utc>> =
                    row.try_get("last_heartbeat_at").ok();
                let created_at: chrono::DateTime<chrono::Utc> =
                    row.try_get("created_at").unwrap_or_else(|_| chrono::Utc::now());
                let updated_at: chrono::DateTime<chrono::Utc> =
                    row.try_get("updated_at").unwrap_or_else(|_| chrono::Utc::now());
                serde_json::json!({
                    "node_key": row.try_get::<String, _>("node_key").unwrap_or_default(),
                    "node_type": row.try_get::<String, _>("node_type").unwrap_or_default(),
                    "status": row.try_get::<String, _>("status").unwrap_or_default(),
                    "attempt_count": row.try_get::<i32, _>("attempt_count").unwrap_or_default(),
                    "max_attempts": row.try_get::<i32, _>("max_attempts").unwrap_or_default(),
                    "input": row.try_get::<serde_json::Value, _>("input").unwrap_or_default(),
                    "output": row.try_get::<serde_json::Value, _>("output").unwrap_or_default(),
                    "error_message": row.try_get::<Option<String>, _>("error_message").ok().flatten(),
                    "started_at": started_at.map(|ts| ts.to_rfc3339()),
                    "completed_at": completed_at.map(|ts| ts.to_rfc3339()),
                    "last_heartbeat_at": last_heartbeat_at.map(|ts| ts.to_rfc3339()),
                    "created_at": created_at.to_rfc3339(),
                    "updated_at": updated_at.to_rfc3339(),
                })
            })
            .collect::<Vec<_>>()
    })
    .unwrap_or_default();

    let node_summary = crate::services::WorkflowRuntime::new(state.db_pool.clone())
        .node_progress(id)
        .await
        .map(|progress| {
            let active_node = progress
                .running_node
                .as_ref()
                .or(progress.waiting_node.as_ref())
                .or(progress.next_node.as_ref());
            let mut policy_counts = BTreeMap::<String, usize>::new();
            for node in &progress.nodes {
                if let Some(policy) = node
                    .input
                    .get("durable_policy")
                    .and_then(|value| value.as_str())
                {
                    *policy_counts.entry(policy.to_string()).or_insert(0) += 1;
                }
            }

            serde_json::json!({
                "progress_percent": progress.progress_percent,
                "completed_nodes": progress.completed_nodes,
                "failed_nodes": progress.failed_nodes,
                "total_nodes": progress.total_nodes,
                "running_node": progress.running_node.as_ref().map(|node| node.node_key.as_str()),
                "waiting_node": progress.waiting_node.as_ref().map(|node| node.node_key.as_str()),
                "next_node": progress.next_node.as_ref().map(|node| node.node_key.as_str()),
                "blocked_reason": progress.blocked_reason.as_deref(),
                "durable_policy_counts": policy_counts,
                "active_node": active_node.map(|node| serde_json::json!({
                    "node_key": node.node_key.as_str(),
                    "node_type": node.node_type.as_str(),
                    "status": node.status.as_str(),
                    "attempt_count": node.attempt_count,
                    "max_attempts": node.max_attempts,
                    "tool_name": node.input.get("tool_name").and_then(|value| value.as_str()),
                    "durable_policy": node.input.get("durable_policy").and_then(|value| value.as_str()),
                    "requires_durable_node": node.input.get("requires_durable_node").and_then(|value| value.as_bool()),
                    "timeout_hint_seconds": node.input.get("timeout_hint_seconds").and_then(|value| value.as_i64()),
                    "error_message": node.error_message.as_deref(),
                })),
            })
        })
        .unwrap_or_else(|error| {
            serde_json::json!({
                "progress_percent": 0,
                "completed_nodes": 0,
                "failed_nodes": 0,
                "total_nodes": 0,
                "blocked_reason": format!("Failed to summarize workflow nodes: {error}"),
            })
        });

    let session_jobs = if let Some(session_uuid) = session_uuid.as_deref() {
        state
            .job_manager
            .get_session_jobs(session_uuid)
            .await
            .into_iter()
            .map(|job| {
                serde_json::json!({
                    "job_id": job.id,
                    "job_type": job.job_type,
                    "status": job.status,
                    "created_at": job.created_at.to_rfc3339(),
                    "started_at": job.started_at.map(|ts| ts.to_rfc3339()),
                    "completed_at": job.completed_at.map(|ts| ts.to_rfc3339()),
                    "last_heartbeat": job.last_heartbeat.to_rfc3339(),
                })
            })
            .collect::<Vec<_>>()
    } else {
        Vec::new()
    };

    let response = serde_json::json!({
        "workflow": {
            "workflow_id": id.to_string(),
            "idempotency_key": idempotency_key,
            "workflow_type": workflow_type,
            "status": status,
            "current_step": current_step,
            "request_summary": request_summary,
            "error_message": error_message,
            "retry_count": retry_count,
            "last_heartbeat_at": last_heartbeat_at.to_rfc3339(),
            "created_at": created_at.to_rfc3339(),
            "completed_at": completed_at.map(|ts| ts.to_rfc3339()),
            "source_table": source_table,
            "source_record_id": source_record_id.map(|value| value.to_string()),
            "metadata": metadata,
            "artifact_requirements": artifact_requirements,
            "artifact_status": artifact_status
        },
        "identity": {
            "session_uuid": session_uuid,
            "workflow_user_id": workflow_user_id,
            "session_owner_user_id": session_owner,
            "requesting_user_id": requesting_user_id
        },
        "generated_artifacts": artifacts,
        "node_summary": node_summary,
        "workflow_nodes": nodes.clone(),
        "nodes": nodes,
        "events": events,
        "session_jobs": session_jobs
    });

    (StatusCode::OK, Json(response)).into_response()
}

// ── Workflow Feedback / Cancel / Events ──────────────────────────────────
// These endpoints allow re-editing, cancellation, and progress polling
// for StatefulGeminiAgent runs (including the Managed Campaign pipeline).

/// POST /api/workflows/{workflow_id}/feedback
/// Send a follow-up message to a running agent for re-editing / add-on requests.
pub async fn workflow_feedback(
    Path(workflow_id): Path<String>,
    Extension(state): Extension<Arc<AppState>>,
    Json(body): Json<serde_json::Value>,
) -> impl IntoResponse {
    let session_id = match uuid::Uuid::parse_str(&workflow_id) {
        Ok(_) => workflow_id,
        Err(_) => return (StatusCode::BAD_REQUEST, "Invalid workflow id").into_response(),
    };

    let message = body
        .get("message")
        .and_then(|v| v.as_str())
        .unwrap_or("")
        .to_string();
    if message.is_empty() {
        return (StatusCode::BAD_REQUEST, "Missing 'message' field").into_response();
    }

    // Publish feedback to Redis pub/sub channel
    let published = if let Some(ref bus) = state.pubsub_bus {
        let channel = format!("feedback:{}", session_id);
        match bus.publish(&channel, &message).await {
            Ok(n) if n > 0 => {
                tracing::info!("📨 Feedback sent to workflow {} via Redis: {:.60}", session_id, message);
                true
            }
            Ok(_) => false,
            Err(e) => {
                tracing::warn!("Redis publish failed for feedback: {}", e);
                false
            }
        }
    } else {
        false
    };

    if published {
        (StatusCode::OK, "Feedback sent to running agent").into_response()
    } else {
        (StatusCode::NOT_FOUND, "No active agent found for this workflow").into_response()
    }
}

/// POST /api/workflows/{workflow_id}/cancel
/// Cancel a queued or running workflow.
///
/// Two delivery mechanisms, both best-effort + complementary:
/// 1. DURABLE FLAG (`app_workflows.cancel_requested_at`) — honored by the
///    pipeline_worker runner: queued rows are skipped at claim time; running
///    renders abort at the next agent turn boundary. Works across Fargate
///    tasks and survives restarts.
/// 2. REDIS pub/sub `__CANCEL__` — reaches interactively-attached agents
///    (chat sessions) mid-tool-call.
///
/// Status is flipped to 'cancelled' for any non-terminal workflow so queue
/// claims, lease renewal, and the campaign watchdog all observe it.
pub async fn workflow_cancel(
    Path(workflow_id): Path<String>,
    Extension(state): Extension<Arc<AppState>>,
) -> impl IntoResponse {
    let Ok(session_id) = uuid::Uuid::parse_str(&workflow_id) else {
        return (StatusCode::BAD_REQUEST, "Invalid workflow id").into_response();
    };

    // 1. Durable cancellation flag (pipeline runner honors this).
    let runtime = crate::services::workflow_runtime::WorkflowRuntime::new(state.db_pool.clone());
    let flagged = match runtime.request_cancel(session_id).await {
        Ok(f) => f,
        Err(e) => {
            tracing::warn!("cancel flag set failed for {}: {}", session_id, e);
            false
        }
    };

    // 2. Redis signal for interactively-attached agents (best-effort).
    let published = if let Some(ref bus) = state.pubsub_bus {
        let channel = format!("feedback:{}", session_id);
        match bus.publish(&channel, "__CANCEL__").await {
            Ok(n) if n > 0 => {
                tracing::info!("🛑 Cancel signal sent to workflow {} via Redis", session_id);
                true
            }
            _ => false,
        }
    } else {
        false
    };

    // Flip status for anything non-terminal (queued rows leave the claim
    // pool instantly; running rows make the runner's ownership probes fail
    // so its terminal writes are discarded).
    let status_flipped = sqlx::query(
        "UPDATE app_workflows SET status = 'cancelled', current_step = 'cancelled_by_user', \
         completed_at = NOW(), updated_at = NOW() \
         WHERE id = $1 AND status NOT IN ('completed', 'failed', 'cancelled')",
    )
    .bind(session_id)
    .execute(&state.db_pool)
    .await
    .map(|r| r.rows_affected() > 0)
    .unwrap_or(false);

    tracing::info!(
        workflow = %session_id,
        flagged, published, status_flipped,
        "🛑 cancellation requested"
    );

    if flagged || published || status_flipped {
        (StatusCode::OK, "Cancellation requested").into_response()
    } else {
        (StatusCode::NOT_FOUND, "Workflow not found or already finished").into_response()
    }
}

/// GET /api/workflows/{workflow_id}/trace
/// Operator-facing observability surface for one workflow: identity/ownership,
/// usage ledger rollup, durable node timeline, and the last 200 events
/// (incl. per-tool `tool_trace` latency records).
pub async fn workflow_trace(
    Path(workflow_id): Path<String>,
    Extension(state): Extension<Arc<AppState>>,
) -> impl IntoResponse {
    let Ok(workflow_uuid) = uuid::Uuid::parse_str(&workflow_id) else {
        return (StatusCode::BAD_REQUEST, "Invalid workflow id").into_response();
    };

    let workflow = sqlx::query_as::<_, sqlx::postgres::PgRow>(
        "SELECT id, workflow_type, status, current_step, claimed_by, lease_expires_at, \
                cancel_requested_at, error_message, retry_count, request_summary, \
                usage, created_at, updated_at, last_heartbeat_at, completed_at \
         FROM app_workflows WHERE id = $1",
    )
    .bind(workflow_uuid)
    .fetch_optional(&state.db_pool)
    .await;

    let workflow = match workflow {
        Ok(Some(row)) => serde_json::json!({
            "id": row.try_get::<uuid::Uuid, _>("id").ok(),
            "workflow_type": row.try_get::<String, _>("workflow_type").ok(),
            "status": row.try_get::<String, _>("status").ok(),
            "current_step": row.try_get::<Option<String>, _>("current_step").ok(),
            "claimed_by": row.try_get::<Option<String>, _>("claimed_by").ok(),
            "lease_expires_at": row.try_get::<Option<chrono::DateTime<chrono::Utc>>, _>("lease_expires_at").ok().map(|d| d.to_rfc3339()),
            "cancel_requested": row.try_get::<Option<chrono::DateTime<chrono::Utc>>, _>("cancel_requested_at").ok().map(|d| d.to_rfc3339()),
            "error_message": row.try_get::<Option<String>, _>("error_message").ok(),
            "retry_count": row.try_get::<Option<i32>, _>("retry_count").ok(),
            "request_summary": row.try_get::<Option<String>, _>("request_summary").ok(),
            "usage": row.try_get::<Option<serde_json::Value>, _>("usage").ok(),
            "created_at": row.try_get::<Option<chrono::DateTime<chrono::Utc>>, _>("created_at").ok().map(|d| d.to_rfc3339()),
            "updated_at": row.try_get::<Option<chrono::DateTime<chrono::Utc>>, _>("updated_at").ok().map(|d| d.to_rfc3339()),
            "last_heartbeat_at": row.try_get::<Option<chrono::DateTime<chrono::Utc>>, _>("last_heartbeat_at").ok().map(|d| d.to_rfc3339()),
            "completed_at": row.try_get::<Option<chrono::DateTime<chrono::Utc>>, _>("completed_at").ok().map(|d| d.to_rfc3339()),
        }),
        Ok(None) => return (StatusCode::NOT_FOUND, "Workflow not found").into_response(),
        Err(e) => {
            return (StatusCode::INTERNAL_SERVER_ERROR, format!("query failed: {e}")).into_response()
        }
    };

    let runtime = crate::services::workflow_runtime::WorkflowRuntime::new(state.db_pool.clone());
    let nodes = runtime
        .list_nodes(workflow_uuid)
        .await
        .unwrap_or_default();

    let events: Vec<serde_json::Value> = sqlx::query_as::<_, (String, Option<String>, String, Option<serde_json::Value>, chrono::DateTime<chrono::Utc>)>(
        "SELECT event_type, node_name, message, details, created_at \
         FROM app_workflow_events WHERE workflow_id = $1 ORDER BY created_at DESC LIMIT 200",
    )
    .bind(workflow_uuid)
    .fetch_all(&state.db_pool)
    .await
    .unwrap_or_default()
    .into_iter()
    .map(|(event_type, node_name, message, details, created_at)| {
        serde_json::json!({
            "type": event_type,
            "node": node_name,
            "message": message,
            "details": details,
            "at": created_at.to_rfc3339(),
        })
    })
    .collect();

    (
        StatusCode::OK,
        axum::Json(serde_json::json!({
            "workflow": workflow,
            "nodes": nodes.iter().map(|n| serde_json::json!({
                "key": n.node_key,
                "type": n.node_type,
                "status": n.status,
                "attempts": n.attempt_count,
                "error": n.error_message,
            })).collect::<Vec<_>>(),
            "events": events,
        })),
    )
        .into_response()
}

/// GET /api/workflows/{workflow_id}/events
/// Poll progress events from a running or completed workflow.
pub async fn workflow_events(
    Path(workflow_id): Path<String>,
    Extension(state): Extension<Arc<AppState>>,
) -> impl IntoResponse {
    let workflow_uuid = match uuid::Uuid::parse_str(&workflow_id) {
        Ok(id) => id,
        Err(_) => return (StatusCode::BAD_REQUEST, "Invalid workflow id").into_response(),
    };

    let events = sqlx::query_as::<_, (String, Option<String>, String, serde_json::Value, chrono::DateTime<chrono::Utc>)>(
        r#"SELECT event_type, node_name, message, details, created_at
             FROM app_workflow_events
            WHERE workflow_id = $1
            ORDER BY created_at ASC"#,
    )
    .bind(workflow_uuid)
    .fetch_all(&state.db_pool)
    .await
    .map(|rows| {
        rows.into_iter()
            .map(|(event_type, node_name, message, details, created_at)| {
                serde_json::json!({
                    "event_type": event_type,
                    "node_name": node_name,
                    "message": message,
                    "details": details,
                    "created_at": created_at.to_rfc3339(),
                })
            })
            .collect::<Vec<_>>()
    })
    .unwrap_or_default();

    // Also include the workflow's current status
    let status: Option<(String, Option<String>, Option<chrono::DateTime<chrono::Utc>>)> =
        sqlx::query_as(
            r#"SELECT status, current_step, completed_at FROM app_workflows WHERE id = $1"#,
        )
        .bind(workflow_uuid)
        .fetch_optional(&state.db_pool)
        .await
        .unwrap_or(None);

    let response = serde_json::json!({
        "workflow_id": workflow_id,
        "status": status.as_ref().map(|s| s.0.as_str()).unwrap_or("unknown"),
        "current_step": status.as_ref().and_then(|s| s.1.as_deref()).unwrap_or(""),
        "completed_at": status.as_ref().and_then(|s| s.2.map(|t| t.to_rfc3339())),
        "events": events,
        "event_count": events.len(),
    });

    (StatusCode::OK, Json(response)).into_response()
}

/// Routes for job management
pub fn job_routes() -> Router {
    Router::new()
        .route("/api/jobs/:job_id/status", get(get_job_status))
        .route("/api/jobs/:job_id/control", post(control_job))
        .route("/api/jobs/session/:session_id", get(get_session_jobs))
        .route(
            "/api/workflows/:workflow_id/events",
            get(workflow_events),
        )
        .route(
            "/api/workflows/:workflow_id/status",
            get(get_workflow_status),
        )
        .route(
            "/api/workflows/:workflow_id/debug",
            get(get_workflow_debug),
        )
        .route(
            "/api/workflows/:workflow_id/feedback",
            post(workflow_feedback),
        )
        .route(
            "/api/workflows/:workflow_id/cancel",
            post(workflow_cancel),
        )
        .route(
            "/api/workflows/:workflow_id/trace",
            get(workflow_trace),
        )
        .route_layer(axum::middleware::from_fn(
            crate::middleware::auth::auth_middleware,
        ))
}
