use crate::agent::simple_gemini_agent::SimpleGeminiAgent;
use crate::agent::tool_executor::{execute_tool_gemini_with_context, ToolExecutionContext};
use crate::render_review::{review_render, ReviewResult};
use crate::services::workflow_runtime::{NewWorkflow, WorkflowRuntime, WorkflowStatus};
use crate::AppState;
use serde::{Deserialize, Serialize};
use serde_json::{json, Value};
use std::sync::Arc;
use uuid::Uuid;

pub const AGENT_MAX_RETRIES: i32 = 5;

#[derive(Debug, Clone, Copy, Serialize, Deserialize)]
pub enum ServiceType {
    LandingPage,
    ProductMockup,
    Thumbnails,
    Education,
    Clipping,
    VoiceAudio,
    FullStack,
}

impl ServiceType {
    pub fn from_normalized(s: &str) -> Self {
        match s {
            "landing_page" | "saas_demo" => Self::LandingPage,
            "product_mockup" => Self::ProductMockup,
            "thumbnails" => Self::Thumbnails,
            "education" => Self::Education,
            "clipping" => Self::Clipping,
            "ugc" | "voice_audio" | "voice" => Self::VoiceAudio,
            "full_stack" | "agency_bundle" | "agency" => Self::FullStack,
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
        }
    }

    pub fn expects_video(&self) -> bool {
        matches!(self, Self::LandingPage | Self::ProductMockup | Self::Education | Self::Clipping)
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
    pub async fn start(
        state: Arc<AppState>,
        service_type: ServiceType,
        input: ServiceInput,
    ) -> Result<Uuid, String> {
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
                }),
                artifact_requirements: json!({
                    "expects_video": service_type.expects_video(),
                    "expects_image": service_type.expects_image(),
                    "expects_audio": service_type.expects_audio(),
                }),
            })
            .await?;

        spawn_agentic_pipeline_run(
            state.clone(),
            workflow_id,
            service_type,
            input,
        );

        Ok(workflow_id)
    }

    async fn run(
        state: Arc<AppState>,
        workflow_id: Uuid,
        service_type: ServiceType,
        input: ServiceInput,
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

        let output_dir = format!("outputs/agentic_{}", input.delivery_id);
        std::fs::create_dir_all(&output_dir).map_err(|e| format!("Failed to create output dir: {e}"))?;

        let agent_prompt = Self::build_agent_prompt(service_type, &input, &output_dir);

        let mut current_prompt = agent_prompt.clone();
        let mut best_output_path: Option<String> = None;
        let mut best_score: i32 = -1;
        let mut best_feedback = String::new();
        let mut retries_used: i32 = 0;

        for attempt in 0..AGENT_MAX_RETRIES {
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

            let agent = SimpleGeminiAgent::new(gemini_client.clone());
            let system_prompt = Self::system_prompt(service_type, &input);

            let tools = crate::ai_tool_selector::select_tools_for_request(
                &format!("{} {}", system_prompt, current_prompt),
                state.nvidia_nim_client.as_ref(),
                Some(gemini_client.as_ref()),
            )
            .await;

            let tool_executor: crate::agent::simple_gemini_agent::GeminiToolExecutor =
                Arc::new(|name, args, ctx| {
                    Box::pin(async move {
                        Value::String(execute_tool_gemini_with_context(name, args, ctx).await)
                    })
                });

            let agent_result = agent
                .execute_with_custom_tools(
                    &current_prompt,
                    &session_id,
                    input.user_id,
                    state.clone(),
                    None,
                    &system_prompt,
                    tools,
                    Some("submit_final_answer"),
                    tool_executor,
                    Some(workflow_id),
                )
                .await;

            let produced_path = locate_output_on_disk(&agent_result, &output_dir);
            retries_used = attempt as i32;

            let template_name = match service_type {
                ServiceType::Thumbnails => "agentic_thumbnail",
                _ => "agentic_service_video",
            };

            let review = if let Some(ref path) = produced_path {
                review_render(
                    &state,
                    path,
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
                best_output_path = produced_path.clone();
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
                current_prompt = format!(
                    "PREVIOUS ATTEMPT FAILED QA REVIEW (score {}/10).\n\
                     Feedback: {}\n\
                     Retry hint: {}\n\n\
                     Apply the feedback above, then run the full pipeline below:\n\n{}",
                    review.score, review.feedback, hint, agent_prompt,
                );
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
            .await;

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
        let r2 = state
            .r2_client
            .as_ref()
            .ok_or("R2 not configured for publishing")?;

        let output_key = format!("agentic_output/{}/{}.mp4", input.delivery_id, input.service_filename());
        let public_url = r2
            .upload_file(output_path, &output_key)
            .await
            .map_err(|e| format!("R2 upload failed: {e}"))?;

        let qa_note: Option<String> = if qa_score < 6 {
            Some(format!("QA final score {} after {} retries: {}", qa_score, retries_used, qa_feedback))
        } else {
            None
        };

        if input.source_table.as_deref() == Some("deliveries") {
            let _ = sqlx::query(
                "UPDATE deliveries SET status = 'completed', output_r2_url = $1, \
                 output_filename = $2, qa_score = $3, qa_note = $4, completed_at = NOW() \
                 WHERE id = $5",
            )
            .bind(&public_url)
            .bind(&output_key)
            .bind(qa_score)
            .bind(qa_note)
            .bind(input.delivery_id)
            .execute(&state.db_pool)
            .await;

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

    fn system_prompt(service_type: ServiceType, input: &ServiceInput) -> String {
        let base = r#"You are VideoSync's AI video production agent. Your job is to create professional media assets by calling tools.

## CRITICAL RULES
- DO call tools to produce actual files — never just describe what you would do
- DO save output files to the specified output directory
- DO review your output before submitting
- DO iterate if the review fails — fix the issue and try again
- use submit_final_answer ONLY when you have a completed, reviewed output

## YOUR CAPABILITIES
You have access to all of these tools:
1. Web Tools: read_website_content(url), fetch_website_image(url)
2. Image Generation: generate_image(prompt, aspect_ratio, image_size)
3. BlenderMCP Tools (27 tools for 3D scenes, UI mockups, title cards, charts, data viz, LaTeX, manim animations, lower thirds, text animations, and more)
4. Audio Tools: generate_text_to_speech, generate_music, generate_sound_effect, add_voiceover_to_video, add_audio
5. Video Editing (320+ FFmpeg tools): trim_video, merge_videos, add_text_overlay, apply_filter, crop, resize, etc.
6. Review Tools: review_video, analyze_video, view_video, run_video_qa

## QUALITY REVIEW - MANDATORY
After producing ANY output:
1. Call review_video or run_video_qa to check quality
2. If the review fails, fix the issue and re-render
3. Only call submit_final_answer when review passes"#;

        let service_specific = match service_type {
            ServiceType::LandingPage => Self::landing_page_prompt(input),
            ServiceType::ProductMockup => Self::product_mockup_prompt(input),
            ServiceType::Thumbnails => Self::thumbnail_prompt(input),
            ServiceType::Education => Self::education_prompt(input),
            ServiceType::Clipping => Self::clipping_prompt(input),
            ServiceType::VoiceAudio => Self::voice_audio_prompt(input),
            ServiceType::FullStack => Self::full_stack_prompt(input),
        };

        format!("{}\n\n{}", base, service_specific)
    }

    fn landing_page_prompt(input: &ServiceInput) -> String {
        let url = input.source_url.as_deref().unwrap_or("");
        format!(
            r#"## SERVICE: SaaS/App Demo Video
GOAL: Create a polished {duration_seconds}s SaaS demo video.

Source: {url}
Title: {title}
Style: {style}

## WHAT TO DO
1. First understand the product by reading the website
2. Plan your creative approach — you have 320+ tools available, use whatever combination you think will produce the best result
3. Generate any reference images you need
4. Produce the video — you decide which tools to use and how to combine them
5. Review your output — check quality, fix issues, iterate
6. Use submit_final_answer when the output meets your standards

OUTPUT to: {output_dir}/
Save your final video with .mp4 extension"#,
            url = url,
            title = input.title,
            style = input.style,
            duration_seconds = input.duration_seconds as i32,
            output_dir = format!("outputs/agentic_{}", input.delivery_id),
        )
    }

    fn product_mockup_prompt(input: &ServiceInput) -> String {
        let url = input.source_url.as_deref().unwrap_or("");
        format!(
            r#"## SERVICE: Product Mockup Video
GOAL: Create an animated {duration_seconds}s product/UI mockup video.

Source: {url}
Title: {title}
Style: {style}

## WHAT TO DO
1. Understand the product (read the website if URL provided, or use the brief)
2. Generate any reference images you might need
3. Plan and produce the mockup video using whatever combination of tools you think works best — device mockups, animations, text reveals, any of the 320+ tools
4. Add audio if it improves the result
5. Review your output and iterate until it's solid
6. submit_final_answer

OUTPUT to: {output_dir}/
Save final video as .mp4"#,
            url = url,
            title = input.title,
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

## WHAT TO DO
1. Understand what the thumbnail is for
2. Generate the image using whatever approach you think will maximize click-through
3. Add overlays, text, or effects if they improve it
4. Review with view_image — if it's not right, regenerate
5. submit_final_answer

OUTPUT to: {output_dir}/
Save as .png or .jpg"#,
            title = input.title,
            brief = input.brief,
            style = input.style,
            output_dir = format!("outputs/agentic_{}", input.delivery_id),
        )
    }

    fn education_prompt(input: &ServiceInput) -> String {
        let url = input.source_url.as_deref().unwrap_or("");
        format!(
            r#"## SERVICE: Education Explainer Video
GOAL: Create a narrated {duration_seconds}s educational explainer video.

Topic: {url}
Title: {title}
Style: {style}

## WHAT TO DO
1. Understand the topic (read the URL content or use the brief)
2. Plan the explanation — decide what visual approach will teach the concept best
3. You have all 320+ tools: 3D scenes, animations, charts, LaTeX equations, diagrams, screen mockups, text animations, and more. Use whatever combination delivers the clearest explanation
4. Add narration to make it professional
5. Review and iterate until the explanation is clear and engaging
6. submit_final_answer

OUTPUT to: {output_dir}/
Save final video as .mp4"#,
            url = url,
            title = input.title,
            style = input.style,
            duration_seconds = input.duration_seconds as i32,
            output_dir = format!("outputs/agentic_{}", input.delivery_id),
        )
    }

    fn clipping_prompt(input: &ServiceInput) -> String {
        format!(
            r#"## SERVICE: Clip Enhancement
GOAL: Extract engaging clips from content and make them shine.

Title: {title}
Brief: {brief}

## WHAT TO DO
1. If a video URL is provided, analyze it to understand the content and find the best moments
2. Extract clips — you decide what length, what moments, what style
3. Enhance each clip however you see fit: captions, color grading, transitions, overlays, effects, speed adjustments, stabilization, sound design — you have 320+ tools
4. Each clip should feel complete and professional
5. Review each clip, iterate on any that fall short
6. submit_final_answer with all clip paths

OUTPUT to: {output_dir}/
Save each clip as clip_N.mp4"#,
            title = input.title,
            brief = input.brief,
            output_dir = format!("outputs/agentic_{}", input.delivery_id),
        )
    }

    fn voice_audio_prompt(input: &ServiceInput) -> String {
        format!(
            r#"## SERVICE: Voice & Audio Production
GOAL: Create professional narration/audio output.

Title: {title}
Brief: {brief}

## WHAT TO DO
1. Read the script/brief and plan the audio
2. Generate the audio — you have text-to-speech, music generation, sound effects, mixing tools, and more
3. Enhance with music, effects, or processing if it improves the result
4. Review and iterate on quality
5. submit_final_answer

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
Style: {style}

## WHAT TO DO
This is a multi-format deliverable. Produce ALL of:

1. A MAIN VIDEO — whatever style and approach you think works best for this product
2. A THUMBNAIL — optimized for click-through on the platform
3. AUDIO highlights or a standalone audio version

You have 320+ tools at your disposal. Use them creatively. Each output should be independently reviewed before you finalize.

OUTPUT to: {output_dir}/
Save main_video.mp4, thumbnail.png, audio.mp3"#,
            url = url,
            title = input.title,
            style = input.style,
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
        }
    }
}

fn spawn_agentic_pipeline_run(
    state: Arc<AppState>,
    workflow_id: Uuid,
    service_type: ServiceType,
    input: ServiceInput,
) {
    let error_state = state.clone();
    let delivery_id = input.delivery_id;
    tokio::spawn(async move {
        if let Err(error) = AgenticServicePipeline::run(state, workflow_id, service_type, input).await {
            tracing::error!("Agentic workflow {} failed: {}", workflow_id, error);
            let runtime = WorkflowRuntime::new(error_state.db_pool.clone());
            let _ = runtime
                .mark_failed(workflow_id, Some("agentic_workflow"), &error, None)
                .await;
            let _ = sqlx::query(
                "UPDATE deliveries SET status = 'failed', error_message = $1 WHERE id = $2",
            )
            .bind(&error)
            .bind(delivery_id)
            .execute(&error_state.db_pool)
            .await;
        }
    });
}

fn locate_output_on_disk(agent_result: &Result<String, String>, output_dir: &str) -> Option<String> {
    let text = match agent_result {
        Ok(t) => t.as_str(),
        Err(t) => t.as_str(),
    };

    for line in text.lines() {
        let trimmed = line.trim();
        if trimmed.contains(output_dir) || trimmed.contains(".mp4") || trimmed.contains(".png") {
            let path = trimmed
                .trim_start_matches('`')
                .trim_end_matches('`')
                .trim();
            if std::path::Path::new(path).exists() {
                return Some(path.to_string());
            }
        }
    }

    if let Ok(entries) = std::fs::read_dir(output_dir) {
        for entry in entries.flatten() {
            let path = entry.path();
            if path.extension().map_or(false, |e| e == "mp4" || e == "png" || e == "jpg") {
                return Some(path.to_string_lossy().to_string());
            }
        }
    }

    None
}

pub fn normalize_to_service_type(s: &str) -> ServiceType {
    ServiceType::from_normalized(s)
}
