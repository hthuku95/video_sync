use crate::clipping::gemini_video_analyzer::VideoAnalysis;
use crate::clipping::models::{ChannelLinkage, ClippingJob};
use crate::jobs::clipping_job::handle_download_failure_fallback;
use crate::services::{WorkflowRuntime, WorkflowStatus};
use crate::AppState;
use serde_json::json;
use std::sync::Arc;
use tokio::time::{sleep, Duration};

const ACTIVE_CLIPPING_JOB_STATUSES: &[&str] = &[
    "pending",
    "downloading",
    "analyzing",
    "extracting_clips",
    "posting",
];

pub async fn run_clipping_supervisor_loop(app_state: Arc<AppState>) {
    let interval_secs: u64 = std::env::var("CLIPPING_SUPERVISOR_INTERVAL_SECS")
        .ok()
        .and_then(|v| v.parse().ok())
        .unwrap_or(90);

    tracing::info!(
        "🧠 Clipping supervisor started (interval: {}s)",
        interval_secs
    );

    sleep(Duration::from_secs(10)).await;

    let mut interval = tokio::time::interval(Duration::from_secs(interval_secs));
    loop {
        interval.tick().await;
        if let Err(e) = run_clipping_supervisor_once(&app_state).await {
            tracing::warn!("Clipping supervisor cycle failed: {}", e);
        }
    }
}

pub async fn run_clipping_supervisor_once(app_state: &Arc<AppState>) -> Result<(), String> {
    let duplicate_actions = suppress_duplicate_active_jobs(app_state).await?;
    let quota_waits = annotate_quota_pressure(app_state).await?;
    let pending_waits = annotate_long_pending_jobs(app_state).await?;
    let fallback_escalations = escalate_download_failure_fallbacks(app_state).await?;

    if duplicate_actions > 0 || quota_waits > 0 || pending_waits > 0 || fallback_escalations > 0 {
        tracing::info!(
            "🧠 Clipping supervisor remediations: {} duplicate actions, {} quota waits, {} pending annotations, {} fallback escalations",
            duplicate_actions,
            quota_waits,
            pending_waits,
            fallback_escalations
        );
    }

    Ok(())
}

async fn suppress_duplicate_active_jobs(app_state: &Arc<AppState>) -> Result<usize, String> {
    let duplicate_groups: Vec<(i32, String, Vec<i32>)> = sqlx::query_as(
        "SELECT linkage_id,
                source_video_id,
                ARRAY_AGG(
                    id
                    ORDER BY
                        CASE WHEN claimed_by IS NOT NULL THEN 0 ELSE 1 END,
                        CASE status
                            WHEN 'posting' THEN 0
                            WHEN 'extracting_clips' THEN 1
                            WHEN 'analyzing' THEN 2
                            WHEN 'downloading' THEN 3
                            WHEN 'pending' THEN 4
                            ELSE 5
                        END,
                        created_at ASC
                ) AS job_ids
         FROM clipping_jobs
         WHERE status = ANY($1)
         GROUP BY linkage_id, source_video_id
         HAVING COUNT(*) > 1",
    )
    .bind(ACTIVE_CLIPPING_JOB_STATUSES)
    .fetch_all(&app_state.db_pool)
    .await
    .map_err(|e| format!("Failed to fetch duplicate clipping groups: {}", e))?;

    let mut actions = 0usize;

    for (linkage_id, source_video_id, job_ids) in duplicate_groups {
        let Some(&canonical_job_id) = job_ids.first() else {
            continue;
        };

        for duplicate_job_id in job_ids.iter().skip(1) {
            let row: Option<(String, Option<String>)> = sqlx::query_as(
                "SELECT status, claimed_by FROM clipping_jobs WHERE id = $1",
            )
            .bind(*duplicate_job_id)
            .fetch_optional(&app_state.db_pool)
            .await
            .map_err(|e| format!("Failed to inspect duplicate clipping job {}: {}", duplicate_job_id, e))?;

            let Some((status, claimed_by)) = row else {
                continue;
            };

            let (supervisor_status, supervisor_action, message) =
                if status == "pending" && claimed_by.is_none() {
                    (
                        "duplicate_suppressed",
                        "discarded_duplicate_pending_job",
                        format!(
                            "Discarded duplicate pending job because active job {} already covers linkage {} video {}.",
                            canonical_job_id, linkage_id, source_video_id
                        ),
                    )
                } else {
                    (
                        "duplicate_active_job",
                        "flagged_duplicate_active_job",
                        format!(
                            "Job is a duplicate active execution. Canonical job {} owns linkage {} video {}.",
                            canonical_job_id, linkage_id, source_video_id
                        ),
                    )
                };

            let new_status = if supervisor_status == "duplicate_suppressed" {
                "discarded"
            } else {
                status.as_str()
            };

            sqlx::query(
                "UPDATE clipping_jobs
                 SET status = $1,
                     error_message = CASE
                         WHEN $1 = 'discarded' THEN COALESCE(error_message || ' ', '') || $2
                         ELSE error_message
                     END,
                     supervisor_status = $3,
                     supervisor_reason = $2,
                     supervisor_last_action = $4,
                     supervisor_last_run_at = NOW(),
                     blocked_by_job_id = $5,
                     completed_at = CASE WHEN $1 = 'discarded' THEN COALESCE(completed_at, NOW()) ELSE completed_at END,
                     updated_at = NOW()
                 WHERE id = $6",
            )
            .bind(new_status)
            .bind(&message)
            .bind(supervisor_status)
            .bind(supervisor_action)
            .bind(canonical_job_id)
            .bind(*duplicate_job_id)
            .execute(&app_state.db_pool)
            .await
            .map_err(|e| format!("Failed to mark duplicate clipping job {}: {}", duplicate_job_id, e))?;

            record_supervisor_event(
                app_state,
                *duplicate_job_id,
                supervisor_action,
                &message,
                json!({
                    "canonical_job_id": canonical_job_id,
                    "linkage_id": linkage_id,
                    "source_video_id": source_video_id,
                    "status_before": status,
                }),
            )
            .await?;

            actions += 1;
        }

        sqlx::query(
            "UPDATE clipping_jobs
             SET supervisor_status = 'healthy',
                 supervisor_reason = NULL,
                 supervisor_last_action = 'confirmed_canonical_job',
                 supervisor_last_run_at = NOW(),
                 blocked_by_job_id = NULL,
                 updated_at = NOW()
             WHERE id = $1",
        )
        .bind(canonical_job_id)
        .execute(&app_state.db_pool)
        .await
        .map_err(|e| format!("Failed to confirm canonical clipping job {}: {}", canonical_job_id, e))?;
    }

    Ok(actions)
}

async fn annotate_quota_pressure(app_state: &Arc<AppState>) -> Result<usize, String> {
    let quota_failures: i64 = sqlx::query_scalar(
        "SELECT COUNT(*)
         FROM clipping_jobs
         WHERE status = 'failed'
           AND completed_at > NOW() - INTERVAL '30 minutes'
           AND (
               error_message ILIKE '%RESOURCE_EXHAUSTED%'
               OR error_message ILIKE '%429%'
               OR error_message ILIKE '%Too Many Requests%'
           )",
    )
    .fetch_one(&app_state.db_pool)
    .await
    .map_err(|e| format!("Failed to count quota-related clipping failures: {}", e))?;

    if quota_failures < 5 {
        return Ok(0);
    }

    let reason = format!(
        "Supervisor detected Gemini quota pressure after {} quota-related failures in the last 30 minutes.",
        quota_failures
    );

    let pending_workflows: Vec<(i32, Option<uuid::Uuid>)> = sqlx::query_as(
        "SELECT id, workflow_id
         FROM clipping_jobs
         WHERE status = 'pending'
           AND claimed_by IS NULL
           AND COALESCE(supervisor_status, 'healthy') <> 'duplicate_suppressed'
           AND created_at > NOW() - INTERVAL '24 hours'",
    )
    .fetch_all(&app_state.db_pool)
    .await
    .map_err(|e| format!("Failed to fetch pending clipping jobs for quota annotation: {}", e))?;

    if pending_workflows.is_empty() {
        return Ok(0);
    }

    sqlx::query(
        "UPDATE clipping_jobs
         SET supervisor_status = 'waiting_for_quota_window',
             supervisor_reason = $1,
             supervisor_last_action = 'annotated_quota_window',
             supervisor_last_run_at = NOW(),
             updated_at = NOW()
         WHERE status = 'pending'
           AND claimed_by IS NULL
           AND COALESCE(supervisor_status, 'healthy') <> 'duplicate_suppressed'
           AND created_at > NOW() - INTERVAL '24 hours'",
    )
    .bind(&reason)
    .execute(&app_state.db_pool)
    .await
    .map_err(|e| format!("Failed to annotate pending clipping jobs during quota pressure: {}", e))?;

    let workflow_runtime = WorkflowRuntime::new(app_state.db_pool.clone());
    for (_, workflow_id) in &pending_workflows {
        if let Some(workflow_id) = workflow_id {
            let _ = workflow_runtime
                .heartbeat(
                    *workflow_id,
                    WorkflowStatus::WaitingForExternalService,
                    Some("waiting_for_quota_window"),
                    &reason,
                    json!({"kind": "gemini_quota_pressure"}),
                )
                .await;
        }
    }

    Ok(pending_workflows.len())
}

async fn annotate_long_pending_jobs(app_state: &Arc<AppState>) -> Result<usize, String> {
    let result = sqlx::query(
        "UPDATE clipping_jobs
         SET supervisor_status = 'awaiting_worker_capacity',
             supervisor_reason = 'Supervisor noticed this job has remained pending and unclaimed for more than 15 minutes.',
             supervisor_last_action = 'annotated_pending_capacity_wait',
             supervisor_last_run_at = NOW(),
             updated_at = NOW()
         WHERE status = 'pending'
           AND claimed_by IS NULL
           AND created_at < NOW() - INTERVAL '15 minutes'
           AND COALESCE(supervisor_status, 'healthy') NOT IN ('duplicate_suppressed', 'waiting_for_quota_window')",
    )
    .execute(&app_state.db_pool)
    .await
    .map_err(|e| format!("Failed to annotate long-pending clipping jobs: {}", e))?;

    Ok(result.rows_affected() as usize)
}

async fn escalate_download_failure_fallbacks(app_state: &Arc<AppState>) -> Result<usize, String> {
    let candidate_job_ids: Vec<i32> = sqlx::query_scalar(
        "SELECT id
         FROM clipping_jobs
         WHERE status IN ('failed', 'cancelled')
           AND fallback_delivery_id IS NULL
           AND viral_moments_json IS NOT NULL
           AND (
               error_message ILIKE '%All YouTube download strategies failed%'
               OR error_message ILIKE '%Twitch download also failed%'
               OR error_message ILIKE '%no Twitch mapping exists%'
           )
         ORDER BY updated_at ASC
         LIMIT 3",
    )
    .fetch_all(&app_state.db_pool)
    .await
    .map_err(|e| format!("Failed to fetch clipping fallback candidates: {}", e))?;

    let mut escalated = 0usize;

    for job_id in candidate_job_ids {
        let job = sqlx::query_as::<_, ClippingJob>("SELECT * FROM clipping_jobs WHERE id = $1")
            .bind(job_id)
            .fetch_one(&app_state.db_pool)
            .await
            .map_err(|e| format!("Failed to load clipping job {} for fallback escalation: {}", job_id, e))?;
        let linkage = sqlx::query_as::<_, ChannelLinkage>(
            "SELECT * FROM youtube_channel_linkages WHERE id = $1",
        )
        .bind(job.linkage_id)
        .fetch_one(&app_state.db_pool)
        .await
        .map_err(|e| format!("Failed to load linkage {} for clipping fallback escalation: {}", job.linkage_id, e))?;

        let analysis_value = match job.viral_moments_json.clone() {
            Some(value) => value,
            None => continue,
        };
        let analysis: VideoAnalysis = serde_json::from_value(analysis_value)
            .map_err(|e| format!("Failed to deserialize VideoAnalysis for clipping job {}: {}", job_id, e))?;
        let source_url = job
            .active_video_url
            .clone()
            .unwrap_or_else(|| format!("https://youtube.com/watch?v={}", job.source_video_id));
        let failure_reason = format!(
            "Supervisor escalated clipping job after repeated download-path failure: {}",
            job.error_message
                .clone()
                .unwrap_or_else(|| "unknown download failure".to_string())
        );

        handle_download_failure_fallback(
            &job,
            &linkage,
            &source_url,
            &analysis,
            app_state,
            &failure_reason,
        )
        .await?;

        sqlx::query(
            "UPDATE clipping_jobs
             SET supervisor_status = 'fallback_delivery_triggered',
                 supervisor_reason = $1,
                 supervisor_last_action = 'created_generated_summary_delivery',
                 supervisor_last_run_at = NOW(),
                 updated_at = NOW()
             WHERE id = $2",
        )
        .bind(&failure_reason)
        .bind(job_id)
        .execute(&app_state.db_pool)
        .await
        .map_err(|e| format!("Failed to mark clipping fallback escalation on job {}: {}", job_id, e))?;

        record_supervisor_event(
            app_state,
            job_id,
            "created_generated_summary_delivery",
            &failure_reason,
            json!({
                "source_url": source_url,
                "used_twitch_fallback": job.used_twitch_fallback,
            }),
        )
        .await?;

        escalated += 1;
    }

    Ok(escalated)
}

async fn record_supervisor_event(
    app_state: &Arc<AppState>,
    job_id: i32,
    event_type: &str,
    message: &str,
    details: serde_json::Value,
) -> Result<(), String> {
    sqlx::query(
        "INSERT INTO clipping_supervisor_events (clipping_job_id, event_type, message, details)
         VALUES ($1, $2, $3, $4)",
    )
    .bind(job_id)
    .bind(event_type)
    .bind(message)
    .bind(details)
    .execute(&app_state.db_pool)
    .await
    .map_err(|e| format!("Failed to record clipping supervisor event: {}", e))?;

    Ok(())
}
