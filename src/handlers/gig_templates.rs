// Gig Templates — Fiverr/PPH template info + AI sample video generation
// Accessible to any authenticated (whitelisted) user.
//
// Routes:
//   GET  /gig-templates                        — SSR page (HTML, public — JS handles auth)
//   GET  /api/gig-templates                    — JSON list with samples  (auth)
//   POST /api/gig-templates/:id/generate-sample — spawn sample render    (auth)
//   DELETE /api/gig-samples/:id                — delete a sample         (auth)

use crate::middleware::auth::auth_middleware;
use crate::middleware::clipping_access::clipping_access_middleware;
use crate::models::auth::Claims;
use crate::AppState;
use axum::{
    extract::{Extension, Path},
    http::StatusCode,
    response::{Html, Json},
    routing::{delete, get, post},
    Router,
};
use serde_json::{json, Value};
use sqlx::Row;
use std::sync::Arc;
use uuid::Uuid;

pub fn gig_template_routes() -> Router {
    let public = Router::new().route("/gig-templates", get(gig_templates_page));

    let protected = Router::new()
        .route("/api/gig-templates", get(api_list_gig_templates))
        .route(
            "/api/gig-templates/:id/generate-sample",
            post(api_generate_sample),
        )
        .route("/api/gig-samples/:id", delete(api_delete_sample))
        .layer(axum::middleware::from_fn(clipping_access_middleware))
        .layer(axum::middleware::from_fn(auth_middleware));

    public.merge(protected)
}

// ─── SSR page ────────────────────────────────────────────────────────────────

pub async fn gig_templates_page() -> Html<String> {
    Html(GIG_TEMPLATES_HTML.to_string())
}

// ─── API: list templates + samples ───────────────────────────────────────────

pub async fn api_list_gig_templates(Extension(state): Extension<Arc<AppState>>) -> Json<Value> {
    let templates = sqlx::query(
        "SELECT id, service_type, display_name, tagline, description,
                basic_price, basic_delivery_days, basic_includes,
                standard_price, standard_delivery_days, standard_includes,
                premium_price, premium_delivery_days, premium_includes,
                keywords, gig_titles, sample_prompts, sort_order
         FROM gig_templates ORDER BY sort_order",
    )
    .fetch_all(&state.db_pool)
    .await;

    let templates = match templates {
        Ok(r) => r,
        Err(e) => return Json(json!({"error": format!("DB error: {e}")})),
    };

    let mut result = Vec::new();
    for t in &templates {
        let tid: Uuid = t.get("id");

        let samples = sqlx::query(
            "SELECT id, title, prompt_used, status, output_r2_url, output_filename, error_message, created_at, workflow_id
             FROM gig_sample_videos WHERE template_id = $1 ORDER BY created_at",
        )
        .bind(tid)
        .fetch_all(&state.db_pool)
        .await
        .unwrap_or_default();

        let mut samples_json = Vec::new();
        for s in &samples {
            let refreshed_url = refresh_gig_sample_media_value(
                &state,
                s.try_get::<String, _>("output_r2_url").ok(),
            )
            .await;

            samples_json.push(json!({
                "id":           s.get::<Uuid, _>("id").to_string(),
                "title":        s.get::<String, _>("title"),
                "prompt_used":  s.get::<String, _>("prompt_used"),
                "status":       s.get::<String, _>("status"),
                "r2_url":       refreshed_url,
                "filename":     s.try_get::<String, _>("output_filename").ok(),
                "error":        s.try_get::<String, _>("error_message").ok(),
                "workflow_id":  s.try_get::<Uuid, _>("workflow_id").ok().map(|id| id.to_string()),
                "created_at":   s.get::<chrono::DateTime<chrono::Utc>, _>("created_at").to_rfc3339(),
            }));
        }

        result.push(json!({
            "id":                    tid.to_string(),
            "service_type":          t.get::<String, _>("service_type"),
            "display_name":          t.get::<String, _>("display_name"),
            "tagline":               t.get::<String, _>("tagline"),
            "description":           t.get::<String, _>("description"),
            "basic_price":           t.get::<i32, _>("basic_price"),
            "basic_delivery_days":   t.get::<i32, _>("basic_delivery_days"),
            "basic_includes":        t.get::<String, _>("basic_includes"),
            "standard_price":        t.get::<i32, _>("standard_price"),
            "standard_delivery_days":t.get::<i32, _>("standard_delivery_days"),
            "standard_includes":     t.get::<String, _>("standard_includes"),
            "premium_price":         t.get::<i32, _>("premium_price"),
            "premium_delivery_days": t.get::<i32, _>("premium_delivery_days"),
            "premium_includes":      t.get::<String, _>("premium_includes"),
            "keywords":              t.get::<Value, _>("keywords"),
            "gig_titles":            t.get::<Value, _>("gig_titles"),
            "sample_prompts":        t.get::<Value, _>("sample_prompts"),
            "samples":               samples_json,
        }));
    }

    Json(json!({"templates": result}))
}

// ─── API: generate sample ─────────────────────────────────────────────────────

pub async fn api_generate_sample(
    Path(template_id): Path<Uuid>,
    Extension(state): Extension<Arc<AppState>>,
    Extension(claims): Extension<Claims>,
) -> (StatusCode, Json<Value>) {
    // Load template to get service_type and sample_prompts
    let row = sqlx::query("SELECT service_type, sample_prompts FROM gig_templates WHERE id = $1")
        .bind(template_id)
        .fetch_optional(&state.db_pool)
        .await;

    let row = match row {
        Ok(Some(r)) => r,
        Ok(None) => {
            return (
                StatusCode::NOT_FOUND,
                Json(json!({"error": "Template not found"})),
            )
        }
        Err(e) => {
            return (
                StatusCode::INTERNAL_SERVER_ERROR,
                Json(json!({"error": format!("DB error: {e}")})),
            )
        }
    };

    let service_type: String = row.get("service_type");
    let prompts: Value = row.get("sample_prompts");

    // Count existing samples to pick next prompt (rotate through 5)
    let count: i64 =
        sqlx::query_scalar("SELECT COUNT(*) FROM gig_sample_videos WHERE template_id = $1")
            .bind(template_id)
            .fetch_one(&state.db_pool)
            .await
            .unwrap_or(0);

    if count >= 5 {
        return (
            StatusCode::BAD_REQUEST,
            Json(json!({"error": "Maximum 5 samples per template. Delete one to generate more."})),
        );
    }

    let empty_prompts = vec![];
    let prompts_arr = prompts.as_array().unwrap_or(&empty_prompts);
    let idx = (count as usize) % prompts_arr.len().max(1);
    let prompt = prompts_arr
        .get(idx)
        .and_then(|v| v.as_str())
        .unwrap_or("Professional 3D animation sample")
        .to_string();

    let title = format!("Sample {} — {}", count + 1, &service_type);
    let user_id = claims.sub.parse::<i32>().unwrap_or(0);
    let sample_id = Uuid::new_v4();

    let workflow_runtime = crate::services::WorkflowRuntime::new(state.db_pool.clone());
    let workflow_id = match workflow_runtime
        .create_workflow(crate::services::NewWorkflow {
            idempotency_key: Some(format!("gig-template-sample:{sample_id}")),
            workflow_type: "gig_template_sample_generation".to_string(),
            status: crate::services::WorkflowStatus::Queued,
            session_uuid: None,
            user_id: Some(user_id),
            source_table: Some("gig_sample_videos".to_string()),
            source_record_id: Some(sample_id),
            request_summary: format!("Gig template sample generation for {}", service_type)
                .chars()
                .take(200)
                .collect::<String>(),
            current_step: Some("job_created".to_string()),
            metadata: json!({
                "template_id": template_id,
                "sample_id": sample_id,
                "service_type": service_type.clone(),
                "prompt_preview": prompt.chars().take(240).collect::<String>(),
            }),
            artifact_requirements: json!([
                {
                    "kind": "gig_template_sample",
                    "required": true,
                    "must_be_playable": true
                }
            ]),
        })
        .await
    {
        Ok(id) => id,
        Err(e) => {
            return (
                StatusCode::INTERNAL_SERVER_ERROR,
                Json(json!({"error": format!("Workflow initialization failed: {e}")})),
            )
        }
    };

    let _ = workflow_runtime
        .append_event(
            workflow_id,
            "queued",
            Some("job_created"),
            "Gig template sample request created and waiting for render execution.",
            json!({
                "template_id": template_id,
                "sample_id": sample_id,
            }),
        )
        .await;

    let inserted = sqlx::query(
        "INSERT INTO gig_sample_videos (id, template_id, title, prompt_used, status, workflow_id)
         VALUES ($1, $2, $3, $4, 'pending', $5)",
    )
    .bind(sample_id)
    .bind(template_id)
    .bind(&title)
    .bind(&prompt)
    .bind(workflow_id)
    .execute(&state.db_pool)
    .await;

    if let Err(e) = inserted {
        let _ = workflow_runtime
            .mark_failed(
                workflow_id,
                Some("sample_persistence"),
                &format!("Failed to create gig sample record: {e}"),
                None,
            )
            .await;
        return (
            StatusCode::INTERNAL_SERVER_ERROR,
            Json(json!({"error": format!("DB error: {e}")})),
        );
    }

    // Spawn background render
    let state_clone = state.clone();
    let service_type_clone = service_type.clone();
    let prompt_clone = prompt.clone();
    tokio::spawn(async move {
        run_sample_generation(
            sample_id,
            template_id,
            service_type_clone,
            prompt_clone,
            state_clone,
        )
        .await;
    });

    (
        StatusCode::OK,
        Json(json!({
            "sample_id": sample_id.to_string(),
            "workflow_id": workflow_id.to_string(),
            "prompt": prompt
        })),
    )
}

// ─── API: delete sample ───────────────────────────────────────────────────────

pub async fn api_delete_sample(
    Path(sample_id): Path<Uuid>,
    Extension(state): Extension<Arc<AppState>>,
) -> (StatusCode, Json<Value>) {
    let sample_workflow = sqlx::query_as::<_, (Option<Uuid>, String)>(
        "SELECT workflow_id, status FROM gig_sample_videos WHERE id = $1",
    )
    .bind(sample_id)
    .fetch_optional(&state.db_pool)
    .await
    .ok()
    .flatten();

    if let Some((Some(workflow_id), status)) = sample_workflow {
        if status != "completed" && status != "failed" && status != "cancelled" {
            let workflow_runtime = crate::services::WorkflowRuntime::new(state.db_pool.clone());
            let _ = workflow_runtime
                .mark_cancelled(
                    workflow_id,
                    Some("cancelled"),
                    "Gig template sample workflow was cancelled by the user.",
                )
                .await;
        }
    }

    let result = sqlx::query("DELETE FROM gig_sample_videos WHERE id = $1")
        .bind(sample_id)
        .execute(&state.db_pool)
        .await;

    match result {
        Ok(r) if r.rows_affected() > 0 => (StatusCode::OK, Json(json!({"deleted": true}))),
        Ok(_) => (
            StatusCode::NOT_FOUND,
            Json(json!({"error": "Sample not found"})),
        ),
        Err(e) => (
            StatusCode::INTERNAL_SERVER_ERROR,
            Json(json!({"error": format!("DB error: {e}")})),
        ),
    }
}

async fn refresh_gig_sample_media_value(
    state: &Arc<AppState>,
    existing_url: Option<String>,
) -> Value {
    let Some(existing_url) = existing_url else {
        return Value::Null;
    };

    let refreshed = refresh_gig_sample_presigned_url_from_existing(state, &existing_url)
        .await
        .unwrap_or(existing_url);

    Value::String(refreshed)
}

async fn refresh_gig_sample_presigned_url_from_existing(
    state: &Arc<AppState>,
    existing_url: &str,
) -> Option<String> {
    let r2 = state.r2_client.as_ref()?;
    let key = extract_r2_object_key_from_url(existing_url, &r2.bucket)?;

    if !r2.exists(&key).await {
        tracing::warn!(
            key = %key,
            "R2 object referenced by gig sample media URL does not exist"
        );
        return None;
    }

    match r2.presign_get(&key, 7 * 24 * 3600).await {
        Ok(url) => Some(url),
        Err(error) => {
            tracing::warn!(
                key = %key,
                "Failed to refresh gig sample media URL from existing R2 URL: {}",
                error
            );
            None
        }
    }
}

fn extract_r2_object_key_from_url(existing_url: &str, bucket: &str) -> Option<String> {
    let parsed = reqwest::Url::parse(existing_url).ok()?;
    let host = parsed.host_str().unwrap_or_default();
    let segments = parsed
        .path_segments()?
        .filter(|segment| !segment.is_empty())
        .collect::<Vec<_>>();

    if segments.is_empty() {
        return None;
    }

    if host.starts_with(&format!("{bucket}.")) {
        return Some(segments.join("/"));
    }

    if segments.first().copied() == Some(bucket) {
        if segments.len() < 2 {
            return None;
        }
        return Some(segments[1..].join("/"));
    }

    Some(segments.join("/"))
}

async fn gig_sample_workflow_id(
    sample_id: Uuid,
    pool: &sqlx::PgPool,
) -> Result<Option<Uuid>, String> {
    sqlx::query_scalar::<_, Option<Uuid>>("SELECT workflow_id FROM gig_sample_videos WHERE id = $1")
        .bind(sample_id)
        .fetch_optional(pool)
        .await
        .map(|row| row.flatten())
        .map_err(|e| format!("Failed to fetch gig sample workflow id: {}", e))
}

async fn verify_gig_sample_output(state: &Arc<AppState>, output_url: &str) -> bool {
    if output_url.trim().is_empty() {
        return false;
    }

    let Some(r2) = state.r2_client.as_ref() else {
        return true;
    };

    let Some(key) = extract_r2_object_key_from_url(output_url, &r2.bucket) else {
        return true;
    };

    r2.exists(&key).await
}

// ─── Background task ──────────────────────────────────────────────────────────

async fn run_sample_generation(
    sample_id: Uuid,
    template_id: Uuid,
    service_type: String,
    prompt: String,
    state: Arc<AppState>,
) {
    let workflow_id = gig_sample_workflow_id(sample_id, &state.db_pool)
        .await
        .ok()
        .flatten();

    let blender = match state.blender_mcp_client.as_ref() {
        Some(c) => c.clone(),
        None => {
            if let Some(workflow_id) = workflow_id {
                let workflow_runtime = crate::services::WorkflowRuntime::new(state.db_pool.clone());
                let _ = workflow_runtime
                    .mark_failed(
                        workflow_id,
                        Some("blender_client_check"),
                        "BlenderMCPServer not configured (BLENDER_MCP_URL not set)",
                        None,
                    )
                    .await;
            }
            let _ = sqlx::query(
                "UPDATE gig_sample_videos SET status='failed', error_message=$1, completed_at=NOW() WHERE id=$2",
            )
            .bind("BlenderMCPServer not configured (BLENDER_MCP_URL not set)")
            .bind(sample_id)
            .execute(&state.db_pool)
            .await;
            return;
        }
    };

    let _ = sqlx::query("UPDATE gig_sample_videos SET status='running' WHERE id=$1")
        .bind(sample_id)
        .execute(&state.db_pool)
        .await;
    if let Some(workflow_id) = workflow_id {
        let workflow_runtime = crate::services::WorkflowRuntime::new(state.db_pool.clone());
        let _ = workflow_runtime
            .heartbeat(
                workflow_id,
                crate::services::WorkflowStatus::Running,
                Some("render_queued"),
                "Gig template sample render started and is preparing tool arguments.",
                json!({
                    "sample_id": sample_id,
                    "template_id": template_id,
                    "service_type": service_type.clone(),
                }),
            )
            .await;
    }

    // Map service_type → (tool, args, url_key, ext, duration)
    let (tool, args, url_key, ext) = build_sample_tool_args(&service_type, &prompt);

    let job_id = match blender.submit_job(&tool, args).await {
        Ok(id) => id,
        Err(e) => {
            if let Some(workflow_id) = workflow_id {
                let workflow_runtime = crate::services::WorkflowRuntime::new(state.db_pool.clone());
                let _ = workflow_runtime
                    .mark_failed(workflow_id, Some("submit_render_job"), &e, None)
                    .await;
            }
            let _ = sqlx::query(
                "UPDATE gig_sample_videos SET status='failed', error_message=$1, completed_at=NOW() WHERE id=$2",
            )
            .bind(&e).bind(sample_id).execute(&state.db_pool).await;
            return;
        }
    };

    let mut final_url: Option<String> = None;
    for _ in 0..180u16 {
        tokio::time::sleep(std::time::Duration::from_secs(5)).await;
        let status = match blender.poll_job(&job_id).await {
            Ok(s) => s,
            Err(e) => {
                if let Some(workflow_id) = workflow_id {
                    let workflow_runtime =
                        crate::services::WorkflowRuntime::new(state.db_pool.clone());
                    let _ = workflow_runtime
                        .mark_failed(workflow_id, Some("poll_render_job"), &e, None)
                        .await;
                }
                let _ = sqlx::query(
                    "UPDATE gig_sample_videos SET status='failed', error_message=$1, completed_at=NOW() WHERE id=$2",
                )
                .bind(&e).bind(sample_id).execute(&state.db_pool).await;
                return;
            }
        };

        match status.get("state").and_then(|s| s.as_str()) {
            Some("completed") => {
                if let Some(url) = sample_result_media_url(status.get("result"), url_key) {
                    final_url = Some(url);
                }
                break;
            }
            Some("failed") | Some("error") => {
                let msg = status
                    .get("error")
                    .and_then(|v| v.as_str())
                    .unwrap_or("render failed");
                if let Some(workflow_id) = workflow_id {
                    let workflow_runtime =
                        crate::services::WorkflowRuntime::new(state.db_pool.clone());
                    let _ = workflow_runtime
                        .mark_failed(workflow_id, Some("render_failed"), msg, None)
                        .await;
                }
                let _ = sqlx::query(
                    "UPDATE gig_sample_videos SET status='failed', error_message=$1, completed_at=NOW() WHERE id=$2",
                )
                .bind(msg).bind(sample_id).execute(&state.db_pool).await;
                return;
            }
            _ => {
                if let Some(workflow_id) = workflow_id {
                    let workflow_runtime =
                        crate::services::WorkflowRuntime::new(state.db_pool.clone());
                    let render_state = status
                        .get("state")
                        .and_then(|s| s.as_str())
                        .unwrap_or("running");
                    let _ = workflow_runtime
                        .heartbeat(
                            workflow_id,
                            crate::services::WorkflowStatus::WaitingForExternalService,
                            Some(render_state),
                            "Gig template sample render is still running on the render backend.",
                            json!({
                                "sample_id": sample_id,
                                "render_job_id": job_id,
                                "render_state": render_state,
                            }),
                        )
                        .await;
                }
            }
        }
    }

    match final_url {
        Some(url) => {
            if !verify_gig_sample_output(&state, &url).await {
                if let Some(workflow_id) = workflow_id {
                    let workflow_runtime =
                        crate::services::WorkflowRuntime::new(state.db_pool.clone());
                    let _ = workflow_runtime
                        .mark_failed(
                            workflow_id,
                            Some("artifact_verification"),
                            "Gig sample render completed without a verifiable storage artifact.",
                            None,
                        )
                        .await;
                }
                let _ = sqlx::query(
                    "UPDATE gig_sample_videos SET status='failed', error_message=$1, completed_at=NOW() WHERE id=$2",
                )
                .bind("Render completed without a verifiable sample artifact")
                .bind(sample_id)
                .execute(&state.db_pool)
                .await;
                return;
            }

            let filename = format!(
                "sample_{}_{}.{}",
                template_id.to_string().split('-').next().unwrap_or("x"),
                sample_id.to_string().split('-').next().unwrap_or("y"),
                ext
            );
            let _ = sqlx::query(
                "UPDATE gig_sample_videos SET status='completed', output_r2_url=$1, output_filename=$2, completed_at=NOW() WHERE id=$3",
            )
            .bind(&url).bind(&filename).bind(sample_id)
            .execute(&state.db_pool).await;
            if let Some(workflow_id) = workflow_id {
                let workflow_runtime = crate::services::WorkflowRuntime::new(state.db_pool.clone());
                let _ = workflow_runtime
                    .mark_completed(
                        workflow_id,
                        Some("completed"),
                        "Gig template sample render completed with a verifiable output artifact.",
                        json!({
                            "output_r2_url_present": true,
                            "output_filename": filename,
                            "service_type": service_type.clone(),
                        }),
                    )
                    .await;
            }

            let artifact = crate::services::media_review::MediaReviewArtifact {
                review_id: format!("gig-sample-{}", sample_id),
                asset_kind: "gig_template_sample".to_string(),
                source_type: "gig_templates".to_string(),
                service_slug: Some(service_type.clone()),
                owner_user_id: None,
                output_url: Some(url.clone()),
                source_url: None,
                prompt: Some(prompt.clone()),
                title: Some(title_from_service_type(&service_type)),
                company: None,
                review_status: "completed".to_string(),
                qa_score: None,
                qa_feedback: None,
                narration_text: None,
                visual_direction: None,
                transcript_excerpt: None,
                tags: vec![service_type.clone(), "gig-template".to_string()],
            };

            if let Err(error) =
                crate::services::media_review::MediaReviewService::store_artifact(&state, artifact)
                    .await
            {
                tracing::warn!(
                    "Failed to store media review artifact for gig sample {}: {}",
                    sample_id,
                    error
                );
            }
        }
        None => {
            if let Some(workflow_id) = workflow_id {
                let workflow_runtime = crate::services::WorkflowRuntime::new(state.db_pool.clone());
                let _ = workflow_runtime
                    .mark_failed(
                        workflow_id,
                        Some("render_timeout"),
                        "Gig template sample render timed out without producing an output URL.",
                        None,
                    )
                    .await;
            }
            let _ = sqlx::query(
                "UPDATE gig_sample_videos SET status='failed', error_message='Timed out after 900s', completed_at=NOW() WHERE id=$1",
            )
            .bind(sample_id).execute(&state.db_pool).await;
        }
    }
}

fn title_from_service_type(service_type: &str) -> String {
    service_type
        .split('_')
        .map(|part| {
            let mut chars = part.chars();
            match chars.next() {
                Some(first) => format!("{}{}", first.to_uppercase(), chars.as_str()),
                None => String::new(),
            }
        })
        .collect::<Vec<_>>()
        .join(" ")
}

fn sample_result_media_url(result: Option<&Value>, url_key: &str) -> Option<String> {
    let result = result?;
    if url_key == "video_url" {
        result
            .get("narrated_video_url")
            .and_then(|v| v.as_str())
            .filter(|v| !v.trim().is_empty())
            .or_else(|| result.get("video_url").and_then(|v| v.as_str()))
            .map(|s| s.to_string())
    } else {
        result
            .get(url_key)
            .and_then(|v| v.as_str())
            .map(|s| s.to_string())
    }
}

fn build_sample_tool_args(
    service_type: &str,
    prompt: &str,
) -> (String, Value, &'static str, &'static str) {
    match service_type {
        "thumbnail" => (
            "blender_generate_scene_type".to_string(),
            json!({"prompt": prompt, "params": {"style": "cinematic", "duration": 5.0}, "output_type": "image"}),
            "image_url",
            "png",
        ),
        "title_card" => (
            "blender_generate_scene_type".to_string(),
            json!({"prompt": format!("Title card: {}", prompt), "params": {"style": "professional", "duration": 5.0}}),
            "video_url",
            "mp4",
        ),
        "data_viz" => (
            "manim_execute_script".to_string(),
            json!({"description": format!("Bar chart showing quarterly data for {}", prompt), "duration": 10.0, "quality": "h"}),
            "video_url",
            "mp4",
        ),
        "lower_third" => (
            "blender_generate_scene_type".to_string(),
            json!({"prompt": format!("Lower third: {}", prompt), "params": {"style": "professional", "duration": 5.0}}),
            "video_url",
            "mp4",
        ),
        "latex" => (
            "manim_execute_script".to_string(),
            json!({"description": format!("Animated formula breakdown for {}", prompt), "duration": 10.0, "background": "dark", "quality": "h"}),
            "video_url",
            "mp4",
        ),
        "ui_mockup" => (
            "blender_generate_scene_type".to_string(),
            json!({"prompt": "iPhone UI mockup reveal animation", "params": {"style": "cinematic", "duration": 8.0}}),
            "video_url",
            "mp4",
        ),
        _ => (
            // "scene" / "auto_video" / default → blender_generate_scene_type
            "blender_generate_scene_type".to_string(),
            json!({"prompt": prompt, "params": {"style": "cinematic", "duration": 12.0}}),
            "video_url",
            "mp4",
        ),
    }
}

// ─── SSR HTML ────────────────────────────────────────────────────────────────

const GIG_TEMPLATES_HTML: &str = r###"<!DOCTYPE html>
<html lang="en">
<head>
<meta charset="UTF-8">
<meta name="viewport" content="width=device-width, initial-scale=1">
<title>Gig Templates — VideoSync</title>
<style>
  * { box-sizing: border-box; margin: 0; padding: 0; }
  body { font-family: -apple-system, BlinkMacSystemFont, 'Segoe UI', sans-serif;
         background: #0a0a10; color: #e0e0e0; min-height: 100vh; padding: 40px 24px; }
  .page-header { max-width: 1100px; margin: 0 auto 36px; }
  .page-header h1 { font-size: 26px; font-weight: 700; color: #fff; margin-bottom: 6px; }
  .page-header p { font-size: 14px; color: #9999bb; }
  .back-link { display: inline-block; margin-bottom: 20px; color: #6c5ce7; font-size: 13px; text-decoration: none; }
  .back-link:hover { color: #a99ef7; }
  .grid { display: grid; grid-template-columns: repeat(auto-fill, minmax(520px, 1fr)); gap: 24px; max-width: 1100px; margin: 0 auto; }
  .card { background: #13131e; border: 1px solid #2a2a3a; border-radius: 14px; overflow: hidden; }
  .card-header { padding: 20px 24px 16px; border-bottom: 1px solid #2a2a3a; display: flex; justify-content: space-between; align-items: flex-start; }
  .card-title { font-size: 17px; font-weight: 700; color: #fff; margin-bottom: 4px; }
  .card-tagline { font-size: 12px; color: #9999bb; }
  .gig-tag { background: #6c5ce722; color: #a99ef7; padding: 3px 10px; border-radius: 6px; font-size: 11px; font-weight: 600; white-space: nowrap; }
  .card-body { padding: 20px 24px; }
  .section-label { font-size: 10px; font-weight: 700; text-transform: uppercase; letter-spacing: 0.1em; color: #666680; margin-bottom: 8px; }
  /* Pricing table */
  .pricing { display: grid; grid-template-columns: 1fr 1fr 1fr; gap: 10px; margin-bottom: 20px; }
  .tier { background: #0f0f1a; border: 1px solid #2a2a3a; border-radius: 8px; padding: 12px; }
  .tier-name { font-size: 10px; font-weight: 700; text-transform: uppercase; letter-spacing: 0.08em; margin-bottom: 6px; }
  .tier-basic .tier-name    { color: #9999bb; }
  .tier-standard .tier-name { color: #60a5fa; }
  .tier-premium .tier-name  { color: #facc15; }
  .tier-price { font-size: 22px; font-weight: 800; color: #fff; margin-bottom: 4px; }
  .tier-delivery { font-size: 10px; color: #666680; margin-bottom: 8px; }
  .tier-includes { font-size: 11px; color: #9999bb; line-height: 1.5; }
  /* Description */
  .description { background: #0f0f1a; border-radius: 8px; padding: 14px; margin-bottom: 16px; font-size: 12px; color: #b0b0cc; line-height: 1.7; white-space: pre-wrap; max-height: 140px; overflow-y: auto; }
  /* Keywords */
  .keywords { display: flex; flex-wrap: wrap; gap: 6px; margin-bottom: 16px; }
  .kw { background: #1a1a2e; border: 1px solid #2a2a4a; color: #9999cc; padding: 3px 9px; border-radius: 4px; font-size: 11px; }
  /* Gig titles */
  .gig-titles { display: flex; flex-direction: column; gap: 8px; margin-bottom: 20px; }
  .gig-title-row { display: flex; gap: 8px; align-items: center; background: #0f0f1a; border-radius: 7px; padding: 10px 12px; }
  .gig-title-text { flex: 1; font-size: 12px; color: #d0d0e8; line-height: 1.4; }
  .btn-copy { background: #1a1a2e; border: 1px solid #3a3a5a; color: #9999cc; padding: 5px 12px; border-radius: 6px; font-size: 11px; cursor: pointer; white-space: nowrap; transition: all 0.15s; }
  .btn-copy:hover { border-color: #6c5ce7; color: #fff; }
  .btn-copy.copied { background: #1a3a1a; border-color: #4ade80; color: #4ade80; }
  /* Copy description */
  .copy-desc-btn { width: 100%; padding: 8px; background: #1a1a2e; border: 1px solid #3a3a5a; color: #9999cc; border-radius: 7px; font-size: 12px; cursor: pointer; margin-bottom: 16px; transition: all 0.15s; }
  .copy-desc-btn:hover { border-color: #6c5ce7; color: #fff; }
  /* Sample videos */
  .samples-header { display: flex; justify-content: space-between; align-items: center; margin-bottom: 12px; }
  .samples-header .section-label { margin-bottom: 0; }
  .btn-generate { background: #6c5ce7; color: #fff; border: none; padding: 7px 16px; border-radius: 7px; font-size: 12px; font-weight: 600; cursor: pointer; transition: background 0.15s; }
  .btn-generate:hover { background: #5a4bd1; }
  .btn-generate:disabled { opacity: 0.5; cursor: not-allowed; }
  .samples-grid { display: grid; grid-template-columns: repeat(5, 1fr); gap: 8px; }
  .sample-slot { aspect-ratio: 16/9; background: #0f0f1a; border: 1px solid #2a2a3a; border-radius: 8px; overflow: hidden; position: relative; cursor: pointer; transition: border-color 0.15s; }
  .sample-slot:hover { border-color: #6c5ce7; }
  .sample-slot.empty { display: flex; align-items: center; justify-content: center; color: #444466; font-size: 10px; cursor: default; }
  .sample-slot video, .sample-slot img { width: 100%; height: 100%; object-fit: cover; }
  .sample-status { position: absolute; bottom: 0; left: 0; right: 0; background: rgba(0,0,0,0.7); padding: 3px 6px; font-size: 9px; text-align: center; }
  .status-running  { color: #60a5fa; }
  .status-pending  { color: #9999bb; }
  .status-failed   { color: #f87171; }
  .status-completed { color: #4ade80; }
  .sample-delete { position: absolute; top: 4px; right: 4px; background: rgba(220,38,38,0.8); border: none; color: #fff; width: 18px; height: 18px; border-radius: 50%; font-size: 11px; cursor: pointer; display: none; line-height: 18px; text-align: center; }
  .sample-slot:hover .sample-delete { display: block; }
  /* Spinner */
  .spinner { display: inline-block; width: 12px; height: 12px; border: 2px solid rgba(255,255,255,0.3);
             border-top-color: #fff; border-radius: 50%; animation: spin 0.7s linear infinite; vertical-align: middle; margin-right: 4px; }
  @keyframes spin { to { transform: rotate(360deg); } }
  .loading { text-align: center; padding: 80px; color: #666680; }
  .auth-error { max-width: 400px; margin: 100px auto; text-align: center; }
  .auth-error h2 { color: #f87171; margin-bottom: 12px; }
  .auth-error a { color: #6c5ce7; }
</style>
</head>
<body>
<a class="back-link" href="/admin/dashboard">← Admin Dashboard</a>
<div class="page-header">
  <h1>Gig Templates</h1>
  <p>Copy-paste ready Fiverr & People Per Hour gig information. Generate AI sample videos for each gig. Max 5 samples per template.</p>
</div>
<div id="root"><div class="loading">Loading gig templates…</div></div>

<script>
const token = localStorage.getItem('authToken') || localStorage.getItem('admin_token') || localStorage.getItem('auth_token');
if (!token) { window.location.href = '/admin'; }

let templates = [];
const refreshIntervals = {};

function copy(text, btn) {
  navigator.clipboard.writeText(text).then(() => {
    btn.textContent = '✓ Copied';
    btn.classList.add('copied');
    setTimeout(() => { btn.textContent = 'Copy'; btn.classList.remove('copied'); }, 1500);
  });
}

function fmtStatus(s) {
  const colors = { completed: '#4ade80', running: '#60a5fa', pending: '#9999bb', failed: '#f87171' };
  return `<span style="color:${colors[s]||'#999'}">${s}</span>`;
}

function renderSample(s, templateId) {
  const isEmpty = !s;
  if (isEmpty) return `<div class="sample-slot empty">empty</div>`;

  let media = '';
  if (s.status === 'completed' && s.r2_url) {
    const isImg = s.filename && (s.filename.endsWith('.png') || s.filename.endsWith('.jpg'));
    media = isImg
      ? `<img src="${s.r2_url}" alt="sample">`
      : `<video src="${s.r2_url}" muted loop autoplay playsinline></video>`;
  }

  const statusLabel = s.status === 'running'
    ? `<span class="spinner"></span>${s.status}`
    : s.status;

  return `<div class="sample-slot" onclick="openSample('${s.r2_url||''}','${s.filename||''}')">
    ${media}
    <div class="sample-status ${s.status === 'running' ? 'status-running' : ''}">${statusLabel}</div>
    <button class="sample-delete" onclick="event.stopPropagation();deleteSample('${s.id}','${templateId}')" title="Delete">×</button>
  </div>`;
}

function renderTemplate(t) {
  const samples = (t.samples || []).slice(0, 5);
  while (samples.length < 5) samples.push(null);
  const samplesHtml = samples.map(s => renderSample(s, t.id)).join('');

  const keywords = (t.keywords || []).map(k => `<span class="kw">${k}</span>`).join('');
  const titles = (t.gig_titles || []).map(title =>
    `<div class="gig-title-row">
       <span class="gig-title-text">${title}</span>
       <button class="btn-copy" onclick="copy(${JSON.stringify(title)},this)">Copy</button>
     </div>`).join('');

  const hasRunning = (t.samples || []).some(s => s.status === 'running' || s.status === 'pending');
  const sampleCount = (t.samples || []).length;
  const genDisabled = sampleCount >= 5 || hasRunning;

  return `<div class="card" id="card-${t.id}">
  <div class="card-header">
    <div>
      <div class="card-title">${t.display_name}</div>
      <div class="card-tagline">${t.tagline}</div>
    </div>
    <span class="gig-tag">${t.service_type}</span>
  </div>
  <div class="card-body">
    <div class="section-label">Pricing Tiers</div>
    <div class="pricing">
      <div class="tier tier-basic">
        <div class="tier-name">Basic</div>
        <div class="tier-price">$${t.basic_price}</div>
        <div class="tier-delivery">${t.basic_delivery_days}-day delivery</div>
        <div class="tier-includes">${t.basic_includes}</div>
      </div>
      <div class="tier tier-standard">
        <div class="tier-name">Standard</div>
        <div class="tier-price">$${t.standard_price}</div>
        <div class="tier-delivery">${t.standard_delivery_days}-day delivery</div>
        <div class="tier-includes">${t.standard_includes}</div>
      </div>
      <div class="tier tier-premium">
        <div class="tier-name">Premium</div>
        <div class="tier-price">$${t.premium_price}</div>
        <div class="tier-delivery">${t.premium_delivery_days}-day delivery</div>
        <div class="tier-includes">${t.premium_includes}</div>
      </div>
    </div>

    <div class="section-label">Gig Titles (click to copy)</div>
    <div class="gig-titles">${titles}</div>

    <div class="section-label">Keywords</div>
    <div class="keywords">${keywords}</div>

    <div class="section-label">Description (copy-paste to Fiverr/PPH)</div>
    <div class="description" id="desc-${t.id}">${escHtml(t.description)}</div>
    <button class="copy-desc-btn" onclick="copy(${JSON.stringify(t.description)},this)">📋 Copy Full Description</button>

    <div class="samples-header">
      <div class="section-label">Sample Videos (${sampleCount}/5)</div>
      <button class="btn-generate" id="gen-${t.id}" ${genDisabled?'disabled':''} onclick="generateSample('${t.id}')">
        ${hasRunning ? '<span class="spinner"></span>Rendering…' : '+ Generate Sample'}
      </button>
    </div>
    <div class="samples-grid" id="samples-${t.id}">${samplesHtml}</div>
  </div>
</div>`;
}

function escHtml(s) {
  return s.replace(/&/g,'&amp;').replace(/</g,'&lt;').replace(/>/g,'&gt;');
}

async function ensureGigTemplateAccess() {
  const r = await fetch('/api/clipping/access-check', {
    headers: { 'Authorization': `Bearer ${token}` }
  });
  if (!r.ok) {
    window.location.href = '/dashboard';
    throw new Error('Gig Templates are currently limited to admins and whitelisted users.');
  }
}

async function loadTemplates() {
  try {
    await ensureGigTemplateAccess();
    const r = await fetch('/api/gig-templates', {
      headers: { 'Authorization': `Bearer ${token}` }
    });
    const data = await r.json();
    templates = data.templates || [];
    const html = templates.map(renderTemplate).join('');
    document.getElementById('root').innerHTML = `<div class="grid">${html}</div>`;
    // Auto-refresh cards that have running samples
    templates.forEach(t => {
      const hasRunning = (t.samples||[]).some(s => s.status==='running'||s.status==='pending');
      if (hasRunning) scheduleRefresh(t.id);
    });
  } catch(e) {
    document.getElementById('root').innerHTML = `<div class="loading">Error: ${e.message}</div>`;
  }
}

function scheduleRefresh(templateId) {
  if (refreshIntervals[templateId]) return;
  refreshIntervals[templateId] = setInterval(async () => {
    try {
      const r = await fetch('/api/gig-templates', { headers: { 'Authorization': `Bearer ${token}` } });
      const data = await r.json();
      const t = (data.templates||[]).find(t => t.id === templateId);
      if (!t) return;
      const card = document.getElementById(`card-${templateId}`);
      if (!card) return;
      const samples = (t.samples||[]).slice(0,5);
      while (samples.length < 5) samples.push(null);
      document.getElementById(`samples-${templateId}`).innerHTML = samples.map(s => renderSample(s, templateId)).join('');
      const hasRunning = (t.samples||[]).some(s => s.status==='running'||s.status==='pending');
      const genBtn = document.getElementById(`gen-${templateId}`);
      if (genBtn) {
        genBtn.disabled = t.samples.length >= 5 || hasRunning;
        genBtn.innerHTML = hasRunning ? '<span class="spinner"></span>Rendering…' : '+ Generate Sample';
      }
      if (!hasRunning) {
        clearInterval(refreshIntervals[templateId]);
        delete refreshIntervals[templateId];
      }
    } catch {}
  }, 5000);
}

async function generateSample(templateId) {
  const btn = document.getElementById(`gen-${templateId}`);
  btn.disabled = true;
  btn.innerHTML = '<span class="spinner"></span>Submitting…';
  try {
    const r = await fetch(`/api/gig-templates/${templateId}/generate-sample`, {
      method: 'POST',
      headers: { 'Authorization': `Bearer ${token}`, 'Content-Type': 'application/json' }
    });
    const data = await r.json();
    if (data.error) throw new Error(data.error);
    await loadTemplates();
    scheduleRefresh(templateId);
  } catch(e) {
    btn.disabled = false;
    btn.innerHTML = '+ Generate Sample';
    alert('Error: ' + e.message);
  }
}

async function deleteSample(sampleId, templateId) {
  if (!confirm('Delete this sample?')) return;
  try {
    await fetch(`/api/gig-samples/${sampleId}`, {
      method: 'DELETE',
      headers: { 'Authorization': `Bearer ${token}` }
    });
    await loadTemplates();
  } catch(e) {
    alert('Error: ' + e.message);
  }
}

function openSample(url, filename) {
  if (!url) return;
  window.open(url, '_blank');
}

loadTemplates();
</script>
</body>
</html>"###;
