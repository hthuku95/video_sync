// Background job for clipping workflow orchestration — 5-PHASE ARCHITECTURE
//
// Phase A: Single Gemini video analysis via YouTube URL (replaces frame-by-frame pipeline)
// Phase B: Download video (only if Phase A found quality clips)
// Phase C: Parallel FFmpeg clip extraction
// Phase D: Lightweight vectorization (1 embedding into video_content collection)
// Phase E: Parallel YouTube upload + store_extracted_clip per clip
//
// Smart Resumption: Jobs that fail mid-pipeline resume from the appropriate phase rather
// than re-running from Phase A. auto_retry_failed_jobs() sets job.resume_from to indicate
// the entry point; execute_clipping_job() reads it and skips completed phases.

use crate::clipping::{
    ai_clipper::{AiClipper, ExtractedClipData},
    apify_client::ApifyClient,
    gemini_video_analyzer::VideoAnalysis,
    models::{ChannelLinkage, ClippingConfig, ClippingJob},
    uploader::ClipUploader,
};
use crate::models::youtube::ConnectedYouTubeChannel;
use crate::services::agentic_service_pipeline::{AgenticServicePipeline, ServiceInput, ServiceType};
use crate::services::VideoVectorizationService;
use crate::AppState;
use chrono::Utc;
use tokio::sync::mpsc;
use serde_json::json;
use sqlx::{PgPool, Row};
use std::sync::Arc;
use uuid::Uuid;

/// Phase determination for smart job resumption.
#[derive(Debug, PartialEq, PartialOrd)]
enum StartingPhase {
    A, // Gemini analysis (default — start from scratch)
    B, // Download only (Phase A already done; analysis stored in viral_moments_json)
    C, // Clip extraction only (Phase A + B done; video file on disk)
    E, // Upload only (Phases A–D done; clips already in extracted_clips table)
}

/// Execute clipping job workflow
///
/// Replaced the 5-phase procedural pipeline with an agentic workflow.
/// The agent has the full editing and analysis toolset and decides how to analyze, extract,
/// enhance, and publish clips.
pub async fn execute_clipping_job(job_id: i32, app_state: Arc<AppState>) -> Result<String, String> {
    tracing::info!("🎬 Starting agentic clipping job {}", job_id);

    let job = fetch_job_details(job_id, &app_state.db_pool).await?;
    let video_url = format!("https://youtube.com/watch?v={}", job.source_video_id);

    // Create a delivery to track this in the unified pipeline
    let delivery_id = uuid::Uuid::new_v4();
    let _ = sqlx::query(
        "INSERT INTO deliveries (id, client_ref, title, gig_type, prompt, status, source_url, extra_args)
         VALUES ($1, $2, $3, $4, $5, 'pending', $6, $7)",
    )
    .bind(delivery_id)
    .bind(format!("clipping-job:{}", job_id))
    .bind(format!("Clipping: {}", job.source_video_id))
    .bind("clip_enhancement")
    .bind(format!("Extract and enhance clips from video: {}", video_url))
    .bind(&video_url)
    .bind(serde_json::json!({
        "job_id": job_id,
        "source_video_id": job.source_video_id,
    }))
    .execute(&app_state.db_pool)
    .await
    .ok();

    match crate::services::AgenticServicePipeline::start(
        app_state.clone(),
        crate::services::ServiceType::Clipping,
        crate::services::ServiceInput {
            title: format!("Clipping: {}", job.source_video_id),
            brief: format!("Extract and enhance engaging clips from this YouTube video: {}. Each clip should be 15-60 seconds and professionally enhanced with captions, effects, or color grading as appropriate.", video_url),
            source_url: Some(video_url.clone()),
            style: "modern, high-retention, captioned".to_string(),
            duration_seconds: 30.0,
            delivery_id,
            prospect_id: None,
            session_uuid: None,
            user_id: None,
            source_table: Some("deliveries".to_string()),
            source_record_id: Some(delivery_id),
            idempotency_key: Some(format!("agentic-clipping-job:{}", job_id)),
            reference_images: vec![],
        },
    )
    .await
    {
        Ok(workflow_id) => {
            let _ = sqlx::query("UPDATE clipping_jobs SET workflow_id = $1 WHERE id = $2")
                .bind(workflow_id)
                .bind(job_id)
                .execute(&app_state.db_pool)
                .await;

            // Post-process: compile clips into a single compilation video + store clip URLs
            // Best-effort — failure doesn't fail the core job
            if let Err(e) = post_process_clipping_delivery(job_id, delivery_id, &app_state).await {
                tracing::warn!("Clip compilation failed (delivery still works): {e}");
            }

            Ok(format!("Agentic clipping workflow completed for job {}", job_id))
        }
        Err(e) => Err(format!("Agentic clipping failed: {}", e)),
    }
}


pub(crate) fn should_fallback_from_gemini_video_analysis_error(error: &str) -> bool {
    let lower = error.to_lowercase();
    error.contains("429")
        || error.contains("403")
        || error.contains("PERMISSION_DENIED")
        || error.contains("RESOURCE_EXHAUSTED")
        || lower.contains("quota")
        || lower.contains("rate limit")
        || lower.contains("permission denied")
        || lower.contains("model")
}

/// Load unpublished clips from the extracted_clips table for Phase E resume.
///
/// Returns only clips with upload_status != 'published' so we retry failed/pending
/// clips without re-uploading clips that already succeeded on a previous partial run.
/// Returns (clips, clip_db_ids) — empty vecs if all clips are already published.
pub async fn load_clips_from_db(
    job_id: i32,
    pool: &PgPool,
) -> Result<(Vec<ExtractedClipData>, Vec<i32>), String> {
    let rows = sqlx::query(
        "SELECT id, clip_number, local_clip_path,
                start_time_seconds, end_time_seconds, duration_seconds,
                ai_title, ai_description, ai_tags, ai_confidence_score,
                viral_factors, custom_thumbnail_path, thumbnail_generation_method,
                enhancement_applied, enhancement_tools, enhancement_reasoning,
                r2_clip_key, r2_thumb_key, r2_clip_url,
                qa_status, qa_score, qa_feedback, qa_retry_hint
         FROM extracted_clips
         WHERE clipping_job_id = $1
           AND upload_status != 'published'
         ORDER BY clip_number",
    )
    .bind(job_id)
    .fetch_all(pool)
    .await
    .map_err(|e| format!("Failed to load clips from DB for Phase E resume: {}", e))?;

    let mut clips = Vec::new();
    let mut ids = Vec::new();

    for row in rows {
        let id: i32 = row.get("id");
        let ai_title: Option<String> = row.try_get("ai_title").ok().flatten();
        let ai_description: Option<String> = row.try_get("ai_description").ok().flatten();
        let ai_tags: Option<Vec<String>> = row.try_get("ai_tags").ok().flatten();
        let ai_confidence_score: Option<f64> = row.try_get("ai_confidence_score").ok().flatten();
        let viral_factors: Option<Vec<String>> = row.try_get("viral_factors").ok().flatten();
        let custom_thumbnail_path: Option<String> =
            row.try_get("custom_thumbnail_path").ok().flatten();
        let thumbnail_generation_method: Option<String> =
            row.try_get("thumbnail_generation_method").ok().flatten();
        let enhancement_applied: bool = row.try_get("enhancement_applied").ok().unwrap_or(false);
        let enhancement_tools: Vec<String> =
            row.try_get("enhancement_tools").ok().unwrap_or_default();
        let enhancement_reasoning: Option<String> =
            row.try_get("enhancement_reasoning").ok().flatten();

        clips.push(ExtractedClipData {
            clip_number: row.get("clip_number"),
            local_clip_path: row.get("local_clip_path"),
            start_time_seconds: row.get("start_time_seconds"),
            end_time_seconds: row.get("end_time_seconds"),
            duration_seconds: row.get("duration_seconds"),
            ai_title: ai_title.unwrap_or_else(|| "Untitled".to_string()),
            ai_description: ai_description.unwrap_or_default(),
            ai_tags: ai_tags.unwrap_or_default(),
            ai_confidence_score: ai_confidence_score.unwrap_or(0.0),
            viral_factors: viral_factors.unwrap_or_default(),
            custom_thumbnail_path,
            thumbnail_generation_method,
            enhancement_applied,
            enhancement_tools,
            enhancement_reasoning,
            r2_clip_key: row.try_get("r2_clip_key").ok().flatten(),
            r2_thumb_key: row.try_get("r2_thumb_key").ok().flatten(),
            r2_clip_url: row.try_get("r2_clip_url").ok().flatten(),
            qa_status: row.try_get("qa_status").ok().flatten(),
            qa_score: row.try_get("qa_score").ok().flatten(),
            qa_feedback: row.try_get("qa_feedback").ok().flatten(),
            qa_retry_hint: row.try_get("qa_retry_hint").ok().flatten(),
        });
        ids.push(id);
    }

    Ok((clips, ids))
}

/// Store an extracted clip in the Qdrant extracted_clips collection (best-effort, non-fatal).
async fn store_clip_in_qdrant(
    clip_id: i32,
    job_id: i32,
    source_video_id: &str,
    clip: &ExtractedClipData,
    app_state: &Arc<AppState>,
) {
    let qdrant_client = match &app_state.qdrant_client {
        Some(c) => c,
        None => return,
    };

    let text_to_embed = format!("{} {}", clip.ai_title, clip.ai_description);

    let embedding = if let Some(ref gemini) = app_state.video_gemini_client.as_ref().or(app_state.gemini_client.as_ref()) {
        // Tier 1: Gemini Embedding 2
        match gemini.embed_content_with_model(&text_to_embed, "models/gemini-embedding-2", Some(1536)).await {
            Ok(emb) => Some(emb),
            Err(e) => {
                tracing::warn!("Gemini Embedding 2 failed for clip: {}", e);
                // Tier 2: Voyage
                if let Some(ref voyage) = app_state.voyage_embeddings {
                    voyage.generate_single_embedding(text_to_embed.clone()).await.ok()
                } else {
                    // Tier 3: Gemini text-embedding-004
                    gemini.embed_content(&text_to_embed).await.ok()
                }
            }
        }
    } else if let Some(ref voyage) = app_state.voyage_embeddings {
        voyage.generate_single_embedding(text_to_embed).await.ok()
    } else {
        None
    };

    let embedding = match embedding {
        Some(e) => e,
        None => return,
    };

    let _ = qdrant_client.ensure_extracted_clips_collection().await;

    let payload = serde_json::json!({
        "clip_id": clip_id,
        "clipping_job_id": job_id,
        "source_video_id": source_video_id,
        "title": clip.ai_title,
        "hook": clip.ai_description,
        "start_sec": clip.start_time_seconds,
        "end_sec": clip.end_time_seconds,
        "duration_sec": clip.duration_seconds,
        "quality_score": clip.ai_confidence_score,
        "viral_factors": clip.viral_factors,
        "upload_status": "uploaded",
        "uploaded_at": chrono::Utc::now().to_rfc3339(),
    });

    if let Err(e) = qdrant_client
        .store_extracted_clip(clip_id, payload, embedding)
        .await
    {
        tracing::debug!(
            "Failed to store clip {} in Qdrant (non-fatal): {}",
            clip_id,
            e
        );
    }
}

/// Infer the safest clipping resume point from durable workflow node state.
pub async fn infer_clipping_resume_from_nodes(
    pool: &PgPool,
    workflow_id: Option<uuid::Uuid>,
) -> Result<Option<String>, String> {
    let Some(workflow_id) = workflow_id else {
        return Ok(None);
    };

    let rows: Vec<(String, String)> = sqlx::query_as(
        "SELECT node_key, status
         FROM app_workflow_nodes
         WHERE workflow_id = $1
           AND node_key = ANY($2)",
    )
    .bind(workflow_id)
    .bind(vec![
        "analysis",
        "download",
        "extract_clips",
        "vectorize",
        "post_to_youtube",
    ])
    .fetch_all(pool)
    .await
    .map_err(|e| format!("Failed to infer clipping resume state from workflow nodes: {e}"))?;

    let has = |node: &str, statuses: &[&str]| -> bool {
        rows.iter()
            .any(|(node_key, status)| node_key == node && statuses.iter().any(|s| status == s))
    };

    if has("post_to_youtube", &["running", "failed", "completed"])
        || has("vectorize", &["completed"])
    {
        return Ok(Some("clips_extracted".to_string()));
    }

    if has("download", &["completed"])
        || has("extract_clips", &["running", "failed", "completed"])
        || has("vectorize", &["running", "failed"])
    {
        return Ok(Some("downloaded".to_string()));
    }

    if has("analysis", &["completed"]) || has("download", &["running", "failed"]) {
        return Ok(Some("analyzed".to_string()));
    }

    Ok(None)
}

// Helper functions

async fn ensure_clipping_workflow_plan(
    app_state: &Arc<AppState>,
    workflow_id: Option<uuid::Uuid>,
    job_id: i32,
    source_video_id: &str,
    video_url: &str,
) {
    let Some(workflow_id) = workflow_id else {
        return;
    };

    let runtime = crate::services::WorkflowRuntime::new(app_state.db_pool.clone());
    let node_specs = [
        (
            "analysis",
            "gemini_video_analysis",
            json!({
                "job_id": job_id,
                "source_video_id": source_video_id,
                "video_url": video_url,
            }),
        ),
        (
            "download",
            "source_download",
            json!({
                "job_id": job_id,
                "source_video_id": source_video_id,
                "video_url": video_url,
            }),
        ),
        (
            "extract_clips",
            "clip_extraction",
            json!({
                "job_id": job_id,
                "source_video_id": source_video_id,
            }),
        ),
        (
            "vectorize",
            "source_vectorization",
            json!({
                "job_id": job_id,
                "source_video_id": source_video_id,
                "model_lane": "gemini_multimodal_or_configured_embedding",
            }),
        ),
        (
            "post_to_youtube",
            "youtube_upload",
            json!({
                "job_id": job_id,
                "source_video_id": source_video_id,
            }),
        ),
        (
            "fallback_summary",
            "generated_summary_delivery",
            json!({
                "job_id": job_id,
                "source_video_id": source_video_id,
                "video_url": video_url,
                "activation": "only_after_source_download_failure",
            }),
        ),
    ];

    for (node_key, node_type, input) in node_specs {
        let _ = runtime
            .ensure_node(workflow_id, node_key, node_type, input, 3)
            .await;
    }
}

async fn start_clipping_node(
    app_state: &Arc<AppState>,
    workflow_id: Option<uuid::Uuid>,
    node_key: &str,
    node_type: &str,
    input: serde_json::Value,
    message: &str,
) {
    let Some(workflow_id) = workflow_id else {
        return;
    };
    let runtime = crate::services::WorkflowRuntime::new(app_state.db_pool.clone());
    let _ = runtime
        .ensure_node(workflow_id, node_key, node_type, input.clone(), 3)
        .await;
    let _ = runtime
        .start_node(workflow_id, node_key, message, input)
        .await;
}

async fn complete_clipping_node(
    app_state: &Arc<AppState>,
    workflow_id: Option<uuid::Uuid>,
    node_key: &str,
    output: serde_json::Value,
    message: &str,
) {
    let Some(workflow_id) = workflow_id else {
        return;
    };
    let runtime = crate::services::WorkflowRuntime::new(app_state.db_pool.clone());
    let _ = runtime
        .complete_node(workflow_id, node_key, output, message)
        .await;
}

async fn fail_clipping_node(
    app_state: &Arc<AppState>,
    workflow_id: Option<uuid::Uuid>,
    node_key: &str,
    error_message: &str,
    details: serde_json::Value,
) {
    let Some(workflow_id) = workflow_id else {
        return;
    };
    let runtime = crate::services::WorkflowRuntime::new(app_state.db_pool.clone());
    let _ = runtime
        .fail_node(workflow_id, node_key, error_message, details)
        .await;
}

async fn skip_clipping_node(
    app_state: &Arc<AppState>,
    workflow_id: Option<uuid::Uuid>,
    node_key: &str,
    reason: &str,
) {
    let Some(workflow_id) = workflow_id else {
        return;
    };
    let runtime = crate::services::WorkflowRuntime::new(app_state.db_pool.clone());
    let _ = runtime
        .skip_node(workflow_id, node_key, reason, json!({ "reason": reason }))
        .await;
}

fn should_pause_after_clipping_node() -> bool {
    std::env::var("CLIPPING_NODE_STEP_MODE")
        .map(|value| {
            !matches!(
                value.trim().to_ascii_lowercase().as_str(),
                "0" | "false" | "no" | "off" | "disabled"
            )
        })
        .unwrap_or(true)
}

async fn pause_clipping_after_node(
    app_state: &Arc<AppState>,
    job_id: i32,
    workflow_id: Option<uuid::Uuid>,
    completed_node: &str,
    resume_from: &str,
    progress: i32,
    message: &str,
) -> Result<(), String> {
    sqlx::query(
        "UPDATE clipping_jobs
         SET status = 'pending',
             progress_percent = $2,
             current_step = $3,
             resume_from = $4,
             claimed_by = NULL,
             claimed_at = NULL,
             worker_heartbeat_at = NULL,
             updated_at = NOW()
         WHERE id = $1",
    )
    .bind(job_id)
    .bind(progress)
    .bind(completed_node)
    .bind(resume_from)
    .execute(&app_state.db_pool)
    .await
    .map_err(|e| format!("Failed to pause clipping job after {completed_node}: {e}"))?;

    if let Some(workflow_id) = workflow_id {
        let runtime = crate::services::WorkflowRuntime::new(app_state.db_pool.clone());
        let _ = runtime
            .mark_retrying(workflow_id, Some(completed_node), 0, message)
            .await;
        let _ = runtime
            .append_event(
                workflow_id,
                "node_checkpoint",
                Some(completed_node),
                message,
                json!({
                    "job_id": job_id,
                    "resume_from": resume_from,
                    "progress_percent": progress,
                    "mode": "clipping_node_step_mode",
                }),
            )
            .await;
    }

    tracing::info!(
        "Clipping job {} paused after node '{}' and requeued with resume_from='{}'",
        job_id,
        completed_node,
        resume_from
    );

    Ok(())
}

pub async fn fetch_job_details(job_id: i32, pool: &PgPool) -> Result<ClippingJob, String> {
    sqlx::query_as::<_, ClippingJob>("SELECT * FROM clipping_jobs WHERE id = $1")
        .bind(job_id)
        .fetch_one(pool)
        .await
        .map_err(|e| format!("Failed to fetch job: {}", e))
}

pub async fn fetch_linkage(linkage_id: i32, pool: &PgPool) -> Result<ChannelLinkage, String> {
    sqlx::query_as::<_, ChannelLinkage>("SELECT * FROM youtube_channel_linkages WHERE id = $1")
        .bind(linkage_id)
        .fetch_one(pool)
        .await
        .map_err(|e| format!("Failed to fetch linkage: {}", e))
}

pub async fn fetch_destination_channel(
    channel_id: i32,
    pool: &PgPool,
) -> Result<ConnectedYouTubeChannel, String> {
    sqlx::query_as::<_, ConnectedYouTubeChannel>(
        "SELECT * FROM connected_youtube_channels WHERE id = $1",
    )
    .bind(channel_id)
    .fetch_one(pool)
    .await
    .map_err(|e| format!("Failed to fetch destination channel: {}", e))
}

pub async fn load_reusable_source_analysis(
    current_job_id: i32,
    source_video_id: &str,
    pool: &PgPool,
) -> Result<Option<VideoAnalysis>, String> {
    let row = sqlx::query(
        "SELECT viral_moments_json
         FROM clipping_jobs
         WHERE source_video_id = $1
           AND id <> $2
           AND viral_moments_json IS NOT NULL
           AND COALESCE(analysis_quality, 0) >= 0.6
         ORDER BY
           CASE WHEN status = 'completed' THEN 0 ELSE 1 END,
           analysis_quality DESC NULLS LAST,
           updated_at DESC
         LIMIT 1",
    )
    .bind(source_video_id)
    .bind(current_job_id)
    .fetch_optional(pool)
    .await
    .map_err(|e| format!("Failed to load reusable source analysis: {}", e))?;

    let Some(row) = row else {
        return Ok(None);
    };

    let value: serde_json::Value = row
        .try_get("viral_moments_json")
        .map_err(|e| format!("Reusable source analysis missing payload: {}", e))?;
    let analysis = serde_json::from_value::<VideoAnalysis>(value)
        .map_err(|e| format!("Failed to deserialize reusable source analysis: {}", e))?;

    Ok(Some(analysis))
}

pub async fn persist_job_analysis(
    job_id: i32,
    analysis: &VideoAnalysis,
    pool: &PgPool,
) -> Result<(), String> {
    sqlx::query(
        "UPDATE clipping_jobs
         SET viral_moments_json = $1, analysis_quality = $2, updated_at = NOW()
         WHERE id = $3",
    )
    .bind(serde_json::to_value(analysis).unwrap_or(serde_json::Value::Null))
    .bind(analysis.overall_quality)
    .bind(job_id)
    .execute(pool)
    .await
    .map_err(|e| format!("Failed to persist video analysis: {}", e))?;

    Ok(())
}

pub async fn store_source_analysis_vector(
    source_video_id: &str,
    video_url: &str,
    user_id: Option<i32>,
    channel_id: Option<&str>,
    analysis: &VideoAnalysis,
    app_state: &Arc<AppState>,
    phase: &str,
) {
    match VideoVectorizationService::store_video_analysis_from_gemini(
        source_video_id,
        video_url,
        user_id,
        channel_id,
        analysis,
        app_state,
    )
    .await
    {
        Ok(()) => tracing::info!(
            "✅ {}: source video_content stored/reused for {}",
            phase,
            source_video_id
        ),
        Err(e) => tracing::warn!(
            "{} source vectorization failed (non-fatal) for {}: {}",
            phase,
            source_video_id,
            e
        ),
    }
}

pub async fn update_job_status(
    job_id: i32,
    status: &str,
    progress: i32,
    error: Option<&str>,
    pool: &PgPool,
) -> Result<(), String> {
    sqlx::query(
        "UPDATE clipping_jobs
         SET status = $1, progress_percent = $2, current_step = $1, error_message = $3, updated_at = NOW()
         WHERE id = $4",
    )
    .bind(status)
    .bind(progress)
    .bind(error)
    .bind(job_id)
    .execute(pool)
    .await
    .map_err(|e| format!("Failed to update job status: {}", e))?;

    Ok(())
}

async fn update_job_video_path(job_id: i32, video_path: &str, pool: &PgPool) -> Result<(), String> {
    sqlx::query("UPDATE clipping_jobs SET local_video_path = $1, started_at = $2 WHERE id = $3")
        .bind(video_path)
        .bind(Utc::now())
        .bind(job_id)
        .execute(pool)
        .await
        .map_err(|e| format!("Failed to update video path: {}", e))?;

    Ok(())
}

pub async fn mark_job_completed(job_id: i32, pool: &PgPool) -> Result<(), String> {
    sqlx::query("UPDATE clipping_jobs SET completed_at = $1 WHERE id = $2")
        .bind(Utc::now())
        .bind(job_id)
        .execute(pool)
        .await
        .map_err(|e| format!("Failed to mark job completed: {}", e))?;

    Ok(())
}

pub async fn update_linkage_session_timestamp(
    linkage_id: i32,
    pool: &PgPool,
) -> Result<(), String> {
    sqlx::query(
        "UPDATE youtube_channel_linkages SET last_clipping_session_at = NOW() WHERE id = $1",
    )
    .bind(linkage_id)
    .execute(pool)
    .await
    .map_err(|e| format!("Failed to update session timestamp: {}", e))?;

    Ok(())
}

pub async fn handle_download_failure_fallback(
    job: &ClippingJob,
    linkage: &ChannelLinkage,
    original_video_url: &str,
    analysis: &VideoAnalysis,
    app_state: &Arc<AppState>,
    failure_reason: &str,
) -> Result<String, String> {
    let top_moments = analysis
        .top_moments(3)
        .into_iter()
        .enumerate()
        .map(|(index, moment)| {
            format!(
                "{}. {} — hook: {} — why it works: {}",
                index + 1,
                moment.title,
                moment.hook,
                moment.reason
            )
        })
        .collect::<Vec<_>>()
        .join("\n");

    let fallback_prompt = format!(
        "Create a polished narrated fallback summary video based on this source content.\n\nVideo summary:\n{}\n\nHighest-value moments to emphasize:\n{}\n\nThe original source video could not be downloaded for clipping, so turn the topic into a standalone animated short that preserves the main ideas and hooks without requiring the raw footage. Use segmented motion graphics, concise narration, and visual variety from the available creative tools.",
        analysis.video_summary,
        top_moments
    );

    let delivery_title = format!(
        "{} — Animated Summary",
        job.source_video_title
            .as_deref()
            .filter(|v| !v.trim().is_empty())
            .unwrap_or(&job.source_video_id)
    );

    let extra_args = json!({
        "service_offer": "clipper_enhancement_pack",
        "source_url": original_video_url,
        "sample_owner_user_id": linkage.user_id,
        "include_narration": true,
        "narration_text": analysis.video_summary,
        "visual_direction": format!(
            "Fallback delivery generated after clipping download failure. Emphasize the strongest moments: {}",
            analysis
                .top_moments(3)
                .into_iter()
                .map(|moment| moment.title.clone())
                .collect::<Vec<_>>()
                .join(", ")
        ),
        "fallback_reason": failure_reason,
        "source_video_id": job.source_video_id,
        "source_video_title": job.source_video_title,
        "analysis_quality": analysis.overall_quality,
        "workflow_engine": "long_form_video_assembly",
        "target_duration_seconds": 60.0,
        "segment_duration_seconds": 15.0,
    });

    let delivery_id: uuid::Uuid = sqlx::query_scalar(
        "INSERT INTO deliveries
         (client_ref, title, gig_type, prompt, style, duration, extra_args, status, source_url)
         VALUES ($1, $2, 'long_form_video', $3, 'cinematic animated summary', 60.0, $4, 'pending', $5)
         RETURNING id",
    )
    .bind(format!("clipping-fallback:{}", job.id))
    .bind(&delivery_title)
    .bind(&fallback_prompt)
    .bind(&extra_args)
    .bind(original_video_url)
    .fetch_one(&app_state.db_pool)
    .await
    .map_err(|e| format!("Failed to create fallback delivery: {}", e))?;

    let gemini_client = match app_state
        .video_gemini_client
        .as_ref()
        .or(app_state.gemini_client.as_ref())
    {
        Some(c) => Arc::new(c.clone()),
        None => {
            return Err(
                "No Gemini client available for fallback video".to_string(),
            )
        }
    };

    let agent = crate::agent::stateful_agent::StatefulGeminiAgent::new(gemini_client);

    let fallback_session_id = format!("clipping-fallback:{}:{}", job.id, uuid::Uuid::new_v4());

    let top_moments_str = analysis
        .top_moments(3)
        .into_iter()
        .enumerate()
        .map(|(i, m)| format!("{}. {} - {}", i + 1, m.title, m.hook))
        .collect::<Vec<_>>()
        .join("\n");

    let agent_prompt = format!(
        r#"Create a 10-minute animated summary video based on this content analysis.

The original video '{title}' covered:
{video_summary}

Key segments to cover:
{top_moments}

Use Blender scenes, Manim animations, and VibeVoice narration to create a standalone long-form video that feels like an intentional animated documentary — NOT a placeholder or error recovery. The viewer should have no idea this was automatically generated. Upload the final video to R2 and return the cloud URL.

Target: 600 seconds (10 minutes). Use multiple scenes — don't try to fit everything into one shot. Add narration via add_voiceover_to_video using the content analysis as your script."#,
        title = job.source_video_title.as_deref().unwrap_or("Unknown Video"),
        video_summary = analysis.video_summary,
        top_moments = top_moments_str,
    );

    let (progress_tx, _progress_rx) = tokio::sync::mpsc::unbounded_channel::<String>();

    let agent_result = agent
        .chat(
            &agent_prompt,
            &fallback_session_id,
            String::new(),
            app_state.clone(),
            app_state.job_manager.clone(),
            Some(progress_tx),
            None, // workflow_id
            None, // user_message_rx
            Some(linkage.user_id),
        )
        .await?;

    let output_url = agent_result.trim().to_string();

    sqlx::query(
        "UPDATE deliveries SET output_r2_url = $1, status = 'completed', completed_at = NOW() WHERE id = $2",
    )
    .bind(&output_url)
    .bind(delivery_id)
    .execute(&app_state.db_pool)
    .await
    .map_err(|e| format!("Failed to update fallback delivery: {}", e))?;

    sqlx::query(
        "UPDATE clipping_jobs
         SET status = 'fallback_rendering',
             progress_percent = 85,
             current_step = 'fallback_delivery_completed',
             error_message = $1,
             fallback_delivery_id = $2,
             fallback_strategy = 'generated_summary_via_agent',
             fallback_activated_at = NOW(),
             completed_at = NOW(),
             updated_at = NOW()
         WHERE id = $3",
    )
    .bind(format!(
        "Source download failed; generated fallback animated summary via full agent pipeline: {}",
        failure_reason
    ))
    .bind(delivery_id)
    .bind(job.id)
    .execute(&app_state.db_pool)
    .await
    .map_err(|e| {
        format!(
            "Failed to update clipping job with fallback delivery: {}",
            e
        )
    })?;

    Ok(format!(
        "Clipping download failed, so an AI-generated animated summary was created via the full agent pipeline. Delivery ID: {}. Output URL: {}.",
        delivery_id,
        output_url,
    ))
}

fn clipping_download_timeout_secs() -> u64 {
    std::env::var("CLIPPING_DOWNLOAD_TIMEOUT_SECS")
        .ok()
        .and_then(|value| value.parse().ok())
        .unwrap_or(900)
}

pub(crate) async fn enhance_clips_with_full_agent(
    job_id: i32,
    user_id: i32,
    source_video_url: &str,
    content_type: &str,
    clips: &mut Vec<ExtractedClipData>,
    app_state: &Arc<AppState>,
    source_type: &str,
) {
    let enhancer = crate::clipping::clip_enhancer::ClipEnhancer::new(app_state.clone());

    for clip in clips.iter_mut() {
        enhancer.enhance_clip(clip, content_type).await;

        let mut review =
            review_clip_with_qa(clip, app_state, source_type, source_video_url, content_type).await;
        let mut reviewed_after_retry = false;

        if !review.pass {
            if let Some(retry_hint) = review.retry_hint.as_deref() {
                tracing::warn!(
                    job_id = job_id,
                    clip_number = clip.clip_number,
                    score = review.score,
                    retry_hint = retry_hint,
                    "Clip QA failed; retrying enhancement once with reviewer hint recorded"
                );
                append_clip_retry_hint(clip, retry_hint);
                enhancer.enhance_clip(clip, content_type).await;
                review = review_clip_with_qa(
                    clip,
                    app_state,
                    source_type,
                    source_video_url,
                    content_type,
                )
                .await;
                reviewed_after_retry = true;
            }
        }

        apply_review_to_clip(clip, &review, reviewed_after_retry);
        persist_clip_review_artifact(
            job_id,
            user_id,
            source_video_url,
            clip,
            app_state,
            source_type,
        )
        .await;
    }
}

fn append_clip_retry_hint(clip: &mut ExtractedClipData, retry_hint: &str) {
    let merged = match clip.enhancement_reasoning.take() {
        Some(existing) if !existing.trim().is_empty() => {
            format!("{existing}\nQA retry hint: {retry_hint}")
        }
        _ => format!("QA retry hint: {retry_hint}"),
    };
    clip.enhancement_reasoning = Some(merged);
}

async fn review_clip_with_qa(
    clip: &ExtractedClipData,
    app_state: &Arc<AppState>,
    source_type: &str,
    source_video_url: &str,
    content_type: &str,
) -> crate::render_review::ReviewResult {
    let prompt = format!(
        "Review this {source_type} short clip.\nTitle: {title}\nDescription: {description}\nSource URL: {source_video_url}\nContent type: {content_type}\nEnhancement notes: {enhancement}",
        source_type = source_type,
        title = clip.ai_title,
        description = clip.ai_description,
        source_video_url = source_video_url,
        content_type = content_type,
        enhancement = clip.enhancement_reasoning.as_deref().unwrap_or("none"),
    );

    crate::render_review::review_render(
        app_state,
        &clip.local_clip_path,
        &prompt,
        "clip_enhancement",
        None,
    )
    .await
}

fn apply_review_to_clip(
    clip: &mut ExtractedClipData,
    review: &crate::render_review::ReviewResult,
    reviewed_after_retry: bool,
) {
    let status = if review.score <= 0 {
        "review_skipped"
    } else if review.pass && reviewed_after_retry {
        "passed_after_retry"
    } else if review.pass {
        "passed"
    } else if reviewed_after_retry {
        "warning_after_retry"
    } else {
        "warning"
    };

    clip.qa_status = Some(status.to_string());
    clip.qa_score = Some(review.score);
    clip.qa_feedback = Some(review.feedback.clone());
    clip.qa_retry_hint = review.retry_hint.clone();
}

async fn persist_clip_review_artifact(
    job_id: i32,
    user_id: i32,
    source_video_url: &str,
    clip: &ExtractedClipData,
    app_state: &Arc<AppState>,
    source_type: &str,
) {
    let artifact = crate::services::media_review::MediaReviewArtifact {
        review_id: format!("{source_type}-{job_id}-{}", clip.clip_number),
        asset_kind: "clip".to_string(),
        source_type: source_type.to_string(),
        service_slug: Some("clipper-enhancement-pack".to_string()),
        owner_user_id: Some(user_id),
        output_url: Some(clip.local_clip_path.clone()),
        source_url: Some(source_video_url.to_string()),
        prompt: Some(format!("{} {}", clip.ai_title, clip.ai_description)),
        title: Some(clip.ai_title.clone()),
        company: None,
        review_status: clip
            .qa_status
            .clone()
            .unwrap_or_else(|| "not_reviewed".to_string()),
        qa_score: clip.qa_score,
        qa_feedback: clip.qa_feedback.clone(),
        narration_text: None,
        visual_direction: clip.enhancement_reasoning.clone(),
        transcript_excerpt: Some(clip.ai_description.clone()),
        tags: vec![
            "clip".to_string(),
            source_type.to_string(),
            format!("clip_number_{}", clip.clip_number),
        ],
    };

    if let Err(error) =
        crate::services::media_review::MediaReviewService::store_artifact(app_state, artifact).await
    {
        tracing::warn!(
            job_id = job_id,
            clip_number = clip.clip_number,
            error = %error,
            "Failed to store clip media review artifact"
        );
    }
}

pub async fn save_clips_to_database(
    job_id: i32,
    clips: &[ExtractedClipData],
    linkage: &ChannelLinkage,
    pool: &PgPool,
) -> Result<Vec<i32>, String> {
    // Delete any stale clips from a previous failed attempt for this job.
    // Without this, retried jobs accumulate duplicate extracted_clips rows.
    sqlx::query("DELETE FROM extracted_clips WHERE clipping_job_id = $1")
        .bind(job_id)
        .execute(pool)
        .await
        .map_err(|e| format!("Failed to clean stale clips before save: {}", e))?;

    let mut clip_ids = Vec::new();

    for clip in clips {
        let clip_id: i32 = sqlx::query_scalar(
            "INSERT INTO extracted_clips
             (clipping_job_id, clip_number, local_clip_path,
             start_time_seconds, end_time_seconds, duration_seconds,
              ai_title, ai_description, ai_tags, ai_confidence_score, viral_factors,
              destination_channel_id, custom_thumbnail_path, thumbnail_generation_method,
              enhancement_applied, enhancement_tools, enhancement_reasoning,
              r2_clip_key, r2_thumb_key, r2_clip_url,
              qa_status, qa_score, qa_feedback, qa_retry_hint)
             VALUES ($1, $2, $3, $4, $5, $6, $7, $8, $9, $10, $11, $12, $13, $14, $15, $16, $17, $18, $19, $20, $21, $22, $23, $24)
             RETURNING id",
        )
        .bind(job_id)
        .bind(clip.clip_number)
        .bind(&clip.local_clip_path)
        .bind(clip.start_time_seconds)
        .bind(clip.end_time_seconds)
        .bind(clip.duration_seconds)
        .bind(&clip.ai_title)
        .bind(&clip.ai_description)
        .bind(&clip.ai_tags)
        .bind(clip.ai_confidence_score)
        .bind(&clip.viral_factors)
        .bind(linkage.destination_channel_id)
        .bind(&clip.custom_thumbnail_path)
        .bind(&clip.thumbnail_generation_method)
        .bind(clip.enhancement_applied)
        .bind(&clip.enhancement_tools)
        .bind(&clip.enhancement_reasoning)
        .bind(&clip.r2_clip_key)
        .bind(&clip.r2_thumb_key)
        .bind(&clip.r2_clip_url)
        .bind(clip.qa_status.as_deref().unwrap_or("not_reviewed"))
        .bind(clip.qa_score)
        .bind(&clip.qa_feedback)
        .bind(&clip.qa_retry_hint)
        .fetch_one(pool)
        .await
        .map_err(|e| format!("Failed to save clip: {}", e))?;

        clip_ids.push(clip_id);
    }

    Ok(clip_ids)
}

/// Count clips successfully published for a destination channel in the last 24 hours.
pub async fn count_clips_posted_today(
    destination_channel_id: i32,
    pool: &PgPool,
) -> Result<i64, String> {
    sqlx::query_scalar::<_, i64>(
        "SELECT COUNT(*) FROM extracted_clips
         WHERE destination_channel_id = $1
           AND upload_status = 'published'
           AND published_at >= NOW() - INTERVAL '24 hours'",
    )
    .bind(destination_channel_id)
    .fetch_one(pool)
    .await
    .map_err(|e| format!("Failed to count daily clips: {}", e))
}

pub async fn update_linkage_stats(
    linkage_id: i32,
    clips_generated: i32,
    clips_posted: i32,
    pool: &PgPool,
) -> Result<(), String> {
    sqlx::query(
        "UPDATE youtube_channel_linkages
         SET total_clips_generated = total_clips_generated + $1,
             total_clips_posted = total_clips_posted + $2,
             last_clip_generated_at = NOW()
         WHERE id = $3",
    )
    .bind(clips_generated)
    .bind(clips_posted)
    .bind(linkage_id)
    .execute(pool)
    .await
    .map_err(|e| format!("Failed to update linkage stats: {}", e))?;

    Ok(())
}

/// After the agentic pipeline or clip extraction completes, compile all extracted clips
/// into a single MP4 compilation, upload to R2, and store all clip R2 keys in
/// deliveries.extra_args for the gallery view on the delivery page.
///
/// Supports two data sources:
/// 1. `deliveries.extra_args.clip_urls` — presigned URLs set by the pipeline from agent response
/// 2. `extracted_clips` table — rows from the legacy deterministic clipping path
pub async fn post_process_clipping_delivery(
    job_id: i32,
    delivery_id: uuid::Uuid,
    app_state: &crate::AppState,
) -> Result<(), String> {
    // 1a. Try fetching clip URLs from deliveries.extra_args.clip_urls first (default agentic path)
    let delivery_row = sqlx::query(
        "SELECT extra_args FROM deliveries WHERE id = $1",
    )
    .bind(delivery_id)
    .fetch_optional(&app_state.db_pool)
    .await
    .map_err(|e| format!("Failed to fetch delivery {delivery_id}: {e}"))?;

    let (mut clip_urls, mut clip_keys): (Vec<String>, Vec<String>) = (vec![], vec![]);
    if let Some(row) = delivery_row {
        if let Ok(Some(extra)) = row.try_get::<Option<serde_json::Value>, _>("extra_args") {
            if let Some(urls) = extra.get("clip_urls").and_then(|v| v.as_array()) {
                let parsed: Vec<String> = urls
                    .iter()
                    .filter_map(|v| v.as_str().map(|s| s.to_string()))
                    .collect();
                if !parsed.is_empty() {
                    clip_urls = parsed;
                    // Extract R2 keys from presigned URLs for gallery regeneration
                    if let Some(ref r2) = app_state.r2_client {
                        clip_keys = clip_urls
                            .iter()
                            .filter_map(|u| r2_key_from_presigned_url(u, &r2.bucket))
                            .collect();
                    }
                }
            }
        }
    }

    // 1b. Fall back to extracted_clips table (legacy deterministic path)
    if clip_urls.is_empty() {
        let clip_rows = sqlx::query(
            "SELECT r2_clip_key, r2_clip_url, clip_number FROM extracted_clips \
             WHERE clipping_job_id = $1 AND r2_clip_url IS NOT NULL \
             ORDER BY clip_number",
        )
        .bind(job_id)
        .fetch_all(&app_state.db_pool)
        .await
        .map_err(|e| format!("Failed to fetch clips for compilation: {e}"))?;

        if clip_rows.is_empty() {
            tracing::info!("No clips to compile for job {job_id}");
            return Ok(());
        }

        clip_urls = clip_rows
            .iter()
            .filter_map(|r| r.try_get::<Option<String>, _>("r2_clip_url").ok().flatten())
            .collect();
        clip_keys = clip_rows
            .iter()
            .filter_map(|r| r.try_get::<Option<String>, _>("r2_clip_key").ok().flatten())
            .collect();
    }

    tracing::info!("Compiling {} clips for delivery {delivery_id}", clip_urls.len());

    // 2. Download each clip from R2 to temp files
    let http = reqwest::Client::new();
    let mut temp_paths: Vec<String> = Vec::new();
    for (i, url) in clip_urls.iter().enumerate() {
        let path = crate::utils::ffmpeg_utils::create_temp_file(
            &format!("clip_compilation_{job_id}_{i}"),
            "mp4",
        );
        let resp = http
            .get(url)
            .send()
            .await
            .map_err(|e| format!("Failed to download clip {i}: {e}"))?;
        let bytes = resp
            .bytes()
            .await
            .map_err(|e| format!("Failed to read clip {i}: {e}"))?;
        tokio::fs::write(&path, &bytes)
            .await
            .map_err(|e| format!("Failed to write clip {i}: {e}"))?;
        temp_paths.push(path);
    }

    // 3. Merge clips into a single compilation video
    let output_path = crate::utils::ffmpeg_utils::create_temp_file(
        &format!("compilation_{job_id}"),
        "mp4",
    );
    let _ = crate::core::merge_videos(&temp_paths, &output_path)?;

    // 4. Upload compilation to R2
    let compilation_key = format!("clip_compilation/{delivery_id}/{job_id}/compilation.mp4");
    let compilation_url = app_state
        .r2_client
        .as_ref()
        .ok_or("R2 client not configured for clip compilation")?
        .upload_file(&output_path, &compilation_key)
        .await
        .map_err(|e| format!("R2 compilation upload failed: {e}"))?;

    // 5. Build clip keys JSON for gallery on delivery page
    let extra_args_update = serde_json::json!({
        "clip_keys": clip_keys,
        "clip_count": clip_urls.len(),
    });

    // 6. Update delivery: output_r2_url = compilation, extra_args with clip metadata
    let _ = sqlx::query(
        "UPDATE deliveries SET \
         output_r2_url = COALESCE($1, output_r2_url), \
         extra_args = COALESCE(extra_args, '{}'::jsonb) || $2::jsonb \
         WHERE id = $3",
    )
    .bind(&compilation_url)
    .bind(&extra_args_update)
    .bind(delivery_id)
    .execute(&app_state.db_pool)
    .await
    .map_err(|e| tracing::warn!("Failed to update delivery {delivery_id}: {e}"));

    // 7. Cleanup temp files
    for p in &temp_paths {
        let _ = tokio::fs::remove_file(p).await;
    }
    let _ = tokio::fs::remove_file(&output_path).await;

    tracing::info!(
        "✅ Compiled {n} clips into compilation for delivery {delivery_id}: {compilation_url}",
        n = clip_urls.len()
    );
    Ok(())
}

/// Extract the R2 object key from a presigned URL like:
/// `https://<bucket>.<account>.r2.cloudflarestorage.com/<key>?<query>`
fn r2_key_from_presigned_url(url: &str, bucket: &str) -> Option<String> {
    let pattern = format!(".r2.cloudflarestorage.com/");
    let after_bucket = url.split(&pattern).nth(1)?;
    let key = after_bucket.split('?').next()?;
    Some(urlencoding::decode(key).ok()?.into_owned())
}
