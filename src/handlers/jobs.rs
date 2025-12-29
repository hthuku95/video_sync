// src/handlers/jobs.rs
//! Job control endpoints - pause, resume, cancel, status

use axum::{
    extract::{Extension, Path},
    http::StatusCode,
    response::{IntoResponse, Json},
    routing::{get, post},
    Router,
};
use serde::{Deserialize, Serialize};
use std::sync::Arc;
use crate::AppState;
use crate::jobs::{JobControl, JobId};

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
        None => {
            (StatusCode::NOT_FOUND, "Job not found").into_response()
        }
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
            let message = format!("Job {} action '{}' sent successfully", job_id, request.action);
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

/// Routes for job management
pub fn job_routes() -> Router {
    Router::new()
        .route("/api/jobs/:job_id/status", get(get_job_status))
        .route("/api/jobs/:job_id/control", post(control_job))
        .route("/api/jobs/session/:session_id", get(get_session_jobs))
}
