// Gig Templates — Fiverr/PPH template info + AI sample video generation
// Accessible to any authenticated (whitelisted) user.
//
// Routes:
//   GET  /gig-templates                        — SSR page (HTML, public — JS handles auth)
//   GET  /api/gig-templates                    — JSON list with samples  (auth)
//   POST /api/gig-templates/:id/generate-sample — spawn sample render    (auth)
//   DELETE /api/gig-samples/:id                — delete a sample         (auth)

use crate::AppState;
use crate::middleware::auth::auth_middleware;
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
    let public = Router::new()
        .route("/gig-templates", get(gig_templates_page));

    let protected = Router::new()
        .route("/api/gig-templates", get(api_list_gig_templates))
        .route("/api/gig-templates/:id/generate-sample", post(api_generate_sample))
        .route("/api/gig-samples/:id", delete(api_delete_sample))
        .layer(axum::middleware::from_fn(auth_middleware));

    public.merge(protected)
}

// ─── SSR page ────────────────────────────────────────────────────────────────

pub async fn gig_templates_page() -> Html<String> {
    Html(GIG_TEMPLATES_HTML.to_string())
}

// ─── API: list templates + samples ───────────────────────────────────────────

pub async fn api_list_gig_templates(
    Extension(state): Extension<Arc<AppState>>,
) -> Json<Value> {
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
            "SELECT id, title, prompt_used, status, output_r2_url, output_filename, error_message, created_at
             FROM gig_sample_videos WHERE template_id = $1 ORDER BY created_at",
        )
        .bind(tid)
        .fetch_all(&state.db_pool)
        .await
        .unwrap_or_default();

        let samples_json: Vec<Value> = samples.iter().map(|s| json!({
            "id":           s.get::<Uuid, _>("id").to_string(),
            "title":        s.get::<String, _>("title"),
            "prompt_used":  s.get::<String, _>("prompt_used"),
            "status":       s.get::<String, _>("status"),
            "r2_url":       s.try_get::<String, _>("output_r2_url").ok(),
            "filename":     s.try_get::<String, _>("output_filename").ok(),
            "error":        s.try_get::<String, _>("error_message").ok(),
            "created_at":   s.get::<chrono::DateTime<chrono::Utc>, _>("created_at").to_rfc3339(),
        })).collect();

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
) -> (StatusCode, Json<Value>) {
    // Load template to get service_type and sample_prompts
    let row = sqlx::query(
        "SELECT service_type, sample_prompts FROM gig_templates WHERE id = $1",
    )
    .bind(template_id)
    .fetch_optional(&state.db_pool)
    .await;

    let row = match row {
        Ok(Some(r)) => r,
        Ok(None) => return (StatusCode::NOT_FOUND, Json(json!({"error": "Template not found"}))),
        Err(e) => return (StatusCode::INTERNAL_SERVER_ERROR, Json(json!({"error": format!("DB error: {e}")}))),
    };

    let service_type: String = row.get("service_type");
    let prompts: Value = row.get("sample_prompts");

    // Count existing samples to pick next prompt (rotate through 5)
    let count: i64 = sqlx::query_scalar(
        "SELECT COUNT(*) FROM gig_sample_videos WHERE template_id = $1",
    )
    .bind(template_id)
    .fetch_one(&state.db_pool)
    .await
    .unwrap_or(0);

    if count >= 5 {
        return (StatusCode::BAD_REQUEST, Json(json!({"error": "Maximum 5 samples per template. Delete one to generate more."})));
    }

    let empty_prompts = vec![];
    let prompts_arr = prompts.as_array().unwrap_or(&empty_prompts);
    let idx = (count as usize) % prompts_arr.len().max(1);
    let prompt = prompts_arr.get(idx)
        .and_then(|v| v.as_str())
        .unwrap_or("Professional 3D animation sample")
        .to_string();

    let title = format!("Sample {} — {}", count + 1, &service_type);

    // Insert sample record
    let sample_id: Uuid = sqlx::query_scalar(
        "INSERT INTO gig_sample_videos (template_id, title, prompt_used, status)
         VALUES ($1, $2, $3, 'pending') RETURNING id",
    )
    .bind(template_id)
    .bind(&title)
    .bind(&prompt)
    .fetch_one(&state.db_pool)
    .await
    .unwrap_or_else(|_| Uuid::new_v4());

    // Spawn background render
    let state_clone = state.clone();
    let service_type_clone = service_type.clone();
    let prompt_clone = prompt.clone();
    tokio::spawn(async move {
        run_sample_generation(sample_id, template_id, service_type_clone, prompt_clone, state_clone).await;
    });

    (StatusCode::OK, Json(json!({"sample_id": sample_id.to_string(), "prompt": prompt})))
}

// ─── API: delete sample ───────────────────────────────────────────────────────

pub async fn api_delete_sample(
    Path(sample_id): Path<Uuid>,
    Extension(state): Extension<Arc<AppState>>,
) -> (StatusCode, Json<Value>) {
    let result = sqlx::query("DELETE FROM gig_sample_videos WHERE id = $1")
        .bind(sample_id)
        .execute(&state.db_pool)
        .await;

    match result {
        Ok(r) if r.rows_affected() > 0 => (StatusCode::OK, Json(json!({"deleted": true}))),
        Ok(_) => (StatusCode::NOT_FOUND, Json(json!({"error": "Sample not found"}))),
        Err(e) => (StatusCode::INTERNAL_SERVER_ERROR, Json(json!({"error": format!("DB error: {e}")}))),
    }
}

// ─── Background task ──────────────────────────────────────────────────────────

async fn run_sample_generation(
    sample_id: Uuid,
    template_id: Uuid,
    service_type: String,
    prompt: String,
    state: Arc<AppState>,
) {
    let blender = match state.blender_mcp_client.as_ref() {
        Some(c) => c.clone(),
        None => {
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

    // Map service_type → (tool, args, url_key, ext, duration)
    let (tool, args, url_key, ext) = build_sample_tool_args(&service_type, &prompt);

    let job_id = match blender.submit_job(&tool, args).await {
        Ok(id) => id,
        Err(e) => {
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
                let _ = sqlx::query(
                    "UPDATE gig_sample_videos SET status='failed', error_message=$1, completed_at=NOW() WHERE id=$2",
                )
                .bind(&e).bind(sample_id).execute(&state.db_pool).await;
                return;
            }
        };

        match status.get("state").and_then(|s| s.as_str()) {
            Some("completed") => {
                if let Some(url) = status.get("result")
                    .and_then(|r| r.get(url_key))
                    .and_then(|v| v.as_str())
                    .map(|s| s.to_string())
                {
                    final_url = Some(url);
                }
                break;
            }
            Some("failed") | Some("error") => {
                let msg = status.get("error").and_then(|v| v.as_str()).unwrap_or("render failed");
                let _ = sqlx::query(
                    "UPDATE gig_sample_videos SET status='failed', error_message=$1, completed_at=NOW() WHERE id=$2",
                )
                .bind(msg).bind(sample_id).execute(&state.db_pool).await;
                return;
            }
            _ => {}
        }
    }

    match final_url {
        Some(url) => {
            let filename = format!("sample_{}_{}.{}", template_id.to_string().split('-').next().unwrap_or("x"), sample_id.to_string().split('-').next().unwrap_or("y"), ext);
            let _ = sqlx::query(
                "UPDATE gig_sample_videos SET status='completed', output_r2_url=$1, output_filename=$2, completed_at=NOW() WHERE id=$3",
            )
            .bind(&url).bind(&filename).bind(sample_id)
            .execute(&state.db_pool).await;
        }
        None => {
            let _ = sqlx::query(
                "UPDATE gig_sample_videos SET status='failed', error_message='Timed out after 900s', completed_at=NOW() WHERE id=$1",
            )
            .bind(sample_id).execute(&state.db_pool).await;
        }
    }
}

fn build_sample_tool_args(service_type: &str, prompt: &str) -> (String, Value, &'static str, &'static str) {
    match service_type {
        "thumbnail" => (
            "blender_generate_thumbnail".to_string(),
            json!({"prompt": prompt, "title_text": prompt.split("—").next().unwrap_or(prompt).trim(), "style": "cinematic"}),
            "image_url", "png",
        ),
        "title_card" => (
            "blender_generate_title_card".to_string(),
            json!({"title": prompt.split("—").next().unwrap_or(prompt).trim(),
                   "subtitle": prompt.split("—").nth(1).unwrap_or("").trim(),
                   "duration": 5.0, "style": "professional"}),
            "video_url", "mp4",
        ),
        "data_viz" => (
            "blender_generate_data_viz".to_string(),
            json!({"data_json": "[{\"label\":\"Q1\",\"value\":120},{\"label\":\"Q2\",\"value\":185},{\"label\":\"Q3\",\"value\":230},{\"label\":\"Q4\",\"value\":310}]",
                   "chart_type": "bar", "title": prompt.split("—").next().unwrap_or(prompt).trim(), "duration": 10.0}),
            "video_url", "mp4",
        ),
        "lower_third" => (
            "blender_generate_lower_third".to_string(),
            json!({"name_text": prompt.split("—").next().unwrap_or(prompt).trim(),
                   "subtitle_text": prompt.split("—").nth(1).unwrap_or("").trim(),
                   "style": "professional", "duration": 5.0}),
            "video_url", "mp4",
        ),
        "latex" => (
            "blender_generate_latex".to_string(),
            json!({"latex_expression": prompt.split(" ").next().unwrap_or(prompt),
                   "animation_type": "step_by_step", "duration": 10.0, "background_style": "dark"}),
            "video_url", "mp4",
        ),
        "ui_mockup" => (
            "blender_generate_ui_mockup".to_string(),
            json!({"device": "iPhone", "animation": "reveal", "duration": 8.0, "screenshot_url": ""}),
            "video_url", "mp4",
        ),
        _ => ( // "scene" / "auto_video" / default → generate_scene
            "blender_generate_scene".to_string(),
            json!({"prompt": prompt, "duration": 12.0, "style": "cinematic"}),
            "video_url", "mp4",
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
const token = localStorage.getItem('auth_token') || localStorage.getItem('authToken') || localStorage.getItem('auth_token') || localStorage.getItem('admin_token');
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

async function loadTemplates() {
  try {
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
