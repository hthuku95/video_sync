use crate::services::{GeneratedArtifactService, NewWorkflow, WorkflowRuntime, WorkflowStatus};
use crate::utils::ffmpeg_utils::execute_ffmpeg_command_with_sync_timeout;
use crate::AppState;
use serde::{Deserialize, Serialize};
use serde_json::{json, Value};
use std::process::Command;
use std::sync::Arc;
use std::time::Duration;
use uuid::Uuid;

#[derive(Debug, Clone, Deserialize)]
pub struct LongFormVideoRequest {
    pub title: String,
    pub brief: String,
    #[serde(default = "default_target_duration_seconds")]
    pub target_duration_seconds: f64,
    #[serde(default = "default_segment_duration_seconds")]
    pub segment_duration_seconds: f64,
    #[serde(default = "default_style")]
    pub style: String,
    #[serde(default = "default_offer_type")]
    pub offer_type: String,
    #[serde(default = "default_speaker")]
    pub narration_speaker: String,
    #[serde(default = "default_include_narration")]
    pub include_narration: bool,
    pub reference_url: Option<String>,
    pub session_uuid: Option<String>,
    pub user_id: Option<i32>,
    pub source_table: Option<String>,
    pub source_record_id: Option<Uuid>,
    pub idempotency_key: Option<String>,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct LongFormSegmentPlan {
    pub index: usize,
    pub title: String,
    pub objective: String,
    pub visual_tool: String,
    pub narration: String,
    pub duration_seconds: f64,
}

#[derive(Debug, Clone)]
struct PublishedLongFormVideo {
    public_url: Option<String>,
    storage_key: Option<String>,
    output_filename: String,
}

fn default_target_duration_seconds() -> f64 {
    600.0
}

fn default_segment_duration_seconds() -> f64 {
    30.0
}

fn default_style() -> String {
    "premium SaaS explainer, cinematic, clean motion graphics".to_string()
}

fn default_offer_type() -> String {
    "long_form_video".to_string()
}

fn default_speaker() -> String {
    "Emma".to_string()
}

fn default_include_narration() -> bool {
    true
}

pub struct LongFormVideoWorkflow;

impl LongFormVideoWorkflow {
    pub async fn start(state: Arc<AppState>, req: LongFormVideoRequest) -> Result<Uuid, String> {
        let runtime = WorkflowRuntime::new(state.db_pool.clone());
        let target_duration = req.target_duration_seconds.max(15.0);
        let segment_duration = normalized_segment_duration(target_duration, req.segment_duration_seconds);
        let estimated_segments = (target_duration / segment_duration).ceil() as usize;
        let workflow_id = runtime
            .create_or_reuse_workflow(NewWorkflow {
                workflow_type: "long_form_video_assembly".to_string(),
                idempotency_key: req.idempotency_key.clone(),
                status: WorkflowStatus::Queued,
                session_uuid: req.session_uuid.clone(),
                user_id: req.user_id,
                source_table: req.source_table.clone(),
                source_record_id: req.source_record_id,
                request_summary: format!(
                    "{} ({:.0}s, ~{} segments)",
                    req.title, target_duration, estimated_segments
                ),
                current_step: Some("queued".to_string()),
                metadata: json!({
                    "title": req.title,
                    "brief": req.brief,
                    "style": req.style,
                    "offer_type": req.offer_type,
                    "reference_url": req.reference_url,
                    "target_duration_seconds": target_duration,
                    "segment_duration_seconds": segment_duration,
                    "estimated_segments": estimated_segments,
                    "include_narration": req.include_narration,
                }),
                artifact_requirements: json!({
                    "kind": "long_form_video",
                    "strategy": "plan_segments_render_review_assemble",
                    "supports_any_length": true,
                    "segment_duration_seconds": segment_duration,
                }),
            })
            .await?;

        let background_state = state.clone();
        let source_table = req.source_table.clone();
        let source_record_id = req.source_record_id;
        tokio::spawn(async move {
            if let Err(error) = Self::run(background_state.clone(), workflow_id, req).await {
                let runtime = WorkflowRuntime::new(background_state.db_pool.clone());
                let _ = runtime
                    .mark_failed(workflow_id, Some("failed"), &error, None)
                    .await;
                if source_table.as_deref() == Some("deliveries") {
                    if let Some(delivery_id) = source_record_id {
                        let _ = sqlx::query(
                            "UPDATE deliveries
                             SET status = 'failed',
                                 error_message = $1,
                                 completed_at = NOW()
                             WHERE id = $2",
                        )
                        .bind(&error)
                        .bind(delivery_id)
                        .execute(&background_state.db_pool)
                        .await;
                    }
                }
                tracing::error!(workflow_id = %workflow_id, error = %error, "Long-form video workflow failed");
            }
        });

        Ok(workflow_id)
    }

    async fn run(
        state: Arc<AppState>,
        workflow_id: Uuid,
        req: LongFormVideoRequest,
    ) -> Result<(), String> {
        let runtime = WorkflowRuntime::new(state.db_pool.clone());
        if req.source_table.as_deref() == Some("deliveries") {
            if let Some(delivery_id) = req.source_record_id {
                let _ = sqlx::query(
                    "UPDATE deliveries
                     SET status = 'running',
                         error_message = NULL,
                         updated_at = NOW()
                     WHERE id = $1
                       AND status IN ('pending', 'queued')",
                )
                .bind(delivery_id)
                .execute(&state.db_pool)
                .await;
            }
        }
        std::fs::create_dir_all("outputs")
            .map_err(|e| format!("Failed to create outputs directory: {e}"))?;

        let plan_node = runtime
            .ensure_node(
                workflow_id,
                "plan",
                "plan_segments",
                json!({ "target_duration_seconds": req.target_duration_seconds }),
                3,
            )
            .await?;
        let plans = if plan_node.status == "completed" {
            if let Some(segments) = plan_node.output.get("segments") {
                match serde_json::from_value::<Vec<LongFormSegmentPlan>>(segments.clone()) {
                    Ok(plans) if !plans.is_empty() => {
                        runtime
                            .append_event(
                                workflow_id,
                                "resume",
                                Some("plan"),
                                "Reusing persisted long-form segment plan",
                                json!({ "segment_count": plans.len() }),
                            )
                            .await?;
                        plans
                    }
                    _ => {
                        runtime
                            .start_node(
                                workflow_id,
                                "plan",
                                "Planning long-form video as bounded segments",
                                json!({ "target_duration_seconds": req.target_duration_seconds }),
                            )
                            .await?;
                        let plans = Self::plan_segments(&state, &req).await;
                        runtime
                            .complete_node(
                                workflow_id,
                                "plan",
                                json!({ "segments": &plans }),
                                "Long-form segment plan persisted",
                            )
                            .await?;
                        plans
                    }
                }
            } else {
                runtime
                    .start_node(
                        workflow_id,
                        "plan",
                        "Planning long-form video as bounded segments",
                        json!({ "target_duration_seconds": req.target_duration_seconds }),
                    )
                    .await?;
                let plans = Self::plan_segments(&state, &req).await;
                runtime
                    .complete_node(
                        workflow_id,
                        "plan",
                        json!({ "segments": &plans }),
                        "Long-form segment plan persisted",
                    )
                    .await?;
                plans
            }
        } else {
            runtime
                .start_node(
                    workflow_id,
                    "plan",
                    "Planning long-form video as bounded segments",
                    json!({ "target_duration_seconds": req.target_duration_seconds }),
                )
                .await?;
            let plans = Self::plan_segments(&state, &req).await;
            runtime
                .complete_node(
                    workflow_id,
                    "plan",
                    json!({ "segments": &plans }),
                    "Long-form segment plan persisted",
                )
                .await?;
            plans
        };

        runtime
            .heartbeat(
                workflow_id,
                WorkflowStatus::Running,
                Some("render_segments"),
                &format!("Rendering {} planned video segments", plans.len()),
                json!({ "segments": &plans }),
            )
            .await?;

        let mut segment_paths = Vec::new();
        for plan in &plans {
            let node_key = format!("render_segment_{:04}", plan.index);
            let segment_node = runtime
                .ensure_node(
                    workflow_id,
                    &node_key,
                    "render_segment",
                    json!({
                        "segment_index": plan.index,
                        "plan": plan,
                    }),
                    3,
                )
                .await?;

            if segment_node.status == "completed" {
                if let Some(path) = segment_node.output.get("path").and_then(|v| v.as_str()) {
                    if std::path::Path::new(path).exists() {
                        runtime
                            .append_event(
                                workflow_id,
                                "resume",
                                Some(&node_key),
                                "Reusing completed rendered segment",
                                json!({ "segment_index": plan.index, "path": path }),
                            )
                            .await?;
                        segment_paths.push(path.to_string());
                        continue;
                    }
                    runtime
                        .append_event(
                            workflow_id,
                            "resume",
                            Some(&node_key),
                            "Persisted segment path is missing locally; re-rendering segment",
                            json!({ "segment_index": plan.index, "missing_path": path }),
                        )
                        .await?;
                }
            }

            runtime
                .start_node(
                    workflow_id,
                    &node_key,
                    &format!("Rendering segment {}: {}", plan.index, plan.title),
                    json!({
                        "segment_index": plan.index,
                        "visual_tool": &plan.visual_tool,
                        "duration_seconds": plan.duration_seconds
                    }),
                )
                .await?;

            let path = match Self::render_segment(&state, workflow_id, &req, plan).await {
                Ok(path) => path,
                Err(error) => {
                    let _ = runtime
                        .fail_node(
                            workflow_id,
                            &node_key,
                            &error,
                            json!({ "segment_index": plan.index }),
                        )
                        .await;
                    return Err(error);
                }
            };
            let with_audio = if req.include_narration {
                match Self::attach_narration(&state, workflow_id, &req, plan, &path).await {
                    Ok(path) => path,
                    Err(error) => {
                        tracing::warn!(
                            workflow_id = %workflow_id,
                            segment_index = plan.index,
                            error = %error,
                            "Narration failed; keeping silent segment"
                        );
                        path
                    }
                }
            } else {
                path
            };
            runtime
                .complete_node(
                    workflow_id,
                    &node_key,
                    json!({
                        "path": &with_audio,
                        "segment_index": plan.index,
                        "visual_tool": &plan.visual_tool,
                    }),
                    "Rendered long-form segment persisted",
                )
                .await?;
            segment_paths.push(with_audio);
        }

        let assemble_node = runtime
            .ensure_node(
                workflow_id,
                "assemble",
                "assemble_segments",
                json!({ "segment_paths": &segment_paths }),
                3,
            )
            .await?;
        let final_path = format!("outputs/long_form_{}.mp4", workflow_id);
        if assemble_node.status == "completed" {
            if let Some(path) = assemble_node.output.get("final_path").and_then(|v| v.as_str()) {
                if std::path::Path::new(path).exists() {
                    runtime
                        .append_event(
                            workflow_id,
                            "resume",
                            Some("assemble"),
                            "Reusing completed assembled long-form video",
                            json!({ "final_path": path }),
                        )
                        .await?;
                } else {
                    runtime
                        .append_event(
                            workflow_id,
                            "resume",
                            Some("assemble"),
                            "Persisted assembled video is missing locally; reassembling",
                            json!({ "missing_path": path }),
                        )
                        .await?;
                    Self::run_assemble_node(&runtime, workflow_id, &plans, &segment_paths, &final_path)
                        .await?;
                }
            } else {
                Self::run_assemble_node(&runtime, workflow_id, &plans, &segment_paths, &final_path)
                    .await?;
            }
        } else {
            Self::run_assemble_node(&runtime, workflow_id, &plans, &segment_paths, &final_path)
                .await?;
        }

        let publish_node = runtime
            .ensure_node(
                workflow_id,
                "publish",
                "publish_final_video",
                json!({ "final_path": &final_path }),
                3,
            )
            .await?;
        let published = if publish_node.status == "completed" {
            runtime
                .append_event(
                    workflow_id,
                    "resume",
                    Some("publish"),
                    "Reusing completed publish metadata",
                    json!({ "output": &publish_node.output }),
                )
                .await?;
            PublishedLongFormVideo {
                public_url: publish_node
                    .output
                    .get("public_url")
                    .and_then(|v| v.as_str())
                    .map(|v| v.to_string()),
                storage_key: publish_node
                    .output
                    .get("storage_key")
                    .and_then(|v| v.as_str())
                    .map(|v| v.to_string()),
                output_filename: publish_node
                    .output
                    .get("output_filename")
                    .and_then(|v| v.as_str())
                    .unwrap_or("")
                    .to_string(),
            }
        } else {
            Self::run_publish_node(&runtime, &state, &req, workflow_id, &final_path).await?
        };

        let bytes = std::fs::metadata(&final_path).map(|m| m.len() as i64).ok();
        let artifact = GeneratedArtifactService::register_local_artifact(
            &state.db_pool,
            req.session_uuid.as_deref(),
            Some(workflow_id),
            "long_form_video",
            &final_path,
            Some("video/mp4"),
            bytes,
            "app_workflows",
            &workflow_id.to_string(),
        )
        .await
        .map_err(|e| format!("Failed to register long-form artifact: {e}"))?;

        runtime
            .mark_completed(
                workflow_id,
                Some("completed"),
                "Long-form video generated from segmented assembly workflow",
                json!({
                    "final_path": final_path,
                    "public_url": published.public_url,
                    "storage_key": published.storage_key,
                    "output_filename": published.output_filename,
                    "artifact_id": artifact.artifact_id,
                    "segment_count": segment_paths.len(),
                    "supports_any_length": true,
                    "node_backed": true
                }),
            )
            .await?;

        Ok(())
    }

    async fn run_assemble_node(
        runtime: &WorkflowRuntime,
        workflow_id: Uuid,
        plans: &[LongFormSegmentPlan],
        segment_paths: &[String],
        final_path: &str,
    ) -> Result<(), String> {
        runtime
            .start_node(
                workflow_id,
                "assemble",
                "Assembling long-form video from generated segments",
                json!({ "segment_count": segment_paths.len() }),
            )
            .await?;

        let assemble_timeout_secs = long_form_assemble_timeout_secs(plans);
        runtime
            .append_event(
                workflow_id,
                "progress",
                Some("assemble"),
                "Starting FFmpeg segment assembly",
                json!({
                    "segment_count": segment_paths.len(),
                    "timeout_seconds": assemble_timeout_secs,
                    "final_path": final_path,
                }),
            )
            .await?;

        let segment_paths_for_task = segment_paths.to_vec();
        let final_path_for_task = final_path.to_string();
        let assemble_task = tokio::task::spawn_blocking(move || {
            Self::assemble_segments(
                &segment_paths_for_task,
                &final_path_for_task,
                assemble_timeout_secs,
            )
        });
        match tokio::time::timeout(Duration::from_secs(assemble_timeout_secs + 15), assemble_task)
            .await
        {
            Ok(Ok(Ok(()))) => {
                runtime
                    .append_event(
                        workflow_id,
                        "progress",
                        Some("assemble"),
                        "Finished FFmpeg segment assembly",
                        json!({
                            "segment_count": segment_paths.len(),
                            "final_path": final_path,
                        }),
                    )
                    .await?;
            }
            Ok(Ok(Err(error))) => {
                let _ = runtime
                    .append_event(
                        workflow_id,
                        "error",
                        Some("assemble"),
                        "FFmpeg segment assembly failed",
                        json!({ "error": &error }),
                    )
                    .await;
                return Err(error);
            }
            Ok(Err(join_error)) => {
                let error = format!("Assembly task failed: {join_error}");
                let _ = runtime
                    .append_event(
                        workflow_id,
                        "error",
                        Some("assemble"),
                        "FFmpeg segment assembly worker failed",
                        json!({ "error": &error }),
                    )
                    .await;
                return Err(error);
            }
            Err(_) => {
                let error = format!(
                    "FFmpeg segment assembly timed out after {} seconds",
                    assemble_timeout_secs + 15
                );
                let _ = runtime
                    .append_event(
                        workflow_id,
                        "error",
                        Some("assemble"),
                        "FFmpeg segment assembly timed out",
                        json!({ "error": &error }),
                    )
                    .await;
                return Err(error);
            }
        }

        runtime
            .complete_node(
                workflow_id,
                "assemble",
                json!({
                    "final_path": final_path,
                    "segment_count": segment_paths.len(),
                }),
                "Assembled long-form video persisted",
            )
            .await?;

        Ok(())
    }

    async fn run_publish_node(
        runtime: &WorkflowRuntime,
        state: &Arc<AppState>,
        req: &LongFormVideoRequest,
        workflow_id: Uuid,
        final_path: &str,
    ) -> Result<PublishedLongFormVideo, String> {
        runtime
            .start_node(
                workflow_id,
                "publish",
                "Publishing long-form video output",
                json!({ "final_path": &final_path }),
            )
            .await?;

        let bytes = std::fs::metadata(&final_path).map(|m| m.len() as i64).ok();
        let publish_timeout_secs = long_form_publish_timeout_secs(bytes);
        runtime
            .append_event(
                workflow_id,
                "progress",
                Some("publish"),
                "Starting long-form media publish",
                json!({
                    "final_path": &final_path,
                    "bytes": bytes,
                    "timeout_seconds": publish_timeout_secs,
                }),
            )
            .await?;

        let published = match tokio::time::timeout(
            Duration::from_secs(publish_timeout_secs),
            Self::publish_final_video(&state, &req, workflow_id, &final_path),
        )
        .await
        {
            Ok(Ok(published)) => {
                runtime
                    .append_event(
                        workflow_id,
                        "progress",
                        Some("publish"),
                        "Finished long-form media publish",
                        json!({
                            "public_url_available": published.public_url.is_some(),
                            "storage_key": &published.storage_key,
                            "output_filename": &published.output_filename,
                        }),
                    )
                    .await?;
                published
            }
            Ok(Err(error)) => {
                let _ = runtime
                    .append_event(
                        workflow_id,
                        "error",
                        Some("publish"),
                        "Long-form media publish failed",
                        json!({ "error": &error }),
                    )
                    .await;
                return Err(error);
            }
            Err(_) => {
                let error = format!(
                    "Long-form media publish timed out after {publish_timeout_secs} seconds"
                );
                let _ = runtime
                    .append_event(
                        workflow_id,
                        "error",
                        Some("publish"),
                        "Long-form media publish timed out",
                        json!({ "error": &error }),
                    )
                    .await;
                return Err(error);
            }
        };
        let public_url = published.public_url.clone();
        let storage_key = published.storage_key.clone();
        let output_filename = published.output_filename.clone();
        runtime
            .complete_node(
                workflow_id,
                "publish",
                json!({
                    "public_url": public_url,
                    "storage_key": storage_key,
                    "output_filename": output_filename,
                }),
                "Published long-form media output",
            )
            .await?;

        Ok(published)
    }

    async fn publish_final_video(
        state: &Arc<AppState>,
        req: &LongFormVideoRequest,
        workflow_id: Uuid,
        final_path: &str,
    ) -> Result<PublishedLongFormVideo, String> {
        let output_filename = format!("long_form_{}.mp4", workflow_id);
        let mut public_url = None;
        let mut storage_key = None;
        let user_id = req.user_id.unwrap_or(0).max(0);
        let generated_key = crate::r2_client::R2Client::key_generated_video(user_id, &output_filename);

        if let Some(r2) = state.r2_client.as_ref() {
            match r2.upload(final_path, &generated_key).await {
                Ok(()) => {
                    let url = r2.presign_get(&generated_key, 7 * 24 * 3600).await?;
                    storage_key = Some(generated_key.clone());
                    public_url = Some(url);
                }
                Err(upload_error) => {
                    tracing::warn!(
                        workflow_id = %workflow_id,
                        error = %upload_error,
                        "R2 upload failed"
                    );
                }
            }
        }

        if req.source_table.as_deref() == Some("deliveries") {
            if let Some(delivery_id) = req.source_record_id {
                sqlx::query(
                    "UPDATE deliveries
                     SET status = 'completed',
                         output_r2_url = COALESCE($1, output_r2_url),
                         output_filename = $2,
                         error_message = NULL,
                         completed_at = NOW()
                     WHERE id = $3",
                )
                .bind(public_url.as_deref())
                .bind(&output_filename)
                .bind(delivery_id)
                .execute(&state.db_pool)
                .await
                .map_err(|e| format!("Failed to update long-form delivery output: {e}"))?;
            }
        }

        Ok(PublishedLongFormVideo {
            public_url,
            storage_key,
            output_filename,
        })
    }

    async fn plan_segments(
        state: &Arc<AppState>,
        req: &LongFormVideoRequest,
    ) -> Vec<LongFormSegmentPlan> {
        let segment_duration =
            normalized_segment_duration(req.target_duration_seconds, req.segment_duration_seconds);
        let count = (req.target_duration_seconds.max(15.0) / segment_duration)
            .ceil()
            .max(1.0) as usize;

        if let Some(plan) = Self::try_ai_plan(state, req, count, segment_duration).await {
            return plan;
        }

        (0..count)
            .map(|idx| {
                let visual_tool = match idx % 5 {
                    0 => "title_card",
                    1 => "ui_mockup",
                    2 => "data_viz",
                    3 => "latex",
                    _ => "scene",
                };
                LongFormSegmentPlan {
                    index: idx + 1,
                    title: format!("{} - Part {}", req.title, idx + 1),
                    objective: format!(
                        "Advance the story for {} using the brief: {}",
                        req.offer_type, req.brief
                    ),
                    visual_tool: visual_tool.to_string(),
                    narration: format!(
                        "{}. Part {} explains one useful angle clearly and keeps the viewer moving toward the call to action.",
                        req.title,
                        idx + 1
                    ),
                    duration_seconds: if idx + 1 == count {
                        let used = segment_duration * (count.saturating_sub(1) as f64);
                        (req.target_duration_seconds - used).max(8.0)
                    } else {
                        segment_duration
                    },
                }
            })
            .collect()
    }

    async fn try_ai_plan(
        state: &Arc<AppState>,
        req: &LongFormVideoRequest,
        count: usize,
        segment_duration: f64,
    ) -> Option<Vec<LongFormSegmentPlan>> {
        let prompt = format!(
            "Plan a segmented long-form video. Return ONLY JSON array. Each item must have: title, objective, visual_tool, narration. visual_tool must be one of title_card, ui_mockup, data_viz, latex, scene, manim. Target title: {}. Brief: {}. Style: {}. Segment count: {}. Each segment about {:.0}s. Reference URL: {}.",
            req.title,
            req.brief,
            req.style,
            count,
            segment_duration,
            req.reference_url.as_deref().unwrap_or("")
        );

        let text = if let Some(client) = state.ollama_client.as_ref() {
            client.generate_text(&prompt).await.ok()
        } else if let Some(client) = state.gemma_client.as_ref() {
            client.generate_text(&prompt).await.ok()
        } else if let Some(client) = state.nvidia_nim_client.as_ref() {
            client.generate_text(&prompt).await.ok()
        } else if let Some(client) = state.video_gemini_client.as_ref().or(state.gemini_client.as_ref()) {
            client.generate_text(&prompt).await.ok()
        } else {
            None
        }?;

        let json_text = extract_json_array(&text)?;
        let raw: Vec<Value> = serde_json::from_str(&json_text).ok()?;
        let mut plans = Vec::new();
        for (idx, item) in raw.into_iter().take(count).enumerate() {
            plans.push(LongFormSegmentPlan {
                index: idx + 1,
                title: item
                    .get("title")
                    .and_then(|v| v.as_str())
                    .unwrap_or("Segment")
                    .to_string(),
                objective: item
                    .get("objective")
                    .and_then(|v| v.as_str())
                    .unwrap_or("")
                    .to_string(),
                visual_tool: item
                    .get("visual_tool")
                    .and_then(|v| v.as_str())
                    .unwrap_or("scene")
                    .to_string(),
                narration: item
                    .get("narration")
                    .and_then(|v| v.as_str())
                    .unwrap_or("")
                    .to_string(),
                duration_seconds: if idx + 1 == count {
                    let used = segment_duration * (count.saturating_sub(1) as f64);
                    (req.target_duration_seconds - used).max(8.0)
                } else {
                    segment_duration
                },
            });
        }
        if plans.is_empty() {
            None
        } else {
            Some(plans)
        }
    }

    async fn render_segment(
        state: &Arc<AppState>,
        workflow_id: Uuid,
        req: &LongFormVideoRequest,
        plan: &LongFormSegmentPlan,
    ) -> Result<String, String> {
        let Some(blender) = state.blender_mcp_client.as_ref() else {
            return Self::render_fallback_segment(state, workflow_id, req, plan, "BlenderMCP client is not configured").await;
        };

        let timeout_secs = std::env::var("LONG_FORM_PRIMARY_RENDER_TIMEOUT_SECS")
            .ok()
            .and_then(|value| value.parse::<u64>().ok())
            .unwrap_or(300)
            .clamp(60, 1800);

        let blender = blender.clone();
        let req_clone = req.clone();
        let plan_clone = plan.clone();
        let mut render_task = tokio::spawn(async move {
            let duration = plan_clone.duration_seconds.clamp(8.0, 60.0);

            match plan_clone.visual_tool.as_str() {
            "title_card" => {
                let prompt = format!(
                    "Title card for '{}'. Subtitle: {}. Style: {}.",
                    plan_clone.title, plan_clone.objective, req_clone.style
                );
                blender
                    .execute_bpy_script(&prompt, duration, &req_clone.style, "")
                    .await
            }
            "ui_mockup" => {
                let prompt = format!(
                    "UI mockup for '{}': {}. Device: desktop, animation: reveal, style: {}.",
                    req_clone.title, plan_clone.objective, req_clone.style
                );
                blender
                    .execute_bpy_script(&prompt, duration, &req_clone.style, "")
                    .await
            }
            "data_viz" => {
                blender
                    .execute_manim_script(
                        &format!("Bar chart showing '{}': Problem 75%, Proof 54%, CTA 31%", plan_clone.title),
                        duration,
                        "dark",
                        false,
                        "m",
                    )
                    .await
            }
            "latex" => {
                blender
                    .execute_manim_script(
                        "LaTeX equation: Outcome = Clarity + Proof + CTA, written with animation",
                        duration,
                        "dark",
                        false,
                        "m",
                    )
                    .await
            }
            "manim" => {
                blender
                    .execute_manim_script(&plan_clone.objective, duration, "dark", false, "m")
                    .await
            }
            _ => {
                let prompt = format!(
                    "{}. {}. Style: {}. Reference: {}",
                    plan_clone.title,
                    plan_clone.objective,
                    req_clone.style,
                    req_clone.reference_url.as_deref().unwrap_or("")
                );
                blender
                    .execute_bpy_script(&prompt, duration, &req_clone.style, "")
                    .await
            }
            }
        });

        tokio::select! {
            render_result = &mut render_task => match render_result {
            Ok(Ok(path)) => Ok(path),
            Ok(Err(error)) => {
                Self::render_fallback_segment(state, workflow_id, req, plan, &error).await
            }
            Err(join_error) => {
                Self::render_fallback_segment(
                    state,
                    workflow_id,
                    req,
                    plan,
                    &format!("BlenderMCP render task failed: {join_error}"),
                )
                .await
            }
            },
            _ = tokio::time::sleep(Duration::from_secs(timeout_secs)) => {
                render_task.abort();
                Self::render_fallback_segment(
                    state,
                    workflow_id,
                    req,
                    plan,
                    &format!("BlenderMCP render timed out after {timeout_secs} seconds"),
                )
                .await
            }
        }
    }

    async fn render_fallback_segment(
        state: &Arc<AppState>,
        workflow_id: Uuid,
        req: &LongFormVideoRequest,
        plan: &LongFormSegmentPlan,
        reason: &str,
    ) -> Result<String, String> {
        let runtime = WorkflowRuntime::new(state.db_pool.clone());
        let _ = runtime
            .append_event(
                workflow_id,
                "fallback",
                Some("render_segment"),
                "Using local FFmpeg fallback segment because primary visual renderer was unavailable",
                json!({
                    "segment_index": plan.index,
                    "visual_tool": plan.visual_tool,
                    "reason": reason,
                }),
            )
            .await;

        let duration = plan.duration_seconds.clamp(8.0, 60.0);
        let output_path = format!(
            "outputs/long_form_{}_segment_{}_fallback.mp4",
            workflow_id, plan.index
        );
        let color = match plan.visual_tool.as_str() {
            "ui_mockup" => "0x0f766e",
            "data_viz" => "0x1d4ed8",
            "latex" | "manim" => "0x312e81",
            "scene" => "0x7c2d12",
            _ => "0x111827",
        };

        let mut command = Command::new("ffmpeg");
        command
            .arg("-f")
            .arg("lavfi")
            .arg("-i")
            .arg(format!("color=c={}:s=1280x720:r=30:d={:.2}", color, duration))
            .arg("-f")
            .arg("lavfi")
            .arg("-i")
            .arg(format!("anullsrc=channel_layout=stereo:sample_rate=44100:d={:.2}", duration))
            .arg("-shortest")
            .arg("-c:v")
            .arg("libx264")
            .arg("-preset")
            .arg("veryfast")
            .arg("-crf")
            .arg("23")
            .arg("-pix_fmt")
            .arg("yuv420p")
            .arg("-c:a")
            .arg("aac")
            .arg("-movflags")
            .arg("faststart")
            .arg("-metadata")
            .arg(format!("title={}", plan.title))
            .arg("-metadata")
            .arg(format!("comment={} | {}", req.offer_type, plan.objective))
            .arg("-y")
            .arg(&output_path);
        execute_ffmpeg_command_with_sync_timeout(command, Some(180))?;
        Ok(output_path)
    }

    async fn attach_narration(
        state: &Arc<AppState>,
        workflow_id: Uuid,
        req: &LongFormVideoRequest,
        plan: &LongFormSegmentPlan,
        video_path: &str,
    ) -> Result<String, String> {
        let vibevoice = state
            .vibevoice_client
            .as_ref()
            .ok_or_else(|| "VibeVoice client is not configured".to_string())?;
        let audio_bytes = vibevoice
            .text_to_speech_base64(
                &plan.narration,
                &req.narration_speaker,
                "mp3",
                Some(&format!("long-form-{workflow_id}-segment-{}", plan.index)),
                Some(json!({ "workflow_id": workflow_id, "segment_index": plan.index })),
            )
            .await?;
        let audio_path = format!("outputs/long_form_{}_segment_{}.mp3", workflow_id, plan.index);
        std::fs::write(&audio_path, audio_bytes)
            .map_err(|e| format!("Failed to write narration audio: {e}"))?;
        let output_path = format!(
            "outputs/long_form_{}_segment_{}_narrated.mp4",
            workflow_id, plan.index
        );
        crate::audio::add_audio(video_path, &audio_path, &output_path)
            .map_err(|e| format!("Failed to attach narration: {e}"))?;
        Ok(output_path)
    }

    fn assemble_segments(
        segment_paths: &[String],
        output_path: &str,
        timeout_secs: u64,
    ) -> Result<(), String> {
        if segment_paths.is_empty() {
            return Err("No segments generated for assembly".to_string());
        }
        let list_path = format!("outputs/concat_{}.txt", Uuid::new_v4());
        let mut list_lines = Vec::with_capacity(segment_paths.len());
        for path in segment_paths {
            let absolute_path = std::fs::canonicalize(path)
                .map_err(|e| format!("Failed to resolve segment path {path}: {e}"))?;
            let path_text = absolute_path.to_string_lossy().replace('\'', "'\\''");
            list_lines.push(format!("file '{}'", path_text));
        }
        let list_body = list_lines.join("\n");
        std::fs::write(&list_path, list_body)
            .map_err(|e| format!("Failed to write concat list: {e}"))?;

        let mut command = Command::new("ffmpeg");
        command
            .arg("-f")
            .arg("concat")
            .arg("-safe")
            .arg("0")
            .arg("-i")
            .arg(&list_path)
            .arg("-c:v")
            .arg("libx264")
            .arg("-preset")
            .arg("fast")
            .arg("-crf")
            .arg("23")
            .arg("-pix_fmt")
            .arg("yuv420p")
            .arg("-c:a")
            .arg("aac")
            .arg("-b:a")
            .arg("192k")
            .arg("-movflags")
            .arg("faststart")
            .arg("-y")
            .arg(output_path);
        let result = execute_ffmpeg_command_with_sync_timeout(command, Some(timeout_secs));
        let _ = std::fs::remove_file(list_path);
        result.map(|_| ())
    }
}

fn normalized_segment_duration(target_duration: f64, requested_segment_duration: f64) -> f64 {
    if target_duration <= 90.0 {
        requested_segment_duration.clamp(8.0, 20.0)
    } else if target_duration <= 600.0 {
        requested_segment_duration.clamp(15.0, 45.0)
    } else {
        requested_segment_duration.clamp(30.0, 60.0)
    }
}

fn long_form_assemble_timeout_secs(plans: &[LongFormSegmentPlan]) -> u64 {
    let planned_seconds: f64 = plans.iter().map(|plan| plan.duration_seconds.max(1.0)).sum();
    let default_timeout = (planned_seconds * 4.0).ceil() as u64 + 90;
    let configured_timeout = std::env::var("LONG_FORM_ASSEMBLE_TIMEOUT_SECS")
        .ok()
        .and_then(|value| value.parse::<u64>().ok());
    configured_timeout.unwrap_or(default_timeout).clamp(120, 1800)
}

fn long_form_publish_timeout_secs(bytes: Option<i64>) -> u64 {
    let default_timeout = bytes
        .and_then(|bytes| u64::try_from(bytes).ok())
        .map(|bytes| {
            let mib = (bytes / (1024 * 1024)).max(1);
            // R2/S3 uploads can be bursty from Cloud Run, and publish is the
            // revenue-critical handoff step. Give it room instead of timing
            // out a completed render during the final upload.
            600 + (mib * 15)
        })
        .unwrap_or(900);
    let configured_timeout = std::env::var("LONG_FORM_PUBLISH_TIMEOUT_SECS")
        .ok()
        .and_then(|value| value.parse::<u64>().ok());
    configured_timeout.unwrap_or(default_timeout).clamp(600, 7200)
}

fn extract_json_array(text: &str) -> Option<String> {
    let start = text.find('[')?;
    let end = text.rfind(']')?;
    if start <= end {
        Some(text[start..=end].to_string())
    } else {
        None
    }
}
