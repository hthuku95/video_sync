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
    let search_query = match payload.prospect_type.as_str() {
        "clipper"        => "video editor clips shorts creator".to_string(),
        "podcaster"      => format!("{} podcast episode", base_category),
        "educator"       => format!("{} tutorial explained course", base_category),
        "business_owner" => format!("{} company brand products", base_category),
        _                => base_category.clone(), // content_creator default
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

    Json(json!({
        "success": true,
        "total_fetched": count,
        "imported_to_prospects": imported,
        "message": format!("{} LinkedIn leads imported. View them at /admin/prospect-finder", imported)
    }))
}
