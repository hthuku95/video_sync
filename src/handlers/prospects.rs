// Admin-only prospect finder — uses YouTube + Twitch APIs to discover
// potential clients (content creators and clippers), scores them with Gemini,
// and generates personalized DM scripts.

use crate::middleware::admin::admin_middleware;
use crate::middleware::auth::auth_middleware;
use crate::models::auth::ErrorResponse;
use crate::AppState;
use axum::{
    extract::{Extension, Path, Query},
    http::StatusCode,
    response::{Html, Json},
    routing::{delete, get, patch, post},
    Router,
};
use serde::Deserialize;
use serde_json::json;
use sqlx::Row;
use std::sync::Arc;
use uuid::Uuid;

pub fn prospect_routes() -> Router {
    Router::new()
        .route("/admin/prospect-finder", get(prospect_finder_page))
        .route("/api/admin/prospects/search", post(search_prospects))
        .route("/api/admin/prospects", get(list_prospects))
        .route("/api/admin/prospects/:id", patch(update_prospect))
        .route("/api/admin/prospects/:id/dm-script", post(regenerate_dm_script))
        .route("/api/admin/prospects/:id", delete(delete_prospect))
        .layer(axum::middleware::from_fn(admin_middleware))
        .layer(axum::middleware::from_fn(auth_middleware))
}

#[derive(Debug, Deserialize)]
struct SearchRequest {
    platform: String,           // "youtube" | "twitch"
    prospect_type: String,      // "content_creator" | "clipper"
    category: Option<String>,
    min_viewers: Option<i64>,
    max_viewers: Option<i64>,
    limit: Option<usize>,
}

#[derive(Debug, Deserialize)]
struct ListQuery {
    prospect_type: Option<String>,
    contact_status: Option<String>,
    platform: Option<String>,
}

#[derive(Debug, Deserialize)]
struct UpdateProspectRequest {
    contact_status: Option<String>,
    notes: Option<String>,
}

// ============================================================================
// SSR page
// ============================================================================

async fn prospect_finder_page() -> Html<String> {
    Html(PROSPECT_FINDER_HTML.to_string())
}

// ============================================================================
// Search — AI-powered prospect discovery
// ============================================================================

async fn search_prospects(
    Extension(state): Extension<Arc<AppState>>,
    Json(payload): Json<SearchRequest>,
) -> Result<Json<serde_json::Value>, (StatusCode, Json<ErrorResponse>)> {
    let limit = payload.limit.unwrap_or(20).min(50);
    let mut found = 0usize;

    if payload.platform == "youtube" {
        found += search_youtube_prospects(&state, &payload, limit).await?;
    } else if payload.platform == "twitch" {
        found += search_twitch_prospects(&state, &payload, limit).await?;
    } else {
        return Err((
            StatusCode::BAD_REQUEST,
            Json(ErrorResponse { success: false, message: "platform must be 'youtube' or 'twitch'".to_string() }),
        ));
    }

    Ok(Json(json!({ "success": true, "found": found, "message": format!("Found and scored {} prospects", found) })))
}

async fn search_youtube_prospects(
    state: &Arc<AppState>,
    payload: &SearchRequest,
    limit: usize,
) -> Result<usize, (StatusCode, Json<ErrorResponse>)> {
    let api_key = std::env::var("YOUTUBE_API_KEY").unwrap_or_default();
    if api_key.is_empty() {
        return Err((StatusCode::SERVICE_UNAVAILABLE, Json(ErrorResponse { success: false, message: "YOUTUBE_API_KEY not configured".to_string() })));
    }

    let search_query = if payload.prospect_type == "clipper" {
        "video editor clips shorts creator".to_string()
    } else {
        payload.category.clone().unwrap_or_else(|| "gaming streamer".to_string())
    };

    let client = reqwest::Client::new();
    let search_url = format!(
        "https://www.googleapis.com/youtube/v3/search?part=snippet&type=channel&q={}&maxResults={}&order=relevance&key={}",
        urlencoding::encode(&search_query), limit.min(50), api_key
    );

    let search_resp: serde_json::Value = client.get(&search_url)
        .send().await
        .map_err(|e| (StatusCode::BAD_GATEWAY, Json(ErrorResponse { success: false, message: format!("YouTube API error: {}", e) })))?
        .json().await
        .map_err(|e| (StatusCode::BAD_GATEWAY, Json(ErrorResponse { success: false, message: format!("YouTube parse error: {}", e) })))?;

    let items = match search_resp["items"].as_array() {
        Some(arr) => arr.clone(),
        None => return Ok(0),
    };

    // Collect channel IDs for stats lookup
    let channel_ids: Vec<String> = items.iter()
        .filter_map(|item| item["id"]["channelId"].as_str().map(String::from))
        .collect();

    if channel_ids.is_empty() { return Ok(0); }

    let stats_url = format!(
        "https://www.googleapis.com/youtube/v3/channels?part=snippet,statistics&id={}&key={}",
        channel_ids.join(","), api_key
    );
    let stats_resp: serde_json::Value = client.get(&stats_url)
        .send().await.map_err(|e| (StatusCode::BAD_GATEWAY, Json(ErrorResponse { success: false, message: e.to_string() })))?
        .json().await.map_err(|e| (StatusCode::BAD_GATEWAY, Json(ErrorResponse { success: false, message: e.to_string() })))?;

    let channels = stats_resp["items"].as_array().cloned().unwrap_or_default();
    let mut count = 0;

    for channel in &channels {
        let channel_id = channel["id"].as_str().unwrap_or("").to_string();
        let display_name = channel["snippet"]["title"].as_str().unwrap_or("").to_string();
        let description = channel["snippet"]["description"].as_str().unwrap_or("").to_string();
        let sub_count: i64 = channel["statistics"]["subscriberCount"]
            .as_str().unwrap_or("0").parse().unwrap_or(0);

        if display_name.is_empty() || channel_id.is_empty() { continue; }

        // Apply viewer range filter
        if let Some(min) = payload.min_viewers {
            if sub_count < min { continue; }
        }
        if let Some(max) = payload.max_viewers {
            if sub_count > max { continue; }
        }

        let platform_url = format!("https://youtube.com/channel/{}", channel_id);
        let category = payload.category.clone().unwrap_or_else(|| "general".to_string());

        let (score, reasoning, dm_creator, dm_clipper) =
            score_prospect_with_ai(state, &display_name, sub_count, &description, &category, &payload.prospect_type).await;

        sqlx::query(
            "INSERT INTO prospects (platform, channel_id, display_name, platform_url,
             subscriber_count, content_category, channel_description, prospect_type,
             ai_score, ai_reasoning, dm_script_creator, dm_script_clipper)
             VALUES ($1,$2,$3,$4,$5,$6,$7,$8,$9,$10,$11,$12)
             ON CONFLICT (platform, channel_id) DO UPDATE SET
               ai_score = EXCLUDED.ai_score,
               ai_reasoning = EXCLUDED.ai_reasoning,
               dm_script_creator = EXCLUDED.dm_script_creator,
               dm_script_clipper = EXCLUDED.dm_script_clipper,
               updated_at = NOW()"
        )
        .bind("youtube")
        .bind(&channel_id)
        .bind(&display_name)
        .bind(&platform_url)
        .bind(sub_count)
        .bind(&category)
        .bind(&description)
        .bind(&payload.prospect_type)
        .bind(score)
        .bind(&reasoning)
        .bind(&dm_creator)
        .bind(&dm_clipper)
        .execute(&state.db_pool)
        .await
        .ok();

        count += 1;
    }

    Ok(count)
}

async fn search_twitch_prospects(
    state: &Arc<AppState>,
    payload: &SearchRequest,
    limit: usize,
) -> Result<usize, (StatusCode, Json<ErrorResponse>)> {
    let client_id = std::env::var("TWITCH_CLIENT_ID").unwrap_or_default();
    let client_secret = std::env::var("TWITCH_CLIENT_SECRET").unwrap_or_default();
    if client_id.is_empty() {
        return Err((StatusCode::SERVICE_UNAVAILABLE, Json(ErrorResponse { success: false, message: "TWITCH_CLIENT_ID not configured".to_string() })));
    }

    // Get app access token
    let client = reqwest::Client::new();
    let token_resp: serde_json::Value = client
        .post("https://id.twitch.tv/oauth2/token")
        .form(&[
            ("client_id", client_id.as_str()),
            ("client_secret", client_secret.as_str()),
            ("grant_type", "client_credentials"),
        ])
        .send().await
        .map_err(|e| (StatusCode::BAD_GATEWAY, Json(ErrorResponse { success: false, message: e.to_string() })))?
        .json().await
        .map_err(|e| (StatusCode::BAD_GATEWAY, Json(ErrorResponse { success: false, message: e.to_string() })))?;

    let access_token = token_resp["access_token"].as_str().unwrap_or("").to_string();

    // Search streams
    let streams_url = format!(
        "https://api.twitch.tv/helix/streams?first={}",
        limit.min(100)
    );
    let streams_resp: serde_json::Value = client.get(&streams_url)
        .header("Client-Id", &client_id)
        .header("Authorization", format!("Bearer {}", access_token))
        .send().await
        .map_err(|e| (StatusCode::BAD_GATEWAY, Json(ErrorResponse { success: false, message: e.to_string() })))?
        .json().await
        .map_err(|e| (StatusCode::BAD_GATEWAY, Json(ErrorResponse { success: false, message: e.to_string() })))?;

    let streams = streams_resp["data"].as_array().cloned().unwrap_or_default();
    let mut count = 0;

    for stream in &streams {
        let channel_id = stream["user_id"].as_str().unwrap_or("").to_string();
        let display_name = stream["user_name"].as_str().unwrap_or("").to_string();
        let viewer_count: i64 = stream["viewer_count"].as_i64().unwrap_or(0);
        let game_name = stream["game_name"].as_str().unwrap_or("").to_string();

        if display_name.is_empty() { continue; }

        if let Some(min) = payload.min_viewers { if viewer_count < min { continue; } }
        if let Some(max) = payload.max_viewers { if viewer_count > max { continue; } }

        let platform_url = format!("https://twitch.tv/{}", display_name.to_lowercase());
        let category = if game_name.is_empty() {
            payload.category.clone().unwrap_or_else(|| "gaming".to_string())
        } else {
            game_name.clone()
        };

        let (score, reasoning, dm_creator, dm_clipper) =
            score_prospect_with_ai(state, &display_name, viewer_count, "", &category, &payload.prospect_type).await;

        sqlx::query(
            "INSERT INTO prospects (platform, channel_id, display_name, platform_url,
             avg_viewer_count, content_category, prospect_type,
             ai_score, ai_reasoning, dm_script_creator, dm_script_clipper)
             VALUES ($1,$2,$3,$4,$5,$6,$7,$8,$9,$10,$11)
             ON CONFLICT (platform, channel_id) DO UPDATE SET
               avg_viewer_count = EXCLUDED.avg_viewer_count,
               ai_score = EXCLUDED.ai_score,
               ai_reasoning = EXCLUDED.ai_reasoning,
               dm_script_creator = EXCLUDED.dm_script_creator,
               dm_script_clipper = EXCLUDED.dm_script_clipper,
               updated_at = NOW()"
        )
        .bind("twitch")
        .bind(&channel_id)
        .bind(&display_name)
        .bind(&platform_url)
        .bind(viewer_count)
        .bind(&category)
        .bind(&payload.prospect_type)
        .bind(score)
        .bind(&reasoning)
        .bind(&dm_creator)
        .bind(&dm_clipper)
        .execute(&state.db_pool)
        .await
        .ok();

        count += 1;
    }

    Ok(count)
}

/// Use Gemini to score a prospect and generate DM scripts.
/// Returns (score, reasoning, dm_creator, dm_clipper).
async fn score_prospect_with_ai(
    state: &Arc<AppState>,
    name: &str,
    audience_size: i64,
    description: &str,
    category: &str,
    prospect_type: &str,
) -> (f64, String, String, String) {
    let gemini = match state.gemini_client.as_ref() {
        Some(g) => g,
        None => return (0.5, "Gemini not configured".to_string(), default_dm_creator(name), default_dm_clipper(name)),
    };

    let prompt = format!(
        r#"You are an AI assistant helping a video clipping service find clients.
Analyze this channel and respond with ONLY valid JSON (no markdown, no code blocks):

Channel name: {}
Audience size: {} ({})
Content category: {}
Description: {}
Prospect type: {}

Return JSON with exactly these fields:
{{
  "score": <float 0.0-1.0, how likely they need/want a clipping service>,
  "reasoning": "<1-2 sentences explaining the score>",
  "dm_creator": "<personalized DM to send to this content creator offering to clip their content — 2-3 sentences, mention their name and category>",
  "dm_clipper": "<personalized DM to send if this person is a clipper looking for better tools — 2-3 sentences>"
}}"#,
        name, audience_size, category, category, description, prospect_type
    );

    match gemini.generate_text(&prompt).await {
        Ok(text) => {
            let cleaned = text.trim().trim_start_matches("```json").trim_start_matches("```").trim_end_matches("```").trim();
            match serde_json::from_str::<serde_json::Value>(cleaned) {
                Ok(v) => {
                    let score = v["score"].as_f64().unwrap_or(0.5).clamp(0.0, 1.0);
                    let reasoning = v["reasoning"].as_str().unwrap_or("").to_string();
                    let dm_creator = v["dm_creator"].as_str().unwrap_or(&default_dm_creator(name)).to_string();
                    let dm_clipper = v["dm_clipper"].as_str().unwrap_or(&default_dm_clipper(name)).to_string();
                    (score, reasoning, dm_creator, dm_clipper)
                }
                Err(_) => (0.5, "Parse error".to_string(), default_dm_creator(name), default_dm_clipper(name)),
            }
        }
        Err(_) => (0.5, "AI unavailable".to_string(), default_dm_creator(name), default_dm_clipper(name)),
    }
}

fn default_dm_creator(name: &str) -> String {
    format!("Hey {}! I run an AI clipping service that turns your streams/videos into viral shorts automatically. I'd love to send you a free sample — interested?", name)
}

fn default_dm_clipper(name: &str) -> String {
    format!("Hey {}! I built an AI clipping platform that processes YouTube and Twitch videos 10x faster. Want free access to try it?", name)
}

// ============================================================================
// CRUD endpoints
// ============================================================================

async fn list_prospects(
    Extension(state): Extension<Arc<AppState>>,
    Query(q): Query<ListQuery>,
) -> Result<Json<serde_json::Value>, (StatusCode, Json<ErrorResponse>)> {
    let mut sql = "SELECT id, platform, channel_id, display_name, platform_url, \
                   subscriber_count, avg_viewer_count, content_category, prospect_type, \
                   ai_score, ai_reasoning, dm_script_creator, dm_script_clipper, \
                   contact_status, notes, created_at \
                   FROM prospects WHERE 1=1".to_string();

    if let Some(ref pt) = q.prospect_type { sql.push_str(&format!(" AND prospect_type = '{}'", pt.replace('\'', "''"))); }
    if let Some(ref cs) = q.contact_status { sql.push_str(&format!(" AND contact_status = '{}'", cs.replace('\'', "''"))); }
    if let Some(ref pl) = q.platform       { sql.push_str(&format!(" AND platform = '{}'", pl.replace('\'', "''"))); }
    sql.push_str(" ORDER BY ai_score DESC NULLS LAST, created_at DESC LIMIT 200");

    let rows = sqlx::query(&sql)
        .fetch_all(&state.db_pool)
        .await
        .map_err(|e| (StatusCode::INTERNAL_SERVER_ERROR, Json(ErrorResponse { success: false, message: e.to_string() })))?;

    let prospects: Vec<serde_json::Value> = rows.iter().map(|r| json!({
        "id": r.get::<Uuid, _>("id").to_string(),
        "platform": r.get::<String, _>("platform"),
        "channel_id": r.get::<String, _>("channel_id"),
        "display_name": r.get::<String, _>("display_name"),
        "platform_url": r.get::<String, _>("platform_url"),
        "subscriber_count": r.get::<Option<i64>, _>("subscriber_count"),
        "avg_viewer_count": r.get::<Option<i64>, _>("avg_viewer_count"),
        "content_category": r.get::<Option<String>, _>("content_category"),
        "prospect_type": r.get::<String, _>("prospect_type"),
        "ai_score": r.get::<Option<f64>, _>("ai_score"),
        "ai_reasoning": r.get::<Option<String>, _>("ai_reasoning"),
        "dm_script_creator": r.get::<Option<String>, _>("dm_script_creator"),
        "dm_script_clipper": r.get::<Option<String>, _>("dm_script_clipper"),
        "contact_status": r.get::<String, _>("contact_status"),
        "notes": r.get::<Option<String>, _>("notes"),
        "created_at": r.get::<chrono::DateTime<chrono::Utc>, _>("created_at"),
    })).collect();

    Ok(Json(json!({ "success": true, "prospects": prospects })))
}

async fn update_prospect(
    Extension(state): Extension<Arc<AppState>>,
    Path(id): Path<Uuid>,
    Json(payload): Json<UpdateProspectRequest>,
) -> Result<Json<serde_json::Value>, (StatusCode, Json<ErrorResponse>)> {
    if let Some(ref status) = payload.contact_status {
        sqlx::query("UPDATE prospects SET contact_status=$1, updated_at=NOW() WHERE id=$2")
            .bind(status).bind(id).execute(&state.db_pool).await.ok();
    }
    if let Some(ref notes) = payload.notes {
        sqlx::query("UPDATE prospects SET notes=$1, updated_at=NOW() WHERE id=$2")
            .bind(notes).bind(id).execute(&state.db_pool).await.ok();
    }
    Ok(Json(json!({ "success": true })))
}

async fn regenerate_dm_script(
    Extension(state): Extension<Arc<AppState>>,
    Path(id): Path<Uuid>,
) -> Result<Json<serde_json::Value>, (StatusCode, Json<ErrorResponse>)> {
    let row = sqlx::query(
        "SELECT display_name, subscriber_count, avg_viewer_count, content_category, channel_description, prospect_type
         FROM prospects WHERE id=$1"
    )
    .bind(id)
    .fetch_optional(&state.db_pool)
    .await
    .map_err(|e| (StatusCode::INTERNAL_SERVER_ERROR, Json(ErrorResponse { success: false, message: e.to_string() })))?
    .ok_or_else(|| (StatusCode::NOT_FOUND, Json(ErrorResponse { success: false, message: "Prospect not found".to_string() })))?;

    let name: String = row.get("display_name");
    let subs: Option<i64> = row.get("subscriber_count");
    let viewers: Option<i64> = row.get("avg_viewer_count");
    let category: String = row.get::<Option<String>, _>("content_category").unwrap_or_default();
    let description: String = row.get::<Option<String>, _>("channel_description").unwrap_or_default();
    let pt: String = row.get("prospect_type");

    let audience = subs.or(viewers).unwrap_or(0);
    let (score, reasoning, dm_creator, dm_clipper) =
        score_prospect_with_ai(&state, &name, audience, &description, &category, &pt).await;

    sqlx::query(
        "UPDATE prospects SET ai_score=$1, ai_reasoning=$2, dm_script_creator=$3, dm_script_clipper=$4, updated_at=NOW() WHERE id=$5"
    )
    .bind(score).bind(&reasoning).bind(&dm_creator).bind(&dm_clipper).bind(id)
    .execute(&state.db_pool).await.ok();

    Ok(Json(json!({ "success": true, "dm_creator": dm_creator, "dm_clipper": dm_clipper, "score": score })))
}

async fn delete_prospect(
    Extension(state): Extension<Arc<AppState>>,
    Path(id): Path<Uuid>,
) -> Result<Json<serde_json::Value>, (StatusCode, Json<ErrorResponse>)> {
    sqlx::query("DELETE FROM prospects WHERE id=$1")
        .bind(id).execute(&state.db_pool).await
        .map_err(|e| (StatusCode::INTERNAL_SERVER_ERROR, Json(ErrorResponse { success: false, message: e.to_string() })))?;
    Ok(Json(json!({ "success": true })))
}

// ============================================================================
// SSR HTML
// ============================================================================

const PROSPECT_FINDER_HTML: &str = r###"<!DOCTYPE html>
<html lang="en">
<head>
<meta charset="UTF-8">
<meta name="viewport" content="width=device-width, initial-scale=1.0">
<title>Prospect Finder — Admin</title>
<style>
*{box-sizing:border-box;margin:0;padding:0}
body{font-family:'Inter',system-ui,sans-serif;background:#1a1a2e;color:#e0e0e0;min-height:100vh}
.header{background:#16213e;border-bottom:1px solid #0f3460;padding:14px 24px;display:flex;align-items:center;gap:16px}
.header h1{font-size:1.2rem;color:#dbd8e3}
.back{color:#5c5470;text-decoration:none;font-size:0.9rem}
.back:hover{color:#dbd8e3}
.container{max-width:1200px;margin:0 auto;padding:24px}
.search-card{background:#16213e;border:1px solid #0f3460;border-radius:12px;padding:24px;margin-bottom:24px}
.search-card h2{font-size:1rem;color:#dbd8e3;margin-bottom:16px}
.form-grid{display:grid;grid-template-columns:repeat(auto-fit,minmax(180px,1fr));gap:12px;margin-bottom:16px}
label{font-size:0.8rem;color:#9ca3af;display:block;margin-bottom:4px}
select,input{width:100%;padding:8px 12px;background:#0f3460;border:1px solid #1e3a5f;border-radius:6px;color:#e0e0e0;font-size:0.9rem}
select:focus,input:focus{outline:none;border-color:#5c5470}
.btn{padding:10px 20px;border:none;border-radius:8px;cursor:pointer;font-size:0.9rem;font-weight:500}
.btn-primary{background:#5c5470;color:#fff}
.btn-primary:hover{background:#7a6e8a}
.btn-sm{padding:5px 10px;font-size:0.78rem;border-radius:5px}
.btn-copy{background:#0f3460;color:#dbd8e3;border:1px solid #1e3a5f}
.btn-copy:hover{background:#1e3a5f}
.tabs{display:flex;gap:8px;margin-bottom:16px;flex-wrap:wrap}
.tab{padding:6px 14px;border-radius:6px;cursor:pointer;font-size:0.85rem;background:#0f3460;color:#9ca3af;border:1px solid transparent}
.tab.active{background:#5c5470;color:#fff}
table{width:100%;border-collapse:collapse;font-size:0.85rem}
th{text-align:left;padding:10px 12px;color:#9ca3af;border-bottom:1px solid #1e3a5f;font-weight:500}
td{padding:10px 12px;border-bottom:1px solid #0f3460;vertical-align:top}
tr:hover td{background:#0a0a1a}
.score-badge{display:inline-block;padding:2px 8px;border-radius:10px;font-size:0.75rem;font-weight:600}
.score-high{background:#065f46;color:#6ee7b7}
.score-mid{background:#78350f;color:#fcd34d}
.score-low{background:#7f1d1d;color:#fca5a5}
.status-select{background:#0f3460;border:1px solid #1e3a5f;color:#e0e0e0;padding:3px 6px;border-radius:4px;font-size:0.78rem}
.platform-yt{color:#ff4444}
.platform-tw{color:#9147ff}
.dm-text{background:#0f3460;border:1px solid #1e3a5f;border-radius:6px;padding:8px;font-size:0.8rem;color:#ccc;white-space:pre-wrap;max-height:80px;overflow-y:auto;margin-bottom:4px}
.loading{text-align:center;padding:40px;color:#5c5470}
.msg{padding:10px 16px;border-radius:8px;margin-bottom:12px;font-size:0.9rem}
.msg-success{background:#065f46;color:#6ee7b7}
.msg-error{background:#7f1d1d;color:#fca5a5}
.empty{text-align:center;padding:40px;color:#5c5470}
.row-notes{width:100%;padding:4px 8px;background:#0f3460;border:1px solid #1e3a5f;border-radius:4px;color:#e0e0e0;font-size:0.78rem;margin-top:4px}
</style>
</head>
<body>
<div class="header">
  <a href="/admin" class="back">← Admin</a>
  <h1>🎯 Prospect Finder</h1>
</div>
<div class="container">
  <div id="msg"></div>

  <!-- Search Form -->
  <div class="search-card">
    <h2>Find Prospects</h2>
    <div class="form-grid">
      <div>
        <label>Platform</label>
        <select id="platform">
          <option value="youtube">YouTube</option>
          <option value="twitch">Twitch</option>
        </select>
      </div>
      <div>
        <label>Prospect Type</label>
        <select id="prospect_type">
          <option value="content_creator">Content Creator</option>
          <option value="clipper">Clipper / Editor</option>
        </select>
      </div>
      <div>
        <label>Category / Game</label>
        <input id="category" placeholder="gaming, tech, cooking…" value="gaming">
      </div>
      <div>
        <label>Min Viewers/Subscribers</label>
        <input id="min_viewers" type="number" placeholder="500" value="500">
      </div>
      <div>
        <label>Max Viewers/Subscribers</label>
        <input id="max_viewers" type="number" placeholder="50000" value="50000">
      </div>
      <div>
        <label>Results Limit</label>
        <input id="limit" type="number" value="20" min="5" max="50">
      </div>
    </div>
    <button class="btn btn-primary" onclick="runSearch()">🔍 Find &amp; Score Prospects</button>
    <span id="search-status" style="margin-left:12px;color:#9ca3af;font-size:0.85rem"></span>
  </div>

  <!-- Filter Tabs -->
  <div class="tabs">
    <div class="tab active" onclick="setFilter('all',this)">All</div>
    <div class="tab" onclick="setFilter('new',this)">New</div>
    <div class="tab" onclick="setFilter('contacted',this)">Contacted</div>
    <div class="tab" onclick="setFilter('replied',this)">Replied</div>
    <div class="tab" onclick="setFilter('converted',this)">Converted</div>
  </div>

  <!-- Results Table -->
  <div style="background:#16213e;border:1px solid #0f3460;border-radius:12px;overflow:hidden">
    <div id="table-area"><div class="loading">Loading prospects…</div></div>
  </div>
</div>

<script>
const token = localStorage.getItem('admin_token') || localStorage.getItem('authToken');
if (!token) window.location.href = '/admin';

let currentFilter = 'all';

function showMsg(text, ok=true){
  const el = document.getElementById('msg');
  el.innerHTML = `<div class="msg ${ok?'msg-success':'msg-error'}">${text}</div>`;
  setTimeout(()=>el.innerHTML='', 4000);
}

function scoreBadge(score){
  if(score == null) return '<span style="color:#666">—</span>';
  const pct = Math.round(score*100);
  const cls = score>=0.7?'score-high':score>=0.4?'score-mid':'score-low';
  return `<span class="score-badge ${cls}">${pct}%</span>`;
}

function platformIcon(p){
  return p==='youtube'?'<span class="platform-yt">▶ YouTube</span>':'<span class="platform-tw">🎮 Twitch</span>';
}

function formatNum(n){
  if(!n) return '—';
  if(n>=1000000) return (n/1000000).toFixed(1)+'M';
  if(n>=1000) return (n/1000).toFixed(1)+'K';
  return n;
}

async function loadProspects(){
  let url = '/api/admin/prospects';
  const params = [];
  if(currentFilter!=='all') params.push(`contact_status=${currentFilter}`);
  if(params.length) url += '?' + params.join('&');

  const res = await fetch(url, {headers:{'Authorization':'Bearer '+token}});
  const data = await res.json();
  if(!data.success){ document.getElementById('table-area').innerHTML = '<div class="empty">Failed to load prospects</div>'; return; }

  const prospects = data.prospects||[];
  if(!prospects.length){ document.getElementById('table-area').innerHTML = '<div class="empty">No prospects found. Run a search above.</div>'; return; }

  let html = `<table>
    <thead><tr>
      <th>Channel</th><th>Platform</th><th>Audience</th><th>Category</th>
      <th>AI Score</th><th>DM Scripts</th><th>Status</th><th>Actions</th>
    </tr></thead><tbody>`;

  for(const p of prospects){
    const dm = p.dm_script_creator||'';
    const dmClip = p.dm_script_clipper||'';
    html += `<tr id="row-${p.id}">
      <td><a href="${p.platform_url}" target="_blank" style="color:#dbd8e3;font-weight:500">${p.display_name}</a>
        ${p.ai_reasoning?`<div style="font-size:0.75rem;color:#9ca3af;margin-top:2px">${p.ai_reasoning}</div>`:''}
      </td>
      <td>${platformIcon(p.platform)}</td>
      <td>${formatNum(p.subscriber_count||p.avg_viewer_count)}</td>
      <td style="color:#9ca3af">${p.content_category||'—'}</td>
      <td>${scoreBadge(p.ai_score)}</td>
      <td style="min-width:220px">
        <div class="dm-text">${dm||'—'}</div>
        <button class="btn btn-sm btn-copy" onclick="copyText('${encodeURIComponent(dm)}')">📋 Copy Creator DM</button>
        <div class="dm-text" style="margin-top:6px">${dmClip||'—'}</div>
        <button class="btn btn-sm btn-copy" onclick="copyText('${encodeURIComponent(dmClip)}')">📋 Copy Clipper DM</button>
        <button class="btn btn-sm btn-copy" style="margin-top:4px" onclick="regen('${p.id}')">🔄 Refresh</button>
      </td>
      <td>
        <select class="status-select" onchange="updateStatus('${p.id}',this.value)">
          ${['new','contacted','replied','converted','rejected'].map(s=>`<option value="${s}" ${s===p.contact_status?'selected':''}>${s}</option>`).join('')}
        </select>
        <input class="row-notes" placeholder="Notes…" value="${(p.notes||'').replace(/"/g,'&quot;')}" onblur="saveNotes('${p.id}',this.value)">
      </td>
      <td>
        <a href="${p.platform_url}" target="_blank" class="btn btn-sm btn-copy">🔗 Open</a>
        <button class="btn btn-sm" style="background:#7f1d1d;color:#fff;margin-top:4px" onclick="deleteProspect('${p.id}')">🗑</button>
      </td>
    </tr>`;
  }
  html += '</tbody></table>';
  document.getElementById('table-area').innerHTML = html;
}

function copyText(encoded){
  navigator.clipboard.writeText(decodeURIComponent(encoded)).then(()=>showMsg('Copied to clipboard!'));
}

function setFilter(f, el){
  currentFilter = f;
  document.querySelectorAll('.tab').forEach(t=>t.classList.remove('active'));
  el.classList.add('active');
  loadProspects();
}

async function runSearch(){
  const payload = {
    platform: document.getElementById('platform').value,
    prospect_type: document.getElementById('prospect_type').value,
    category: document.getElementById('category').value||undefined,
    min_viewers: parseInt(document.getElementById('min_viewers').value)||undefined,
    max_viewers: parseInt(document.getElementById('max_viewers').value)||undefined,
    limit: parseInt(document.getElementById('limit').value)||20,
  };
  const status = document.getElementById('search-status');
  status.textContent = 'Searching…';
  const res = await fetch('/api/admin/prospects/search', {
    method:'POST', headers:{'Authorization':'Bearer '+token,'Content-Type':'application/json'},
    body: JSON.stringify(payload)
  });
  const data = await res.json();
  if(data.success){ showMsg(data.message); loadProspects(); }
  else showMsg(data.message||'Search failed', false);
  status.textContent = '';
}

async function updateStatus(id, status){
  await fetch(`/api/admin/prospects/${id}`, {
    method:'PATCH', headers:{'Authorization':'Bearer '+token,'Content-Type':'application/json'},
    body: JSON.stringify({contact_status: status})
  });
}

async function saveNotes(id, notes){
  await fetch(`/api/admin/prospects/${id}`, {
    method:'PATCH', headers:{'Authorization':'Bearer '+token,'Content-Type':'application/json'},
    body: JSON.stringify({notes})
  });
}

async function regen(id){
  const res = await fetch(`/api/admin/prospects/${id}/dm-script`, {
    method:'POST', headers:{'Authorization':'Bearer '+token}
  });
  const data = await res.json();
  if(data.success){ showMsg('DM scripts refreshed'); loadProspects(); }
  else showMsg('Refresh failed', false);
}

async function deleteProspect(id){
  if(!confirm('Remove this prospect?')) return;
  await fetch(`/api/admin/prospects/${id}`, {method:'DELETE', headers:{'Authorization':'Bearer '+token}});
  document.getElementById(`row-${id}`)?.remove();
}

loadProspects();
</script>
</body>
</html>
"###;
