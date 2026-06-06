// Manual clipping job — same pipeline as auto-clipping but:
//   - No linkage_id required (user_id only)
//   - Skips Phase E (no YouTube upload)
//   - Uploads clips to R2 and returns presigned download URLs

use crate::clipping::{
    ai_clipper::AiClipper, apify_client::ApifyClient, gemini_video_analyzer::VideoAnalysis,
};
use crate::services::agentic_service_pipeline::{AgenticServicePipeline, ServiceInput, ServiceType};
use crate::AppState;
use serde_json::json;
use sqlx::Row;
use std::sync::Arc;
use uuid::Uuid;

/// Update manual job status in DB.
async fn update_status(
    job_id: Uuid,
    status: &str,
    progress: i32,
    error: Option<&str>,
    pool: &sqlx::PgPool,
) -> Result<(), String> {
    sqlx::query(
        "UPDATE manual_clipping_jobs
         SET status = $1, progress_percent = $2, error_message = $3, updated_at = NOW()
         WHERE id = $4",
    )
    .bind(status)
    .bind(progress)
    .bind(error)
    .bind(job_id)
    .execute(pool)
    .await
    .map_err(|e| format!("Failed to update job status: {}", e))?;

    if let Some(workflow_id) = workflow_id_for_manual_job(job_id, pool).await? {
        let workflow_runtime = crate::services::WorkflowRuntime::new(pool.clone());
        let step_message = error.unwrap_or(status);
        match status {
            "cancelled" => {
                workflow_runtime
                    .mark_cancelled(workflow_id, Some(status), step_message)
                    .await?;
            }
            "failed" => {
                workflow_runtime
                    .mark_failed(workflow_id, Some(status), step_message, None)
                    .await?;
            }
            "completed" => {
                workflow_runtime
                    .heartbeat(
                        workflow_id,
                        crate::services::WorkflowStatus::Running,
                        Some(status),
                        step_message,
                        serde_json::json!({
                            "job_id": job_id,
                            "progress_percent": progress,
                        }),
                    )
                    .await?;
            }
            _ => {
                let workflow_status = if status == "pending" {
                    crate::services::WorkflowStatus::Queued
                } else {
                    crate::services::WorkflowStatus::Running
                };
                workflow_runtime
                    .heartbeat(
                        workflow_id,
                        workflow_status,
                        Some(status),
                        step_message,
                        serde_json::json!({
                            "job_id": job_id,
                            "progress_percent": progress,
                        }),
                    )
                    .await?;
            }
        }
    }
    Ok(())
}

async fn workflow_id_for_manual_job(
    job_id: Uuid,
    pool: &sqlx::PgPool,
) -> Result<Option<Uuid>, String> {
    sqlx::query_scalar::<_, Option<Uuid>>(
        "SELECT workflow_id FROM manual_clipping_jobs WHERE id = $1",
    )
    .bind(job_id)
    .fetch_optional(pool)
    .await
    .map(|row| row.flatten())
    .map_err(|e| format!("Failed to fetch manual clipping workflow id: {}", e))
}

async fn mark_manual_job_completed(
    job_id: Uuid,
    clip_count: i32,
    pool: &sqlx::PgPool,
) -> Result<(), String> {
    sqlx::query(
        "UPDATE manual_clipping_jobs
         SET status = 'completed', progress_percent = 100, clips_count = $1,
             completed_at = NOW(), updated_at = NOW()
         WHERE id = $2",
    )
    .bind(clip_count)
    .bind(job_id)
    .execute(pool)
    .await
    .map_err(|e| format!("Failed to mark manual clipping job completed: {}", e))?;

    let verified_clip_count = sqlx::query_scalar::<_, i64>(
        "SELECT COUNT(*) FROM manual_clipping_clips WHERE job_id = $1",
    )
    .bind(job_id)
    .fetch_one(pool)
    .await
    .map_err(|e| format!("Failed to verify manual clipping artifacts: {}", e))?;

    if verified_clip_count == 0 {
        if let Some(workflow_id) = workflow_id_for_manual_job(job_id, pool).await? {
            let workflow_runtime = crate::services::WorkflowRuntime::new(pool.clone());
            let _ = workflow_runtime
                .mark_failed(
                    workflow_id,
                    Some("artifact_verification"),
                    "Manual clipping workflow completed without any persisted clip records.",
                    None,
                )
                .await;
        }
        return Err("Manual clipping workflow completed without any persisted clip records.".to_string());
    }

    if let Some(workflow_id) = workflow_id_for_manual_job(job_id, pool).await? {
        let workflow_runtime = crate::services::WorkflowRuntime::new(pool.clone());
        workflow_runtime
            .mark_completed(
                workflow_id,
                Some("completed"),
                "Manual clipping workflow completed with persisted clip artifacts.",
                serde_json::json!({
                    "verified_clip_count": verified_clip_count,
                    "reported_clip_count": clip_count,
                }),
            )
            .await?;
    }

    Ok(())
}

/// Main executor for a manual clipping job.
pub async fn execute_manual_clipping_job(
    job_id: Uuid,
    app_state: Arc<AppState>,
) -> Result<String, String> {
    tracing::info!("🎬 Manual agentic clipping job {} started", job_id);

    let row = sqlx::query(
        "SELECT user_id, video_url, video_platform
         FROM manual_clipping_jobs WHERE id = $1",
    )
    .bind(job_id)
    .fetch_one(&app_state.db_pool)
    .await
    .map_err(|e| format!("Job not found: {}", e))?;

    let video_url: String = row.get("video_url");

    let delivery_id = Uuid::new_v4();
    let _ = sqlx::query(
        "INSERT INTO deliveries (id, client_ref, title, gig_type, prompt, status, source_url, extra_args)
         VALUES ($1, $2, $3, $4, $5, 'pending', $6, $7)",
    )
    .bind(delivery_id)
    .bind(format!("manual-clipping:{}", job_id))
    .bind(format!("Manual clip: {}", video_url))
    .bind("clip_enhancement")
    .bind(format!("Extract and enhance clips from: {}", video_url))
    .bind(&video_url)
    .bind(json!({ "manual_job_id": job_id.to_string() }))
    .execute(&app_state.db_pool)
    .await
    .ok();

    match AgenticServicePipeline::start(
        app_state.clone(),
        ServiceType::Clipping,
        ServiceInput {
            title: format!("Manual clipping: {}", video_url),
            brief: format!("Extract and enhance engaging clips from this video: {}. Each clip should be 15-60 seconds and professionally enhanced with captions, effects, or color grading as appropriate. Output at least 3 clips.", video_url),
            source_url: Some(video_url),
            style: "modern, high-retention, captioned".to_string(),
            duration_seconds: 30.0,
            delivery_id,
            prospect_id: None,
            session_uuid: None,
            user_id: Some(row.get::<i32, _>("user_id")),
            source_table: Some("deliveries".to_string()),
            source_record_id: Some(delivery_id),
            idempotency_key: Some(format!("agentic-manual-clipping:{}", job_id)),
            reference_images: vec![],
        },
    )
    .await
    {
        Ok(workflow_id) => {
            let _ = sqlx::query("UPDATE manual_clipping_jobs SET status = 'running', workflow_id = $1 WHERE id = $2")
                .bind(workflow_id)
                .bind(job_id)
                .execute(&app_state.db_pool)
                .await;
            Ok(format!("Started agentic clipping workflow for manual job {}", job_id))
        }
        Err(e) => Err(format!("Agentic manual clipping failed: {}", e)),
    }
}
