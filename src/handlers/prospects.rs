// Admin-only prospect finder — uses YouTube + Twitch APIs to discover
// potential clients (content creators and clippers), scores them with Gemini,
// and generates personalized DM scripts.

use crate::llm_utils::generate_text_best_effort;
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
        .route("/api/admin/prospects/linkedin/agents", get(linkedin_list_agents))
        .route("/api/admin/prospects/linkedin/launch", post(linkedin_launch_search))
        .route("/api/admin/prospects/linkedin/search", post(linkedin_smart_search))
        .route("/api/admin/prospects/linkedin/jobs", get(linkedin_list_jobs))
        .route("/api/admin/prospects/linkedin/jobs/:job_id/results", get(linkedin_fetch_results))
        .route("/api/admin/prospects", get(list_prospects))
        .route("/api/admin/prospects/:id", patch(update_prospect))
        .route("/api/admin/prospects/:id/dm-script", post(regenerate_dm_script))
        .route("/api/admin/prospects/:id/generate-outreach", post(generate_outreach_message))
        .route("/api/admin/prospects/:id", delete(delete_prospect))
        .layer(axum::middleware::from_fn(admin_middleware))
        .layer(axum::middleware::from_fn(auth_middleware))
}

/// Routes accessible to any authenticated (whitelisted) user — NOT admin-only.
/// Used by content_machine for Instagram lead generation.
pub fn instagram_routes() -> Router {
    Router::new()
        .route("/api/instagram/leads/search",                post(instagram_search_leads))
        .route("/api/instagram/leads/auto-discover",         post(instagram_auto_discover))
        .route("/api/instagram/leads/top",                   get(instagram_top_leads))
        .route("/api/instagram/leads",                       get(instagram_list_leads))
        .route("/api/instagram/leads/:id/generate-dm",       post(instagram_generate_dm))
        .route("/api/instagram/leads/:id/contact-status",    patch(instagram_update_contact_status))
        .layer(axum::middleware::from_fn(auth_middleware))
}

#[derive(Debug, Deserialize)]
struct SearchRequest {
    platform: String,      // "youtube" | "twitch"
    prospect_type: String, // "content_creator" | "clipper" | "podcaster" | "educator" | "business_owner"
    category: Option<String>,
    min_viewers: Option<i64>,
    max_viewers: Option<i64>,
    limit: Option<usize>,
}

#[derive(Debug, Deserialize)]
struct GenerateOutreachRequest {
    delivery_url: String,
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

    let base_category = payload.category.clone().unwrap_or_else(|| "general".to_string());

    // AI generates the most effective YouTube search query for this prospect type + category
    let search_query = ai_generate_youtube_query(state, &payload.prospect_type, &base_category).await;
    tracing::info!("🔍 YouTube prospect search query (AI): \"{}\"", search_query);

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

        // Extract Twitter/X handle from description
        let twitter_handle = extract_twitter_handle(&description);

        let (score, reasoning, dm_creator, dm_clipper) =
            score_prospect_with_ai(state, &display_name, sub_count, &description, &category, &payload.prospect_type).await;

        sqlx::query(
            "INSERT INTO prospects (platform, channel_id, display_name, platform_url,
             subscriber_count, content_category, channel_description, prospect_type,
             ai_score, ai_reasoning, dm_script_creator, dm_script_clipper, twitter_handle)
             VALUES ($1,$2,$3,$4,$5,$6,$7,$8,$9,$10,$11,$12,$13)
             ON CONFLICT (platform, channel_id) DO UPDATE SET
               ai_score = EXCLUDED.ai_score,
               ai_reasoning = EXCLUDED.ai_reasoning,
               dm_script_creator = EXCLUDED.dm_script_creator,
               dm_script_clipper = EXCLUDED.dm_script_clipper,
               twitter_handle = COALESCE(EXCLUDED.twitter_handle, prospects.twitter_handle),
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
        .bind(twitter_handle.as_deref())
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
    let client_id = std::env::var("TWITCH_TV_CLIENT_ID").unwrap_or_default();
    let client_secret = std::env::var("TWITCH_TV_CLIENT_SECRET").unwrap_or_default();
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

    // AI picks the best Twitch game/category to search for this prospect type
    let game_name = ai_generate_twitch_category(state, &payload.prospect_type,
        &payload.category.clone().unwrap_or_else(|| "general".to_string())).await;
    tracing::info!("🎮 Twitch prospect category (AI): \"{}\"", game_name);

    // Look up the game ID on Twitch so we can filter streams by category
    let game_id: Option<String> = if !game_name.is_empty() {
        let game_url = format!(
            "https://api.twitch.tv/helix/games?name={}",
            urlencoding::encode(&game_name)
        );
        if let Ok(resp) = client.get(&game_url)
            .header("Client-Id", &client_id)
            .header("Authorization", format!("Bearer {}", access_token))
            .send().await
        {
            if let Ok(val) = resp.json::<serde_json::Value>().await {
                val["data"].as_array()
                    .and_then(|arr| arr.first())
                    .and_then(|g| g["id"].as_str())
                    .map(String::from)
            } else { None }
        } else { None }
    } else { None };

    // Build streams URL — filter by game_id if found, otherwise top streams
    let streams_url = if let Some(ref gid) = game_id {
        format!("https://api.twitch.tv/helix/streams?first={}&game_id={}", limit.min(100), gid)
    } else {
        format!("https://api.twitch.tv/helix/streams?first={}", limit.min(100))
    };

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

/// Extract a Twitter/X handle from a channel description using simple pattern matching.
fn extract_twitter_handle(description: &str) -> Option<String> {
    // Look for twitter.com/handle or x.com/handle patterns
    for pattern in &["twitter.com/", "x.com/"] {
        if let Some(pos) = description.to_lowercase().find(pattern) {
            let after = &description[pos + pattern.len()..];
            let handle: String = after.chars()
                .take_while(|c| c.is_alphanumeric() || *c == '_')
                .collect();
            if !handle.is_empty() && handle.len() <= 50 {
                return Some(format!("@{}", handle));
            }
        }
    }
    // Look for standalone @handle pattern (only if it looks like a Twitter mention)
    // We skip this to avoid false positives — url-based extraction is sufficient
    None
}

/// Use best available LLM to score a prospect and generate DM scripts.
/// Priority: NVIDIA NIM → Gemma 4 → Gemini Flash.
/// Returns (score, reasoning, dm_creator, dm_clipper).
async fn score_prospect_with_ai(
    state: &Arc<AppState>,
    name: &str,
    audience_size: i64,
    description: &str,
    category: &str,
    prospect_type: &str,
) -> (f64, String, String, String) {
    if state.nvidia_nim_client.is_none() && state.gemma_client.is_none() && state.gemini_client.is_none() {
        return (0.5, "No LLM configured".to_string(), default_dm_creator(name), default_dm_clipper(name));
    }

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

    match generate_text_best_effort(
        state.nvidia_nim_client.as_ref(),
        state.gemma_client.as_ref(),
        state.gemini_client.as_ref(),
        &prompt,
    ).await {
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
    // Build query with parameterized binds — collect active filters first so
    // we know which $N placeholders to emit.
    let mut conditions: Vec<&str> = vec![];
    if q.prospect_type.is_some() { conditions.push("prospect_type"); }
    if q.contact_status.is_some() { conditions.push("contact_status"); }
    if q.platform.is_some()       { conditions.push("platform"); }

    let mut sql = "SELECT id, platform, channel_id, display_name, platform_url, \
                   subscriber_count, avg_viewer_count, content_category, prospect_type, \
                   ai_score, ai_reasoning, dm_script_creator, dm_script_clipper, \
                   contact_status, notes, twitter_handle, created_at \
                   FROM prospects WHERE 1=1".to_string();

    for (i, col) in conditions.iter().enumerate() {
        sql.push_str(&format!(" AND {} = ${}", col, i + 1));
    }
    sql.push_str(" ORDER BY ai_score DESC NULLS LAST, created_at DESC LIMIT 200");

    // Bind only the filter values that are present, in the same order.
    let mut query = sqlx::query(&sql);
    if let Some(ref pt) = q.prospect_type { query = query.bind(pt); }
    if let Some(ref cs) = q.contact_status { query = query.bind(cs); }
    if let Some(ref pl) = q.platform       { query = query.bind(pl); }

    let rows = query
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
        "twitter_handle": r.get::<Option<String>, _>("twitter_handle"),
        "created_at": r.get::<chrono::DateTime<chrono::Utc>, _>("created_at"),
    })).collect();

    Ok(Json(json!({ "success": true, "prospects": prospects })))
}

const VALID_CONTACT_STATUSES: &[&str] = &["new", "contacted", "interested", "deal", "rejected"];

async fn update_prospect(
    Extension(state): Extension<Arc<AppState>>,
    Path(id): Path<Uuid>,
    Json(payload): Json<UpdateProspectRequest>,
) -> Result<Json<serde_json::Value>, (StatusCode, Json<ErrorResponse>)> {
    if let Some(ref status) = payload.contact_status {
        if !VALID_CONTACT_STATUSES.contains(&status.as_str()) {
            return Err((StatusCode::BAD_REQUEST, Json(ErrorResponse {
                success: false,
                message: format!("Invalid contact_status '{}'. Must be one of: {}", status, VALID_CONTACT_STATUSES.join(", ")),
            })));
        }
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

/// Generate a personalized cold outreach DM using Gemini, given a delivery URL.
async fn generate_outreach_message(
    Extension(state): Extension<Arc<AppState>>,
    Path(id): Path<Uuid>,
    Json(payload): Json<GenerateOutreachRequest>,
) -> Result<Json<serde_json::Value>, (StatusCode, Json<ErrorResponse>)> {
    let row = sqlx::query(
        "SELECT display_name, subscriber_count, avg_viewer_count, content_category,
                prospect_type, dm_script_creator
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
    let category: String = row.get::<Option<String>, _>("content_category").unwrap_or_else(|| "content".to_string());
    let pt: String = row.get("prospect_type");
    let existing_dm: String = row.get::<Option<String>, _>("dm_script_creator").unwrap_or_default();
    let audience = subs.or(viewers).unwrap_or(0);

    if state.nvidia_nim_client.is_none() && state.gemma_client.is_none() && state.video_gemini_client.is_none() && state.gemini_client.is_none() {
        return Err((StatusCode::SERVICE_UNAVAILABLE, Json(ErrorResponse { success: false, message: "No LLM configured".to_string() })));
    }

    let audience_label = if audience >= 1_000_000 {
        format!("{:.1}M", audience as f64 / 1_000_000.0)
    } else if audience >= 1_000 {
        format!("{:.0}K", audience as f64 / 1_000.0)
    } else {
        audience.to_string()
    };

    let pitch_focus = match pt.as_str() {
        "podcaster"      => "turning podcast episodes into viral short clips for social media",
        "educator"       => "turning educational videos into animated highlight clips and Shorts",
        "business_owner" => "creating a professional product demo video or explainer for their brand",
        _                => "turning their long-form content into 30-50 viral Shorts per month",
    };

    let prompt = format!(
        r#"Write a SHORT cold outreach DM (under 90 words) from a video AI agency to {name}, a {category} {pt} with {audience} audience.

Purpose: {pitch_focus}
Include this delivery link naturally: {delivery_url}
Mention you already created a free sample for them.
Be conversational, specific to their niche, and end with a soft call-to-action.
DO NOT use emojis excessively. Return only the message text, no preamble.
Tone reference: {existing_dm}"#,
        name = name,
        category = category,
        pt = pt.replace('_', " "),
        audience = audience_label,
        pitch_focus = pitch_focus,
        delivery_url = payload.delivery_url,
        existing_dm = if existing_dm.is_empty() { "professional and friendly".to_string() } else { existing_dm.chars().take(200).collect() },
    );

    let message = match generate_text_best_effort(
        state.nvidia_nim_client.as_ref(),
        state.gemma_client.as_ref(),
        state.video_gemini_client.as_ref().or(state.gemini_client.as_ref()),
        &prompt,
    ).await {
        Ok(text) => text.trim().to_string(),
        Err(e) => return Err((StatusCode::BAD_GATEWAY, Json(ErrorResponse { success: false, message: format!("LLM error: {}", e) }))),
    };

    Ok(Json(json!({ "success": true, "outreach_message": message })))
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
          <option value="podcaster">Podcaster</option>
          <option value="educator">Educator / STEM</option>
          <option value="business_owner">Business / Brand</option>
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

  <!-- View Switcher -->
  <div style="display:flex;gap:8px;margin-bottom:16px">
    <button class="btn btn-primary" id="btn-prospects" onclick="showView('prospects')">📋 Prospects</button>
    <button class="btn" id="btn-clipgen" style="background:#0f3460;color:#9ca3af;border:1px solid #1e3a5f" onclick="showView('clipgen')">🎬 Clip Generator</button>
  </div>

  <!-- PROSPECTS VIEW -->
  <div id="view-prospects">
    <!-- Filter Tabs -->
    <div class="tabs">
      <div class="tab active" onclick="setFilter('all',this)">All</div>
      <div class="tab" onclick="setFilter('new',this)">New</div>
      <div class="tab" onclick="setFilter('contacted',this)">Contacted</div>
      <div class="tab" onclick="setFilter('replied',this)">Replied</div>
      <div class="tab" onclick="setFilter('converted',this)">Converted</div>
    </div>
    <div style="background:#16213e;border:1px solid #0f3460;border-radius:12px;overflow:hidden">
      <div id="table-area"><div class="loading">Loading prospects…</div></div>
    </div>
  </div>

  <!-- CLIP GENERATOR VIEW -->
  <div id="view-clipgen" style="display:none">
    <div class="search-card">
      <h2>🎬 Generate Demo Clips for a Prospect</h2>
      <div class="form-grid" style="margin-bottom:12px">
        <div style="grid-column:1/-1">
          <label>Select Prospect</label>
          <select id="cg-prospect" onchange="onProspectSelect(this)">
            <option value="">— select a prospect —</option>
          </select>
        </div>
        <div style="grid-column:1/-1">
          <label>Video URL (auto-filled from prospect)</label>
          <input id="cg-url" placeholder="https://youtube.com/watch?v=...">
        </div>
        <div>
          <label>Number of Clips</label>
          <input id="cg-clips" type="number" value="3" min="1" max="10">
        </div>
        <div>
          <label>Min Duration (seconds)</label>
          <input id="cg-min-dur" type="number" value="30" min="15" max="120">
        </div>
        <div>
          <label>Max Duration (seconds)</label>
          <input id="cg-max-dur" type="number" value="90" min="30" max="180">
        </div>
      </div>
      <button class="btn btn-primary" onclick="startClipJob()">🎬 Generate Demo Clips</button>
      <span id="cg-status" style="margin-left:12px;color:#9ca3af;font-size:0.85rem"></span>
    </div>

    <!-- Progress -->
    <div id="cg-progress" style="display:none;background:#16213e;border:1px solid #0f3460;border-radius:12px;padding:20px;margin-bottom:16px">
      <div style="color:#dbd8e3;margin-bottom:8px" id="cg-progress-label">Analyzing…</div>
      <div style="background:#0f3460;border-radius:4px;height:8px;overflow:hidden">
        <div id="cg-progress-bar" style="background:#5c5470;height:100%;width:0%;transition:width 0.5s"></div>
      </div>
    </div>

    <!-- Results -->
    <div id="cg-results" style="display:none;background:#16213e;border:1px solid #0f3460;border-radius:12px;padding:20px;margin-bottom:16px">
      <h3 style="color:#dbd8e3;margin-bottom:12px">Generated Clips</h3>
      <div id="cg-clips-list"></div>
      <button class="btn btn-primary" style="margin-top:16px" onclick="createDelivery()">📦 Create Delivery Package</button>
    </div>

    <!-- Delivery + Outreach -->
    <div id="cg-delivery" style="display:none;background:#16213e;border:1px solid #0f3460;border-radius:12px;padding:20px">
      <h3 style="color:#dbd8e3;margin-bottom:8px">Delivery Link</h3>
      <div style="display:flex;gap:8px;align-items:center;margin-bottom:16px">
        <input id="cg-delivery-url" readonly style="flex:1;background:#0f3460;border:1px solid #1e3a5f;border-radius:6px;padding:8px 12px;color:#dbd8e3;font-size:0.9rem">
        <button class="btn btn-sm btn-copy" onclick="copyEl('cg-delivery-url')">📋 Copy</button>
      </div>
      <button class="btn btn-primary" onclick="generateOutreach()">🤖 Generate AI Outreach Message</button>
      <div id="cg-outreach" style="display:none;margin-top:16px">
        <div class="dm-text" id="cg-outreach-text" style="max-height:none;margin-bottom:8px"></div>
        <button class="btn btn-sm btn-copy" onclick="copyEl('cg-outreach-text')">📋 Copy Message</button>
      </div>
    </div>
  </div>
</div>

<script>
const token = localStorage.getItem('auth_token') || localStorage.getItem('admin_token') || localStorage.getItem('authToken');
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
      <th>Twitter</th><th>AI Score</th><th>DM Scripts</th><th>Status</th><th>Actions</th>
    </tr></thead><tbody>`;

  for(const p of prospects){
    const dm = p.dm_script_creator||'';
    const dmClip = p.dm_script_clipper||'';
    const tw = p.twitter_handle||'';
    html += `<tr id="row-${p.id}">
      <td><a href="${p.platform_url}" target="_blank" style="color:#dbd8e3;font-weight:500">${p.display_name}</a>
        ${p.ai_reasoning?`<div style="font-size:0.75rem;color:#9ca3af;margin-top:2px">${p.ai_reasoning}</div>`:''}
      </td>
      <td>${platformIcon(p.platform)}</td>
      <td>${formatNum(p.subscriber_count||p.avg_viewer_count)}</td>
      <td style="color:#9ca3af">${p.content_category||'—'}</td>
      <td style="color:#1d9bf0;white-space:nowrap">${tw?`<a href="https://twitter.com/${tw.replace('@','')}" target="_blank" style="color:#1d9bf0">${tw}</a><button class="btn btn-sm btn-copy" style="margin-left:4px" onclick="copyText('${encodeURIComponent(tw)}')">📋</button>`:'—'}</td>
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

// ── View switcher ──────────────────────────────────────────────
function showView(v) {
  document.getElementById('view-prospects').style.display = v==='prospects'?'':'none';
  document.getElementById('view-clipgen').style.display = v==='clipgen'?'':'none';
  document.getElementById('btn-prospects').style.background = v==='prospects'?'#5c5470':'#0f3460';
  document.getElementById('btn-prospects').style.color = v==='prospects'?'#fff':'#9ca3af';
  document.getElementById('btn-clipgen').style.background = v==='clipgen'?'#5c5470':'#0f3460';
  document.getElementById('btn-clipgen').style.color = v==='clipgen'?'#fff':'#9ca3af';
  if(v==='clipgen') loadProspectsDropdown();
}

// ── Clip Generator ─────────────────────────────────────────────
let cgProspects = [];
let cgJobId = null;
let cgPollInterval = null;
let cgSelectedProspectId = null;

async function loadProspectsDropdown() {
  const res = await fetch('/api/admin/prospects?limit=200', {headers:{'Authorization':'Bearer '+token}});
  const data = await res.json();
  cgProspects = data.prospects||[];
  const sel = document.getElementById('cg-prospect');
  sel.innerHTML = '<option value="">— select a prospect —</option>' +
    cgProspects.map(p=>`<option value="${p.id}" data-url="${p.platform_url}">${p.display_name} (${p.content_category||p.platform})</option>`).join('');
}

function onProspectSelect(sel) {
  const opt = sel.options[sel.selectedIndex];
  if(opt.value) {
    cgSelectedProspectId = opt.value;
    document.getElementById('cg-url').value = opt.dataset.url||'';
  }
}

async function startClipJob() {
  const url = document.getElementById('cg-url').value.trim();
  if(!url) { showMsg('Enter a video URL', false); return; }
  document.getElementById('cg-status').textContent = 'Starting…';
  document.getElementById('cg-results').style.display='none';
  document.getElementById('cg-delivery').style.display='none';
  document.getElementById('cg-outreach').style.display='none';

  const res = await fetch('/api/manual-clipping/jobs', {
    method:'POST',
    headers:{'Authorization':'Bearer '+token,'Content-Type':'application/json'},
    body: JSON.stringify({
      video_url: url,
      clips_requested: parseInt(document.getElementById('cg-clips').value)||3,
      min_clip_duration_seconds: parseInt(document.getElementById('cg-min-dur').value)||30,
      max_clip_duration_seconds: parseInt(document.getElementById('cg-max-dur').value)||90,
    })
  });
  const data = await res.json();
  if(!data.success||!data.job_id) { showMsg(data.message||'Failed to start job', false); document.getElementById('cg-status').textContent=''; return; }

  cgJobId = data.job_id;
  document.getElementById('cg-status').textContent = '';
  document.getElementById('cg-progress').style.display='';
  startPolling();
}

function startPolling() {
  if(cgPollInterval) clearInterval(cgPollInterval);
  cgPollInterval = setInterval(pollJob, 4000);
  pollJob();
}

async function pollJob() {
  if(!cgJobId) return;
  const res = await fetch(`/api/manual-clipping/jobs/${cgJobId}`, {headers:{'Authorization':'Bearer '+token}});
  const data = await res.json();
  if(!data.job) return;
  const job = data.job;
  document.getElementById('cg-progress-label').textContent = job.status + (job.progress_percent?` — ${job.progress_percent}%`:'');
  document.getElementById('cg-progress-bar').style.width = (job.progress_percent||0)+'%';

  if(job.status==='completed') {
    clearInterval(cgPollInterval);
    document.getElementById('cg-progress').style.display='none';
    showClipResults(data.clips||[]);
  } else if(job.status==='failed') {
    clearInterval(cgPollInterval);
    document.getElementById('cg-progress').style.display='none';
    showMsg('Job failed: '+(job.error_message||'unknown'), false);
  }
}

function showClipResults(clips) {
  const list = document.getElementById('cg-clips-list');
  if(!clips.length) { list.innerHTML='<div style="color:#9ca3af">No clips generated</div>'; }
  else {
    list.innerHTML = clips.map((c,i)=>`
      <div style="display:flex;align-items:center;gap:8px;margin-bottom:8px;padding:8px;background:#0f3460;border-radius:6px">
        <span style="color:#dbd8e3;flex:1">Clip ${i+1}: ${c.title||'Clip '+(i+1)} (${Math.round(c.duration_seconds||0)}s)</span>
        ${c.r2_clip_url?`<a href="${c.r2_clip_url}" target="_blank" class="btn btn-sm btn-copy">⬇ Download</a>`:''}
        ${c.r2_clip_url?`<button class="btn btn-sm btn-copy" onclick="copyText('${encodeURIComponent(c.r2_clip_url)}')">🔗 Copy</button>`:''}
      </div>`).join('');
  }
  document.getElementById('cg-results').style.display='';
}

async function createDelivery() {
  const prospect = cgProspects.find(p=>p.id===cgSelectedProspectId);
  const title = prospect ? `Demo clips for ${prospect.display_name}` : 'Demo clip delivery';
  const res = await fetch('/api/admin/deliveries', {
    method:'POST',
    headers:{'Authorization':'Bearer '+token,'Content-Type':'application/json'},
    body: JSON.stringify({
      client_ref: prospect?.display_name||'prospect',
      title,
      gig_type: 'clips',
      prompt: `Demo clips from manual clipping job ${cgJobId}`,
      style: 'viral',
      duration: 60,
      extra_args: {job_id: cgJobId},
    })
  });
  const data = await res.json();
  if(!data.id&&!data.delivery_id) { showMsg('Failed to create delivery', false); return; }
  const id = data.id||data.delivery_id;
  const deliveryUrl = `${window.location.origin}/delivery/${id}`;
  document.getElementById('cg-delivery-url').value = deliveryUrl;
  document.getElementById('cg-delivery').style.display='';
  showMsg('Delivery package created!');
}

async function generateOutreach() {
  const deliveryUrl = document.getElementById('cg-delivery-url').value;
  if(!cgSelectedProspectId||!deliveryUrl) { showMsg('Select a prospect and create delivery first', false); return; }
  const res = await fetch(`/api/admin/prospects/${cgSelectedProspectId}/generate-outreach`, {
    method:'POST',
    headers:{'Authorization':'Bearer '+token,'Content-Type':'application/json'},
    body: JSON.stringify({delivery_url: deliveryUrl})
  });
  const data = await res.json();
  if(!data.success) { showMsg(data.message||'Failed to generate message', false); return; }
  document.getElementById('cg-outreach-text').textContent = data.outreach_message;
  document.getElementById('cg-outreach').style.display='';
}

function copyEl(id) {
  const el = document.getElementById(id);
  navigator.clipboard.writeText(el.value||el.textContent).then(()=>showMsg('Copied!'));
}
</script>
</body>
</html>
"###;

// ─── LinkedIn / PhantomBuster handlers ────────────────────────────────────────

#[derive(Debug, serde::Deserialize)]
struct LinkedInLaunchRequest {
    agent_id:       String,
    search_url:     String,
    session_cookie: String,
    max_profiles:   Option<u32>,
}

/// GET /api/admin/prospects/linkedin/agents
/// Returns PhantomBuster agents visible to this account.
async fn linkedin_list_agents(
    Extension(state): Extension<Arc<AppState>>,
) -> Json<serde_json::Value> {
    let Some(pb) = state.phantombuster_client.as_ref() else {
        return Json(json!({"success": false, "error": "PhantomBuster not configured (PHANTOMBUSTER_API_KEY missing)"}));
    };
    match pb.list_agents().await {
        Ok(agents) => Json(json!({"success": true, "agents": agents})),
        Err(e)     => Json(json!({"success": false, "error": e})),
    }
}

/// POST /api/admin/prospects/linkedin/launch
/// Launch a Sales Navigator search export phantom.
async fn linkedin_launch_search(
    Extension(state): Extension<Arc<AppState>>,
    Json(req): Json<LinkedInLaunchRequest>,
) -> Json<serde_json::Value> {
    let Some(pb) = state.phantombuster_client.as_ref() else {
        return Json(json!({"success": false, "error": "PhantomBuster not configured"}));
    };

    let max = req.max_profiles.unwrap_or(100);
    match pb.launch_agent(&req.agent_id, &req.search_url, &req.session_cookie, max).await {
        Err(e) => return Json(json!({"success": false, "error": e})),
        Ok(container_id) => {
            // Record the job in DB
            let job_id = match sqlx::query_scalar::<_, uuid::Uuid>(
                "INSERT INTO phantombuster_jobs (agent_id, search_url, status, launched_at)
                 VALUES ($1, $2, 'running', NOW()) RETURNING id"
            )
            .bind(&req.agent_id)
            .bind(&req.search_url)
            .fetch_one(&state.db_pool)
            .await {
                Ok(id) => id,
                Err(e) => return Json(json!({"success": false, "error": format!("DB error: {}", e)})),
            };

            Json(json!({
                "success": true,
                "job_id": job_id.to_string(),
                "container_id": container_id,
                "message": format!("Phantom launched. Poll /api/admin/prospects/linkedin/jobs/{}/results to fetch leads when complete.", job_id)
            }))
        }
    }
}

/// GET /api/admin/prospects/linkedin/jobs
async fn linkedin_list_jobs(
    Extension(state): Extension<Arc<AppState>>,
) -> Json<serde_json::Value> {
    let rows = sqlx::query(
        "SELECT id, agent_id, search_url, status, leads_found, error, launched_at, completed_at
         FROM phantombuster_jobs ORDER BY created_at DESC LIMIT 50"
    )
    .fetch_all(&state.db_pool)
    .await
    .unwrap_or_default();

    let jobs: Vec<serde_json::Value> = rows.iter().map(|r| json!({
        "id":           r.try_get::<uuid::Uuid,_>("id").map(|u| u.to_string()).unwrap_or_default(),
        "agent_id":     r.try_get::<String,_>("agent_id").unwrap_or_default(),
        "search_url":   r.try_get::<Option<String>,_>("search_url").unwrap_or_default(),
        "status":       r.try_get::<String,_>("status").unwrap_or_default(),
        "leads_found":  r.try_get::<Option<i32>,_>("leads_found").unwrap_or_default(),
        "error":        r.try_get::<Option<String>,_>("error").unwrap_or_default(),
        "launched_at":  r.try_get::<Option<chrono::DateTime<chrono::Utc>>,_>("launched_at").ok().flatten().map(|t| t.to_rfc3339()),
        "completed_at": r.try_get::<Option<chrono::DateTime<chrono::Utc>>,_>("completed_at").ok().flatten().map(|t| t.to_rfc3339()),
    })).collect();

    Json(json!({"success": true, "jobs": jobs}))
}

/// GET /api/admin/prospects/linkedin/jobs/:job_id/results
/// Fetches output from PhantomBuster and imports leads into prospects table.
async fn linkedin_fetch_results(
    Extension(state): Extension<Arc<AppState>>,
    axum::extract::Path(job_id): axum::extract::Path<uuid::Uuid>,
) -> Json<serde_json::Value> {
    let Some(pb) = state.phantombuster_client.as_ref() else {
        return Json(json!({"success": false, "error": "PhantomBuster not configured"}));
    };

    // Get agent_id from DB
    let agent_id: String = match sqlx::query_scalar(
        "SELECT agent_id FROM phantombuster_jobs WHERE id = $1"
    )
    .bind(job_id)
    .fetch_optional(&state.db_pool)
    .await {
        Ok(Some(id)) => id,
        _ => return Json(json!({"success": false, "error": "Job not found"})),
    };

    // Fetch output
    let rows = match pb.fetch_output(&agent_id).await {
        Ok(r) => r,
        Err(e) => return Json(json!({"success": false, "error": e})),
    };

    // If PB returned no rows, confirm whether the run actually succeeded or
    // errored — otherwise the DB row stays 'running' forever and the caller
    // has no idea the phantom failed.
    if rows.is_empty() {
        if let Ok((errored, log_tail)) = pb.fetch_run_error(&agent_id).await {
            if errored {
                let err_msg = log_tail.unwrap_or_else(|| "PhantomBuster script exited with error".to_string());
                let _ = sqlx::query(
                    "UPDATE phantombuster_jobs SET status = 'failed', error = $1, completed_at = NOW() WHERE id = $2"
                )
                .bind(&err_msg)
                .bind(job_id)
                .execute(&state.db_pool)
                .await;
                return Json(json!({
                    "success": false,
                    "error": err_msg,
                    "message": "PhantomBuster run failed — see error field. Job marked failed."
                }));
            }
        }
    }

    let leads = crate::phantombuster_client::PhantomBusterClient::parse_leads(rows);
    let count = leads.len();

    // Upsert into prospects table
    let mut imported = 0usize;
    for lead in &leads {
        let linkedin_id = lead.linkedin_url.clone()
            .unwrap_or_else(|| format!("li_{}", lead.full_name.to_lowercase().replace(' ', "_")));

        let result = sqlx::query(
            "INSERT INTO prospects
               (platform, channel_id, display_name, platform_url, prospect_type,
                linkedin_url, job_title, company_name, company_size, seniority_level, email, contact_status)
             VALUES ('linkedin', $1, $2, $3, 'linkedin_lead', $4, $5, $6, $7, $8, $9, 'new')
             ON CONFLICT (platform, channel_id) DO UPDATE
               SET job_title = EXCLUDED.job_title,
                   company_name = EXCLUDED.company_name,
                   updated_at = NOW()"
        )
        .bind(&linkedin_id)
        .bind(&lead.full_name)
        .bind(lead.linkedin_url.as_deref().unwrap_or(""))
        .bind(&lead.linkedin_url)
        .bind(&lead.job_title)
        .bind(&lead.company_name)
        .bind(&lead.company_size)
        .bind(&lead.seniority)
        .bind(&lead.email)
        .execute(&state.db_pool)
        .await;

        if result.is_ok() { imported += 1; }
    }

    // Update job record
    let _ = sqlx::query(
        "UPDATE phantombuster_jobs SET status = 'completed', leads_found = $1, completed_at = NOW() WHERE id = $2"
    )
    .bind(imported as i32)
    .bind(job_id)
    .execute(&state.db_pool)
    .await;

    // Score imported leads with AI in the background (non-blocking)
    if imported > 0 {
        let state_clone = state.clone();
        let job_id_clone = job_id;
        tokio::spawn(async move {
            ai_score_linkedin_leads(&state_clone, job_id_clone).await;
        });
    }

    Json(json!({
        "success": true,
        "total_fetched": count,
        "imported_to_prospects": imported,
        "message": format!("{} LinkedIn leads imported. AI scoring started in background — check /admin/prospect-finder in ~1 minute.", imported)
    }))
}

// ─── Smart search: build URL from filters + auto-launch ───────────────────────

#[derive(Debug, serde::Deserialize)]
struct SmartSearchRequest {
    /// Plain-English description — AI will generate all filters from this.
    /// e.g. "YouTubers and podcasters in the US with 10k+ followers who make money from content"
    /// If provided, all filter fields below are IGNORED and AI generates them.
    description:   Option<String>,
    /// e.g. ["YouTuber", "Podcast Host", "Content Creator", "Marketing Manager"]
    job_titles:    Option<Vec<String>>,
    /// e.g. ["Online Media", "E-Learning", "Marketing and Advertising"]
    industries:    Option<Vec<String>>,
    /// e.g. ["1-10", "11-50"] or LinkedIn codes ["A", "B"]
    company_sizes: Option<Vec<String>>,
    /// e.g. ["United States", "United Kingdom"]
    locations:     Option<Vec<String>>,
    /// e.g. ["OWNER", "CXO", "VP", "DIRECTOR", "MANAGER"]
    seniority:     Option<Vec<String>>,
    /// Max profiles to scrape (default 100)
    max_profiles:  Option<u32>,
    /// Optional: use a saved Sales Navigator list URL instead of building a search URL.
    list_url:      Option<String>,
}

/// POST /api/admin/prospects/linkedin/search
/// Builds a Sales Navigator search URL from filters and immediately launches
/// a PhantomBuster export. No manual URL construction needed.
///
/// Example body:
/// {
///   "job_titles": ["YouTuber", "Content Creator", "Podcast Host"],
///   "company_sizes": ["1-10", "11-50"],
///   "locations": ["United States", "United Kingdom"],
///   "seniority": ["OWNER", "CXO", "DIRECTOR"],
///   "max_profiles": 150
/// }
async fn linkedin_smart_search(
    Extension(state): Extension<Arc<AppState>>,
    Json(req): Json<SmartSearchRequest>,
) -> Json<serde_json::Value> {
    let Some(pb) = state.phantombuster_client.as_ref() else {
        return Json(json!({"success": false, "error": "PhantomBuster not configured (PHANTOMBUSTER_API_KEY missing)"}));
    };

    // Read session cookie from env
    let session_cookie = match std::env::var("LINKEDIN_SESSION_COOKIE") {
        Ok(c) if !c.is_empty() => c,
        _ => return Json(json!({"success": false, "error": "LINKEDIN_SESSION_COOKIE not set in environment"})),
    };

    let max = req.max_profiles.unwrap_or(100);

    // If a plain-English description is given, use AI to generate all filter params
    let req = if let Some(ref desc) = req.description.clone() {
        match ai_generate_linkedin_filters(&state, desc).await {
            Ok(ai_req) => {
                tracing::info!("🤖 LinkedIn AI filters for \"{}\": {:?} / {:?} / {:?}",
                    desc, ai_req.job_titles, ai_req.industries, ai_req.locations);
                ai_req
            }
            Err(e) => {
                tracing::warn!("LinkedIn AI filter generation failed: {} — using raw request", e);
                req
            }
        }
    } else {
        req
    };

    // Branch: list_url → List Export phantom; filters → Search Export phantom
    let (agent, search_url, container_id) = if let Some(list_url) = req.list_url.as_deref() {
        // Use saved Sales Navigator list
        let agent = match pb.find_list_export_agent().await {
            Ok(Some(a)) => a,
            Ok(None)    => return Json(json!({"success": false, "error": "No Sales Navigator List Export phantom found. Add it from the PhantomBuster Phantom Store."})),
            Err(e)      => return Json(json!({"success": false, "error": format!("Failed to list agents: {}", e)})),
        };
        let cid = match pb.launch_list_export(&agent.id, list_url, &session_cookie, max).await {
            Ok(id) => id,
            Err(e) => return Json(json!({"success": false, "error": e})),
        };
        (agent, list_url.to_string(), cid)
    } else {
        // Build search URL from filters and use Search Export phantom
        let agent = match pb.find_sales_nav_agent().await {
            Ok(Some(a)) => a,
            Ok(None)    => return Json(json!({"success": false, "error": "No Sales Navigator Search Export phantom found. Add 'LinkedIn Sales Navigator Search Export' from the PhantomBuster Phantom Store."})),
            Err(e)      => return Json(json!({"success": false, "error": format!("Failed to list agents: {}", e)})),
        };
        let empty = vec![];
        let url = crate::phantombuster_client::PhantomBusterClient::build_search_url(
            req.job_titles.as_deref().unwrap_or(&empty),
            req.industries.as_deref().unwrap_or(&empty),
            req.company_sizes.as_deref().unwrap_or(&empty),
            req.locations.as_deref().unwrap_or(&empty),
            req.seniority.as_deref().unwrap_or(&empty),
        );
        tracing::info!("LinkedIn smart search URL: {}", url);
        let cid = match pb.launch_agent(&agent.id, &url, &session_cookie, max).await {
            Ok(id) => id,
            Err(e) => return Json(json!({"success": false, "error": e})),
        };
        (agent, url, cid)
    };

    // Record in DB
    let job_id = match sqlx::query_scalar::<_, uuid::Uuid>(
        "INSERT INTO phantombuster_jobs (agent_id, agent_name, search_url, status, launched_at)
         VALUES ($1, $2, $3, 'running', NOW()) RETURNING id"
    )
    .bind(&agent.id)
    .bind(&agent.name)
    .bind(&search_url)
    .fetch_one(&state.db_pool)
    .await {
        Ok(id) => id,
        Err(e) => return Json(json!({"success": false, "error": format!("DB insert failed: {}", e)})),
    };

    Json(json!({
        "success":      true,
        "job_id":       job_id.to_string(),
        "container_id": container_id,
        "agent_name":   agent.name,
        "search_url":   search_url,
        "max_profiles": max,
        "message":      format!(
            "PhantomBuster launched with {} filters. Poll GET /api/admin/prospects/linkedin/jobs/{}/results in ~5 minutes to import leads.",
            req.job_titles.as_ref().map(|t| t.len()).unwrap_or(0) +
            req.industries.as_ref().map(|t| t.len()).unwrap_or(0) +
            req.locations.as_ref().map(|t| t.len()).unwrap_or(0),
            job_id
        )
    }))
}

// ============================================================================
// Instagram lead generation — accessible to all whitelisted users
// ============================================================================

#[derive(Debug, Deserialize)]
struct InstagramSearchRequest {
    hashtag:   String,           // e.g. "contentcreator" or "#videographer"
    max_posts: Option<u32>,      // default 50, max 200
    category:  Option<String>,   // label for this search (stored on leads)
}

#[derive(Debug, Deserialize)]
struct InstagramDmRequest {
    niche: Option<String>,
}

#[derive(Debug, Deserialize)]
struct InstagramContactStatusRequest {
    contact_status: String, // new | contacted | replied | converted | skipped
}

#[derive(Debug, Deserialize)]
struct InstagramListQuery {
    hashtag:        Option<String>,
    contact_status: Option<String>,
    min_followers:  Option<i64>,
    limit:          Option<i64>,
    offset:         Option<i64>,
}

/// POST /api/instagram/leads/search
/// Launches a PhantomBuster Instagram Hashtag Search and records leads.
async fn instagram_search_leads(
    Extension(state): Extension<Arc<AppState>>,
    Json(req):        Json<InstagramSearchRequest>,
) -> Json<serde_json::Value> {
    let Some(pb) = state.phantombuster_client.as_ref() else {
        return Json(json!({"success": false, "error": "PhantomBuster not configured (PHANTOMBUSTER_API_KEY missing)"}));
    };

    let session_cookie = match std::env::var("INSTAGRAM_SESSION_COOKIE") {
        Ok(c) if !c.is_empty() => c,
        _ => return Json(json!({"success": false, "error": "INSTAGRAM_SESSION_COOKIE not set. Add your Instagram sessionid cookie to the server env vars."})),
    };

    let agent = match pb.find_instagram_hashtag_agent().await {
        Ok(Some(a)) => a,
        Ok(None)    => return Json(json!({"success": false, "error": "No Instagram Hashtag phantom found. Add 'Instagram Hashtag Search Export' from the PhantomBuster Phantom Store."})),
        Err(e)      => return Json(json!({"success": false, "error": format!("Failed to list PhantomBuster agents: {}", e)})),
    };

    let max_posts = req.max_posts.unwrap_or(50).min(200);
    let hashtag   = req.hashtag.trim_start_matches('#').to_string();
    let category  = req.category.clone().unwrap_or_else(|| hashtag.clone());

    let container_id = match pb.launch_instagram_hashtag_search(
        &agent.id, &session_cookie, &hashtag, max_posts,
    ).await {
        Ok(id) => id,
        Err(e) => return Json(json!({"success": false, "error": e})),
    };

    // Record the PB job
    let job_id = match sqlx::query_scalar::<_, uuid::Uuid>(
        "INSERT INTO phantombuster_jobs (agent_id, agent_name, search_url, status, launched_at)
         VALUES ($1, $2, $3, 'running', NOW()) RETURNING id"
    )
    .bind(&agent.id)
    .bind(&agent.name)
    .bind(format!("instagram:#{}", hashtag))
    .fetch_one(&state.db_pool)
    .await {
        Ok(id) => id,
        Err(e) => return Json(json!({"success": false, "error": format!("DB insert failed: {}", e)})),
    };

    Json(json!({
        "success":      true,
        "job_id":       job_id.to_string(),
        "container_id": container_id,
        "agent_name":   agent.name,
        "hashtag":      hashtag,
        "category":     category,
        "max_posts":    max_posts,
        "message":      format!(
            "PhantomBuster Instagram Hashtag Search launched for #{}. Results typically ready in 3–10 minutes. Job ID: {}",
            hashtag, job_id
        )
    }))
}

/// GET /api/instagram/leads
/// List Instagram leads with optional filtering.
async fn instagram_list_leads(
    Extension(state): Extension<Arc<AppState>>,
    Query(q):         Query<InstagramListQuery>,
) -> Json<serde_json::Value> {
    let limit  = q.limit.unwrap_or(50).min(200);
    let offset = q.offset.unwrap_or(0);

    let mut sql = String::from(
        "SELECT id, username, full_name, bio, followers_count, following_count, posts_count,
                profile_url, profile_pic_url, is_private, is_verified, category,
                hashtag_source, email, external_url, dm_script, contact_status,
                pb_job_id, created_at
         FROM instagram_leads WHERE 1=1"
    );
    let mut binds: Vec<String> = Vec::new();

    if let Some(ref ht) = q.hashtag {
        binds.push(ht.trim_start_matches('#').to_string());
        sql.push_str(&format!(" AND hashtag_source = ${}", binds.len()));
    }
    if let Some(ref cs) = q.contact_status {
        binds.push(cs.clone());
        sql.push_str(&format!(" AND contact_status = ${}", binds.len()));
    }
    if let Some(mf) = q.min_followers {
        binds.push(mf.to_string());
        sql.push_str(&format!(" AND followers_count >= ${}::bigint", binds.len()));
    }
    sql.push_str(" ORDER BY followers_count DESC NULLS LAST");
    sql.push_str(&format!(" LIMIT {} OFFSET {}", limit, offset));

    // Build and execute dynamically — use raw query for variable bind count
    let mut query = sqlx::query(&sql);
    for b in &binds {
        query = query.bind(b);
    }

    let rows = match query.fetch_all(&state.db_pool).await {
        Ok(r) => r,
        Err(e) => return Json(json!({"success": false, "error": format!("DB query failed: {}", e)})),
    };

    let leads: Vec<serde_json::Value> = rows.iter().map(|r| {
        json!({
            "id":              r.get::<Option<uuid::Uuid>, _>("id").map(|u| u.to_string()),
            "username":        r.get::<Option<String>, _>("username"),
            "full_name":       r.get::<Option<String>, _>("full_name"),
            "bio":             r.get::<Option<String>, _>("bio"),
            "followers_count": r.get::<Option<i64>, _>("followers_count"),
            "profile_url":     r.get::<Option<String>, _>("profile_url"),
            "profile_pic_url": r.get::<Option<String>, _>("profile_pic_url"),
            "is_verified":     r.get::<Option<bool>, _>("is_verified").unwrap_or(false),
            "category":        r.get::<Option<String>, _>("category"),
            "hashtag_source":  r.get::<Option<String>, _>("hashtag_source"),
            "email":           r.get::<Option<String>, _>("email"),
            "external_url":    r.get::<Option<String>, _>("external_url"),
            "dm_script":       r.get::<Option<String>, _>("dm_script"),
            "contact_status":  r.get::<Option<String>, _>("contact_status"),
        })
    }).collect();

    Json(json!({"success": true, "leads": leads, "count": leads.len()}))
}

/// POST /api/instagram/leads/:id/generate-dm
/// Generate a personalized Instagram cold DM script using AI.
async fn instagram_generate_dm(
    Extension(state): Extension<Arc<AppState>>,
    Path(id):         Path<uuid::Uuid>,
    Json(req):        Json<Option<InstagramDmRequest>>,
) -> Json<serde_json::Value> {
    // Fetch the lead
    let row = match sqlx::query(
        "SELECT username, full_name, bio, followers_count, category, hashtag_source, external_url
         FROM instagram_leads WHERE id = $1"
    )
    .bind(id)
    .fetch_optional(&state.db_pool)
    .await {
        Ok(Some(r)) => r,
        Ok(None)    => return Json(json!({"success": false, "error": "Lead not found"})),
        Err(e)      => return Json(json!({"success": false, "error": format!("DB error: {}", e)})),
    };

    let username:  String = row.get::<Option<String>, _>("username").unwrap_or_default();
    let full_name: String = row.get::<Option<String>, _>("full_name").unwrap_or_else(|| username.clone());
    let bio:       String = row.get::<Option<String>, _>("bio").unwrap_or_default();
    let followers: i64    = row.get::<Option<i64>, _>("followers_count").unwrap_or(0);
    let category:  String = row.get::<Option<String>, _>("category").unwrap_or_default();
    let ext_url:   String = row.get::<Option<String>, _>("external_url").unwrap_or_default();

    let niche = req.as_ref().and_then(|r| r.niche.as_deref()).unwrap_or(&category);

    let prompt = format!(
        r#"Write a short, personalized Instagram DM (under 120 words) from a video clipping service to a content creator.

Creator info:
- Instagram handle: @{username}
- Name: {full_name}
- Followers: {followers}
- Bio: {bio}
- Niche/category: {niche}
- Link in bio: {ext_url}

The DM should:
1. Be personal and reference something specific from their bio or niche
2. Mention that you already made a free demo clip of their content
3. Offer 30–50 short clips per month for $297–$497/month
4. End with a clear call to action (reply to see the demo)
5. Sound natural, NOT salesy or copy-paste generic

Output ONLY the DM message text, no labels or quotes."#,
        username  = username,
        full_name = full_name,
        followers = followers,
        bio       = bio,
        niche     = niche,
        ext_url   = ext_url,
    );

    let dm_text = match generate_text_best_effort(
        state.nvidia_nim_client.as_ref(),
        state.gemma_client.as_ref(),
        state.gemini_client.as_ref(),
        &prompt,
    ).await {
        Ok(t)  => t,
        Err(e) => return Json(json!({"success": false, "error": format!("AI generation failed: {}", e)})),
    };

    // Save the DM script to the DB
    let _ = sqlx::query(
        "UPDATE instagram_leads SET dm_script = $1, updated_at = NOW() WHERE id = $2"
    )
    .bind(&dm_text)
    .bind(id)
    .execute(&state.db_pool)
    .await;

    Json(json!({"success": true, "dm_script": dm_text}))
}

/// PATCH /api/instagram/leads/:id/contact-status
async fn instagram_update_contact_status(
    Extension(state): Extension<Arc<AppState>>,
    Path(id):         Path<uuid::Uuid>,
    Json(req):        Json<InstagramContactStatusRequest>,
) -> Json<serde_json::Value> {
    let valid = ["new", "contacted", "replied", "converted", "skipped"];
    if !valid.contains(&req.contact_status.as_str()) {
        return Json(json!({"success": false, "error": "contact_status must be one of: new, contacted, replied, converted, skipped"}));
    }

    match sqlx::query(
        "UPDATE instagram_leads SET contact_status = $1, updated_at = NOW() WHERE id = $2"
    )
    .bind(&req.contact_status)
    .bind(id)
    .execute(&state.db_pool)
    .await {
        Ok(_)  => Json(json!({"success": true})),
        Err(e) => Json(json!({"success": false, "error": format!("DB error: {}", e)})),
    }
}

// ============================================================================
// Instagram auto-discovery — AI-driven hashtag selection + lead scoring
// ============================================================================

#[derive(Debug, Deserialize)]
struct AutoDiscoverRequest {
    /// e.g. "content_creator", "youtuber", "podcaster", "fitness", "gaming"
    niche:                 Option<String>,
    /// Max profiles to pull per hashtag (default 30, max 100)
    max_posts_per_hashtag: Option<u32>,
    /// How many hashtags to search (default 3, max 6)
    hashtag_count:         Option<usize>,
}

/// POST /api/instagram/leads/auto-discover
///
/// AI picks the best hashtags for the niche, then launches one PhantomBuster
/// search per hashtag. The background poller (`poll_instagram_jobs`) will
/// auto-import results and score leads once PhantomBuster finishes (~5–10 min).
async fn instagram_auto_discover(
    Extension(state): Extension<Arc<AppState>>,
    Json(req):        Json<Option<AutoDiscoverRequest>>,
) -> Json<serde_json::Value> {
    let req          = req.unwrap_or(AutoDiscoverRequest { niche: None, max_posts_per_hashtag: None, hashtag_count: None });
    let niche        = req.niche.as_deref().unwrap_or("content creator");
    let max_posts    = req.max_posts_per_hashtag.unwrap_or(30).min(100);
    let hashtag_count = req.hashtag_count.unwrap_or(3).min(6).max(1);

    let Some(pb) = state.phantombuster_client.as_ref() else {
        return Json(json!({"success": false, "error": "PhantomBuster not configured"}));
    };

    let session_cookie = match std::env::var("INSTAGRAM_SESSION_COOKIE") {
        Ok(c) if !c.is_empty() => c,
        _ => return Json(json!({"success": false, "error": "INSTAGRAM_SESSION_COOKIE not set"})),
    };

    let agent = match pb.find_instagram_hashtag_agent().await {
        Ok(Some(a)) => a,
        Ok(None)    => return Json(json!({"success": false, "error": "No Instagram Hashtag phantom found in PhantomBuster. Add 'Instagram Hashtag Search Export' from the Phantom Store."})),
        Err(e)      => return Json(json!({"success": false, "error": format!("Agent lookup failed: {}", e)})),
    };

    // ── Ask AI to pick the best hashtags for this niche ──────────────────────
    let prompt = format!(
        r#"You are helping a video clipping agency find potential clients on Instagram.

The agency creates short-form video clips (YouTube Shorts, TikTok clips, Reels) for content creators, podcasters, YouTubers, and online educators.

Target niche: "{niche}"

List {count} Instagram hashtags (no # symbol) where the ideal potential clients would be posting their own content.
Choose hashtags that:
1. Content creators in this niche actually use when posting
2. Have active, public-posting creators (NOT just audience/fans tags)
3. Will surface creators with 5K–500K followers who need help repurposing content

Return ONLY a JSON array of strings. No explanation. Example: ["youtuber", "contentcreator", "videomarketing"]"#,
        niche = niche,
        count = hashtag_count,
    );

    let hashtags_json = match generate_text_best_effort(
        state.nvidia_nim_client.as_ref(),
        state.gemma_client.as_ref(),
        state.gemini_client.as_ref(),
        &prompt,
    ).await {
        Ok(t) => t,
        Err(e) => return Json(json!({"success": false, "error": format!("AI hashtag selection failed: {}", e)})),
    };

    // Strip markdown fences and parse JSON array
    let cleaned = hashtags_json.trim()
        .trim_start_matches("```json").trim_start_matches("```")
        .trim_end_matches("```").trim();

    let hashtags: Vec<String> = match serde_json::from_str(cleaned) {
        Ok(v) => v,
        Err(e) => {
            tracing::warn!("Failed to parse AI hashtag response: {} | raw: {}", e, cleaned);
            // Fallback hashtags based on niche keyword
            vec![niche.replace(' ', ""), "contentcreator".to_string(), "youtuber".to_string()]
        }
    };

    tracing::info!("🎯 Instagram auto-discover for niche '{}': hashtags = {:?}", niche, hashtags);

    // ── Launch a PB search for each hashtag ──────────────────────────────────
    let mut launched_jobs: Vec<serde_json::Value> = Vec::new();
    let mut errors: Vec<String> = Vec::new();

    for hashtag in &hashtags {
        let tag = hashtag.trim_start_matches('#').to_string();
        match pb.launch_instagram_hashtag_search(&agent.id, &session_cookie, &tag, max_posts).await {
            Ok(container_id) => {
                // Record in DB
                let job_insert = sqlx::query_scalar::<_, uuid::Uuid>(
                    "INSERT INTO phantombuster_jobs (agent_id, agent_name, search_url, status, launched_at)
                     VALUES ($1, $2, $3, 'running', NOW()) RETURNING id"
                )
                .bind(&agent.id)
                .bind(&agent.name)
                .bind(format!("instagram:#{}", tag))
                .fetch_one(&state.db_pool)
                .await;

                match job_insert {
                    Ok(job_id) => {
                        launched_jobs.push(json!({
                            "job_id":       job_id.to_string(),
                            "container_id": container_id,
                            "hashtag":      tag,
                        }));
                    }
                    Err(e) => errors.push(format!("#{}: DB insert failed: {}", tag, e)),
                }
            }
            Err(e) => errors.push(format!("#{}: {}", tag, e)),
        }
        // Small delay between launches to avoid PB rate limits
        tokio::time::sleep(tokio::time::Duration::from_millis(500)).await;
    }

    Json(json!({
        "success":     !launched_jobs.is_empty(),
        "niche":       niche,
        "hashtags":    hashtags,
        "jobs":        launched_jobs,
        "errors":      errors,
        "message":     format!(
            "Auto-discovery launched {} PB searches for niche '{}'. Results auto-import in ~5–10 minutes. Top leads will be scored automatically.",
            launched_jobs.len(), niche
        )
    }))
}

/// GET /api/instagram/leads/top
/// Returns the highest-scored leads (score >= 60), ready for outreach.
async fn instagram_top_leads(
    Extension(state): Extension<Arc<AppState>>,
) -> Json<serde_json::Value> {
    let rows = match sqlx::query(
        "SELECT id, username, full_name, bio, followers_count, profile_url, profile_pic_url,
                is_verified, category, hashtag_source, email, external_url,
                dm_script, contact_status, score, score_reason
         FROM instagram_leads
         WHERE score >= 60
           AND contact_status = 'new'
           AND is_private = FALSE
         ORDER BY score DESC, followers_count DESC
         LIMIT 50"
    )
    .fetch_all(&state.db_pool)
    .await {
        Ok(r) => r,
        Err(e) => return Json(json!({"success": false, "error": format!("DB error: {}", e)})),
    };

    let leads: Vec<serde_json::Value> = rows.iter().map(|r| {
        json!({
            "id":              r.get::<Option<uuid::Uuid>, _>("id").map(|u| u.to_string()),
            "username":        r.get::<Option<String>, _>("username"),
            "full_name":       r.get::<Option<String>, _>("full_name"),
            "bio":             r.get::<Option<String>, _>("bio"),
            "followers_count": r.get::<Option<i64>, _>("followers_count"),
            "profile_url":     r.get::<Option<String>, _>("profile_url"),
            "profile_pic_url": r.get::<Option<String>, _>("profile_pic_url"),
            "is_verified":     r.get::<Option<bool>, _>("is_verified").unwrap_or(false),
            "category":        r.get::<Option<String>, _>("category"),
            "hashtag_source":  r.get::<Option<String>, _>("hashtag_source"),
            "email":           r.get::<Option<String>, _>("email"),
            "external_url":    r.get::<Option<String>, _>("external_url"),
            "dm_script":       r.get::<Option<String>, _>("dm_script"),
            "contact_status":  r.get::<Option<String>, _>("contact_status"),
            "score":           r.get::<Option<i32>, _>("score"),
            "score_reason":    r.get::<Option<String>, _>("score_reason"),
        })
    }).collect();

    Json(json!({"success": true, "leads": leads, "count": leads.len()}))
}

// ============================================================================
// Background poller — auto-imports PhantomBuster Instagram results
// ============================================================================

/// Called by the background task every 5 minutes.
/// Checks running Instagram PB jobs, imports new leads, and scores them.
pub async fn poll_instagram_jobs(state: &Arc<AppState>) {
    let pb = match state.phantombuster_client.as_ref() {
        Some(pb) => pb,
        None     => return,
    };

    // Find running Instagram jobs older than 3 minutes (give PB time to finish)
    let running_jobs = match sqlx::query(
        "SELECT id, agent_id, search_url
         FROM phantombuster_jobs
         WHERE status = 'running'
           AND search_url LIKE 'instagram:%'
           AND launched_at < NOW() - INTERVAL '3 minutes'
         ORDER BY launched_at ASC
         LIMIT 10"
    )
    .fetch_all(&state.db_pool)
    .await {
        Ok(rows) => rows,
        Err(e) => {
            tracing::warn!("Instagram job poll: DB query failed: {}", e);
            return;
        }
    };

    if running_jobs.is_empty() { return; }

    tracing::info!("🔄 Instagram job poller: checking {} running jobs", running_jobs.len());

    for row in running_jobs {
        let job_id:   uuid::Uuid = row.get("id");
        let agent_id: String     = row.get("agent_id");
        let search_url: String   = row.get("search_url");

        // Extract hashtag from search_url ("instagram:#contentcreator")
        let hashtag_source = search_url
            .trim_start_matches("instagram:#")
            .trim_start_matches("instagram:")
            .trim_start_matches('#')
            .to_string();

        // Fetch PB output
        let rows = match pb.fetch_output(&agent_id).await {
            Ok(r) if !r.is_empty() => r,
            Ok(_) => {
                // Empty output could mean "still running" OR "phantom errored with
                // no rows produced". Ask PB directly before assuming still running.
                if let Ok((errored, log_tail)) = pb.fetch_run_error(&agent_id).await {
                    if errored {
                        let err_msg = log_tail.unwrap_or_else(|| "PhantomBuster script exited with error".to_string());
                        tracing::warn!("Instagram job {}: PB reported error → marking failed: {}", job_id, err_msg);
                        let _ = sqlx::query(
                            "UPDATE phantombuster_jobs SET status = 'failed', error = $1, completed_at = NOW() WHERE id = $2"
                        )
                        .bind(&err_msg)
                        .bind(job_id)
                        .execute(&state.db_pool)
                        .await;
                        continue;
                    }
                }
                tracing::debug!("Instagram job {}: no output yet, will retry", job_id);
                continue;
            }
            Err(e) => {
                tracing::warn!("Instagram job {}: fetch_output failed: {}", job_id, e);
                // Mark as failed if we've been trying for > 30 minutes
                let _ = sqlx::query(
                    "UPDATE phantombuster_jobs SET status = 'failed', error = $1
                     WHERE id = $2 AND launched_at < NOW() - INTERVAL '30 minutes'"
                )
                .bind(e)
                .bind(job_id)
                .execute(&state.db_pool)
                .await;
                continue;
            }
        };

        let leads = crate::phantombuster_client::PhantomBusterClient::parse_instagram_leads(rows);
        let total  = leads.len();
        let mut imported = 0usize;

        for lead in &leads {
            // Skip private accounts — they can't receive DMs from non-followers
            if lead.is_private { continue; }
            // Skip micro-nano accounts unlikely to pay for clipping
            if lead.followers_count.unwrap_or(0) < 1_000 { continue; }

            let result = sqlx::query(
                "INSERT INTO instagram_leads
                    (username, full_name, bio, followers_count, following_count, posts_count,
                     profile_url, profile_pic_url, is_private, is_verified, external_url, email,
                     category, hashtag_source, contact_status)
                 VALUES ($1,$2,$3,$4,$5,$6,$7,$8,$9,$10,$11,$12,$13,$14,'new')
                 ON CONFLICT (username) DO UPDATE
                   SET followers_count = EXCLUDED.followers_count,
                       bio = COALESCE(EXCLUDED.bio, instagram_leads.bio),
                       updated_at = NOW()"
            )
            .bind(&lead.username)
            .bind(&lead.full_name)
            .bind(&lead.bio)
            .bind(lead.followers_count)
            .bind(lead.following_count)
            .bind(lead.posts_count)
            .bind(&lead.profile_url)
            .bind(&lead.profile_pic_url)
            .bind(lead.is_private)
            .bind(lead.is_verified)
            .bind(&lead.external_url)
            .bind(&lead.email)
            .bind(&hashtag_source)
            .bind(&hashtag_source)
            .execute(&state.db_pool)
            .await;

            if result.is_ok() { imported += 1; }
        }

        tracing::info!(
            "✅ Instagram job {}: {} total / {} imported (hashtag: #{})",
            job_id, total, imported, hashtag_source
        );

        // Mark job completed
        let _ = sqlx::query(
            "UPDATE phantombuster_jobs SET status='completed', leads_found=$1, completed_at=NOW() WHERE id=$2"
        )
        .bind(imported as i32)
        .bind(job_id)
        .execute(&state.db_pool)
        .await;

        // Score unscored leads (top 20 by followers to conserve LLM quota)
        if state.gemini_client.is_some() || state.nvidia_nim_client.is_some() {
            score_instagram_leads(state, &hashtag_source).await;
        }
    }
}

/// Score unscored Instagram leads for a given hashtag using AI.
async fn score_instagram_leads(state: &Arc<AppState>, hashtag: &str) {
    let unscored = match sqlx::query(
        "SELECT id, username, full_name, bio, followers_count, external_url
         FROM instagram_leads
         WHERE score IS NULL
           AND hashtag_source = $1
           AND is_private = FALSE
           AND followers_count >= 1000
         ORDER BY followers_count DESC
         LIMIT 20"
    )
    .bind(hashtag)
    .fetch_all(&state.db_pool)
    .await {
        Ok(r) => r,
        Err(_) => return,
    };

    for row in &unscored {
        let id:       uuid::Uuid = row.get("id");
        let username: String     = row.get::<Option<String>, _>("username").unwrap_or_default();
        let bio:      String     = row.get::<Option<String>, _>("bio").unwrap_or_default();
        let followers: i64       = row.get::<Option<i64>, _>("followers_count").unwrap_or(0);
        let ext_url:  String     = row.get::<Option<String>, _>("external_url").unwrap_or_default();

        let prompt = format!(
            r#"Score this Instagram creator as a potential client for a professional video clipping service (score 0–100).

The clipping service:
- Takes long-form videos (YouTube, podcast, streams) and repurposes them into 30–90s Shorts/Reels clips
- Charges $297–$497/month
- Ideal clients: YouTubers, podcasters, educators, online course creators, coaches with video content

Creator profile:
- Username: @{username}
- Followers: {followers}
- Bio: {bio}
- Link in bio: {ext_url}

Score guidelines:
- 80–100: Clear content creator with video content, 10K–500K followers, appears monetised
- 60–79: Likely content creator, decent following, video content likely
- 40–59: Could be a creator but unclear from profile
- 0–39: Fan page, brand account, private/spammy, or clearly not a content creator

Return ONLY valid JSON (no markdown):
{{"score": 75, "reason": "Podcaster with 45K followers, podcast link in bio, posts video clips"}}"#,
            username  = username,
            followers = followers,
            bio       = bio,
            ext_url   = ext_url,
        );

        let result = generate_text_best_effort(
            state.nvidia_nim_client.as_ref(),
            state.gemma_client.as_ref(),
            state.gemini_client.as_ref(),
            &prompt,
        ).await;

        if let Ok(text) = result {
            let cleaned = text.trim()
                .trim_start_matches("```json").trim_start_matches("```")
                .trim_end_matches("```").trim();

            if let Ok(v) = serde_json::from_str::<serde_json::Value>(cleaned) {
                let score  = v.get("score").and_then(|s| s.as_i64()).unwrap_or(0) as i32;
                let reason = v.get("reason").and_then(|r| r.as_str()).unwrap_or("").to_string();

                let _ = sqlx::query(
                    "UPDATE instagram_leads SET score = $1, score_reason = $2 WHERE id = $3"
                )
                .bind(score)
                .bind(&reason)
                .bind(id)
                .execute(&state.db_pool)
                .await;
            }
        }

        // Small delay to avoid quota hammering
        tokio::time::sleep(tokio::time::Duration::from_millis(300)).await;
    }

    tracing::info!("📊 Scored {} Instagram leads for hashtag #{}", unscored.len(), hashtag);
}

// ============================================================================
// AI helpers — drive discovery, not just scoring
// ============================================================================

/// Ask the AI to generate the most effective YouTube search query for a prospect type + category.
/// Replaces all hardcoded search strings.
async fn ai_generate_youtube_query(state: &Arc<AppState>, prospect_type: &str, category: &str) -> String {
    let prompt = format!(
        r#"You are helping a video clipping agency find potential clients on YouTube.

Prospect type: {prospect_type}
Content category/niche: {category}

Generate ONE concise YouTube search query (5–10 words max) that will find active YouTube CHANNELS
in this niche who are likely to be content creators with regular uploads.

The query should:
- Target the channel owners/creators themselves (not tutorials about them)
- Be specific enough to find real people, not brands or media companies
- Work as a YouTube channel search

Return ONLY the raw search query string, nothing else."#
    );

    match generate_text_best_effort(
        state.nvidia_nim_client.as_ref(),
        state.gemma_client.as_ref(),
        state.gemini_client.as_ref(),
        &prompt,
    ).await {
        Ok(t) => {
            let q = t.trim().trim_matches('"').trim_matches('\'').to_string();
            if q.is_empty() || q.len() > 100 { category.to_string() } else { q }
        }
        Err(_) => category.to_string(), // graceful fallback
    }
}

/// Ask the AI to pick the best Twitch game/category to find streamers matching the prospect type.
async fn ai_generate_twitch_category(state: &Arc<AppState>, prospect_type: &str, category: &str) -> String {
    let prompt = format!(
        r#"You are helping a video clipping agency find Twitch streamers as potential clients.

Prospect type: {prospect_type}
Niche: {category}

Return ONE Twitch game or category name (exactly as it appears on Twitch) that best matches
this niche. The streamers in this category are likely to be content creators who'd pay for
short-form clip editing ($300/month).

If the niche is not gaming, pick the closest non-gaming Twitch category (e.g. "Just Chatting",
"Science & Technology", "Music", "Art", "Fitness & Health", "Podcasts", "Software and Game Dev").

Return ONLY the category name, nothing else."#
    );

    match generate_text_best_effort(
        state.nvidia_nim_client.as_ref(),
        state.gemma_client.as_ref(),
        state.gemini_client.as_ref(),
        &prompt,
    ).await {
        Ok(t) => t.trim().trim_matches('"').trim_matches('\'').to_string(),
        Err(_) => category.to_string(),
    }
}

/// Ask the AI to generate Sales Navigator filter params from a plain-English description.
/// Returns a populated SmartSearchRequest ready to use.
async fn ai_generate_linkedin_filters(
    state: &Arc<AppState>,
    description: &str,
) -> Result<SmartSearchRequest, String> {
    let prompt = format!(
        r#"You are generating LinkedIn Sales Navigator search filters for a video clipping agency.

The agency creates short-form clips (YouTube Shorts, Reels, TikTok) for content creators.
They are looking for potential clients based on this description:

"{description}"

Generate Sales Navigator filters that will find these people. Return ONLY valid JSON:
{{
  "job_titles": ["<title1>", "<title2>", "<title3>"],
  "industries": ["<industry1>", "<industry2>"],
  "locations": ["<location1>"],
  "seniority": ["OWNER", "PARTNER"],
  "company_sizes": ["A", "B"]
}}

Rules:
- job_titles: 3–6 titles. Use real LinkedIn job titles (e.g. "YouTuber", "Podcast Host", "Content Creator", "Online Course Creator", "Social Media Influencer")
- industries: 2–4 industries from LinkedIn's list (e.g. "Online Media", "E-Learning", "Entertainment", "Broadcast Media", "Marketing and Advertising")
- locations: 1–3 countries/regions (e.g. "United States", "United Kingdom", "Canada")
- seniority: always ["OWNER", "PARTNER", "CXO"] for independent creators
- company_sizes: always ["A", "B"] (1–50 employees — solo creators and small teams)

Return ONLY the JSON, no explanation."#
    );

    let text = generate_text_best_effort(
        state.nvidia_nim_client.as_ref(),
        state.gemma_client.as_ref(),
        state.gemini_client.as_ref(),
        &prompt,
    ).await.map_err(|e| format!("LLM error: {}", e))?;

    let cleaned = text.trim()
        .trim_start_matches("```json").trim_start_matches("```")
        .trim_end_matches("```").trim();

    #[derive(serde::Deserialize)]
    struct AiFilters {
        job_titles:    Option<Vec<String>>,
        industries:    Option<Vec<String>>,
        locations:     Option<Vec<String>>,
        seniority:     Option<Vec<String>>,
        company_sizes: Option<Vec<String>>,
    }

    let filters: AiFilters = serde_json::from_str(cleaned)
        .map_err(|e| format!("JSON parse error: {} | raw: {}", e, cleaned))?;

    Ok(SmartSearchRequest {
        description:   None, // already consumed
        job_titles:    filters.job_titles,
        industries:    filters.industries,
        company_sizes: filters.company_sizes,
        locations:     filters.locations,
        seniority:     filters.seniority,
        max_profiles:  None,
        list_url:      None,
    })
}

/// Score LinkedIn leads imported from a specific PB job using the same AI scoring
/// used for YouTube/Twitch prospects.
async fn ai_score_linkedin_leads(state: &Arc<AppState>, job_id: uuid::Uuid) {
    // Get unscored LinkedIn prospects imported via this job
    let rows = match sqlx::query(
        "SELECT id, display_name, channel_description, subscriber_count, content_category,
                job_title, company_name, seniority_level
         FROM prospects
         WHERE platform = 'linkedin'
           AND (ai_score IS NULL OR ai_score = 0.5)
           AND phantombuster_job_id = $1
         LIMIT 30"
    )
    .bind(job_id)
    .fetch_all(&state.db_pool)
    .await {
        Ok(r) => r,
        Err(e) => { tracing::warn!("LinkedIn scoring: DB error: {}", e); return; }
    };

    tracing::info!("📊 AI-scoring {} LinkedIn leads for job {}", rows.len(), job_id);

    for row in &rows {
        let id:          uuid::Uuid = row.get("id");
        let name:        String     = row.get::<Option<String>, _>("display_name").unwrap_or_default();
        let description: String     = row.get::<Option<String>, _>("channel_description").unwrap_or_default();
        let audience:    i64        = row.get::<Option<i64>, _>("subscriber_count").unwrap_or(0);
        let category:    String     = row.get::<Option<String>, _>("content_category").unwrap_or_default();
        let job_title:   String     = row.get::<Option<String>, _>("job_title").unwrap_or_default();
        let company:     String     = row.get::<Option<String>, _>("company_name").unwrap_or_default();
        let seniority:   String     = row.get::<Option<String>, _>("seniority_level").unwrap_or_default();

        // Build a richer description from LinkedIn fields
        let enriched_desc = format!(
            "Job title: {}. Company: {}. Seniority: {}. Bio: {}",
            job_title, company, seniority, description
        );

        let (score, reasoning, dm_creator, _) =
            score_prospect_with_ai(state, &name, audience, &enriched_desc, &category, "linkedin_lead").await;

        let _ = sqlx::query(
            "UPDATE prospects SET ai_score = $1, ai_reasoning = $2, dm_script_creator = $3, updated_at = NOW() WHERE id = $4"
        )
        .bind(score)
        .bind(&reasoning)
        .bind(&dm_creator)
        .bind(id)
        .execute(&state.db_pool)
        .await;

        tokio::time::sleep(tokio::time::Duration::from_millis(200)).await;
    }

    tracing::info!("✅ LinkedIn AI scoring complete for job {}", job_id);
}
