use crate::render_review::{review_render, ReviewResult};
use crate::services::workflow_runtime::{NewWorkflow, WorkflowRuntime, WorkflowStatus};
use crate::AppState;
use serde::{Deserialize, Serialize};
use serde_json::{json, Value};
use std::sync::Arc;
use uuid::Uuid;

pub const AGENT_MAX_RETRIES: i32 = 5;

#[derive(Debug, Clone, Copy, PartialEq, Serialize, Deserialize)]
pub enum ServiceType {
    LandingPage,
    ProductMockup,
    Thumbnails,
    Education,
    Clipping,
    VoiceAudio,
    FullStack,
    BusinessExplainer,
    ManimExplainer,
    WhiteboardAnimation,
    KineticTypography,
    AnimatedInfographic,
    AlgorithmViz,
    InvestorPitch,
    YearInReview,
    IsometricExplainer,
}

impl ServiceType {
    pub fn from_normalized(s: &str) -> Self {
        match s {
            "landing_page" | "saas_demo" | "saas_demo_pack" | "long_form_video" | "saas_launch_pack" => Self::LandingPage,
            "product_mockup" | "product_explainer" | "product_mockup_pack" | "three_d_scene" | "blender_scene_pack" => Self::ProductMockup,
            "thumbnails" | "thumbnail" | "thumbnail_hero_pack" => Self::Thumbnails,
            "education" | "course_lesson" | "explainer" | "tutorial" | "education_explainer_pack" => Self::Education,
            "clipping" | "clip" | "short" | "clip_pack" | "kick_auto_clipper" | "kick" => Self::Clipping,
            "business_explainer" | "business" | "business_case_study" | "saas_explainer" | "business_explainer_pack" => Self::BusinessExplainer,
            "voice_audio" | "voice" | "voice_audio_pack" | "narration" | "podcast" => Self::VoiceAudio,
            "full_stack" | "agency_bundle" | "agency" | "fullstack" | "full_stack_production_pack" | "agency_bundle_pack" => Self::FullStack,
            "manim_explainer" | "manim" | "manim_only" | "manim_pack" => Self::ManimExplainer,
            "whiteboard" | "whiteboard_animation" | "hand_drawn" | "sketch" | "whiteboard_pack" => Self::WhiteboardAnimation,
            "kinetic_typography" | "kinetic_type" | "text_animation" | "type_pack" => Self::KineticTypography,
            "animated_infographic" | "infographic" | "data_story" | "chart_animation" | "infographic_pack" => Self::AnimatedInfographic,
            "algorithm_viz" | "algorithm_visualization" | "code_viz" | "data_structure" | "algorithm_pack" => Self::AlgorithmViz,
            "investor_pitch" | "pitch_deck" | "investor_video" | "pitch_pack" => Self::InvestorPitch,
            "year_in_review" | "wrapped" | "annual_recap" | "year_recap" | "wrapped_pack" => Self::YearInReview,
            "isometric_explainer" | "isometric" | "isometric_pack" => Self::IsometricExplainer,
            "fallback_summary" | "summary" | "ad" | "advert" => Self::LandingPage,
            _ => Self::LandingPage,
        }
    }

    pub fn as_str(&self) -> &'static str {
        match self {
            Self::LandingPage => "landing_page",
            Self::ProductMockup => "product_mockup",
            Self::Thumbnails => "thumbnails",
            Self::Education => "education",
            Self::Clipping => "clipping",
            Self::VoiceAudio => "voice_audio",
            Self::FullStack => "full_stack",
            Self::BusinessExplainer => "business_explainer",
            Self::ManimExplainer => "manim_explainer",
            Self::WhiteboardAnimation => "whiteboard_animation",
            Self::KineticTypography => "kinetic_typography",
            Self::AnimatedInfographic => "animated_infographic",
            Self::AlgorithmViz => "algorithm_viz",
            Self::InvestorPitch => "investor_pitch",
            Self::YearInReview => "year_in_review",
            Self::IsometricExplainer => "isometric_explainer",
        }
    }

    pub fn default_style(&self) -> &'static str {
        match self {
            Self::LandingPage => "premium SaaS explainer, cinematic, clean motion graphics",
            Self::ProductMockup => "modern product showcase, minimal, device-focused",
            Self::Thumbnails => "bold, high-contrast, click-optimized, branded",
            Self::Education => "clear narrated educational explainer, technical diagrams, Manim animations",
            Self::Clipping => "high-retention creator clip pack, captions, branded lower thirds",
            Self::VoiceAudio => "professional podcast-quality audio production, clean mixing",
            Self::FullStack => "full-stack production backend, consistent branding across formats",
            Self::BusinessExplainer => "narrated business explainer, data-driven visuals, professional tone, clean motion graphics",
            Self::ManimExplainer => "Manim-powered animated explainer, clean motion graphics, narrated, no 3D required",
            Self::WhiteboardAnimation => "hand-drawn whiteboard sketch style, marker-on-board, educational, narrated",
            Self::KineticTypography => "dynamic text animation, word-by-word reveal, kinetic type, narrated",
            Self::AnimatedInfographic => "data-driven animated infographic, charts, counters, statistics, narrated",
            Self::AlgorithmViz => "algorithm visualization, code execution flow, data structures, narrated technical explainer",
            Self::InvestorPitch => "professional investor pitch video, motion graphics, narrated, clean brand presentation",
            Self::YearInReview => "personalized year-in-review recap, data-driven highlights, wrapped-style, narrated",
            Self::IsometricExplainer => "isometric 3D explainer, angled perspective view, clean motion graphics, narrated",
        }
    }

    pub fn default_duration_seconds(&self) -> f64 {
        match self {
            Self::LandingPage => 60.0,
            Self::ProductMockup => 30.0,
            Self::Thumbnails => 0.0,
            Self::Education => 90.0,
            Self::Clipping => 30.0,
            Self::VoiceAudio => 45.0,
            Self::FullStack => 90.0,
            Self::BusinessExplainer => 60.0,
            Self::ManimExplainer => 60.0,
            Self::WhiteboardAnimation => 60.0,
            Self::KineticTypography => 30.0,
            Self::AnimatedInfographic => 45.0,
            Self::AlgorithmViz => 90.0,
            Self::InvestorPitch => 90.0,
            Self::YearInReview => 60.0,
            Self::IsometricExplainer => 45.0,
        }
    }

    pub fn expects_video(&self) -> bool {
        matches!(self,
            Self::LandingPage | Self::ProductMockup | Self::Education |
            Self::Clipping | Self::BusinessExplainer | Self::ManimExplainer |
            Self::WhiteboardAnimation | Self::KineticTypography | Self::AnimatedInfographic |
            Self::AlgorithmViz | Self::InvestorPitch | Self::YearInReview |
            Self::IsometricExplainer
        )
    }

    pub fn expects_image(&self) -> bool {
        matches!(self, Self::Thumbnails)
    }

    pub fn expects_audio(&self) -> bool {
        matches!(self, Self::VoiceAudio)
    }
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct ServiceInput {
    pub title: String,
    pub brief: String,
    pub source_url: Option<String>,
    pub style: String,
    pub duration_seconds: f64,
    pub delivery_id: Uuid,
    pub prospect_id: Option<Uuid>,
    pub session_uuid: Option<String>,
    pub user_id: Option<i32>,
    pub source_table: Option<String>,
    pub source_record_id: Option<Uuid>,
    pub idempotency_key: Option<String>,
    pub reference_images: Vec<String>,
}

impl ServiceInput {
    pub fn service_filename(&self) -> &str {
        "output"
    }
}

pub struct AgenticServicePipeline;

impl AgenticServicePipeline {
    /// Enqueue a service render. Returns immediately after persisting the
    /// workflow row — execution is owned by the durable pipeline_worker pool,
    /// which claims queued rows with a DB lease. Work therefore survives
    /// Fargate task replacement, OOM kills, and deploys.
    pub async fn start(
        state: Arc<AppState>,
        service_type: ServiceType,
        input: ServiceInput,
    ) -> Result<Uuid, String> {
        // Full input snapshot so a worker on ANY Fargate task can reconstruct
        // and execute this job from the DB alone.
        let input_snapshot = serde_json::to_value(&input)
            .map_err(|e| format!("Failed to serialize ServiceInput: {e}"))?;

        let runtime = WorkflowRuntime::new(state.db_pool.clone());
        let workflow_id = runtime
            .create_or_reuse_workflow(NewWorkflow {
                workflow_type: format!("agentic_{}", service_type.as_str()),
                idempotency_key: input.idempotency_key.clone(),
                status: WorkflowStatus::Queued,
                session_uuid: input.session_uuid.clone(),
                user_id: input.user_id,
                source_table: input.source_table.clone(),
                source_record_id: input.source_record_id,
                request_summary: format!("{} ({})", input.title, service_type.as_str()),
                current_step: Some("queued".to_string()),
                metadata: json!({
                    "service_type": service_type.as_str(),
                    "delivery_id": input.delivery_id,
                    "source_url": input.source_url,
                    "style": input.style,
                    "duration": input.duration_seconds,
                    "input": input_snapshot,
                }),
                artifact_requirements: json!({
                    "expects_video": service_type.expects_video(),
                    "expects_image": service_type.expects_image(),
                    "expects_audio": service_type.expects_audio(),
                }),
            })
            .await?;

        // Link the workflow to the source delivery so admin UI / campaign
        // watchdog can correlate render state.
        let _ = sqlx::query("UPDATE deliveries SET workflow_id = $1 WHERE id = $2")
            .bind(workflow_id)
            .bind(input.delivery_id)
            .execute(&state.db_pool)
            .await;

        // Queue priority band: time-sensitive renders (scheduled campaign
        // posts, admin/clipping deliveries) claim before bulk/backfillable
        // sample generation. See migrations/20260824000002_queue_priority.sql.
        let priority: i16 = match input.source_table.as_deref() {
            Some("deliveries") if input.prospect_id.is_none() => 50,
            Some("service_portfolio_samples") | Some("app_workflows") | Some("gig_sample_videos") => 200,
            _ => 100,
        };
        let _ = sqlx::query("UPDATE app_workflows SET priority = $1 WHERE id = $2")
            .bind(priority)
            .bind(workflow_id)
            .execute(&state.db_pool)
            .await;

        tracing::info!(
            workflow_id = %workflow_id,
            service = %service_type.as_str(),
            delivery = %input.delivery_id,
            "📥 agentic render enqueued (durable queue)"
        );

        Ok(workflow_id)
    }

    /// Execute a claimed workflow. Called ONLY by pipeline_worker after it
    /// atomically claimed the row and while its lease-renewal ticker runs.
    ///
    /// `claimed_by` is threaded for future ownership probes; terminal failure
    /// marking stays in the worker (conditional, zombie-safe).
    pub async fn execute_claimed(
        state: Arc<AppState>,
        job: &crate::services::workflow_runtime::ClaimedWorkflow,
        claimed_by: &str,
    ) -> Result<(), String> {
        let workflow_id = job.id;

        let service_type = job
            .workflow_type
            .strip_prefix("agentic_")
            .map(ServiceType::from_normalized)
            .unwrap_or_else(|| {
                tracing::warn!(
                    "workflow {}: unexpected type '{}', defaulting to landing_page",
                    workflow_id,
                    job.workflow_type
                );
                ServiceType::from_normalized("")
            });

        let input: ServiceInput = match job.metadata.get("input") {
            Some(v) => serde_json::from_value(v.clone())
                .map_err(|e| format!("Failed to deserialize ServiceInput for {workflow_id}: {e}"))?,
            None => {
                return Err(format!(
                    "Workflow {} has no serialized input snapshot — cannot execute",
                    workflow_id
                ));
            }
        };

        Self::run(state, workflow_id, service_type, input, Some(claimed_by)).await
    }

    async fn run(
        state: Arc<AppState>,
        workflow_id: Uuid,
        service_type: ServiceType,
        input: ServiceInput,
        claimed_by: Option<&str>,
    ) -> Result<(), String> {
        let runtime = WorkflowRuntime::new(state.db_pool.clone());

        runtime
            .heartbeat(
                workflow_id,
                WorkflowStatus::Running,
                Some("agentic_workflow_started"),
                &format!("Agentic {} workflow started", service_type.as_str()),
                json!({}),
            )
            .await;

        let gemini_client = match state
            .video_gemini_client
            .as_ref()
            .or(state.gemini_client.as_ref())
        {
            Some(c) => Arc::new(c.clone()),
            None => return Err("No Gemini client available for agentic workflow".to_string()),
        };

        let ollama_client = state.ollama_client.clone().map(Arc::new);

        let output_dir = format!("outputs/agentic_{}", input.delivery_id);
        std::fs::create_dir_all(&output_dir).map_err(|e| format!("Failed to create output dir: {e}"))?;

        // ── PLAN-THEN-EXECUTE (§36.9 rec #1) ──
        // One cheap pre-flight LLM call emits an explicit stage plan that is
        // injected into every attempt's context. Regular workflow classes
        // benefit from synthesize-once/execute-deterministically: the agent
        // stops improvising detours mid-render and operators get an inspectable
        // plan artifact (app_workflow_events / trace endpoint). Bounded cost:
        // exactly one extra fast-chain call per RUN (not per attempt).
        let execution_plan = generate_execution_plan(&state, service_type, &input).await;
        let mut agent_prompt = Self::build_agent_prompt(service_type, &input, &output_dir);
        match &execution_plan {
            Ok(plan) if !plan.trim().is_empty() => {
                let _ = runtime
                    .append_event(
                        workflow_id,
                        "execution_plan",
                        None,
                        "Execution plan generated",
                        json!({ "plan": plan }),
                    )
                    .await;
                tracing::info!(workflow_id = %workflow_id, "🗺️ execution plan generated");
                agent_prompt = format!(
                    "## YOUR PRE-COMMITTED EXECUTION PLAN\n\
                     A planner reviewed this brief and produced the following step plan.\n\
                     Follow it sequentially. If a step fails, fix it and continue — do NOT\n\
                     skip steps or invent replacements. Mark progress as you go.\n\n{plan}\n\n\
                     ──────────── FULL PIPELINE INSTRUCTIONS ────────────\n\n{agent_prompt}"
                );
            }
            Ok(_) => tracing::warn!(workflow_id = %workflow_id, "execution plan empty — continuing without"),
            Err(e) => tracing::warn!(workflow_id = %workflow_id, "execution plan generation failed (non-fatal): {e}"),
        }

        let mut current_prompt = agent_prompt.clone();
        let mut best_output_path: Option<String> = None;
        let mut best_score: i32 = -1;
        let mut best_feedback = String::new();
        let mut retries_used: i32 = 0;

        for attempt in 0..AGENT_MAX_RETRIES {
            // Cooperative cancellation probe — checked between every attempt
            // so a cancelled workflow stops burning GPU immediately.
            if runtime.is_cancel_requested(workflow_id).await.unwrap_or(false) {
                return Err("WORKFLOW_CANCELLED: cancellation requested".to_string());
            }

            let session_id = format!(
                "{}-{}-try{}",
                service_type.as_str(),
                input.delivery_id,
                attempt
            );

            runtime
                .heartbeat(
                    workflow_id,
                    WorkflowStatus::Running,
                    Some("agentic_attempt"),
                    &format!("Attempt {} of {}", attempt + 1, AGENT_MAX_RETRIES),
                    json!({ "attempt": attempt + 1 }),
                )
                .await;

            let agent = crate::agent::stateful_agent::StatefulGeminiAgent::new_with_nvidia(
                gemini_client.clone(),
                state.bedrock_client.clone(),
                state.nvidia_nim_client.clone().map(Arc::new),
                ollama_client.clone(),
            )
            .with_tool_scope(Some(service_type.as_str().to_string()));

            // ── PROGRESS BRIDGE ──
            // Connect agent's progress_tx to the workflow_events table so callers
            // can poll GET /api/workflows/{workflow_id}/events for live progress.
            let (progress_tx, mut progress_rx) = tokio::sync::mpsc::unbounded_channel::<String>();
            let pool = state.db_pool.clone();
            let wf_id = workflow_id;
            let progress_task = tokio::spawn(async move {
                while let Some(msg) = progress_rx.recv().await {
                    let _ = sqlx::query(
                        r#"INSERT INTO app_workflow_events
                           (workflow_id, event_type, node_name, message, details)
                           VALUES ($1, 'agent_progress', NULL, $2, '{}'::jsonb)"#,
                    )
                    .bind(wf_id)
                    .bind(&msg)
                    .execute(&pool)
                    .await;
                }
            });

            // ── RE-EDITING CHANNEL (Redis pub/sub) ──
            // Subscribe to a Redis feedback channel so
            // POST /api/workflows/{workflow_id}/feedback can send messages to the
            // running agent (re-editing / add-on requests) across Fargate instances.
            let feedback_rx = if let Some(ref bus) = state.pubsub_bus {
                match bus.subscribe(&format!("feedback:{}", session_id)).await {
                    Ok(rx) => Some(rx),
                    Err(e) => {
                        tracing::warn!("Failed to subscribe to feedback channel: {}", e);
                        let (_tx, rx) = tokio::sync::mpsc::unbounded_channel::<String>();
                        Some(rx)
                    }
                }
            } else {
                let (_tx, rx) = tokio::sync::mpsc::unbounded_channel::<String>();
                Some(rx)
            };

            let agent_result = agent
                .chat(
                    &current_prompt,
                    &session_id,
                    String::new(),
                    state.clone(),
                    state.job_manager.clone(),
                    Some(progress_tx),
                    Some(workflow_id),
                    feedback_rx,
                    input.user_id,
                )
                .await;
            // Dropping progress_tx closes the channel, stopping progress_task

            let produced = locate_output_from_result(&agent_result, &output_dir);
            retries_used = attempt as i32;

            // Capture all clip URLs from the agent response for gallery view.
            // The generate_clip_compilation tool emits "Cloud URL:" lines.
            // Store as extra_args.clip_urls so the delivery page can show each clip.
            if service_type == ServiceType::Clipping {
                if let Ok(text) = &agent_result {
                    let clip_urls: Vec<String> = text
                        .lines()
                        .filter_map(|line| {
                            let t = line.trim();
                            t.strip_prefix("📤 Cloud URL: ")
                                .or_else(|| t.strip_prefix("Cloud URL: "))
                                .map(|u| u.trim().to_string())
                        })
                        .collect();
                    if !clip_urls.is_empty() {
                        let clip_meta = serde_json::json!({"clip_urls": clip_urls});
                        let _ = sqlx::query(
                            "UPDATE deliveries SET \
                             extra_args = COALESCE(extra_args, '{}'::jsonb) || $1::jsonb \
                             WHERE id = $2",
                        )
                        .bind(&clip_meta.to_string())
                        .bind(input.delivery_id)
                        .execute(&state.db_pool)
                        .await
                        .map_err(|e| tracing::warn!(
                            "Failed to store clip URLs for delivery {}: {e}",
                            input.delivery_id
                        ));
                    }
                }
            }

            let template_name = match service_type {
                ServiceType::Thumbnails => "agentic_thumbnail",
                _ => "agentic_service_video",
            };

            let review = if let Some(ref loc) = produced {
                // Use local path for review if available; otherwise skip review
                let review_target = loc.review_path.as_deref().unwrap_or(&loc.canonical);
                review_render(
                    &state,
                    review_target,
                    &input.brief,
                    template_name,
                    Some(input.delivery_id),
                )
                .await
            } else {
                ReviewResult {
                    pass: false,
                    score: 0,
                    feedback: "Agent did not produce expected output".to_string(),
                    retry_hint: Some(
                        "Produce the output file in the specified output directory".to_string(),
                    ),
                }
            };

            if review.score > best_score {
                best_score = review.score;
                best_output_path = produced.map(|loc| loc.canonical);
                best_feedback = review.feedback.clone();
            }

            if review.pass {
                runtime
                    .heartbeat(
                        workflow_id,
                        WorkflowStatus::Running,
                        Some("agentic_review_passed"),
                        &format!("Review passed on attempt {} (score {})", attempt + 1, review.score),
                        json!({ "score": review.score, "attempt": attempt + 1 }),
                    )
                    .await;
                break;
            }

            if attempt + 1 < AGENT_MAX_RETRIES {
                let hint = review
                    .retry_hint
                    .clone()
                    .unwrap_or_else(|| review.feedback.clone());

                // ── TARGETED REPAIR (§36.9 rec: repair beats regenerate) ──
                // One fast call decides whether the QA failure can be fixed by
                // redoing ONLY the flagged aspect while reusing surviving
                // artifacts, instead of burning GPU on a full rebuild.
                let artifacts = list_local_artifacts(&output_dir);
                let repair =
                    plan_repair_scope(&state, &review.feedback, &artifacts).await;
                current_prompt = match repair {
                    RepairScope::Partial { instructions } => {
                        tracing::info!(
                            workflow_id = %workflow_id,
                            "🔧 targeted repair on attempt {} (artifacts: {})",
                            attempt + 2,
                            artifacts.len()
                        );
                        format!(
                            "PARTIAL REPAIR REQUIRED — DO NOT REBUILD FROM SCRATCH.\n\
                             Previous attempt produced artifacts that are STILL VALID:\n{artifacts}\n\n\
                             QA review (score {score}/10) flagged ONE issue:\nFeedback: {feedback}\n\n\
                             YOUR REPAIR SCOPE — do exactly this and nothing more:\n{instructions}\n\n\
                             Reuse the existing artifacts wherever possible; re-render only the\n\
                             failing segment/aspect, then merge with the survivors and produce the\n\
                             final output again. Then run the full pipeline reference below:\n\n{agent_prompt}",
                            artifacts = artifacts.join("\n"),
                            score = review.score,
                            feedback = review.feedback,
                            instructions = instructions,
                            agent_prompt = agent_prompt,
                        )
                    }
                    RepairScope::Full => format!(
                        "PREVIOUS ATTEMPT FAILED QA REVIEW (score {}/10).\n\
                         Feedback: {}\n\
                         Retry hint: {}\n\n\
                         Apply the feedback above, then run the full pipeline below:\n\n{}",
                        review.score, review.feedback, hint, agent_prompt,
                    ),
                };
            }
        }

        let Some(output_path) = best_output_path else {
            return Err(format!(
                "No usable output from {} attempts for delivery {}",
                retries_used + 1,
                input.delivery_id
            ));
        };

        Self::publish_output(
            &state,
            &input,
            &output_path,
            best_score,
            &best_feedback,
            retries_used + 1,
        )
        .await?;

        // Clean up local files — all media lives in R2 now
        let _ = std::fs::remove_dir_all(&output_dir);

        // Terminal completion — ownership-aware when running under the
        // durable queue: if our lease was lost (process replaced, supervisor
        // requeued, newer attempt claimed), discard rather than clobber.
        match claimed_by {
            Some(owner) => {
                let owned = runtime
                    .mark_completed_if_owned(
                        workflow_id,
                        owner,
                        Some("completed"),
                        &format!(
                            "Agentic {} workflow completed with score {}/10",
                            service_type.as_str(),
                            best_score
                        ),
                        json!({
                            "output_path": output_path,
                            "delivery_id": input.delivery_id,
                            "qa_score": best_score,
                            "retries_used": retries_used + 1,
                        }),
                    )
                    .await
                    .unwrap_or(false);
                if !owned {
                    return Err("WORKFLOW_LEASE_LOST: ownership lost before completion — discarding result".to_string());
                }
            }
            None => {
                runtime
                    .mark_completed(
                        workflow_id,
                        Some("completed"),
                        &format!(
                            "Agentic {} workflow completed with score {}/10",
                            service_type.as_str(),
                            best_score
                        ),
                        json!({
                            "output_path": output_path,
                            "delivery_id": input.delivery_id,
                            "qa_score": best_score,
                            "retries_used": retries_used + 1,
                        }),
                    )
                    .await?;
            }
        }

        Ok(())
    }

    async fn publish_output(
        state: &Arc<AppState>,
        input: &ServiceInput,
        output_path: &str,
        qa_score: i32,
        qa_feedback: &str,
        retries_used: i32,
    ) -> Result<(), String> {
        let output_key = format!("agentic_output/{}/{}.mp4", input.delivery_id, input.service_filename());

        // If the agent already uploaded to R2, use that URL directly — no re-upload
        let public_url = if output_path.starts_with("https://") {
            let url = output_path.to_string();
            tracing::info!(url = %url, "publish_output: output already in cloud, skipping re-upload");
            url
        } else {
            let r2 = state
                .r2_client
                .as_ref()
                .ok_or("R2 not configured for publishing")?;

            let url = r2
                .upload_file(output_path, &output_key)
                .await
                .map_err(|e| format!("R2 upload failed: {e}"))?;
            url
        };

        let qa_note: Option<String> = if qa_score < 6 {
            Some(format!("QA final score {} after {} retries: {}", qa_score, retries_used, qa_feedback))
        } else {
            None
        };

        if input.source_table.as_deref() == Some("deliveries") {
            let update_result = sqlx::query(
                "UPDATE deliveries SET status = 'completed', output_r2_url = $1, \
                 output_filename = $2, final_qa_score = $3, completed_at = NOW() \
                 WHERE id = $4",
            )
            .bind(&public_url)
            .bind(&output_key)
            .bind(qa_score)
            .bind(input.delivery_id)
            .execute(&state.db_pool)
            .await;
            if let Err(e) = update_result {
                tracing::warn!("publish_output: failed to update delivery {}: {e}", input.delivery_id);
            }

            crate::handlers::social_publish::try_publish_delivery_to_zernio(input.delivery_id, state).await;

            if let Some(prospect_id) = input.prospect_id {
                let _ = sqlx::query(
                    "UPDATE prospects SET sample_delivery_id = $1, updated_at = NOW() WHERE id = $2",
                )
                .bind(input.delivery_id)
                .bind(prospect_id)
                .execute(&state.db_pool)
                .await;
            }
        } else if let Some(workflow_id) = input.source_record_id {
            let bytes = tokio::fs::read(output_path).await.unwrap_or_default();
            let _ = crate::services::GeneratedArtifactService::register_local_artifact(
                &state.db_pool,
                input.session_uuid.as_deref(),
                Some(workflow_id),
                "agentic_service_video",
                output_path,
                Some("video/mp4"),
                Some(bytes.len() as i64),
                "app_workflows",
                &workflow_id.to_string(),
            )
            .await;
        }

        Ok(())
    }

    fn landing_page_prompt(input: &ServiceInput) -> String {
        let url = input.source_url.as_deref().unwrap_or("");
        format!(
            r#"## SERVICE: Business Landing Page Video
GOAL: Create a polished landing page video for the business at {url} (target ~{duration_seconds}s — can be longer if the content requires it).

Source: {url}
Title: {title}
Brief: {brief}
Style: {style}

## ⚠️ CRITICAL: DO NOT USE generate_long_form_video
The tool `generate_long_form_video` is a convenience wrapper that delegates to another agent — it will NOT produce the correct output for this Managed Campaign service. You MUST call the rendering tools directly yourself.

## STEP 0: UNDERSTAND THE WEBSITE
Before making anything, call `browserbase_crawl_website(url="{url}")` to crawl the full website. This fetches all subpages and extracts CSS design tokens (colors, fonts). From the response:
1. Note the `feature_tag` and `pages` fields — you'll need both
2. Call `vectorize_crawled_content(feature_tag="<the tag>", pages=<the pages array>)` to store all pages in Qdrant
3. Use `search_crawled_content(query="brand colors, fonts, and design style", feature_tag="<the tag>")` for design details
4. Use `search_crawled_content(query="features, pricing, and product details", feature_tag="<the tag>")` for product info

Use the extracted content to understand:
- What the product/service does
- Key features and value propositions
- Brand colors, tone, and style
- Call-to-action and target audience

## MANDATORY TOOL SEQUENCE
1. generate_video_script(topic, duration, style, tone) — plan the content using the website info from step 0
2. Render the MAIN VISUAL using blender_generate_scene_type(prompt, params) — this is the core of your landing page video. Use it for product mockups, device frames, animated UI mockups, title cards, logos, and any 3D content.
3. If your video needs animated diagrams, data visualizations, or math/technical content, use manim_execute_script(description, ...) and merge with merge_videos
4. add_voiceover_to_video(video_path, script) or generate_text_to_speech(text, voice) — narrate the video
5. review_video(video_path_or_url) — check quality, fix issues, iterate
6. submit_final_answer(summary, output_files=[path]) — only after review passes

OUTPUT to: {output_dir}/
Save your final video with .mp4 extension"#,
            url = url,
            title = input.title,
            brief = input.brief,
            style = input.style,
            duration_seconds = input.duration_seconds as i32,
            output_dir = format!("outputs/agentic_{}", input.delivery_id),
        )
    }

    fn product_mockup_prompt(input: &ServiceInput) -> String {
        let url = input.source_url.as_deref().unwrap_or("");
        format!(
            r#"## SERVICE: Product Mockup Video
GOAL: Create an animated product/UI mockup video (target ~{duration_seconds}s — can be longer if the content requires it).

Source: {url}
Title: {title}
Brief: {brief}
Style: {style}

## ⚠️ CRITICAL: DO NOT USE generate_long_form_video
The tool `generate_long_form_video` delegates to another agent and will NOT produce the correct output. Call the rendering tools directly.

## STEP 0: UNDERSTAND THE WEBSITE (if source URL is provided)
If a source URL is provided, call `browserbase_crawl_website(url="{url}")` to crawl the full website. From the response, get the `feature_tag` and `pages`, then call `vectorize_crawled_content(feature_tag="<tag>", pages=<array>)` to store in Qdrant. Use `search_crawled_content(query="design and features", feature_tag="<tag>")` to get specific product details. Use the extracted content to understand the product's features, design, and brand style.

## MANDATORY TOOL SEQUENCE
0. (If the product/scene benefits from realistic pre-made geometry — devices,
   packaging, furniture, appliances) fetch real 3D assets:
   sketchfab_search(query="<product>") → pick a downloadable CC model →
   sketchfab_download(uid) → returns an R2 URL. Inside your bpy script, load it
   with the load_model_from_url() helper (provided in your system instructions),
   then position/light/animate the imported root like any Blender object.
   Normalize scale from its printed dimensions. Prefer this over primitive
   shapes whenever a real-world object is central to the scene.
1. generate_video_script(topic, duration, style, tone) — plan the content
2. Render the product mockup using blender_generate_scene_type(prompt, params) — for device mockups, 3D product animations, UI mockups, text reveals, and animated backgrounds
3. add_voiceover_to_video(video_path, script) or generate_text_to_speech(text, voice) — narrate if it improves the result
4. review_video(video_path_or_url) — check quality, iterate
5. submit_final_answer(summary, output_files=[path]) — only after review passes

OUTPUT to: {output_dir}/
Save final video as .mp4"#,
            url = url,
            title = input.title,
            brief = input.brief,
            style = input.style,
            duration_seconds = input.duration_seconds as i32,
            output_dir = format!("outputs/agentic_{}", input.delivery_id),
        )
    }

    fn thumbnail_prompt(input: &ServiceInput) -> String {
        format!(
            r#"## SERVICE: Thumbnail & Hero Visual
GOAL: Create a click-optimized thumbnail image.

Title: {title}
Brief: {brief}
Style: {style}

## ⚠️ CRITICAL: DO NOT USE generate_long_form_video
This is a STATIC IMAGE task — do NOT call any video generation tools.

## MANDATORY TOOL SEQUENCE
1. Create the image using create_thumbnail_hd(prompt, ...) or generate_image(prompt, ...) — whichever you think produces the best click-through result
2. review_video or view_image — verify quality
3. If it's not right, regenerate with improved prompt
4. submit_final_answer with the image path

OUTPUT to: {output_dir}/
Save as .png or .jpg"#,
            title = input.title,
            brief = input.brief,
            style = input.style,
            output_dir = format!("outputs/agentic_{}", input.delivery_id),
        )
    }

    fn education_prompt(input: &ServiceInput) -> String {
        let out = format!("outputs/agentic_{}", input.delivery_id);
        format!(
            r#"## SERVICE: Education Explainer Video
GOAL: Create a narrated educational video (target ~{duration_seconds}s — can be longer if the content requires it).

Topic: {brief}
Title: {title}
Style: {style}

## ⚠️ CRITICAL: DO NOT USE generate_long_form_video
That tool delegates to another agent and will NOT produce the correct educational video. Render directly.

## MANDATORY TOOL SEQUENCE
1. generate_video_script(topic="{brief}", duration={duration_seconds}, style="{style}", tone="professional")
2. Render scenes using the right tool for each visual:
   - manim_execute_script(description=..., quality="h") — for math equations, diagrams, code animations, data charts, LaTeX formulas, and ALL educational/technical visual content
   - blender_generate_scene_type(prompt=..., params=...) — for 3D backgrounds, animated props, intro/outro sequences
   - Use both together for mixed Blender+Manim scenes, then merge with merge_videos
3. add_voiceover_to_video(video_path="{out}/output.mp4", script="from script above")
4. review_video(video_path_or_url="{out}/output.mp4")
5. submit_final_answer(summary="education video {title}", output_files=["{out}/output.mp4"])

OUTPUT dir: {out}/"#,
            duration_seconds = input.duration_seconds as i32,
            brief = input.brief,
            title = input.title,
            style = input.style,
            out = out,
        )
    }

    fn clipping_prompt(input: &ServiceInput) -> String {
        let source = input.source_url.as_deref().unwrap_or("");
        format!(
            r#"## SERVICE: Clip Enhancement
GOAL: Extract engaging clips from content and make them shine.

Title: {title}
Brief: {brief}
Source: {source}

## ⚠️ CRITICAL: NEVER USE blender_generate_scene_type OR manim_execute_script
This is a CLIPPING/EDITING task. Do NOT generate content from scratch. Only edit existing video files.

## ⚠️ CRITICAL: DO NOT USE generate_long_form_video
That tool will start a new agent instead of clipping the actual source video.

## RECOMMENDED TOOL: generate_clip_compilation
Use this tool to automate the entire clipping pipeline:
- Downloads/streams the video from the source URL (YouTube, Kick, Twitch, or any public URL)
- Uses smart scene detection + audio energy analysis to find the most engaging moments
- Accepts explicit `clip_times` array if you already know where the best moments are
- Adds auto-generated subtitles/captions
- Uploads finished clips to R2 cloud storage
- Returns shareable R2 URLs for each clip

### ⚠️ CRITICAL: ANALYZE BEFORE CLIPPING
You have a vision-capable model. BEFORE calling generate_clip_compilation, analyze the video:
1. Download or review the source video to understand its content
2. Identify the most engaging moments — key highlights, funny parts, dramatic moments, educational value
3. THEN call generate_clip_compilation with:
   - `clip_times` set to the exact start times (in seconds) for each clip you identified
   - Example (Kick): generate_clip_compilation(source_url="https://kick.com/neon", clip_times=[12.5, 48.2, 93.7], clip_duration_seconds=15, max_clips=3, kick_style=true, logo_url="<R2 URL from download_asset>", streamer_name="Neon")
   - Example (general): generate_clip_compilation(source_url="{source}", clip_times=[12.5, 48.2, 93.7], clip_duration_seconds=15, max_clips=3, include_captions=true)
   - If you can't analyze the video, omit `clip_times` and the tool will use smart scene detection

If you provide `clip_times`, the tool trims at those exact positions. If you don't, it runs scene detection + audio energy analysis to auto-pick the best moments.

### 🔥 NEW: `kick_style` Auto Post-Processing
When `kick_style=true` is set, `generate_clip_compilation` automatically applies ALL Kick editing specs:
- 9:16 vertical (1080x1920) with blurred background — NO manual FFmpeg needed
- Logo watermark (download_asset → pass R2 URL as `logo_url`)
- Styled captions with bold font, white fill + black outline, lower-third
- Outro card "Watch Live on Kick" with streamer handle
- Proper H.264 encoding with AAC audio at TikTok/Shorts specs
- DO NOT try to do these edits manually with add_overlay/add_subtitles — use `kick_style`!

## ALTERNATIVE: Manual editing (if generate_clip_compilation is unavailable)
1. Download the source video using your available tools
2. Use trim_video or split_video to extract the best moments from the downloaded video
3. Enhance each clip: captions (add_subtitles), color grading, transitions, overlays, sound design
4. review_video on each clip, iterate on any that fall short
5. submit_final_answer with all clip paths

OUTPUT to: {output_dir}/
Save each clip as clip_N.mp4

## KICK CLIP EDITING SPECIFICATIONS (for Kick.com content only, skip for other sources)
When the source video comes from Kick.com or the user explicitly asks for Kick-compliant clips:

### LOGO / WATERMARK
- Place in the **top-left** corner (never overlaps content in the center)
- Size: **~8% of video width** (~86px on 1080p)
- Margin: **30-40px from top and left edges** (never touch the edge)
- Duration: **Entire clip** — must be visible for full duration
- Opacity: **100%** (solid, never translucent)
- Use the exact Kick logo file (download via download_asset tool from kick.com's brand assets)
- Color reference: Kick green is `#53FC18`

### CAPTIONS (Karaoke/Word-by-Word — NOT static subtitles)
- Style: **Word-by-word highlighting** (karaoke-style), NOT full-line static subtitles
- Font: **Montserrat Bold** or Inter Bold (bold weights only)
- Colors: **White fill** with **black/dark outline** (high contrast for mobile viewing)
- Position: **Lower-third** — leave bottom ~20% of frame clear as safe zone
- Minimum size: **30px** when scaled for mobile viewing (1280px width reference)

### LAYOUT (Stack / Split Screen — 9:16 vertical)
- Frame size: **1080×1920** (9:16 vertical, NOT horizontal)
- Facecam zone: **Top ~30-40%** of frame — zoomed and tracked to keep visible
- Gameplay/Content zone: **Middle to bottom ~40-50%** — zoomed to action areas, not full frame
- Gap fill: Blurred original source or gradient background between zones
- Never leave black bars (horizontal video in vertical feed = algorithm penalty)

### OUTRO CARD
- Duration: **1-2 seconds** at end of each clip
- Text: **"Watch Live on Kick"** or the streamer's handle (e.g. `@Neon`, `Kick.com/Neon`)
- Style: Branded lower-third card or full-screen

### ZOOM & RETENTION EFFECTS
- **Zoom on strong reactions**: Punch-in on screaming, laughing, or emotional moments
- **Camera shake**: On impact moments (hits, explosions, jumpscares)
- **Sound effects**: For emphasis on punchlines or dramatic reveals
- Auto-detect emotional peaks via audio analysis and apply zooms

### DEAD-SPACE REMOVAL
Aggressively trim out:
- Silence / pauses in speaking
- Loading screens
- Walking / travel between action
- Menu navigation
- Any segment without engagement
- Use silence detection + scene detection to identify these regions

### EXPORT SPECIFICATIONS
- Resolution: **1080×1920** (9:16 vertical)
- Codec: **H.264** (MP4 container)
- Frame rate: **60fps** (gaming content) / **30fps** (commentary/talking head)
- Bitrate: **15-20 Mbps**
- Max duration: **58 seconds** (safety margin for TikTok/Shorts 60s limit)
- Audio: **AAC 128-256 kbps** stereo
- File size: **Under 500MB**

### ❌ COMMON REJECTION REASONS (AVOID THESE)
1. Logo too small — not visible at mobile viewing size
2. Logo touching edge — no margin between logo and screen border
3. Logo not visible for full duration — fades out or gets covered
4. Modified logo file — recolored, stretched, filtered (use exact file from R2)
5. No captions or captions too small (below 30px when scaled)
6. Horizontal video in vertical feed — black bars = algorithm penalty
7. Stream UI left in frame — alerts, donation tickers, subscriber counters
8. Music copyright — Content ID flagging on YouTube
9. Missing streamer credit — title, description, on-screen handle"#,
            title = input.title,
            brief = input.brief,
            source = source,
            output_dir = format!("outputs/agentic_{}", input.delivery_id),
        )
    }

    fn voice_audio_prompt(input: &ServiceInput) -> String {
        format!(
            r#"## SERVICE: Voice & Audio Production
GOAL: Create professional narration/audio output.

Title: {title}
Brief: {brief}

## ⚠️ CRITICAL: DO NOT USE generate_long_form_video
This is an AUDIO-ONLY task. Do NOT call any video tools.

## MANDATORY TOOL SEQUENCE
1. Read the script/brief and plan the audio
2. Generate the audio using generate_text_to_speech(text, voice) for narration, generate_music(prompt) for background music, or generate_sound_effect(description) for effects
3. Enhance with mixing tools if available
4. Review and iterate on quality
5. submit_final_answer with the audio file path

OUTPUT to: {output_dir}/
Save as .mp3 or .wav"#,
            title = input.title,
            brief = input.brief,
            output_dir = format!("outputs/agentic_{}", input.delivery_id),
        )
    }

    fn full_stack_prompt(input: &ServiceInput) -> String {
        let url = input.source_url.as_deref().unwrap_or("");
        format!(
            r#"## SERVICE: Full Stack Agency Production
GOAL: Create a comprehensive production package (video + thumbnail + audio).

Source: {url}
Title: {title}
Brief: {brief}
Style: {style}

## ⚠️ CRITICAL: DO NOT USE generate_long_form_video
That tool delegates to another agent. For a full-stack deliverable you MUST call the rendering tools directly for each component.

## MANDATORY WORKFLOW — produce ALL three deliverables:

### 1. MAIN VIDEO
- generate_video_script(topic, duration, style, tone) — plan
- Render using blender_generate_scene_type(prompt, params) for the main visual
- Optionally use manim_execute_script for data/technical visuals and merge with merge_videos
- add_voiceover_to_video(video_path, script) or generate_text_to_speech(text, voice) — narrate
- review_video — check quality, iterate

### 2. THUMBNAIL
- Use create_thumbnail_hd or generate_image — create a click-optimized image

### 3. AUDIO
- Use generate_text_to_speech, generate_music, or a mix

### Final step
- submit_final_answer with all 3 output paths

OUTPUT to: {output_dir}/
Save main_video.mp4, thumbnail.png, audio.mp3"#,
            url = url,
            title = input.title,
            brief = input.brief,
            style = input.style,
            output_dir = format!("outputs/agentic_{}", input.delivery_id),
        )
    }

    fn business_explainer_prompt(input: &ServiceInput) -> String {
        let url = input.source_url.as_deref().unwrap_or("");
        format!(
            r#"## SERVICE: Business Explainer Video
GOAL: Create a professional narrated business explainer video (target ~{duration_seconds}s — can be longer if the content requires it).

Source: {url}
Title: {title}
Brief: {brief}
Style: {style}

## ⚠️ CRITICAL: DO NOT USE generate_long_form_video
That tool delegates to another agent. Call rendering tools directly.

## MANDATORY TOOL SEQUENCE
1. generate_video_script(topic, duration, style, tone) — plan the narrative structure
2. Render the main visual using blender_generate_scene_type(prompt, params) — for 3D scenes, title cards, product mockups, abstract backgrounds, and branded visuals
3. Use manim_execute_script(description, ...) — for data visualizations, charts, diagrams, flowcharts, timelines, and any analytical/technical content. Render these as separate clips.
4. Use merge_videos to combine all rendered clips into a single cohesive video
5. add_voiceover_to_video(video_path, script) or generate_text_to_speech(text, voice) — professional narration
6. review_video(video_path_or_url) — check quality, fix issues, iterate
7. submit_final_answer(summary, output_files=[path]) — only after review passes

OUTPUT to: {output_dir}/
Save your final video as .mp4 (landscape 16:9 recommended)"#,
            url = url,
            title = input.title,
            brief = input.brief,
            style = input.style,
            duration_seconds = input.duration_seconds as i32,
            output_dir = format!("outputs/agentic_{}", input.delivery_id),
        )
    }

    fn build_agent_prompt(service_type: ServiceType, input: &ServiceInput, output_dir: &str) -> String {
        match service_type {
            ServiceType::LandingPage => Self::landing_page_prompt(input),
            ServiceType::ProductMockup => Self::product_mockup_prompt(input),
            ServiceType::Thumbnails => Self::thumbnail_prompt(input),
            ServiceType::Education => Self::education_prompt(input),
            ServiceType::Clipping => Self::clipping_prompt(input),
            ServiceType::VoiceAudio => Self::voice_audio_prompt(input),
            ServiceType::FullStack => Self::full_stack_prompt(input),
            ServiceType::BusinessExplainer => Self::business_explainer_prompt(input),
            ServiceType::ManimExplainer => Self::manim_explainer_prompt(input),
            ServiceType::WhiteboardAnimation => Self::whiteboard_animation_prompt(input),
            ServiceType::KineticTypography => Self::kinetic_typography_prompt(input),
            ServiceType::AnimatedInfographic => Self::animated_infographic_prompt(input),
            ServiceType::AlgorithmViz => Self::algorithm_viz_prompt(input),
            ServiceType::InvestorPitch => Self::investor_pitch_prompt(input),
            ServiceType::YearInReview => Self::year_in_review_prompt(input),
            ServiceType::IsometricExplainer => Self::isometric_explainer_prompt(input),
        }
    }

    // ── NEW MANIM-ONLY SERVICE PROMPTS ─────────────────────────────────────

    fn manim_explainer_prompt(input: &ServiceInput) -> String {
        let out = format!("outputs/agentic_{}", input.delivery_id);
        format!(
            r#"## SERVICE: Manim Explainer Video
GOAL: target ~{duration}s narrated animated explainer video — can be longer if the content requires it. Use ONLY Manim.

Topic: {brief}
Title: {title}
Style: {style}

## ⚠️ CRITICAL: DO NOT USE blender_generate_scene_type or any 3D/Blender tools.
Use ONLY manim_execute_script for ALL visual content — every render call uploads to R2 automatically.

## ⚠️ CRITICAL: DO NOT USE generate_long_form_video
That tool delegates to another agent and will NOT produce the correct output.

## MANDATORY TOOL SEQUENCE
1. generate_video_script(topic="{brief}", duration={duration}, style="{style}", tone="professional")
2. Render ALL scenes using manim_execute_script(description=..., quality="h") — for title cards, text animations, diagrams, charts, and ALL visual content. Each call returns a Cloud URL.
3. Use merge_videos if multiple manim clips need combining
4. add_voiceover_to_video(video_path="{out}/output.mp4", script="from script above")
5. review_video(video_path_or_url="{out}/output.mp4")
6. submit_final_answer(summary="manim explainer {title}", output_files=["{out}/output.mp4"])
"#,
            duration = input.duration_seconds,
            brief = input.brief,
            title = input.title,
            style = input.style,
            out = out,
        )
    }

    fn whiteboard_animation_prompt(input: &ServiceInput) -> String {
        let out = format!("outputs/agentic_{}", input.delivery_id);
        format!(
            r#"## SERVICE: Whiteboard Animation Video
GOAL: target ~{duration}s narrated whiteboard-style explainer video — can be longer if the content requires it. Use ONLY Manim.

Topic: {brief}
Title: {title}
Style: whiteboard, hand-drawn sketch, marker-on-board

## ⚠️ CRITICAL: DO NOT USE blender_generate_scene_type or any 3D/Blender tools.
Use ONLY manim_execute_script for ALL visual content — every render call uploads to R2 automatically.

## ⚠️ CRITICAL: DO NOT USE generate_long_form_video

## WHITEBOARD STYLE INSTRUCTIONS
- Use Write() animation for text and drawings to simulate hand-sketching
- Use Create() for shapes to reveal them stroke-by-stroke
- Use a white/off-white background (#F5F0E8 or similar) with dark marker-style strokes (#1A1A1A)
- Keep visuals simple: stick figures, hand-drawn shapes, arrows, boxes
- Text should appear as if being hand-written (use Write with reverse=False)
- Pace the narration to match the drawing speed
- See manim_codegen: use background="light" for whiteboard look

## MANDATORY TOOL SEQUENCE
1. generate_video_script(topic="{brief}", duration={duration}, style="whiteboard hand-drawn", tone="friendly educational")
2. Render ALL scenes using manim_execute_script(description=..., quality="h", background="light") — every visual must be Manim-generated stroke-by-stroke. Each call returns a Cloud URL.
3. Use merge_videos if multiple clips need combining
4. add_voiceover_to_video(video_path="{out}/output.mp4", script="from script above")
5. review_video(video_path_or_url="{out}/output.mp4")
6. submit_final_answer(summary="whiteboard animation {title}", output_files=["{out}/output.mp4"])
"#,
            duration = input.duration_seconds,
            brief = input.brief,
            title = input.title,
            out = out,
        )
    }

    fn kinetic_typography_prompt(input: &ServiceInput) -> String {
        let out = format!("outputs/agentic_{}", input.delivery_id);
        format!(
            r#"## SERVICE: Kinetic Typography Video
GOAL: target ~{duration}s kinetic typography video — can be longer if the content requires it. Use ONLY Manim.

Topic: {brief}
Title: {title}
Style: dynamic text animation, word-by-word reveal, kinetic type

## ⚠️ CRITICAL: DO NOT USE blender_generate_scene_type or any 3D/Blender tools.
Use ONLY manim_execute_script for ALL visual content — every render call uploads to R2 automatically.

## ⚠️ CRITICAL: DO NOT USE generate_long_form_video

## KINETIC TYPOGRAPHY INSTRUCTIONS
- Use animated text as the primary visual element throughout the video
- Words should appear, scale, move, and transform in sync with the narration
- Use Write() for text reveals, Transform() for morphing between phrases
- Vary font sizes and weights for emphasis on key words
- Use color changes to highlight important concepts
- Background should be minimal (solid color or subtle gradient) — text IS the visual
- See manim_codegen: use background="dark" or "light" as appropriate

## MANDATORY TOOL SEQUENCE
1. generate_video_script(topic="{brief}", duration={duration}, style="kinetic typography", tone="dynamic energetic")
2. Render ALL scenes using manim_execute_script(description=..., quality="h") — text animations only, no diagrams or charts. Each call returns a Cloud URL.
3. Use merge_videos if multiple clips need combining
4. add_voiceover_to_video(video_path="{out}/output.mp4", script="from script above")
5. review_video(video_path_or_url="{out}/output.mp4")
6. submit_final_answer(summary="kinetic typography {title}", output_files=["{out}/output.mp4"])
"#,
            duration = input.duration_seconds,
            brief = input.brief,
            title = input.title,
            out = out,
        )
    }

    fn animated_infographic_prompt(input: &ServiceInput) -> String {
        let out = format!("outputs/agentic_{}", input.delivery_id);
        format!(
            r#"## SERVICE: Animated Infographic Video
GOAL: target ~{duration}s animated data infographic video — can be longer if the content requires it. Use ONLY Manim.

Topic: {brief}
Title: {title}
Style: data-driven infographic, animated charts, counters

## ⚠️ CRITICAL: DO NOT USE blender_generate_scene_type or any 3D/Blender tools.
Use ONLY manim_execute_script for ALL visual content — every render call uploads to R2 automatically.

## ⚠️ CRITICAL: DO NOT USE generate_long_form_video

## INFOGRAPHIC INSTRUCTIONS
- Use BarChart, PieChart, or other Manim chart types to visualize data
- Animate data counters with ValueTracker and DecimalNumber
- Use Create() to reveal chart elements progressively
- Highlight key data points with color and labels
- Include title cards and summary cards as text scenes
- Transition between charts with clean fades or slides
- See manim_codegen: use background="dark" or "light" as appropriate

## MANDATORY TOOL SEQUENCE
1. generate_video_script(topic="{brief}", duration={duration}, style="infographic animated charts", tone="professional data-driven")
2. Render ALL scenes using manim_execute_script(description=..., quality="h") — charts, counters, data viz only. Each call returns a Cloud URL.
3. Use merge_videos if multiple clips need combining
4. add_voiceover_to_video(video_path="{out}/output.mp4", script="from script above")
5. review_video(video_path_or_url="{out}/output.mp4")
6. submit_final_answer(summary="animated infographic {title}", output_files=["{out}/output.mp4"])
"#,
            duration = input.duration_seconds,
            brief = input.brief,
            title = input.title,
            out = out,
        )
    }

    fn algorithm_viz_prompt(input: &ServiceInput) -> String {
        let out = format!("outputs/agentic_{}", input.delivery_id);
        format!(
            r#"## SERVICE: Algorithm Visualization Video
GOAL: target ~{duration}s algorithm visualization video — can be longer if the content requires it. Use ONLY Manim.

Topic: {brief}
Title: {title}
Style: algorithm visualization, code execution, data structures

## ⚠️ CRITICAL: DO NOT USE blender_generate_scene_type or any 3D/Blender tools.
Use ONLY manim_execute_script for ALL visual content — every render call uploads to R2 automatically.

## ⚠️ CRITICAL: DO NOT USE generate_long_form_video

## ALGORITHM VIZ INSTRUCTIONS
- Visualize data structures (arrays, trees, graphs, linked lists) using Manim shapes
- Show step-by-step algorithm execution with animated pointers and highlights
- Use value trackers (ValueTracker) for counters and indices
- Color-code elements: unsorted=WHITE, comparing=YELLOW, sorted=GREEN, pivot=BLUE
- Show code snippets alongside the visualization when relevant
- Animate swaps, comparisons, and pointer movements
- Include labeled axes or coordinates when needed

## MANDATORY TOOL SEQUENCE
1. generate_video_script(topic="{brief}", duration={duration}, style="algorithm visualization", tone="educational technical")
2. Render ALL scenes using manim_execute_script(description=..., quality="h") — algorithm visuals, data structures, code animations. Each call returns a Cloud URL.
3. Use merge_videos if multiple clips need combining
4. add_voiceover_to_video(video_path="{out}/output.mp4", script="from script above")
5. review_video(video_path_or_url="{out}/output.mp4")
6. submit_final_answer(summary="algorithm visualization {title}", output_files=["{out}/output.mp4"])
"#,
            duration = input.duration_seconds,
            brief = input.brief,
            title = input.title,
            out = out,
        )
    }

    fn investor_pitch_prompt(input: &ServiceInput) -> String {
        let out = format!("outputs/agentic_{}", input.delivery_id);
        format!(
            r#"## SERVICE: Investor Pitch Video
GOAL: target ~{duration}s professional investor pitch video — can be longer if the content requires it. Use ONLY Manim + voiceover.

Topic: {brief}
Title: {title}
Style: professional investor pitch, clean motion graphics, brand presentation

## ⚠️ CRITICAL: DO NOT USE blender_generate_scene_type or any 3D/Blender tools.
Use ONLY manim_execute_script for ALL visual content — every render call uploads to R2 automatically.

## ⚠️ CRITICAL: DO NOT USE generate_long_form_video

## INVESTOR PITCH INSTRUCTIONS
- Create a professional pitch deck-style video: problem → solution → market → traction → team → ask
- Use clean title cards and animated text for each section
- Include simple data charts for market size and traction metrics
- Animate key numbers (revenue, users, growth %) with ValueTracker counters
- Use a professional color palette (blues, grays, accent color)
- Background music should be subtle and professional
- Voiceover should be energetic and confident

## MANDATORY TOOL SEQUENCE
1. generate_video_script(topic="{brief}", duration={duration}, style="investor pitch deck", tone="professional confident persuasive")
2. Render ALL scenes using manim_execute_script(description=..., quality="h") — pitch deck slides, charts, animated metrics. Each call returns a Cloud URL.
3. Use merge_videos if multiple clips need combining
4. add_voiceover_to_video(video_path="{out}/output.mp4", script="from script above")
5. review_video(video_path_or_url="{out}/output.mp4")
6. submit_final_answer(summary="investor pitch {title}", output_files=["{out}/output.mp4"])
"#,
            duration = input.duration_seconds,
            brief = input.brief,
            title = input.title,
            out = out,
        )
    }

    fn year_in_review_prompt(input: &ServiceInput) -> String {
        let out = format!("outputs/agentic_{}", input.delivery_id);
        format!(
            r#"## SERVICE: Year-in-Review / Wrapped Video
GOAL: target ~{duration}s personalized year-in-review recap video — can be longer if the content requires it. Use ONLY Manim.

Topic: {brief}
Title: {title}
Style: wrapped-style recap, data-driven highlights, personalized

## ⚠️ CRITICAL: DO NOT USE blender_generate_scene_type or any 3D/Blender tools.
Use ONLY manim_execute_script for ALL visual content — every render call uploads to R2 automatically.

## ⚠️ CRITICAL: DO NOT USE generate_long_form_video

## YEAR-IN-REVIEW INSTRUCTIONS
- Create a Spotify Wrapped-style recap video showing key statistics and highlights
- Use bold typography and vibrant colors for each stat reveal
- Animate counters to count up to final numbers (ValueTracker + DecimalNumber)
- Use bar charts or circular progress indicators for comparisons
- Each stat gets its own scene with dramatic reveal
- Include a title card and closing summary
- Fast-paced editing between stats
- See manim_codegen: use background="dark" for wrapped-style aesthetic

## MANDATORY TOOL SEQUENCE
1. generate_video_script(topic="{brief}", duration={duration}, style="year-in-review wrapped", tone="energetic celebratory")
2. Render ALL scenes using manim_execute_script(description=..., quality="h") — stat reveals, counters, charts. Each call returns a Cloud URL.
3. Use merge_videos if multiple clips need combining
4. add_voiceover_to_video(video_path="{out}/output.mp4", script="from script above")
5. review_video(video_path_or_url="{out}/output.mp4")
6. submit_final_answer(summary="year in review {title}", output_files=["{out}/output.mp4"])
"#,
            duration = input.duration_seconds,
            brief = input.brief,
            title = input.title,
            out = out,
        )
    }

    fn isometric_explainer_prompt(input: &ServiceInput) -> String {
        let out = format!("outputs/agentic_{}", input.delivery_id);
        format!(
            r#"## SERVICE: Isometric Explainer Video
GOAL: target ~{duration}s isometric 3D explainer video — can be longer if the content requires it. Use ONLY Manim.

Topic: {brief}
Title: {title}
Style: isometric 3D perspective, angled view, clean motion graphics

## ⚠️ CRITICAL: DO NOT USE blender_generate_scene_type or any 3D/Blender tools.
Use ONLY manim_execute_script for ALL visual content — every render call uploads to R2 automatically.

## ⚠️ CRITICAL: DO NOT USE generate_long_form_video

## ISOMETRIC EXPLAINER INSTRUCTIONS
- Use Manim's ThreeDScene with isometric camera angle (phi=60, theta=-45, gamma=0 or similar)
- Create 3D geometric shapes (cubes, cylinders, arrows) to represent concepts
- Use ThreeDaxes for 3D coordinate reference when needed
- Animate objects rising from the "floor" (build-up reveal)
- Use perspective lines and isometric grids for spatial context
- Color-code different elements/layers of the concept
- Keep the isometric view consistent throughout
- See manim_codegen: use ThreeDScene for isometric perspective rendering
- Set background="dark" for modern tech aesthetic or "light" for clean business look

## MANDATORY TOOL SEQUENCE
1. generate_video_script(topic="{brief}", duration={duration}, style="isometric 3D explainer", tone="professional modern")
2. Render ALL scenes using manim_execute_script(description=..., quality="h") — isometric 3D scenes, animated objects. Each call returns a Cloud URL.
3. Use merge_videos if multiple clips need combining
4. add_voiceover_to_video(video_path="{out}/output.mp4", script="from script above")
5. review_video(video_path_or_url="{out}/output.mp4")
6. submit_final_answer(summary="isometric explainer {title}", output_files=["{out}/output.mp4"])
"#,
            duration = input.duration_seconds,
            brief = input.brief,
            title = input.title,
            out = out,
        )
    }
}

// NOTE: the fire-and-forget spawn path was removed (Aug 2026).
// AgenticServicePipeline::start() now only enqueues (status='queued'), and
// src/services/pipeline_worker.rs owns execution via durable lease claims.

/// Plan-then-execute pre-flight: one fast-chain LLM call that converts the
/// brief + service profile into an explicit numbered stage plan. Non-fatal on
/// failure — the reactive loop still works without a plan.
async fn generate_execution_plan(
    state: &Arc<AppState>,
    service_type: ServiceType,
    input: &ServiceInput,
) -> Result<String, String> {
    let source_hint = match input.source_url.as_deref() {
        Some(url) if !url.is_empty() => format!("\nSource video/website: {url}"),
        _ => String::new(),
    };
    let tool_hints = match service_type {
        ServiceType::Clipping => "generate_clip_compilation (downloads + extracts + captions clips), kick_post_processing happens automatically; final artifacts are clip URLs",
        _ => "generate_video_script, blender_generate_scene_type, manim_execute_script, merge_videos, add_voiceover_to_video, review_video, submit_final_answer",
    };

    let prompt = format!(
        "You are a render-pipeline planner for the '{service}' Managed Campaign service.\n\
         Title: {title}\nBrief: {brief}\nTarget duration: {duration}s\nStyle: {style}{source}\n\n\
         Available pipeline tools: {tools}\n\n\
         Write a concise numbered execution plan (5-9 steps) to produce this video.\n\
         Each step must be ONE line in exactly this format:\n\
         STEP <n>: <tool_name>(<key args>) -> <expected artifact>\n\n\
         Rules:\n\
         - The FINAL step must yield outputs/agentic_{delivery_id}/output.mp4 (or clip URLs for clipping)\n\
         - Include a review_video QA step before submit_final_answer\n\
         - No prose, no markdown headers — ONLY the STEP lines\n",
        service = service_type.as_str(),
        title = input.title,
        brief = input.brief.chars().take(1200).collect::<String>(),
        duration = input.duration_seconds as i32,
        style = input.style,
        source = source_hint,
        tools = tool_hints,
        delivery_id = input.delivery_id,
    );

    crate::llm_utils::generate_text_fast(
        state.ollama_client.as_ref(),
        state.deepseek_client.as_ref(),
        state.gemini_client.as_ref(),
        &prompt,
    )
    .await
    .map_err(|e| e.to_string())
    .map(|t| {
        t.lines()
            .filter(|l| l.trim_start().to_uppercase().starts_with("STEP"))
            .collect::<Vec<_>>()
            .join("\n")
    })
}

// ── Targeted repair scope (QA-failure triage) ────────────────────────────────

enum RepairScope {
    /// Nothing reusable — full pipeline re-run (previous behavior).
    Full,
    /// Re-render only the flagged aspect; reuse surviving artifacts.
    Partial { instructions: String },
}

/// Local files the previous attempt left in its output dir — candidates for
/// reuse during partial repair. Cloud-only outputs (clipping) return empty.
fn list_local_artifacts(output_dir: &str) -> Vec<String> {
    let mut out = Vec::new();
    if let Ok(entries) = std::fs::read_dir(output_dir) {
        for e in entries.flatten() {
            if let Ok(name) = e.file_name().into_string() {
                if name.ends_with(".mp4") || name.ends_with(".png") || name.ends_with(".wav") {
                    out.push(format!("{output_dir}/{name}"));
                }
            }
        }
    }
    out.truncate(20);
    out
}

/// One fast-chain call: given QA feedback + surviving artifacts, decide
/// full-rebuild vs targeted repair with concrete instructions.
/// Defaults to Full on any ambiguity — correctness over cost.
async fn plan_repair_scope(
    state: &Arc<AppState>,
    feedback: &str,
    artifacts: &[String],
) -> RepairScope {
    if artifacts.is_empty() {
        return RepairScope::Full;
    }
    let prompt = format!(
        "A video render failed QA review.\nFeedback: {}\n\n\
         Surviving artifacts from the attempt:\n{}\n\n\
         Can this be fixed by re-rendering ONLY part of the video and merging with\n\
         the survivors, or does it require a full rebuild?\n\
         Respond with ONLY JSON:\n\
         {{\"strategy\": \"partial\" or \"full\", \"instructions\": \"if partial: exactly what to re-render and how to merge; else empty\"}}\n",
        feedback.chars().take(800).collect::<String>(),
        artifacts.join("\n"),
    );
    match crate::llm_utils::generate_text_fast(
        state.ollama_client.as_ref(),
        state.deepseek_client.as_ref(),
        state.gemini_client.as_ref(),
        &prompt,
    )
    .await
    {
        Ok(text) => {
            // Lenient JSON extraction (same pattern as skill extraction).
            let json_text = text.trim();
            let parsed: serde_json::Value =
                serde_json::from_str(json_text).unwrap_or_else(|_| {
                    regex::Regex::new(r"\{[^{}]*\}")
                        .ok()
                        .and_then(|re| re.find(json_text))
                        .and_then(|m| serde_json::from_str(m.as_str()).ok())
                        .unwrap_or(serde_json::json!({}))
                });
            match parsed["strategy"].as_str() {
                Some("partial") => RepairScope::Partial {
                    instructions: parsed["instructions"]
                        .as_str()
                        .unwrap_or("Re-render only the flagged segment and merge.")
                        .to_string(),
                },
                _ => RepairScope::Full,
            }
        }
        Err(_) => RepairScope::Full,
    }
}

struct LocatedOutput {
    /// The canonical output path — may be a cloud URL or a local path.
    canonical: String,
    /// A local path suitable for QA review (None if only a cloud URL exists).
    review_path: Option<String>,
}

/// Extract the output location from the agent result.
/// Returns a LocatedOutput with a canonical path (cloud URL preferred) and
/// a review path (local file for QA review).
fn is_video_url(url: &str) -> bool {
    url.contains(".mp4") || url.contains(".webm") || url.contains(".mov") || url.contains(".avi")
}

fn is_image_url(url: &str) -> bool {
    url.contains(".jpg") || url.contains(".jpeg") || url.contains(".png") || url.contains(".gif") || url.contains(".webp")
}

fn locate_output_from_result(agent_result: &Result<String, String>, output_dir: &str) -> Option<LocatedOutput> {
    let text = match agent_result {
        Ok(t) => t.as_str(),
        Err(t) => t.as_str(),
    };

    tracing::debug!(len = text.len(), output_dir, "locate_output_from_result");

    let mut cloud_url: Option<String> = None;
    let mut local_path: Option<String> = None;

    for line in text.lines() {
        let trimmed = line.trim();
        // Cloud URL via "📤 Cloud URL:" or "Cloud URL:"
        if let Some(url) = trimmed.strip_prefix("📤 Cloud URL: ")
            .or_else(|| trimmed.strip_prefix("Cloud URL: "))
        {
            let url = url.trim().to_string();
            let should_replace = match &cloud_url {
                None => true,
                Some(current) => is_image_url(current) && is_video_url(&url),
            };
            if should_replace {
                cloud_url = Some(url);
            }
        }
        // Raw https:// URL
        if trimmed.starts_with("https://") && (trimmed.contains(".mp4") || trimmed.contains(".png") || trimmed.contains(".mp3") || trimmed.contains(".webm") || trimmed.contains(".jpg")) {
            if cloud_url.is_none() {
                cloud_url = Some(trimmed.to_string());
            } else if let Some(ref current) = cloud_url {
                if is_image_url(current) && is_video_url(trimmed) {
                    cloud_url = Some(trimmed.to_string());
                }
            }
        }
        // Local file path that exists
        if trimmed.contains(output_dir) || trimmed.contains(".mp4") || trimmed.contains(".png") || trimmed.contains(".mp3") || trimmed.contains(".wav") || trimmed.contains(".webm") {
            let path = trimmed
                .trim_start_matches('`')
                .trim_end_matches('`')
                .trim_start_matches("**")
                .trim_end_matches("**")
                .trim();
            if std::path::Path::new(path).exists() && local_path.is_none() {
                local_path = Some(path.to_string());
            }
        }
    }

    // Fallback: scan output_dir only (not global outputs/ — avoids picking up stale files
    // from other deliveries). Only consider files modified within the last hour.
    if local_path.is_none() {
        let cutoff = std::time::SystemTime::now()
            .duration_since(std::time::UNIX_EPOCH)
            .ok()
            .and_then(|n| n.checked_sub(std::time::Duration::from_secs(3600)))
            .and_then(|past| std::time::UNIX_EPOCH.checked_add(past));
        if let Ok(entries) = std::fs::read_dir(output_dir) {
            let mut candidates: Vec<_> = entries
                .flatten()
                .filter_map(|e| {
                    let p = e.path();
                    let ext = p.extension()?.to_str()?;
                    if !matches!(ext, "mp4" | "webm" | "mov" | "avi" | "mkv" | "png" | "jpg" | "jpeg" | "gif" | "mp3" | "wav" | "aac") {
                        return None;
                    }
                    // Only accept files modified within the last hour
                    if let Some(ref cutoff) = cutoff {
                        if let Ok(meta) = std::fs::metadata(&p) {
                            if let Ok(modified) = meta.modified() {
                                if modified < *cutoff {
                                    return None;
                                }
                            }
                        }
                    }
                    let priority = if matches!(ext, "mp4" | "webm" | "mov" | "avi" | "mkv") { 2 }
                        else if matches!(ext, "png" | "jpg" | "jpeg" | "gif") { 1 }
                        else { 0 };
                    Some((p, priority))
                })
                .collect();
            if !candidates.is_empty() {
                candidates.sort_by(|a, b| b.1.cmp(&a.1).then_with(|| {
                    std::fs::metadata(&b.0).ok()
                        .and_then(|m| m.modified().ok())
                        .cmp(&std::fs::metadata(&a.0).ok().and_then(|m| m.modified().ok()))
                }));
                if let Some((p, _)) = candidates.first() {
                    let p = p.to_string_lossy().to_string();
                    tracing::warn!(path = %p, dir = %output_dir, "locate_output_from_result found local file via dir scan");
                    local_path = Some(p);
                }
            }
        }
    }

    // Prefer cloud URL as canonical, fall back to local path
    let canonical = cloud_url.clone().or_else(|| local_path.clone())?;

    tracing::info!(
        canonical = %canonical,
        has_review_path = local_path.is_some(),
        "locate_output_from_result: found output"
    );

    Some(LocatedOutput {
        canonical,
        review_path: local_path,
    })
}

pub fn normalize_to_service_type(s: &str) -> ServiceType {
    ServiceType::from_normalized(s)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_service_type_from_normalized() {
        assert_eq!(ServiceType::from_normalized("landing_page"), ServiceType::LandingPage);
        assert_eq!(ServiceType::from_normalized("landing"), ServiceType::LandingPage);
        assert_eq!(ServiceType::from_normalized("product_mockup"), ServiceType::ProductMockup);
        assert_eq!(ServiceType::from_normalized("product_explainer"), ServiceType::ProductMockup);
        assert_eq!(ServiceType::from_normalized("thumbnails"), ServiceType::Thumbnails);
        assert_eq!(ServiceType::from_normalized("thumbnail"), ServiceType::Thumbnails);
        assert_eq!(ServiceType::from_normalized("education"), ServiceType::Education);
        assert_eq!(ServiceType::from_normalized("clipping"), ServiceType::Clipping);
        assert_eq!(ServiceType::from_normalized("clip"), ServiceType::Clipping);
        assert_eq!(ServiceType::from_normalized("voice_audio"), ServiceType::VoiceAudio);
        assert_eq!(ServiceType::from_normalized("voice"), ServiceType::VoiceAudio);
        assert_eq!(ServiceType::from_normalized("podcast"), ServiceType::VoiceAudio);
        assert_eq!(ServiceType::from_normalized("full_stack"), ServiceType::FullStack);
        assert_eq!(ServiceType::from_normalized("business_explainer"), ServiceType::BusinessExplainer);
        assert_eq!(ServiceType::from_normalized("saas_explainer"), ServiceType::BusinessExplainer);
        assert_eq!(ServiceType::from_normalized("unknown"), ServiceType::LandingPage);
    }

    #[test]
    fn test_service_type_as_str() {
        assert_eq!(ServiceType::LandingPage.as_str(), "landing_page");
        assert_eq!(ServiceType::ProductMockup.as_str(), "product_mockup");
        assert_eq!(ServiceType::Thumbnails.as_str(), "thumbnails");
        assert_eq!(ServiceType::Education.as_str(), "education");
        assert_eq!(ServiceType::Clipping.as_str(), "clipping");
        assert_eq!(ServiceType::VoiceAudio.as_str(), "voice_audio");
        assert_eq!(ServiceType::FullStack.as_str(), "full_stack");
        assert_eq!(ServiceType::BusinessExplainer.as_str(), "business_explainer");
        assert_eq!(ServiceType::ManimExplainer.as_str(), "manim_explainer");
        assert_eq!(ServiceType::WhiteboardAnimation.as_str(), "whiteboard_animation");
        assert_eq!(ServiceType::KineticTypography.as_str(), "kinetic_typography");
        assert_eq!(ServiceType::AnimatedInfographic.as_str(), "animated_infographic");
        assert_eq!(ServiceType::AlgorithmViz.as_str(), "algorithm_viz");
        assert_eq!(ServiceType::InvestorPitch.as_str(), "investor_pitch");
        assert_eq!(ServiceType::YearInReview.as_str(), "year_in_review");
        assert_eq!(ServiceType::IsometricExplainer.as_str(), "isometric_explainer");
    }

    #[test]
    fn test_service_type_round_trip() {
        let variants = [
            ServiceType::LandingPage,
            ServiceType::ProductMockup,
            ServiceType::Thumbnails,
            ServiceType::Education,
            ServiceType::Clipping,
            ServiceType::VoiceAudio,
            ServiceType::FullStack,
            ServiceType::BusinessExplainer,
            ServiceType::ManimExplainer,
            ServiceType::WhiteboardAnimation,
            ServiceType::KineticTypography,
            ServiceType::AnimatedInfographic,
            ServiceType::AlgorithmViz,
            ServiceType::InvestorPitch,
            ServiceType::YearInReview,
            ServiceType::IsometricExplainer,
        ];
        for v in variants {
            let s = v.as_str();
            let back = ServiceType::from_normalized(s);
            assert_eq!(v, back, "round-trip failed for {:?} -> {} -> {:?}", v, s, back);
        }
    }

    #[test]
    fn test_service_type_default_style_non_empty() {
        let variants = [
            ServiceType::LandingPage,
            ServiceType::ProductMockup,
            ServiceType::Thumbnails,
            ServiceType::Education,
            ServiceType::Clipping,
            ServiceType::VoiceAudio,
            ServiceType::FullStack,
            ServiceType::BusinessExplainer,
            ServiceType::ManimExplainer,
            ServiceType::WhiteboardAnimation,
            ServiceType::KineticTypography,
            ServiceType::AnimatedInfographic,
            ServiceType::AlgorithmViz,
            ServiceType::InvestorPitch,
            ServiceType::YearInReview,
            ServiceType::IsometricExplainer,
        ];
        for v in variants {
            let style = v.default_style();
            assert!(!style.is_empty(), "empty style for {:?}", v);
        }
    }

    #[test]
    fn test_service_type_duration_positive() {
        let variants = [
            ServiceType::LandingPage,
            ServiceType::ProductMockup,
            ServiceType::Thumbnails,
            ServiceType::Education,
            ServiceType::Clipping,
            ServiceType::VoiceAudio,
            ServiceType::FullStack,
            ServiceType::BusinessExplainer,
        ];
        for v in variants {
            let d = v.default_duration_seconds();
            assert!(d >= 0.0, "negative duration {} for {:?}", d, v);
        }
    }

    #[test]
    fn test_service_type_expects_video() {
        assert!(ServiceType::LandingPage.expects_video());
        assert!(ServiceType::ProductMockup.expects_video());
        assert!(ServiceType::Education.expects_video());
        assert!(ServiceType::Clipping.expects_video());
        assert!(ServiceType::BusinessExplainer.expects_video());
        assert!(!ServiceType::Thumbnails.expects_video());
        assert!(!ServiceType::VoiceAudio.expects_video());
        assert!(!ServiceType::FullStack.expects_video());
    }

    #[test]
    fn test_old_ugc_deleted() {
        assert!(!matches!(ServiceType::from_normalized("ugc"), ServiceType::BusinessExplainer));
        assert_ne!(ServiceType::from_normalized("ugc").as_str(), "ugc");
    }
}
