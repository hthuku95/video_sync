// Admin-only prospect finder — uses YouTube + Twitch APIs to discover
// potential clients (content creators and clippers), scores them with Gemini,
// and generates personalized DM scripts.

use crate::llm_utils::generate_text_best_effort;
use crate::middleware::admin::admin_middleware;
use crate::middleware::auth::auth_middleware;
use crate::models::auth::{Claims, ErrorResponse};
use crate::services::monetization::service_offer_prompt;
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
    // SSR page — public route, uses JS in the browser to read the JWT from
    // localStorage and call the /api/* endpoints below. Putting middleware
    // here would reject every navigation because browsers don't send
    // `Authorization` headers on `<a href>` or address-bar loads.
    let public_page = Router::new()
        .route("/admin/prospect-finder", get(prospect_finder_page));

    // API endpoints — still protected by JWT + admin claim.
    let protected_api = Router::new()
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
        // Telegram opportunity tab (phase 1: manual entry + AI scoring;
        // phase 2 will add automated grammers-client watcher).
        .route("/api/admin/telegram/channels",       get(telegram_list_channels))
        .route("/api/admin/telegram/channels",       post(telegram_add_channel))
        .route("/api/admin/telegram/channels/:id",   delete(telegram_delete_channel))
        .route("/api/admin/telegram/opportunities",  get(telegram_list_opportunities))
        .route("/api/admin/telegram/opportunities",  post(telegram_add_opportunity_manual))
        .route("/api/admin/telegram/opportunities/:id", patch(telegram_update_opportunity))
        // MTProto watcher — login + status. Bot API lives in telegram_bot.rs.
        .route("/api/admin/telegram/login/start",    post(telegram_login_start))
        .route("/api/admin/telegram/login/verify",   post(telegram_login_verify))
        .route("/api/admin/telegram/status",         get(telegram_watcher_status))
        .layer(axum::middleware::from_fn(admin_middleware))
        .layer(axum::middleware::from_fn(auth_middleware));

    public_page.merge(protected_api)
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
        .route("/api/instagram/leads/:id/generate-sample",   post(instagram_generate_sample))
        .route("/api/instagram/leads/:id/contact-status",    patch(instagram_update_contact_status))
        .route("/api/instagram/leads/:id/service-type",      patch(instagram_update_service_type))
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

        let (score, reasoning, service, dm_creator, dm_clipper) =
            score_prospect_with_ai(state, &display_name, sub_count, &description, &category, &payload.prospect_type).await;

        sqlx::query(
            "INSERT INTO prospects (platform, channel_id, display_name, platform_url,
             subscriber_count, content_category, channel_description, prospect_type,
             ai_score, ai_reasoning, dm_script_creator, dm_script_clipper, twitter_handle,
             service_type)
             VALUES ($1,$2,$3,$4,$5,$6,$7,$8,$9,$10,$11,$12,$13,$14)
             ON CONFLICT (platform, channel_id) DO UPDATE SET
               ai_score = EXCLUDED.ai_score,
               ai_reasoning = EXCLUDED.ai_reasoning,
               dm_script_creator = EXCLUDED.dm_script_creator,
               dm_script_clipper = EXCLUDED.dm_script_clipper,
               twitter_handle = COALESCE(EXCLUDED.twitter_handle, prospects.twitter_handle),
               service_type = EXCLUDED.service_type,
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
        .bind(&service)
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

        let (score, reasoning, service, dm_creator, dm_clipper) =
            score_prospect_with_ai(state, &display_name, viewer_count, "", &category, &payload.prospect_type).await;

        sqlx::query(
            "INSERT INTO prospects (platform, channel_id, display_name, platform_url,
             avg_viewer_count, content_category, prospect_type,
             ai_score, ai_reasoning, dm_script_creator, dm_script_clipper,
             service_type)
             VALUES ($1,$2,$3,$4,$5,$6,$7,$8,$9,$10,$11,$12)
             ON CONFLICT (platform, channel_id) DO UPDATE SET
               avg_viewer_count = EXCLUDED.avg_viewer_count,
               ai_score = EXCLUDED.ai_score,
               ai_reasoning = EXCLUDED.ai_reasoning,
               dm_script_creator = EXCLUDED.dm_script_creator,
               dm_script_clipper = EXCLUDED.dm_script_clipper,
               service_type = EXCLUDED.service_type,
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
        .bind(&service)
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
/// AI-scored prospect data — matches the IG lead pattern.
/// Returns `(score_0_1, reasoning, service_type, dm_script, dm_clipper_fallback)`.
/// `service_type` is one of: clipping | animations | thumbnails | ugc | full_stack.
async fn score_prospect_with_ai(
    state: &Arc<AppState>,
    name: &str,
    audience_size: i64,
    description: &str,
    category: &str,
    prospect_type: &str,
) -> (f64, String, String, String, String) {
    if state.nvidia_nim_client.is_none() && state.gemma_client.is_none() && state.gemini_client.is_none() {
        return (0.5, "No LLM configured".to_string(), "clipping".to_string(),
                default_dm_creator(name), default_dm_clipper(name));
    }

    // Same service-menu pattern as IG leads — AI picks the strongest-fit
    // service for THIS creator and writes a DM locked to that service.
    let prompt = format!(
        r#"You are an outbound copywriter for a video production studio. Score this channel as a potential client (0.0-1.0), pick the best-fit service, and write the DM.

Channel: {name}
Audience size: {audience} ({category_word})
Category: {category}
Description: {description}
Prospect type (already tagged by us): {prospect_type}

The studio offers these services — pick the ONE that fits best:
- **clipping**       — long-form → Shorts/Reels. Best fit: podcasters, long-form YouTubers, Twitch streamers. $297-$899/mo.
- **animations**     — Blender explainer/data-viz/LaTeX scenes. Best fit: educators, finance/crypto channels, news/data accounts. $50-$150 each.
- **thumbnails**     — AI-generated YouTube thumbnails. Best fit: growing YouTubers (5k-100k subs), MrBeast aspirants. $25-$50 each.
- **ugc**            — vertical product-demo ads. Best fit: Shopify/DTC founders, SaaS demos, brand accounts. $200-$500 each.
- **product_mockup** — photorealistic product shot rendered on a device/scene (3D mockup). Best fit: ecommerce stores, hardware brands, app developers, Kickstarter creators. $100-$300 each.
- **landing_page**   — animated hero mockup for a SaaS/startup landing page (we can pull the hero image from their live URL). Best fit: YC/indie-hacker SaaS founders, pre-launch startups, no-code builders. $200-$600 each.
- **full_stack**     — bundle of the above. Best fit: 100k+ creators serious about scaling. $1500-$3000/mo.

Score guidelines:
- 0.8-1.0: clear paying client, monetised, has content we can act on now.
- 0.6-0.8: likely fit, bio + audience size point strongly to one service.
- 0.4-0.6: ambiguous — could go either way from the signals alone.
- 0.0-0.4: bad fit (fan page, brand parody, competitor/fellow editor).

Return ONLY valid JSON (no markdown):
{{
  "score": 0.75,
  "reasoning": "<1-2 sentences explaining the score + why this service>",
  "service": "clipping",
  "dm": "<personalized DM locked to the chosen service — 2-3 sentences, mention the name, reference something specific, state concrete price from the menu, end with an ask>",
  "dm_clipper": "<alt DM treating them as a clipper looking for tooling — 2-3 sentences>"
}}

`service` MUST be one of: clipping, animations, thumbnails, ugc, product_mockup, landing_page, full_stack."#,
        name          = name,
        audience      = audience_size,
        category_word = if audience_size > 0 { "subs/viewers" } else { "unknown" },
        category      = category,
        description   = description,
        prospect_type = prospect_type,
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
                    let score      = v["score"].as_f64().unwrap_or(0.5).clamp(0.0, 1.0);
                    let reasoning  = v["reasoning"].as_str().unwrap_or("").to_string();
                    let dm_creator = v["dm"].as_str()
                        .or_else(|| v["dm_creator"].as_str())  // legacy field name
                        .unwrap_or(&default_dm_creator(name))
                        .to_string();
                    let dm_clipper = v["dm_clipper"].as_str().unwrap_or(&default_dm_clipper(name)).to_string();
                    // Coerce service to one of the 5 valid values.
                    let service_raw = v["service"].as_str().unwrap_or("clipping").to_lowercase();
                    let service = match service_raw.as_str() {
                        "clipping" | "animations" | "thumbnails" | "ugc" | "product_mockup" | "landing_page" | "full_stack" => service_raw,
                        _ => "clipping".to_string(),
                    };
                    (score, reasoning, service, dm_creator, dm_clipper)
                }
                Err(_) => (0.5, "Parse error".to_string(), "clipping".to_string(),
                           default_dm_creator(name), default_dm_clipper(name)),
            }
        }
        Err(_) => (0.5, "AI unavailable".to_string(), "clipping".to_string(),
                   default_dm_creator(name), default_dm_clipper(name)),
    }
}

fn default_dm_creator(name: &str) -> String {
    format!("Hey {}! I run an AI video studio that handles clipping, thumbnails, and Blender animations — happy to send a free sample so you can see which one fits your channel. Want me to send the link?", name)
}

fn default_dm_clipper(name: &str) -> String {
    format!("Hey {}! I built an AI clipping + rendering platform that processes YouTube and Twitch videos 10x faster — short-form clips, animations, and thumbnails in one pipeline. Want free access to try it?", name)
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
                   contact_status, notes, twitter_handle, service_type, created_at \
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
        "service_type": r.get::<Option<String>, _>("service_type"),
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
    let (score, reasoning, service, dm_creator, dm_clipper) =
        score_prospect_with_ai(&state, &name, audience, &description, &category, &pt).await;

    sqlx::query(
        "UPDATE prospects
         SET ai_score=$1, ai_reasoning=$2, dm_script_creator=$3, dm_script_clipper=$4,
             service_type=$5, updated_at=NOW()
         WHERE id=$6"
    )
    .bind(score).bind(&reasoning).bind(&dm_creator).bind(&dm_clipper).bind(&service).bind(id)
    .execute(&state.db_pool).await.ok();

    Ok(Json(json!({
        "success":     true,
        "dm_creator":  dm_creator,
        "dm_clipper":  dm_clipper,
        "score":       score,
        "service":     service,
    })))
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
<link rel="preconnect" href="https://fonts.googleapis.com">
<link rel="preconnect" href="https://fonts.gstatic.com" crossorigin>
<link href="https://fonts.googleapis.com/css2?family=Inter:wght@300;400;500;600;700&display=swap" rel="stylesheet">
<style>
/* Deep-purple palette — matches content_machine Enterprise UI redesign */
:root{
  --bg:#2a2438;
  --surface:#352f44;
  --border:#5c5470;
  --accent:#dbd8e3;
  --accent-strong:#8a7ca8;
  --purple:#7a4cff;
  --muted:#b8b3c8;
  --dim:#9999bb;
  --success:#4ade80;
  --warn:#facc15;
  --danger:#f87171;
}
*{box-sizing:border-box;margin:0;padding:0}
body{
  font-family:'Inter',system-ui,-apple-system,sans-serif;
  background:linear-gradient(135deg,#2a2438 0%,#1f1a2a 100%);
  color:var(--accent);
  min-height:100vh;
  font-weight:400;
  letter-spacing:-0.01em;
}

.header{
  background:rgba(53,47,68,0.6);
  backdrop-filter:blur(18px);
  -webkit-backdrop-filter:blur(18px);
  border-bottom:1px solid rgba(92,84,112,0.4);
  padding:16px 28px;
  display:flex;align-items:center;gap:16px;
  position:sticky;top:0;z-index:10;
}
.header h1{
  font-size:1.2rem;font-weight:600;
  background:linear-gradient(135deg,var(--accent) 0%,#fff 100%);
  -webkit-background-clip:text;background-clip:text;
  -webkit-text-fill-color:transparent;
}
.back{color:var(--dim);text-decoration:none;font-size:0.9rem;transition:color 0.15s}
.back:hover{color:var(--accent)}
.container{max-width:1280px;margin:0 auto;padding:28px}

/* Cards — frosted glass look */
.search-card{
  background:rgba(53,47,68,0.7);
  backdrop-filter:blur(14px);
  -webkit-backdrop-filter:blur(14px);
  border:1px solid rgba(92,84,112,0.5);
  border-radius:14px;
  padding:24px;
  margin-bottom:20px;
  box-shadow:0 4px 24px rgba(0,0,0,0.25);
}
.search-card h2{font-size:1rem;font-weight:600;color:var(--accent);margin-bottom:16px}
.search-card h2 + p{color:var(--muted);font-size:0.85rem;margin-bottom:16px;margin-top:-8px;line-height:1.5}

.form-grid{display:grid;grid-template-columns:repeat(auto-fit,minmax(180px,1fr));gap:14px;margin-bottom:16px}
label{font-size:0.75rem;font-weight:500;color:var(--dim);display:block;margin-bottom:6px;text-transform:uppercase;letter-spacing:0.04em}
select,input{
  width:100%;padding:10px 14px;
  background:rgba(42,36,56,0.6);
  border:1px solid rgba(92,84,112,0.6);
  border-radius:8px;
  color:var(--accent);
  font-size:0.9rem;
  font-family:inherit;
  transition:border-color 0.15s, background 0.15s;
}
select:focus,input:focus{outline:none;border-color:var(--purple);background:rgba(42,36,56,0.9)}

/* Buttons */
.btn{
  padding:10px 20px;border:none;border-radius:8px;cursor:pointer;
  font-size:0.9rem;font-weight:600;font-family:inherit;
  transition:background 0.15s, transform 0.1s;
  letter-spacing:-0.01em;
}
.btn:active{transform:translateY(1px)}
.btn-primary{background:var(--purple);color:#fff}
.btn-primary:hover{background:#6a3def}
.btn-sm{padding:6px 12px;font-size:0.78rem;border-radius:6px}
.btn-copy{background:rgba(92,84,112,0.3);color:var(--accent);border:1px solid rgba(92,84,112,0.5)}
.btn-copy:hover{background:rgba(92,84,112,0.5)}
.btn-ghost{background:transparent;color:var(--dim);border:1px solid rgba(92,84,112,0.5)}
.btn-ghost:hover{color:var(--accent);border-color:var(--border)}

/* Tabs */
.tabs{display:flex;gap:8px;margin-bottom:16px;flex-wrap:wrap}
.tab{
  padding:8px 16px;border-radius:8px;cursor:pointer;
  font-size:0.85rem;font-weight:500;
  background:rgba(42,36,56,0.5);color:var(--dim);
  border:1px solid rgba(92,84,112,0.3);
  transition:all 0.15s;
}
.tab:hover{color:var(--accent);border-color:rgba(92,84,112,0.6)}
.tab.active{background:var(--purple);color:#fff;border-color:var(--purple)}

/* Table */
.table-wrap{
  background:rgba(53,47,68,0.7);
  backdrop-filter:blur(14px);
  border:1px solid rgba(92,84,112,0.5);
  border-radius:14px;
  overflow:hidden;
  box-shadow:0 4px 24px rgba(0,0,0,0.25);
}
table{width:100%;border-collapse:collapse;font-size:0.88rem}
th{
  text-align:left;padding:14px 16px;
  color:var(--dim);font-weight:500;font-size:0.75rem;
  text-transform:uppercase;letter-spacing:0.05em;
  border-bottom:1px solid rgba(92,84,112,0.4);
  background:rgba(42,36,56,0.4);
}
td{
  padding:14px 16px;
  border-bottom:1px solid rgba(92,84,112,0.2);
  vertical-align:top;
  color:var(--accent);
}
tr:last-child td{border-bottom:none}
tr:hover td{background:rgba(92,84,112,0.12)}

/* Score + status badges */
.score-badge{display:inline-block;padding:3px 10px;border-radius:12px;font-size:0.75rem;font-weight:700}
.score-high{background:rgba(74,222,128,0.15);color:var(--success);border:1px solid rgba(74,222,128,0.3)}
.score-mid{background:rgba(250,204,21,0.15);color:var(--warn);border:1px solid rgba(250,204,21,0.3)}
.score-low{background:rgba(248,113,113,0.15);color:var(--danger);border:1px solid rgba(248,113,113,0.3)}
.status-select{
  background:rgba(42,36,56,0.6);border:1px solid rgba(92,84,112,0.5);
  color:var(--accent);padding:4px 8px;border-radius:6px;
  font-size:0.78rem;font-family:inherit;cursor:pointer;
}

/* Platform chips */
.platform-yt{color:#ff4d4d;font-weight:600;font-size:0.82rem}
.platform-tw{color:#9147ff;font-weight:600;font-size:0.82rem}

/* DM preview */
.dm-text{
  background:rgba(42,36,56,0.7);
  border:1px solid rgba(92,84,112,0.4);
  border-radius:8px;
  padding:12px;
  font-size:0.82rem;
  color:var(--muted);
  white-space:pre-wrap;
  max-height:100px;overflow-y:auto;
  margin-bottom:6px;
  line-height:1.5;
}

/* States */
.loading,.empty{
  text-align:center;padding:48px 24px;
  color:var(--dim);
}
.empty-icon{font-size:3rem;opacity:0.5;margin-bottom:12px;display:block}
.empty-hint{font-size:0.85rem;margin-top:8px;color:var(--dim)}

/* Messages */
.msg{padding:12px 18px;border-radius:10px;margin-bottom:14px;font-size:0.9rem;font-weight:500}
.msg-success{background:rgba(74,222,128,0.12);color:var(--success);border:1px solid rgba(74,222,128,0.3)}
.msg-error{background:rgba(248,113,113,0.12);color:var(--danger);border:1px solid rgba(248,113,113,0.3)}

/* Notes input */
.row-notes{
  width:100%;padding:6px 10px;
  background:rgba(42,36,56,0.5);
  border:1px solid rgba(92,84,112,0.3);
  border-radius:6px;
  color:var(--accent);
  font-size:0.78rem;font-family:inherit;
  margin-top:6px;
  transition:border-color 0.15s;
}
.row-notes:focus{outline:none;border-color:var(--purple)}

/* Stats strip */
.stat-strip{
  display:grid;grid-template-columns:repeat(auto-fit,minmax(160px,1fr));
  gap:12px;margin-bottom:20px;
}
.stat-card{
  background:rgba(53,47,68,0.6);
  border:1px solid rgba(92,84,112,0.3);
  border-radius:10px;
  padding:14px 16px;
}
.stat-val{font-size:1.4rem;font-weight:700;color:var(--accent)}
.stat-label{font-size:0.72rem;text-transform:uppercase;color:var(--dim);letter-spacing:0.05em;margin-top:2px}

/* Progress bar */
.pbar-track{background:rgba(42,36,56,0.8);border-radius:4px;height:8px;overflow:hidden}
.pbar-fill{background:linear-gradient(90deg,var(--purple),var(--accent-strong));height:100%;width:0%;transition:width 0.5s}

/* Scrollbar */
::-webkit-scrollbar{width:8px;height:8px}
::-webkit-scrollbar-track{background:rgba(42,36,56,0.3)}
::-webkit-scrollbar-thumb{background:rgba(92,84,112,0.5);border-radius:4px}
::-webkit-scrollbar-thumb:hover{background:rgba(92,84,112,0.8)}

/* PB job list */
.pb-job-row{
  display:grid;
  grid-template-columns:minmax(0,1fr) auto auto auto;
  gap:12px;
  align-items:center;
  padding:12px 14px;
  background:rgba(42,36,56,0.5);
  border:1px solid rgba(92,84,112,0.3);
  border-radius:10px;
  margin-bottom:8px;
}
.pb-job-url{font-size:0.82rem;color:var(--accent);font-family:'JetBrains Mono',monospace;overflow:hidden;text-overflow:ellipsis;white-space:nowrap}
.pb-job-status{font-size:0.72rem;font-weight:600;text-transform:uppercase;letter-spacing:0.04em;padding:3px 10px;border-radius:10px}
.pb-job-status.running   {background:rgba(74,222,128,0.15);color:var(--success)}
.pb-job-status.queued    {background:rgba(250,204,21,0.15);color:var(--warn)}
.pb-job-status.completed {background:rgba(138,124,168,0.2); color:var(--accent)}
.pb-job-status.failed    {background:rgba(248,113,113,0.15);color:var(--danger)}

/* Action row (buttons + status) so the top search bar aligns properly */
.action-row{
  display:flex;align-items:center;gap:12px;flex-wrap:wrap;
  padding-top:4px;
}

/* Mobile / tablet */
@media (max-width: 900px){
  .container{padding:18px}
  .form-grid{grid-template-columns:1fr 1fr}
  .tabs{gap:6px}
  .tab{padding:7px 12px;font-size:0.8rem}
  .stat-strip{grid-template-columns:repeat(2,1fr)}
  .pb-job-row{grid-template-columns:1fr auto;gap:8px}
  .pb-job-url{grid-column:1 / -1}
  /* Horizontal-scroll table instead of squish */
  .table-wrap{overflow-x:auto}
  table{min-width:720px}
}
@media (max-width: 560px){
  .header{padding:14px 16px}
  .container{padding:14px}
  .form-grid{grid-template-columns:1fr}
  .btn{width:100%}
  .action-row .btn{width:auto}
  .action-row{flex-direction:column;align-items:stretch}
  .search-card{padding:18px}
  .stat-strip{grid-template-columns:1fr 1fr}
}
</style>
</head>
<body>
<div class="header">
  <a href="/admin" class="back">← Admin</a>
  <h1>🎯 Prospect Finder</h1>
</div>
<div class="container">
  <div id="msg"></div>

  <!-- Source Selector Tabs -->
  <div class="tabs" style="margin-bottom:14px">
    <div class="tab active" onclick="showSource('yt-tw',this)">▶ YouTube / Twitch</div>
    <div class="tab" onclick="showSource('linkedin',this)">💼 LinkedIn</div>
    <div class="tab" onclick="showSource('telegram',this)">✈️ Telegram Opportunities</div>
    <div class="tab" onclick="showSource('jobs',this)">🚦 PB Job Status</div>
  </div>

  <!-- YouTube / Twitch Search -->
  <div id="src-yt-tw" class="search-card">
    <h2>Find Prospects on YouTube &amp; Twitch</h2>
    <p>AI discovers creators, scores each 0–100 against your ICP, auto-generates cold DM scripts.</p>
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
    <span id="search-status" style="margin-left:12px;color:var(--dim);font-size:0.85rem"></span>
  </div>

  <!-- LinkedIn Search -->
  <div id="src-linkedin" class="search-card" style="display:none">
    <h2>Find Prospects on LinkedIn (Sales Navigator)</h2>
    <p>Describe your ideal client in plain English. AI turns the description into a Sales Navigator filter set (title + industry + company size + geography + seniority) and launches a PhantomBuster scrape. Requires a LinkedIn account with an active Sales Navigator subscription.</p>
    <div class="form-grid">
      <div style="grid-column:1 / -1">
        <label>Describe your ideal client</label>
        <input id="li-description" placeholder="podcasters with 10k–100k monthly downloads who need video editors" value="podcasters with 10k-100k downloads needing video editors">
      </div>
      <div>
        <label>Max Profiles</label>
        <input id="li-max-profiles" type="number" value="25" min="10" max="2500">
      </div>
    </div>
    <button class="btn btn-primary" onclick="runLinkedInSearch()">💼 Launch LinkedIn Scrape</button>
    <button class="btn btn-ghost" onclick="listLinkedInAgents()" style="margin-left:8px">List PB Agents</button>
    <span id="li-status" style="margin-left:12px;color:var(--dim);font-size:0.85rem"></span>
    <div id="li-agents" style="margin-top:14px"></div>
  </div>

  <!-- Telegram Opportunities -->
  <div id="src-telegram" class="search-card" style="display:none">
    <h2>Telegram Opportunities</h2>
    <p>AI-watched Telegram channels for paid-gig opportunities. Once you log in with your phone below, the MTProto watcher polls configured channels and pings <code>@videosync_sales_bot</code> with a pre-written custom DM every time it finds a match. You can also paste messages manually. Watched channels: <code id="tg-watched"></code></p>

    <!-- MTProto login panel -->
    <div id="tg-login" style="margin-bottom:16px;padding:16px;background:rgba(42,36,56,0.6);border:1px solid rgba(122,76,255,0.3);border-radius:10px">
      <div id="tg-status-summary" style="font-size:0.9rem;color:var(--muted);margin-bottom:10px">Checking Telegram watcher status…</div>

      <div id="tg-login-start" style="display:none">
        <div class="form-grid">
          <div>
            <label>Your Telegram phone (with country code)</label>
            <input id="tg-phone" placeholder="+14155551234">
          </div>
        </div>
        <button class="btn btn-primary" onclick="tgLoginStart()">Send code to Telegram</button>
        <span id="tg-login-status" style="margin-left:12px;color:var(--dim);font-size:0.85rem"></span>
      </div>

      <div id="tg-login-verify" style="display:none">
        <div class="form-grid">
          <div>
            <label>Phone (same as before)</label>
            <input id="tg-phone-v" placeholder="+14155551234">
          </div>
          <div>
            <label>Code Telegram sent you</label>
            <input id="tg-code" placeholder="12345" inputmode="numeric">
          </div>
        </div>
        <button class="btn btn-primary" onclick="tgLoginVerify()">Verify &amp; Authorise Watcher</button>
        <span id="tg-verify-status" style="margin-left:12px;color:var(--dim);font-size:0.85rem"></span>
      </div>

      <div id="tg-login-active" style="display:none">
        <div style="display:flex;align-items:center;gap:10px;flex-wrap:wrap">
          <span class="score-badge score-high">✓ Watcher active</span>
          <span id="tg-active-phone" style="color:var(--dim);font-size:0.85rem"></span>
          <span id="tg-last-poll" style="color:var(--dim);font-size:0.85rem"></span>
        </div>
      </div>
    </div>

    <details style="margin-bottom:16px">
      <summary style="cursor:pointer;color:var(--dim);font-size:0.85rem">📋 Paste a new opportunity</summary>
      <div style="margin-top:12px;padding:14px;background:rgba(42,36,56,0.4);border-radius:8px">
        <div class="form-grid">
          <div>
            <label>Channel (without @)</label>
            <input id="tg-channel" placeholder="cryptojobslist" value="cryptojobslist">
          </div>
          <div>
            <label>Sender (optional)</label>
            <input id="tg-sender" placeholder="@username">
          </div>
          <div style="grid-column: 1 / -1">
            <label>Message text</label>
            <textarea id="tg-message" rows="4" style="width:100%;padding:8px 12px;background:rgba(42,36,56,0.6);border:1px solid rgba(92,84,112,0.5);border-radius:8px;color:var(--accent);font-family:inherit;font-size:0.9rem" placeholder="Paste the Telegram message here..."></textarea>
          </div>
          <div style="grid-column: 1 / -1">
            <label>Link to message (optional)</label>
            <input id="tg-link" placeholder="https://t.me/cryptojobslist/12345">
          </div>
        </div>
        <button class="btn btn-primary" onclick="submitTelegramOpportunity()">Score &amp; Save Opportunity</button>
        <span id="tg-submit-status" style="margin-left:12px;color:var(--dim);font-size:0.85rem"></span>
      </div>
    </details>

    <div style="display:flex;gap:8px;align-items:center;margin-bottom:12px;flex-wrap:wrap">
      <button class="btn btn-ghost btn-sm" onclick="loadTgOpportunities('')">All</button>
      <button class="btn btn-ghost btn-sm" onclick="loadTgOpportunities('new')">New</button>
      <button class="btn btn-ghost btn-sm" onclick="loadTgOpportunities('contacted')">Contacted</button>
      <button class="btn btn-ghost btn-sm" onclick="loadTgOpportunities('won')">Won</button>
      <button class="btn btn-ghost btn-sm" onclick="loadTgOpportunities('ignored')">Ignored</button>
    </div>
    <div id="tg-opportunities-list">Loading...</div>
  </div>

  <!-- PB Jobs Status -->
  <div id="src-jobs" class="search-card" style="display:none">
    <h2>PhantomBuster Job Status</h2>
    <p>Live view of all LinkedIn + Instagram PB jobs. Click a completed LinkedIn job to import its leads into the Prospects tab.</p>
    <button class="btn btn-ghost btn-sm" onclick="loadPbJobs()">↻ Refresh</button>
    <div id="pb-jobs-list" style="margin-top:14px"></div>
  </div>

  <!-- View Switcher -->
  <div class="action-row" style="margin-bottom:16px">
    <button class="btn btn-primary" id="btn-prospects" onclick="showView('prospects')">📋 Prospects</button>
    <button class="btn btn-ghost" id="btn-clipgen" onclick="showView('clipgen')">🎬 Clip Generator</button>
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
    <div class="table-wrap">
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
    <div id="cg-progress" class="search-card" style="display:none">
      <div style="color:var(--accent);margin-bottom:10px;font-size:0.9rem" id="cg-progress-label">Analyzing…</div>
      <div class="pbar-track"><div id="cg-progress-bar" class="pbar-fill"></div></div>
    </div>

    <!-- Results -->
    <div id="cg-results" class="search-card" style="display:none">
      <h2>Generated Clips</h2>
      <div id="cg-clips-list"></div>
      <button class="btn btn-primary" style="margin-top:16px" onclick="createDelivery()">📦 Create Delivery Package</button>
    </div>

    <!-- Delivery + Outreach -->
    <div id="cg-delivery" class="search-card" style="display:none">
      <h2>Delivery Link</h2>
      <div style="display:flex;gap:8px;align-items:center;margin-bottom:16px;flex-wrap:wrap">
        <input id="cg-delivery-url" readonly style="flex:1;min-width:240px">
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

// ── Source-card switcher (YouTube/Twitch vs LinkedIn vs PB Jobs) ──────────
function showSource(name, btn){
  document.querySelectorAll('.tabs > .tab').forEach(t=>t.classList.remove('active'));
  btn.classList.add('active');
  ['yt-tw','linkedin','telegram','jobs'].forEach(s=>{
    const el = document.getElementById('src-'+s);
    if(el) el.style.display = (s===name)?'block':'none';
  });
  if (name === 'jobs')     loadPbJobs();
  if (name === 'telegram') { loadTelegramStatus(); loadTelegramChannels(); loadTgOpportunities(''); }
}

// ── LinkedIn ──────────────────────────────────────────────────────────────
async function listLinkedInAgents(){
  const out = document.getElementById('li-agents');
  out.innerHTML = '<div class="loading">Loading PB agents…</div>';
  try{
    const res = await fetch('/api/admin/prospects/linkedin/agents', {headers:{'Authorization':'Bearer '+token}});
    const data = await res.json();
    if(!data.success){ out.innerHTML = `<div class="msg msg-error">${data.error||'Failed'}</div>`; return; }
    const agents = data.agents||[];
    if(!agents.length){ out.innerHTML = '<div class="empty"><span class="empty-icon">🪐</span>No PB agents found.<div class="empty-hint">Add a Sales Navigator phantom in your PhantomBuster account.</div></div>'; return; }
    out.innerHTML = '<div style="font-size:0.85rem;color:var(--muted);margin-bottom:6px">PB workspace agents:</div>' +
      agents.map(a=>`<div class="pb-job-row"><span class="pb-job-url">${a.name}</span><span class="pb-job-status completed">${a.id}</span><span></span><span></span></div>`).join('');
  }catch(e){ out.innerHTML = `<div class="msg msg-error">${e}</div>`; }
}

async function runLinkedInSearch(){
  const desc = document.getElementById('li-description').value.trim();
  const max  = parseInt(document.getElementById('li-max-profiles').value)||25;
  if(!desc){ showMsg('Describe your ideal client first.', false); return; }

  const status = document.getElementById('li-status');
  status.textContent = '⏳ Building Sales Navigator filters via AI + launching PB…';
  try{
    const res = await fetch('/api/admin/prospects/linkedin/search', {
      method:'POST',
      headers:{'Authorization':'Bearer '+token,'Content-Type':'application/json'},
      body: JSON.stringify({description: desc, max_profiles: max})
    });
    const data = await res.json();
    if(!data.success){
      status.textContent = '';
      showMsg(data.error||data.message||'Launch failed', false);
      return;
    }
    status.textContent = `✓ Launched job ${data.job_id?.slice(0,8)} — agent: ${data.agent_name||'?'}`;
    showMsg(`LinkedIn scrape launched. Check "PB Job Status" tab in ~3-5 min, then click "Import leads" to bring them into the Prospects list.`);
  }catch(e){
    status.textContent = '';
    showMsg(`Network error: ${e}`, false);
  }
}

// ── PB Jobs ───────────────────────────────────────────────────────────────
async function loadPbJobs(){
  const out = document.getElementById('pb-jobs-list');
  out.innerHTML = '<div class="loading">Loading PB jobs…</div>';
  try{
    const res = await fetch('/api/admin/prospects/linkedin/jobs', {headers:{'Authorization':'Bearer '+token}});
    const data = await res.json();
    if(!data.success){ out.innerHTML = `<div class="msg msg-error">${data.error||'Failed'}</div>`; return; }
    const jobs = data.jobs||[];
    if(!jobs.length){ out.innerHTML = '<div class="empty"><span class="empty-icon">🚦</span>No PB jobs yet.<div class="empty-hint">Run a LinkedIn or Instagram search above.</div></div>'; return; }
    out.innerHTML = jobs.map(j=>{
      const isLinkedIn = (j.search_url||'').includes('linkedin.com');
      const importBtn = (isLinkedIn && j.status==='completed')
        ? `<button class="btn btn-sm btn-primary" onclick="importLinkedInJob('${j.id}')">⬇ Import leads</button>`
        : '<span></span>';
      const url = (j.search_url||'').slice(0,80);
      const launched = j.launched_at ? new Date(j.launched_at).toLocaleString() : '—';
      return `<div class="pb-job-row" title="${j.error||''}">
        <span class="pb-job-url">${url}</span>
        <span class="pb-job-status ${j.status}">${j.status}</span>
        <span style="font-size:0.78rem;color:var(--dim)">${launched}</span>
        ${importBtn}
      </div>`;
    }).join('');
  }catch(e){ out.innerHTML = `<div class="msg msg-error">${e}</div>`; }
}

async function importLinkedInJob(jobId){
  showMsg('Importing leads from PhantomBuster…');
  try{
    const res = await fetch(`/api/admin/prospects/linkedin/jobs/${jobId}/results`, {headers:{'Authorization':'Bearer '+token}});
    const data = await res.json();
    if(!data.success){ showMsg(data.error||data.message||'Import failed', false); return; }
    showMsg(`Imported ${data.imported_to_prospects||0} leads. Check the Prospects tab.`);
    setTimeout(loadProspects, 800);
    setTimeout(loadPbJobs, 800);
  }catch(e){ showMsg(`Network error: ${e}`, false); }
}

// ── Telegram Opportunities ─────────────────────────────────────────────────

async function loadTelegramStatus(){
  try {
    const r = await fetch('/api/admin/telegram/status', {headers:{'Authorization':'Bearer '+token}});
    const data = await r.json();
    const summary = document.getElementById('tg-status-summary');
    const start   = document.getElementById('tg-login-start');
    const verify  = document.getElementById('tg-login-verify');
    const active  = document.getElementById('tg-login-active');
    start.style.display = 'none';
    verify.style.display = 'none';
    active.style.display = 'none';
    if (data.authorized) {
      summary.innerHTML = '<b style="color:var(--success)">✓ MTProto watcher active</b> — configured channels are being monitored automatically.';
      active.style.display = '';
      document.getElementById('tg-active-phone').textContent = data.phone ? 'Phone: ' + data.phone : '';
      document.getElementById('tg-last-poll').textContent = data.last_poll_at ? 'Last update: ' + new Date(data.last_poll_at).toLocaleString() : 'Waiting for first update…';
    } else {
      summary.innerHTML = 'Watcher not authorised yet. Enter your phone below — Telegram will send you a code via SMS or in-app notification. <b>One-time setup.</b>';
      start.style.display = '';
    }
  } catch (e) {
    document.getElementById('tg-status-summary').textContent = 'Status check failed: ' + e;
  }
}

async function tgLoginStart(){
  const phone = document.getElementById('tg-phone').value.trim();
  const status = document.getElementById('tg-login-status');
  if (!phone) { status.textContent = 'Enter your phone with country code.'; return; }
  status.textContent = 'Requesting code from Telegram…';
  try {
    const r = await fetch('/api/admin/telegram/login/start', {
      method:'POST',
      headers:{'Authorization':'Bearer '+token,'Content-Type':'application/json'},
      body: JSON.stringify({phone}),
    });
    const data = await r.json();
    if (!data.success) { status.textContent = 'Error: ' + (data.error||'unknown'); return; }
    status.textContent = '✓ ' + (data.message || 'Code sent.');
    document.getElementById('tg-login-start').style.display = 'none';
    document.getElementById('tg-login-verify').style.display = '';
    document.getElementById('tg-phone-v').value = phone;
  } catch (e) {
    status.textContent = 'Network error: ' + e;
  }
}

async function tgLoginVerify(){
  const phone = document.getElementById('tg-phone-v').value.trim();
  const code  = document.getElementById('tg-code').value.trim();
  const status = document.getElementById('tg-verify-status');
  if (!phone || !code) { status.textContent = 'Phone + code are required.'; return; }
  status.textContent = 'Verifying…';
  try {
    const r = await fetch('/api/admin/telegram/login/verify', {
      method:'POST',
      headers:{'Authorization':'Bearer '+token,'Content-Type':'application/json'},
      body: JSON.stringify({phone, code}),
    });
    const data = await r.json();
    if (!data.success) { status.textContent = 'Error: ' + (data.error||'unknown'); return; }
    status.textContent = '✓ ' + (data.message || 'Authorised.');
    setTimeout(loadTelegramStatus, 1000);
  } catch (e) {
    status.textContent = 'Network error: ' + e;
  }
}

async function loadTelegramChannels(){
  try {
    const r = await fetch('/api/admin/telegram/channels', {headers:{'Authorization':'Bearer '+token}});
    const data = await r.json();
    if (data.success) {
      const names = data.channels.filter(c=>c.enabled).map(c=>'@'+c.channel).join(', ');
      const el = document.getElementById('tg-watched');
      if (el) el.textContent = names || '(none yet)';
    }
  } catch {}
}

async function submitTelegramOpportunity(){
  const channel = document.getElementById('tg-channel').value.trim().replace(/^@/,'');
  const sender  = document.getElementById('tg-sender').value.trim();
  const message = document.getElementById('tg-message').value.trim();
  const link    = document.getElementById('tg-link').value.trim();
  const status  = document.getElementById('tg-submit-status');
  if (!channel || !message) { status.textContent = 'Channel + message are required.'; return; }
  status.textContent = 'Scoring...';
  try {
    const r = await fetch('/api/admin/telegram/opportunities', {
      method: 'POST',
      headers: {'Authorization':'Bearer '+token, 'Content-Type':'application/json'},
      body: JSON.stringify({channel, message, sender: sender||null, link: link||null}),
    });
    const data = await r.json();
    if (!data.success) { status.textContent = 'Error: ' + (data.error||'unknown'); return; }
    status.textContent = `✓ Scored ${data.score}/100 (service: ${data.service_type||'none'})`;
    document.getElementById('tg-message').value = '';
    document.getElementById('tg-link').value = '';
    loadTgOpportunities('');
  } catch (e) {
    status.textContent = 'Network error: ' + e;
  }
}

async function loadTgOpportunities(filterStatus){
  const out = document.getElementById('tg-opportunities-list');
  out.innerHTML = '<div class="loading">Loading…</div>';
  try {
    const qs = filterStatus ? `?status=${encodeURIComponent(filterStatus)}` : '';
    const r = await fetch('/api/admin/telegram/opportunities'+qs, {headers:{'Authorization':'Bearer '+token}});
    const data = await r.json();
    if (!data.success) { out.innerHTML = `<div class="msg msg-error">${data.error||'Failed'}</div>`; return; }
    const items = data.opportunities || [];
    if (items.length === 0) {
      out.innerHTML = '<div class="empty"><span class="empty-icon">✈️</span>No opportunities yet. Paste one above.</div>';
      return;
    }
    out.innerHTML = items.map(o => {
      const score = o.score != null ? o.score : 0;
      const cls = score >= 70 ? 'score-high' : score >= 40 ? 'score-mid' : 'score-low';
      const svc = o.service_type ? `<span class="badge" style="background:rgba(122,76,255,0.15);color:var(--purple);padding:2px 8px;border-radius:10px;font-size:0.75rem;margin-left:6px">${o.service_type}</span>` : '';
      const linkBtn = o.link ? `<a href="${o.link}" target="_blank" class="btn btn-sm btn-copy">↗ Open</a>` : '';
      const statusBadge = `<span class="score-badge ${o.status==='new'?'score-mid':o.status==='won'?'score-high':'score-low'}" style="margin-right:8px">${o.status}</span>`;
      return `
        <div class="pb-job-row" style="display:block;margin-bottom:10px;padding:14px">
          <div style="display:flex;gap:8px;align-items:center;margin-bottom:6px;flex-wrap:wrap">
            <span class="score-badge ${cls}">${score}/100</span>
            ${svc}
            <span style="color:var(--dim);font-size:0.85rem">@${o.channel}${o.sender?' · '+o.sender:''}</span>
            <span style="margin-left:auto">${statusBadge}</span>
          </div>
          <div style="color:var(--accent);font-size:0.9rem;line-height:1.5;margin-bottom:6px;white-space:pre-wrap">${o.message.replace(/</g,'&lt;')}</div>
          ${o.score_reason ? `<div style="color:var(--dim);font-size:0.78rem;font-style:italic;margin-bottom:8px">${o.score_reason}</div>` : ''}
          <div style="display:flex;gap:6px;flex-wrap:wrap">
            ${linkBtn}
            <button class="btn btn-sm btn-copy" onclick="tgSetStatus('${o.id}','contacted')">Contacted</button>
            <button class="btn btn-sm btn-copy" onclick="tgSetStatus('${o.id}','won')">Won</button>
            <button class="btn btn-sm btn-copy" onclick="tgSetStatus('${o.id}','ignored')">Ignore</button>
          </div>
        </div>`;
    }).join('');
  } catch (e) {
    out.innerHTML = `<div class="msg msg-error">Network error: ${e}</div>`;
  }
}

async function tgSetStatus(id, status){
  try {
    await fetch(`/api/admin/telegram/opportunities/${id}`, {
      method: 'PATCH',
      headers: {'Authorization':'Bearer '+token, 'Content-Type':'application/json'},
      body: JSON.stringify({status}),
    });
    loadTgOpportunities('');
  } catch {}
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
  document.getElementById('view-clipgen').style.display   = v==='clipgen'?'':'none';
  // Toggle the active class instead of inline-style overrides — keeps the
  // ghost/primary palette consistent with the rest of the UI.
  const bp = document.getElementById('btn-prospects');
  const bc = document.getElementById('btn-clipgen');
  bp.classList.toggle('btn-primary', v==='prospects');
  bp.classList.toggle('btn-ghost',   v!=='prospects');
  bc.classList.toggle('btn-primary', v==='clipgen');
  bc.classList.toggle('btn-ghost',   v!=='clipgen');
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
  if(!clips.length) { list.innerHTML='<div class="empty">No clips generated</div>'; }
  else {
    list.innerHTML = clips.map((c,i)=>`
      <div class="pb-job-row">
        <span class="pb-job-url">Clip ${i+1}: ${c.title||'Clip '+(i+1)} (${Math.round(c.duration_seconds||0)}s)</span>
        ${c.r2_clip_url?`<a href="${c.r2_clip_url}" target="_blank" class="btn btn-sm btn-copy">⬇ Download</a>`:'<span></span>'}
        ${c.r2_clip_url?`<button class="btn btn-sm btn-copy" onclick="copyText('${encodeURIComponent(c.r2_clip_url)}')">🔗 Copy</button>`:'<span></span>'}
        <span></span>
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
        use crate::phantombuster_client::LinkedInPhantomKind;

        // Prefer non-Sales-Nav phantom because it works with ANY LinkedIn
        // cookie (Sales Nav requires an active $99/mo subscription on the
        // account backing the cookie). Fall back to SN if that's all there is.
        let (agent, kind) = match pb.find_any_linkedin_search_agent(true).await {
            Ok(Some((a, k))) => (a, k),
            Ok(None) => {
                // Nothing usable. Inspect what IS in the workspace and tell
                // the user what to add.
                let available = pb.list_linkedin_phantoms().await.unwrap_or_default();
                let names: Vec<String> = available.iter().map(|(a, _)| a.name.clone()).collect();
                return Json(json!({
                    "success": false,
                    "error":   "No usable LinkedIn search phantom in your PhantomBuster workspace.",
                    "hint":    "Add either 'LinkedIn Search Export' (works with any LinkedIn cookie) or 'LinkedIn Sales Navigator Search Export' (requires Sales Navigator subscription) from the PhantomBuster Phantom Store.",
                    "available_linkedin_phantoms": names,
                }));
            }
            Err(e) => return Json(json!({"success": false, "error": format!("Failed to list agents: {}", e)})),
        };

        let empty = vec![];
        let (url, cid) = match kind {
            LinkedInPhantomKind::SalesNavSearch => {
                // Sales Nav — build the Sales Navigator search URL.
                let u = crate::phantombuster_client::PhantomBusterClient::build_search_url(
                    req.job_titles.as_deref().unwrap_or(&empty),
                    req.industries.as_deref().unwrap_or(&empty),
                    req.company_sizes.as_deref().unwrap_or(&empty),
                    req.locations.as_deref().unwrap_or(&empty),
                    req.seniority.as_deref().unwrap_or(&empty),
                );
                tracing::info!("LinkedIn smart search (Sales Nav) URL: {}", u);
                let c = match pb.launch_agent(&agent.id, &u, &session_cookie, max).await {
                    Ok(id) => id,
                    Err(e) => return Json(json!({"success": false, "error": e})),
                };
                (u, c)
            }
            LinkedInPhantomKind::LinkedInSearch => {
                // Regular LinkedIn — phantom accepts a keyword string OR a
                // linkedin.com/search/results/people URL. We build a simple
                // keyword query from the AI-generated filters.
                let mut keywords: Vec<String> = Vec::new();
                if let Some(titles) = req.job_titles.as_ref() {
                    keywords.extend(titles.iter().map(|t| t.to_string()));
                }
                if let Some(industries) = req.industries.as_ref() {
                    keywords.extend(industries.iter().map(|i| i.to_string()));
                }
                if let Some(locs) = req.locations.as_ref() {
                    keywords.extend(locs.iter().map(|l| l.to_string()));
                }
                let search = if keywords.is_empty() {
                    "content creator".to_string()
                } else {
                    keywords.join(" OR ")
                };
                tracing::info!("LinkedIn smart search (regular) keywords: {}", search);
                let c = match pb.launch_linkedin_search(&agent.id, &search, &session_cookie, max).await {
                    Ok(id) => id,
                    Err(e) => return Json(json!({"success": false, "error": e})),
                };
                (search, c)
            }
            _ => {
                return Json(json!({
                    "success": false,
                    "error":   format!("Phantom '{}' is a LinkedIn phantom but not a search phantom — can't use it for smart-search.", agent.name),
                }));
            }
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
    Extension(state):  Extension<Arc<AppState>>,
    Extension(claims): Extension<Claims>,
    Json(req):         Json<InstagramSearchRequest>,
) -> Json<serde_json::Value> {
    let user_id: i32 = claims.sub.parse().unwrap_or(0);
    if user_id == 0 {
        return Json(json!({"success": false, "error": "Invalid user id in JWT"}));
    }

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

    let (job_id, status, container_id) = match try_launch_or_queue_ig_hashtag_job(
        &state, pb, &agent, &session_cookie, &hashtag, max_posts, user_id,
    ).await {
        Ok(t)  => t,
        Err(e) => return Json(json!({"success": false, "error": e})),
    };

    let message = match status {
        "queued" => format!(
            "Your PhantomBuster plan is busy with another search. #{} is queued — it'll auto-launch in a minute or two. Job ID: {}",
            hashtag, job_id
        ),
        _ => format!(
            "PhantomBuster Instagram Hashtag Search launched for #{}. Results typically ready in 3–10 minutes. Job ID: {}",
            hashtag, job_id
        ),
    };

    Json(json!({
        "success":      true,
        "job_id":       job_id.to_string(),
        "status":       status,
        "container_id": container_id,
        "agent_name":   agent.name,
        "hashtag":      hashtag,
        "category":     category,
        "max_posts":    max_posts,
        "message":      message,
    }))
}

/// GET /api/instagram/leads
/// List Instagram leads with optional filtering.
async fn instagram_list_leads(
    Extension(state):  Extension<Arc<AppState>>,
    Extension(claims): Extension<Claims>,
    Query(q):          Query<InstagramListQuery>,
) -> Json<serde_json::Value> {
    let user_id: i32 = claims.sub.parse().unwrap_or(0);
    if user_id == 0 {
        return Json(json!({"success": false, "error": "Invalid user id in JWT"}));
    }

    let limit  = q.limit.unwrap_or(50).min(200);
    let offset = q.offset.unwrap_or(0);

    // Scope to the caller's own leads. The first bind is always user_id;
    // subsequent optional filters start at $2.
    let mut sql = String::from(
        "SELECT id, username, full_name, bio, followers_count, following_count, posts_count,
                profile_url, profile_pic_url, is_private, is_verified, category,
                hashtag_source, email, external_url, dm_script, contact_status,
                pb_job_id, score, score_reason, service_type, created_at
         FROM instagram_leads WHERE user_id = $1"
    );
    let mut binds: Vec<String> = Vec::new();

    if let Some(ref ht) = q.hashtag {
        binds.push(ht.trim_start_matches('#').to_string());
        sql.push_str(&format!(" AND hashtag_source = ${}", binds.len() + 1));
    }
    if let Some(ref cs) = q.contact_status {
        binds.push(cs.clone());
        sql.push_str(&format!(" AND contact_status = ${}", binds.len() + 1));
    }
    if let Some(mf) = q.min_followers {
        binds.push(mf.to_string());
        sql.push_str(&format!(" AND followers_count >= ${}::bigint", binds.len() + 1));
    }
    sql.push_str(" ORDER BY followers_count DESC NULLS LAST");
    sql.push_str(&format!(" LIMIT {} OFFSET {}", limit, offset));

    // Build and execute dynamically — use raw query for variable bind count
    let mut query = sqlx::query(&sql).bind(user_id);
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
            "score":           r.get::<Option<i32>, _>("score"),
            "score_reason":    r.get::<Option<String>, _>("score_reason"),
            "service_type":    r.get::<Option<String>, _>("service_type"),
        })
    }).collect();

    Json(json!({"success": true, "leads": leads, "count": leads.len()}))
}

/// POST /api/instagram/leads/:id/generate-dm
/// Generate a personalized Instagram cold DM script using AI.
async fn instagram_generate_dm(
    Extension(state):  Extension<Arc<AppState>>,
    Extension(claims): Extension<Claims>,
    Path(id):          Path<uuid::Uuid>,
    Json(req):         Json<Option<InstagramDmRequest>>,
) -> Json<serde_json::Value> {
    fn format_unlock_price_usd(price: f64) -> String {
        if (price.fract()).abs() < f64::EPSILON {
            format!("{:.0}", price)
        } else {
            format!("{:.2}", price)
        }
    }

    let user_id: i32 = claims.sub.parse().unwrap_or(0);

    // Fetch the lead — scoped to the caller so one user can't DM another
    // user's lead or learn that another user has that lead.
    let row = match sqlx::query(
        "SELECT username, full_name, bio, followers_count, category, hashtag_source,
                external_url, service_type, sample_delivery_id
         FROM instagram_leads WHERE id = $1 AND user_id = $2"
    )
    .bind(id)
    .bind(user_id)
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
    let category:  String          = row.get::<Option<String>, _>("category").unwrap_or_default();
    let ext_url:   String          = row.get::<Option<String>, _>("external_url").unwrap_or_default();
    let service:   Option<String>  = row.get::<Option<String>, _>("service_type");
    let sample_id: Option<uuid::Uuid> = row.try_get("sample_delivery_id").ok().flatten();

    // Build the sample-link block injected into the prompt. If a sample
    // exists, the LLM is instructed to work the URL in naturally; without
    // one, we tell the model not to invent a link.
    let base_url = std::env::var("PUBLIC_BASE_URL").unwrap_or_else(|_| "https://www.videosync.video".to_string());
    let sample_block =
        "NO sample exists yet — do NOT invent a URL. Offer to make one: \"want me to put together a quick sample?\"".to_string();

    let has_website = !ext_url.trim().is_empty();
    let sample_block = match sample_id {
        Some(sid) => {
            let sample_unlock_price = sqlx::query_scalar::<_, sqlx::types::Decimal>(
                "SELECT unlock_price_usdc FROM deliveries WHERE id = $1"
            )
            .bind(sid)
            .fetch_optional(&state.db_pool)
            .await
            .ok()
            .flatten()
            .and_then(|d| d.to_string().parse::<f64>().ok())
            .unwrap_or_else(|| unlock_price_for(service.as_deref(), has_website));
            let sample_unlock_price_str = format_unlock_price_usd(sample_unlock_price);

            format!(
                "SAMPLE LINK (must appear in the DM, woven in naturally - e.g. \"here's a quick mockup I made: <URL>\" - do NOT just paste it at the end): {}/delivery/{}\nThe link shows a free watermarked preview. Full HD download is ${} USDC - the DM can hint at that naturally (\"full HD version is ${} if you want it for prod\") but don't hard-sell it.",
                base_url.trim_end_matches('/'),
                sid,
                sample_unlock_price_str,
                sample_unlock_price_str
            )
        }
        None => sample_block,
    };

    let niche = req.as_ref().and_then(|r| r.niche.as_deref()).unwrap_or(&category);

    let followers_str = if followers > 0 { followers.to_string() } else { "unknown".to_string() };

    let service_block = service_offer_prompt(service.as_deref());

    let prompt = format!(
        r#"You are a senior outbound copywriter for a video production studio. Write a personalized Instagram DM (≤120 words) to this creator.

Creator info:
- Handle:       @{username}
- Name:         {full_name}
- Followers:    {followers}
- Bio / recent post caption: {bio}
- Niche hint:   {niche}
- Link in bio:  {ext_url}

{service_block}

{sample_block}

The DM must:
- Be specific. Reference an actual detail from the bio/caption — never generic flattery.
- State concretely what the studio would do for THEM, with the realistic price from the service block above.
- If a SAMPLE LINK is provided above, weave the full URL into the DM body naturally. Never replace it with "link" or paraphrase — paste the exact URL. If no sample exists, offer to make one.
- End with a clear ask: "want me to send over the breakdown?" or "does this fit what you had in mind?"
- Sound like one founder messaging another. Casual, lowercase ok, no corporate fluff, no emoji spam.
- Adapt tone to follower size — micro creators (<10k) warm + helpful; mid creators (10k–100k) business-direct; big creators (>100k) concise with clear ROI angle.

Output ONLY the DM body. No quotes, no labels, no preamble."#,
        username      = username,
        full_name     = full_name,
        followers     = followers_str,
        bio           = bio,
        niche         = niche,
        ext_url       = ext_url,
        service_block = service_block,
        sample_block  = sample_block,
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
        "UPDATE instagram_leads SET dm_script = $1, updated_at = NOW()
         WHERE id = $2 AND user_id = $3"
    )
    .bind(&dm_text)
    .bind(id)
    .bind(user_id)
    .execute(&state.db_pool)
    .await;

    Json(json!({"success": true, "dm_script": dm_text}))
}

/// POST /api/instagram/leads/:id/generate-sample
///
/// Generates a portfolio sample tailored to the lead's `service_type` and
/// returns a public /delivery/:id link the user can paste into the DM.
///
/// Service routing:
/// * `thumbnails`  → Blender `thumbnail` (PNG)
/// * `animations`  → Blender `title_card` (15s MP4) — cheapest scene the
///                   server reliably renders without input data.
/// * `ugc` / `full_stack` → Blender `ui_mockup` placeholder
/// * `clipping`    → returns `requires_source_url=true` because clipping
///                   needs a video URL the user supplies. The frontend
///                   should prompt for it then call /api/admin/deliveries
///                   directly. (Manual clipping isn't a delivery type yet.)
#[derive(Debug, Deserialize)]
struct GenerateSampleRequest {
    /// Optional override — for `clipping`, the user must paste a video URL.
    source_url: Option<String>,
}

async fn instagram_generate_sample(
    Extension(state):  Extension<Arc<AppState>>,
    Extension(claims): Extension<Claims>,
    Path(id):          Path<uuid::Uuid>,
    Json(req):         Json<Option<GenerateSampleRequest>>,
) -> Json<serde_json::Value> {
    let user_id: i32 = claims.sub.parse().unwrap_or(0);
    let source_url = req.as_ref().and_then(|r| r.source_url.clone());

    let row = match sqlx::query(
        "SELECT username, full_name, bio, service_type, sample_delivery_id, external_url
         FROM instagram_leads WHERE id = $1 AND user_id = $2"
    )
    .bind(id)
    .bind(user_id)
    .fetch_optional(&state.db_pool)
    .await {
        Ok(Some(r)) => r,
        Ok(None)    => return Json(json!({"success": false, "error": "Lead not found"})),
        Err(e)      => return Json(json!({"success": false, "error": format!("DB error: {}", e)})),
    };

    // If a sample is already attached, just return it — re-generating costs
    // money and the user wants the link to paste, not a fresh render.
    if let Some(existing) = row.get::<Option<uuid::Uuid>, _>("sample_delivery_id") {
        return Json(json!({
            "success":      true,
            "delivery_id":  existing.to_string(),
            "delivery_url": format!("/delivery/{}", existing),
            "message":      "Sample already exists for this lead.",
        }));
    }

    let username:     String         = row.get::<Option<String>, _>("username").unwrap_or_default();
    let full_name:    String         = row.get::<Option<String>, _>("full_name").unwrap_or_else(|| username.clone());
    let bio:          String         = row.get::<Option<String>, _>("bio").unwrap_or_default();
    let service:      Option<String> = row.get::<Option<String>, _>("service_type");
    let lead_ext_url: Option<String> = row.get::<Option<String>, _>("external_url");

    // Auto-detect the lead's website: prefer explicit source_url from request,
    // fall back to the lead's external_url from their profile (IG bio link, etc.)
    let source_url = source_url.or(lead_ext_url);

    // For clipping the lead, we'd need their actual video URL. Tell the
    // frontend so it can ask the user for one. Don't burn a render slot on
    // a placeholder for clipping — the value is in clipping THEIR content.
    if matches!(service.as_deref(), Some("clipping")) {
        return Json(json!({
            "success":              false,
            "requires_source_url":  true,
            "error":                "Clipping samples need a YouTube/podcast/Twitch URL. Paste one of @{username}'s long-form videos and try again.".replace("{username}", &username),
        }));
    }

    // Build a delivery row pointing at a lightweight Blender render.
    // Default to `thumbnail` if the scorer hasn't set a service yet.
    //
    // For service types that benefit from a reference image (product_mockup
    // and landing_page), we generate the image via Gemini Nano Banana or
    // use a user-supplied source URL, then pass the resulting S3 URL to
    // the Blender tool.
    let (gig_type, prompt, style, duration, extra) = match service.as_deref() {
        Some("animations") => (
            "title_card",
            format!("{} — channel intro", full_name),
            "modern",
            8.0,
            json!({"subtitle": bio.chars().take(60).collect::<String>()}),
        ),

        Some("product_mockup") => {
            // If the lead has a website, scrape their og:image first —
            // showing their ACTUAL product is way more compelling than
            // a generic AI-generated image.
            let img_url = if let Some(ref url) = source_url {
                match fetch_landing_page_hero(url).await {
                    Some(u) => Some(u),
                    None => {
                        let product_prompt = format!(
                            "Professional product photograph, clean white background, studio lighting: {}",
                            bio.chars().take(200).collect::<String>()
                        );
                        try_generate_image(&state, &product_prompt).await
                    }
                }
            } else {
                let product_prompt = format!(
                    "Professional product photograph, clean white background, studio lighting: {}",
                    bio.chars().take(200).collect::<String>()
                );
                try_generate_image(&state, &product_prompt).await
            };
            (
                "ui_mockup",
                format!("{} — product showcase mockup", full_name),
                "modern",
                8.0,
                json!({
                    "device":         "phone",
                    "animation":      "zoom_in",
                    "screenshot_url": img_url.unwrap_or_default(),
                }),
            )
        }

        Some("landing_page") => {
            // If the user pasted a landing-page URL, try to pull the hero
            // image (og:image meta tag) from it. Fall back to synthesising
            // one via Gemini if the site has no usable meta or we can't
            // reach it. Fall further back to empty — Blender renders a
            // scene without a reference image rather than erroring.
            let hero_url = match source_url.as_deref() {
                Some(url) => match fetch_landing_page_hero(url).await {
                    Some(u) => u,
                    None => {
                        let p = format!("Clean SaaS landing-page hero illustration: {}", bio.chars().take(150).collect::<String>());
                        try_generate_image(&state, &p).await.unwrap_or_default()
                    }
                },
                None => {
                    let p = format!(
                        "Clean SaaS landing-page hero illustration for: {}. Modern, gradient background, tech/startup aesthetic.",
                        bio.chars().take(200).collect::<String>()
                    );
                    try_generate_image(&state, &p).await.unwrap_or_default()
                }
            };
            (
                "scene",
                format!("Animated landing page for {}", full_name),
                "modern",
                15.0,
                json!({
                    "reference_image_url": hero_url,
                    "animation_style":     "parallax",
                    "source_url":          source_url,
                }),
            )
        }

        Some("ugc") | Some("full_stack") => (
            "ui_mockup",
            format!("{}'s product showcase", full_name),
            "modern",
            6.0,
            json!({"device": "phone", "animation": "fade_in"}),
        ),

        _ => {
            // If the lead has a website, generate a scene animation with
            // their hero image instead of a plain thumbnail — far more
            // impressive in a DM.
            if let Some(ref url) = source_url {
                if let Some(hero) = fetch_landing_page_hero(url).await {
                    (
                        "scene",
                        format!("Professional animated showcase for {}", full_name),
                        "modern",
                        10.0,
                        json!({"reference_image_url": hero, "source_url": url}),
                    )
                } else {
                    (
                        "thumbnail",
                        format!("Eye-catching thumbnail for @{} — {}", username, bio.chars().take(80).collect::<String>()),
                        "bold",
                        0.0,
                        json!({"title_text": full_name}),
                    )
                }
            } else {
                (
                    "thumbnail",
                    format!("Eye-catching thumbnail for @{} — {}", username, bio.chars().take(80).collect::<String>()),
                    "bold",
                    0.0,
                    json!({"title_text": full_name}),
                )
            }
        }
    };

    let title = format!("Sample for @{}", username);

    // Stamp the origin lead on the delivery so future x402 unlocks can
    // trace revenue back to the whitelisted user who sourced this lead.
    // Price is market-aligned per service type + whether we have a
    // website URL (agent-generated comprehensive videos cost more).
    let unlock_price = unlock_price_for(service.as_deref(), source_url.is_some());
    let source_url_str: Option<String> = source_url.clone();
    let delivery_id: uuid::Uuid = match sqlx::query_scalar::<_, uuid::Uuid>(
        "INSERT INTO deliveries (client_ref, title, gig_type, prompt, style, duration, extra_args, status,
                                  sourced_from_lead_id, unlock_price_usdc, source_url)
         VALUES ($1, $2, $3, $4, $5, $6, $7, 'pending', $8, $9, $10)
         RETURNING id"
    )
    .bind(format!("ig:{}", username))
    .bind(&title)
    .bind(gig_type)
    .bind(&prompt)
    .bind(style)
    .bind(duration)
    .bind(&extra)
    .bind(id)
    .bind(unlock_price)
    .bind(source_url_str.as_deref())
    .fetch_one(&state.db_pool)
    .await {
        Ok(id) => id,
        Err(e) => return Json(json!({"success": false, "error": format!("DB insert failed: {}", e)})),
    };

    // Attach the sample to the lead so re-clicking returns the same one.
    let _ = sqlx::query(
        "UPDATE instagram_leads SET sample_delivery_id = $1, updated_at = NOW()
         WHERE id = $2 AND user_id = $3"
    )
    .bind(delivery_id)
    .bind(id)
    .bind(user_id)
    .execute(&state.db_pool)
    .await;

    // For high-value leads with a website, route through the full AI agent
    // to generate a comprehensive video (script + Blender scenes + voiceover).
    // For simpler cases, use the direct Blender render path.
    let has_website = source_url.is_some();
    let render_state = state.clone();
    if has_website && gig_type == "scene" {
        let website_url = source_url.clone().unwrap_or_default();
        let lead_name = full_name.clone();
        let lead_bio = bio.clone();
        tokio::spawn(async move {
            run_agent_video_for_lead(delivery_id, &website_url, &lead_name, &lead_bio, render_state).await;
        });
    } else {
        tokio::spawn(async move {
            crate::handlers::admin::run_delivery_job(delivery_id, render_state).await;
        });
    }

    Json(json!({
        "success":      true,
        "delivery_id":  delivery_id.to_string(),
        "delivery_url": format!("/delivery/{}", delivery_id),
        "service":      service,
        "message":      "Sample queued. Render takes 1-3 minutes; the /delivery/:id link is shareable immediately and will show the result when ready.",
    }))
}

/// PATCH /api/instagram/leads/:id/contact-status
async fn instagram_update_contact_status(
    Extension(state):  Extension<Arc<AppState>>,
    Extension(claims): Extension<Claims>,
    Path(id):          Path<uuid::Uuid>,
    Json(req):         Json<InstagramContactStatusRequest>,
) -> Json<serde_json::Value> {
    let user_id: i32 = claims.sub.parse().unwrap_or(0);
    let valid = ["new", "contacted", "replied", "converted", "skipped"];
    if !valid.contains(&req.contact_status.as_str()) {
        return Json(json!({"success": false, "error": "contact_status must be one of: new, contacted, replied, converted, skipped"}));
    }

    // Stamp funnel-stage timestamps as the lead moves through states —
    // the revenue ledger uses these to show per-user conversion rates.
    // COALESCE on first_contacted_at preserves the earliest contact time
    // (don't reset if they flip contacted → replied → contacted).
    match sqlx::query(
        "UPDATE instagram_leads
         SET contact_status     = $1,
             first_contacted_at = CASE
                WHEN $1 IN ('contacted','replied','converted') THEN COALESCE(first_contacted_at, NOW())
                ELSE first_contacted_at
             END,
             converted_at       = CASE
                WHEN $1 = 'converted' THEN COALESCE(converted_at, NOW())
                ELSE converted_at
             END,
             updated_at         = NOW()
         WHERE id = $2 AND user_id = $3"
    )
    .bind(&req.contact_status)
    .bind(id)
    .bind(user_id)
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
    Extension(state):  Extension<Arc<AppState>>,
    Extension(claims): Extension<Claims>,
    Json(req):         Json<Option<AutoDiscoverRequest>>,
) -> Json<serde_json::Value> {
    let user_id: i32 = claims.sub.parse().unwrap_or(0);
    if user_id == 0 {
        return Json(json!({"success": false, "error": "Invalid user id in JWT"}));
    }

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

    // Only the first hashtag will actually launch — the rest queue because
    // PhantomBuster caps parallel runs per workspace. The dispatcher below
    // promotes queued jobs one at a time as slots free up.
    for hashtag in &hashtags {
        let tag = hashtag.trim_start_matches('#').to_string();
        match try_launch_or_queue_ig_hashtag_job(
            &state, pb, &agent, &session_cookie, &tag, max_posts, user_id,
        ).await {
            Ok((job_id, status, container_id)) => {
                launched_jobs.push(json!({
                    "job_id":       job_id.to_string(),
                    "container_id": container_id,
                    "hashtag":      tag,
                    "status":       status,
                }));
            }
            Err(e) => errors.push(format!("#{}: {}", tag, e)),
        }
        // Keep a small gap between DB + PB calls so we don't race ourselves.
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
    Extension(state):  Extension<Arc<AppState>>,
    Extension(claims): Extension<Claims>,
) -> Json<serde_json::Value> {
    let user_id: i32 = claims.sub.parse().unwrap_or(0);
    let rows = match sqlx::query(
        "SELECT id, username, full_name, bio, followers_count, profile_url, profile_pic_url,
                is_verified, category, hashtag_source, email, external_url,
                dm_script, contact_status, score, score_reason
         FROM instagram_leads
         WHERE user_id = $1
           AND score >= 60
           AND contact_status = 'new'
           AND is_private = FALSE
         ORDER BY score DESC, followers_count DESC NULLS LAST
         LIMIT 50"
    )
    .bind(user_id)
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
        "SELECT id, agent_id, search_url, user_id
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
        let job_id:   uuid::Uuid        = row.get("id");
        let agent_id: String            = row.get("agent_id");
        let search_url: String          = row.get("search_url");
        let job_user_id: Option<i32>    = row.try_get("user_id").ok().flatten();

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
            // Skip private accounts — they can't receive DMs from non-followers.
            if lead.is_private { continue; }
            // NOTE: do NOT filter by followers_count here. The Instagram
            // Hashtag Search Export returns post schema (no follower count), so
            // gating on >=1000 would drop every lead. Hashtag relevance is
            // itself a qualification — AI scoring and the DM generator decide
            // which to pursue.

            let result = sqlx::query(
                "INSERT INTO instagram_leads
                    (username, full_name, bio, followers_count, following_count, posts_count,
                     profile_url, profile_pic_url, is_private, is_verified, external_url, email,
                     category, hashtag_source, contact_status, user_id)
                 VALUES ($1,$2,$3,$4,$5,$6,$7,$8,$9,$10,$11,$12,$13,$14,'new',$15)
                 ON CONFLICT (user_id, username) DO UPDATE
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
            .bind(job_user_id)
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

        // Score unscored leads (top 20 by followers to conserve LLM quota).
        // Per-user scope so one user doesn't trigger another user's scoring.
        if state.gemini_client.is_some() || state.nvidia_nim_client.is_some() {
            if let Some(uid) = job_user_id {
                score_instagram_leads(state, &hashtag_source, uid).await;
            }
        }
    }
}

// ============================================================================
// PhantomBuster launch queue
// ============================================================================
//
// PhantomBuster plans cap the number of concurrent phantom runs (free: 1).
// Firing multiple searches at once — or auto-discover's N-hashtag fan-out —
// causes "Maximum number of parallel executions reached" errors from PB.
// We serialize launches by agent: if an agent already has a `running` job,
// newer launches are stored with `status='queued'` and dispatched by a
// background task (`dispatch_queued_pb_jobs`) as slots free up.

/// Returns `true` if the given PB client has a `running` job on `agent_id`.
async fn agent_has_running_job(pool: &sqlx::PgPool, agent_id: &str) -> bool {
    let n: i64 = sqlx::query_scalar(
        "SELECT COUNT(*) FROM phantombuster_jobs
         WHERE agent_id = $1 AND status = 'running'"
    )
    .bind(agent_id)
    .fetch_one(pool)
    .await
    .unwrap_or(0);
    n > 0
}

/// Heuristic: does this PB error look like "parallel executions reached"?
/// PB varies the wording ("Maximum number of parallel executions reached",
/// "parallel_limit", "too many runs"), so we match loosely on the English
/// user-facing phrase.
fn is_pb_parallel_limit_error(err: &str) -> bool {
    let lc = err.to_lowercase();
    lc.contains("parallel") || lc.contains("maximum number")
        || lc.contains("concurrency limit") || lc.contains("too many")
}

/// Try to launch an Instagram Hashtag Search — or enqueue it if the agent
/// already has a running job (or PB rejects the launch for parallelism).
///
/// Returns `(job_id, status, container_id_opt)` where `status` is either
/// `"running"` (launched immediately) or `"queued"` (will be launched by the
/// dispatcher when a slot frees up).
async fn try_launch_or_queue_ig_hashtag_job(
    state:          &Arc<AppState>,
    pb:             &crate::phantombuster_client::PhantomBusterClient,
    agent:          &crate::phantombuster_client::PbAgent,
    session_cookie: &str,
    hashtag:        &str,
    max_posts:      u32,
    user_id:        i32,
) -> Result<(uuid::Uuid, &'static str, Option<String>), String> {
    let tag = hashtag.trim_start_matches('#').to_string();
    let search_url = format!("instagram:#{}", tag);

    // Check occupancy first — cheaper than bouncing off PB's parallel limit.
    // Global occupancy (any user), because PB's limit is per-workspace.
    let occupied = agent_has_running_job(&state.db_pool, &agent.id).await;

    if !occupied {
        match pb.launch_instagram_hashtag_search(&agent.id, session_cookie, &tag, max_posts).await {
            Ok(container_id) => {
                let job_id = sqlx::query_scalar::<_, uuid::Uuid>(
                    "INSERT INTO phantombuster_jobs
                        (agent_id, agent_name, search_url, status, launched_at, user_id)
                     VALUES ($1, $2, $3, 'running', NOW(), $4) RETURNING id"
                )
                .bind(&agent.id)
                .bind(&agent.name)
                .bind(&search_url)
                .bind(user_id)
                .fetch_one(&state.db_pool)
                .await
                .map_err(|e| format!("DB insert failed: {}", e))?;
                return Ok((job_id, "running", Some(container_id)));
            }
            Err(e) if is_pb_parallel_limit_error(&e) => {
                tracing::warn!("PB parallel limit hit launching #{}: {} — queuing", tag, e);
                // fall through to queue insert below
            }
            Err(e) => return Err(e),
        }
    }

    // Queue it — dispatcher will launch when the running job completes.
    let job_id = sqlx::query_scalar::<_, uuid::Uuid>(
        "INSERT INTO phantombuster_jobs
            (agent_id, agent_name, search_url, status, created_at, user_id)
         VALUES ($1, $2, $3, 'queued', NOW(), $4) RETURNING id"
    )
    .bind(&agent.id)
    .bind(&agent.name)
    .bind(&search_url)
    .bind(user_id)
    .fetch_one(&state.db_pool)
    .await
    .map_err(|e| format!("DB insert failed: {}", e))?;

    tracing::info!("📥 Queued Instagram job {} for #{} (agent busy, user={})", job_id, tag, user_id);
    Ok((job_id, "queued", None))
}

/// Background dispatcher — runs periodically, promoting the oldest `queued`
/// job per agent to `running` if the agent has no in-flight job. One promote
/// per tick per agent keeps PB well under the parallel limit even across
/// different phantom types.
pub async fn dispatch_queued_pb_jobs(state: &Arc<AppState>) {
    let Some(pb) = state.phantombuster_client.as_ref() else { return; };
    let Ok(session_cookie) = std::env::var("INSTAGRAM_SESSION_COOKIE") else { return; };
    if session_cookie.is_empty() { return; }

    // Pick agents that have queued work AND no currently-running job.
    let rows = match sqlx::query(
        "SELECT DISTINCT q.agent_id
         FROM phantombuster_jobs q
         WHERE q.status = 'queued'
           AND NOT EXISTS (
             SELECT 1 FROM phantombuster_jobs r
             WHERE r.agent_id = q.agent_id AND r.status = 'running'
           )"
    )
    .fetch_all(&state.db_pool)
    .await {
        Ok(rs) => rs,
        Err(e) => {
            tracing::warn!("PB dispatcher DB query failed: {}", e);
            return;
        }
    };

    if rows.is_empty() { return; }

    for row in rows {
        let agent_id: String = row.get("agent_id");

        // Oldest queued job for this agent.
        let job = match sqlx::query(
            "SELECT id, search_url FROM phantombuster_jobs
             WHERE agent_id = $1 AND status = 'queued'
             ORDER BY created_at ASC LIMIT 1"
        )
        .bind(&agent_id)
        .fetch_optional(&state.db_pool)
        .await {
            Ok(Some(r)) => r,
            Ok(None)    => continue,
            Err(e)      => {
                tracing::warn!("PB dispatcher oldest-job query failed: {}", e);
                continue;
            }
        };

        let job_id:     uuid::Uuid = job.get("id");
        let search_url: String     = job.get("search_url");

        // Only Instagram hashtag jobs are auto-dispatched today. LinkedIn
        // launches require rebuilding the Sales Navigator URL, which is a
        // bigger retrofit — leave those to manual retry for now.
        let Some(tag) = search_url.strip_prefix("instagram:#")
            .or_else(|| search_url.strip_prefix("instagram:"))
            .map(|s| s.trim_start_matches('#').to_string())
        else {
            tracing::debug!("Skipping dispatch for non-IG queued job {} ({})", job_id, search_url);
            continue;
        };

        match pb.launch_instagram_hashtag_search(&agent_id, &session_cookie, &tag, 50).await {
            Ok(_container_id) => {
                let _ = sqlx::query(
                    "UPDATE phantombuster_jobs
                     SET status = 'running', launched_at = NOW()
                     WHERE id = $1"
                )
                .bind(job_id)
                .execute(&state.db_pool)
                .await;
                tracing::info!("🚀 Dispatched queued PB job {} for #{}", job_id, tag);
            }
            Err(e) if is_pb_parallel_limit_error(&e) => {
                tracing::debug!("PB still at parallel limit on agent {}; will retry next tick", agent_id);
                // leave status='queued' and try again next tick
            }
            Err(e) => {
                tracing::warn!("PB launch failed for queued job {}: {} — marking failed", job_id, e);
                let _ = sqlx::query(
                    "UPDATE phantombuster_jobs
                     SET status = 'failed', error = $1, completed_at = NOW()
                     WHERE id = $2"
                )
                .bind(e)
                .bind(job_id)
                .execute(&state.db_pool)
                .await;
            }
        }
    }
}

/// Score unscored Instagram leads for a given hashtag using AI.
async fn score_instagram_leads(state: &Arc<AppState>, hashtag: &str, user_id: i32) {
    // No `followers_count >= 1000` filter — hashtag-mode leads come from the
    // post schema and have NULL follower counts. Filtering on them dropped
    // every lead and the UI showed `—` for every score. Score what we have;
    // the model is told to judge by bio/handle alone when follower count
    // is unknown. Scoped to one user's leads so we don't pay to re-score
    // the same lead for multiple users.
    let unscored = match sqlx::query(
        "SELECT id, username, full_name, bio, followers_count, external_url
         FROM instagram_leads
         WHERE score IS NULL
           AND hashtag_source = $1
           AND user_id = $2
           AND is_private = FALSE
         ORDER BY COALESCE(followers_count, 0) DESC, created_at DESC
         LIMIT 20"
    )
    .bind(hashtag)
    .bind(user_id)
    .fetch_all(&state.db_pool)
    .await {
        Ok(r) => r,
        Err(_) => return,
    };

    for row in &unscored {
        let id:        uuid::Uuid    = row.get("id");
        let username:  String        = row.get::<Option<String>, _>("username").unwrap_or_default();
        let bio:       String        = row.get::<Option<String>, _>("bio").unwrap_or_default();
        let followers: Option<i64>   = row.get::<Option<i64>, _>("followers_count");
        let ext_url:   String        = row.get::<Option<String>, _>("external_url").unwrap_or_default();

        let followers_str = match followers {
            Some(n) if n > 0 => n.to_string(),
            _                => "unknown (came from hashtag search — judge by bio + handle)".to_string(),
        };

        let prompt = format!(
            r#"Score this Instagram creator as a potential client for a video production studio (0–100), and also pick the best service to pitch.

The studio offers:
- **clipping**       — long-form → Shorts/Reels. Best fit: podcasters, long-form YouTubers, Twitch streamers.
- **animations**     — Blender explainer/data-viz/LaTeX scenes. Best fit: educators, finance/crypto channels, news/data accounts.
- **thumbnails**     — AI-generated YouTube thumbnails. Best fit: growing YouTubers (5k–100k subs), MrBeast aspirants.
- **ugc**            — vertical product-demo ads. Best fit: Shopify/DTC founders, SaaS demos, brand accounts.
- **product_mockup** — photorealistic product shot on a device/scene. Best fit: ecommerce, hardware brands, app devs, Kickstarter creators.
- **landing_page**   — animated SaaS hero mockup (we can scrape their existing site URL). Best fit: SaaS/startup founders, no-code builders.
- **full_stack**   — bundle of the above. Best fit: 100k+ creators serious about scaling.

Creator profile:
- Username: @{username}
- Followers: {followers}
- Bio / recent post caption: {bio}
- Link in bio: {ext_url}

Score guidelines:
- 80–100: Clear paying client. Active creator/founder, monetised, has content the studio can act on right now.
- 60–79: Likely fit. Bio strongly hints at one of the service types and the audience size makes sense.
- 40–59: Ambiguous — could be a creator, could be a fan account, can't tell from bio alone.
- 0–39: Bad fit. Fan page, brand parody, private/spammy account, OR another freelance editor (competitor, not client).

Return ONLY valid JSON (no markdown, no code fence):
{{"score": 75, "service": "clipping", "reason": "podcaster with podcast link in bio, posts long-form clips"}}

`service` MUST be one of: clipping, animations, thumbnails, ugc, product_mockup, landing_page, full_stack."#,
            username  = username,
            followers = followers_str,
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
                let score   = v.get("score").and_then(|s| s.as_i64()).unwrap_or(0) as i32;
                let reason  = v.get("reason").and_then(|r| r.as_str()).unwrap_or("").to_string();
                // Service tag — coerced to one of the 5 known values; anything
                // else gets stored as NULL so the DM generator falls back to
                // "all services, AI picks one inline".
                let service_raw = v.get("service").and_then(|s| s.as_str()).unwrap_or("").to_lowercase();
                let service = match service_raw.as_str() {
                    "clipping" | "animations" | "thumbnails" | "ugc" | "product_mockup" | "landing_page" | "full_stack" => Some(service_raw),
                    _ => None,
                };

                let _ = sqlx::query(
                    "UPDATE instagram_leads
                     SET score = $1, score_reason = $2, service_type = $3
                     WHERE id = $4"
                )
                .bind(score)
                .bind(&reason)
                .bind(service.as_deref())
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

        let (score, reasoning, service, dm_creator, _) =
            score_prospect_with_ai(state, &name, audience, &enriched_desc, &category, "linkedin_lead").await;

        let _ = sqlx::query(
            "UPDATE prospects
             SET ai_score = $1, ai_reasoning = $2, dm_script_creator = $3,
                 service_type = $4, updated_at = NOW()
             WHERE id = $5"
        )
        .bind(score)
        .bind(&reasoning)
        .bind(&dm_creator)
        .bind(&service)
        .bind(id)
        .execute(&state.db_pool)
        .await;

        tokio::time::sleep(tokio::time::Duration::from_millis(200)).await;
    }

    tracing::info!("✅ LinkedIn AI scoring complete for job {}", job_id);
}

// ============================================================================
// Telegram opportunities (phase 1 — manual entry + AI scoring)
// ============================================================================
//
// Phase 2 will add an automated grammers-client userbot that polls the
// channels in `telegram_watch_channels` and inserts matching messages.
// For now admins paste messages they saw in Telegram into a form and
// the AI scores them as potential paid gigs.

#[derive(Debug, Deserialize)]
struct AddChannelReq {
    channel:    String,
    keyword_re: Option<String>,
}

async fn telegram_list_channels(
    Extension(state): Extension<Arc<AppState>>,
) -> Json<serde_json::Value> {
    let rows = match sqlx::query(
        "SELECT id, channel, keyword_re, enabled, created_at
         FROM telegram_watch_channels ORDER BY created_at ASC"
    )
    .fetch_all(&state.db_pool)
    .await {
        Ok(r) => r,
        Err(e) => return Json(json!({"success": false, "error": format!("DB error: {}", e)})),
    };
    let channels: Vec<serde_json::Value> = rows.iter().map(|r| json!({
        "id":         r.get::<i32, _>("id"),
        "channel":    r.get::<String, _>("channel"),
        "keyword_re": r.try_get::<Option<String>, _>("keyword_re").ok().flatten(),
        "enabled":    r.get::<bool, _>("enabled"),
    })).collect();
    Json(json!({"success": true, "channels": channels}))
}

async fn telegram_add_channel(
    Extension(state): Extension<Arc<AppState>>,
    Json(req):        Json<AddChannelReq>,
) -> Json<serde_json::Value> {
    let channel = req.channel.trim().trim_start_matches('@').to_string();
    if channel.is_empty() {
        return Json(json!({"success": false, "error": "channel is required"}));
    }
    match sqlx::query(
        "INSERT INTO telegram_watch_channels (channel, keyword_re)
         VALUES ($1, $2)
         ON CONFLICT (channel) DO UPDATE SET keyword_re = EXCLUDED.keyword_re, enabled = TRUE
         RETURNING id"
    )
    .bind(&channel)
    .bind(req.keyword_re.as_deref())
    .fetch_one(&state.db_pool)
    .await {
        Ok(_) => Json(json!({"success": true, "channel": channel})),
        Err(e) => Json(json!({"success": false, "error": format!("DB error: {}", e)})),
    }
}

async fn telegram_delete_channel(
    Extension(state):   Extension<Arc<AppState>>,
    Path(id):           Path<i32>,
) -> Json<serde_json::Value> {
    match sqlx::query("DELETE FROM telegram_watch_channels WHERE id = $1")
        .bind(id).execute(&state.db_pool).await {
        Ok(_) => Json(json!({"success": true})),
        Err(e) => Json(json!({"success": false, "error": format!("DB error: {}", e)})),
    }
}

#[derive(Debug, Deserialize)]
struct ListOpportunitiesQuery {
    status: Option<String>,
    limit:  Option<i64>,
}

async fn telegram_list_opportunities(
    Extension(state):  Extension<Arc<AppState>>,
    Extension(claims): Extension<Claims>,
    Query(q):          Query<ListOpportunitiesQuery>,
) -> Json<serde_json::Value> {
    let user_id: i32 = claims.sub.parse().unwrap_or(0);
    let limit = q.limit.unwrap_or(50).clamp(1, 200);

    let mut sql = String::from(
        "SELECT id, channel, message_id, sender, message, matched_kw, link,
                score, score_reason, service_type, status, source, created_at
         FROM telegram_opportunities
         WHERE user_id = $1"
    );
    if q.status.is_some() {
        sql.push_str(" AND status = $2");
    }
    sql.push_str(" ORDER BY COALESCE(score, 0) DESC, created_at DESC");
    sql.push_str(&format!(" LIMIT {}", limit));

    let rows = if let Some(s) = q.status.as_deref() {
        sqlx::query(&sql).bind(user_id).bind(s).fetch_all(&state.db_pool).await
    } else {
        sqlx::query(&sql).bind(user_id).fetch_all(&state.db_pool).await
    };
    let rows = match rows {
        Ok(r) => r,
        Err(e) => return Json(json!({"success": false, "error": format!("DB error: {}", e)})),
    };

    let items: Vec<serde_json::Value> = rows.iter().map(|r| json!({
        "id":           r.get::<uuid::Uuid, _>("id").to_string(),
        "channel":      r.get::<String, _>("channel"),
        "message":      r.get::<String, _>("message"),
        "sender":       r.try_get::<Option<String>, _>("sender").ok().flatten(),
        "link":         r.try_get::<Option<String>, _>("link").ok().flatten(),
        "score":        r.try_get::<Option<i32>, _>("score").ok().flatten(),
        "score_reason": r.try_get::<Option<String>, _>("score_reason").ok().flatten(),
        "service_type": r.try_get::<Option<String>, _>("service_type").ok().flatten(),
        "status":       r.get::<String, _>("status"),
        "source":       r.get::<String, _>("source"),
    })).collect();
    Json(json!({"success": true, "opportunities": items, "count": items.len()}))
}

#[derive(Debug, Deserialize)]
struct AddOpportunityReq {
    channel: String,
    message: String,
    sender:  Option<String>,
    link:    Option<String>,
}

async fn telegram_add_opportunity_manual(
    Extension(state):  Extension<Arc<AppState>>,
    Extension(claims): Extension<Claims>,
    Json(req):         Json<AddOpportunityReq>,
) -> Json<serde_json::Value> {
    let user_id: i32 = claims.sub.parse().unwrap_or(0);
    let channel = req.channel.trim().trim_start_matches('@').to_string();
    if channel.is_empty() || req.message.trim().is_empty() {
        return Json(json!({"success": false, "error": "channel and message are required"}));
    }

    // AI-score the opportunity using the same service menu as IG leads.
    // Returns (score 0-100, reason, service).
    let (score, reason, service) = score_telegram_opportunity(&state, &channel, &req.message).await;

    let id = match sqlx::query_scalar::<_, uuid::Uuid>(
        "INSERT INTO telegram_opportunities
           (channel, message, sender, link, score, score_reason, service_type,
            status, source, user_id)
         VALUES ($1, $2, $3, $4, $5, $6, $7, 'new', 'manual', $8)
         RETURNING id"
    )
    .bind(&channel)
    .bind(&req.message)
    .bind(req.sender.as_deref())
    .bind(req.link.as_deref())
    .bind(score)
    .bind(&reason)
    .bind(service.as_deref())
    .bind(user_id)
    .fetch_one(&state.db_pool)
    .await {
        Ok(id) => id,
        Err(e) => return Json(json!({"success": false, "error": format!("DB error: {}", e)})),
    };

    Json(json!({
        "success":      true,
        "id":           id.to_string(),
        "score":        score,
        "score_reason": reason,
        "service_type": service,
    }))
}

#[derive(Debug, Deserialize)]
struct UpdateOpportunityReq {
    status: Option<String>,
}

async fn telegram_update_opportunity(
    Extension(state):  Extension<Arc<AppState>>,
    Extension(claims): Extension<Claims>,
    Path(id):          Path<uuid::Uuid>,
    Json(req):         Json<UpdateOpportunityReq>,
) -> Json<serde_json::Value> {
    let user_id: i32 = claims.sub.parse().unwrap_or(0);
    let valid = ["new", "contacted", "won", "lost", "ignored"];
    let Some(status) = req.status else {
        return Json(json!({"success": false, "error": "status is required"}));
    };
    if !valid.contains(&status.as_str()) {
        return Json(json!({"success": false, "error": format!("status must be one of: {}", valid.join(", "))}));
    }
    match sqlx::query(
        "UPDATE telegram_opportunities
         SET status = $1, updated_at = NOW()
         WHERE id = $2 AND user_id = $3"
    )
    .bind(&status)
    .bind(id)
    .bind(user_id)
    .execute(&state.db_pool)
    .await {
        Ok(_)  => Json(json!({"success": true})),
        Err(e) => Json(json!({"success": false, "error": format!("DB error: {}", e)})),
    }
}

/// AI-score a pasted Telegram message as a potential paid gig.
/// Mirrors the IG lead scorer — same service menu, same 0-100 scale.
async fn score_telegram_opportunity(
    state:   &Arc<AppState>,
    channel: &str,
    message: &str,
) -> (i32, String, Option<String>) {
    let prompt = format!(
        r#"A message was posted in Telegram channel @{channel}. Decide whether it's a REAL paid gig opportunity for a video production studio and which service to pitch.

The studio offers (pick ONE):
- clipping       — long-form → Shorts/Reels. Best fit: podcasts, streams, long YouTubers. $297–$899/mo.
- animations     — Blender explainer / data-viz / LaTeX scenes. Best fit: educators, crypto/finance, news/data. $50–$150 each.
- thumbnails     — AI YouTube thumbnails. Best fit: growing channels, MrBeast aspirants. $25–$50 each.
- ugc            — vertical product-demo ads. Best fit: Shopify / DTC / SaaS founders. $200–$500 each.
- product_mockup — photorealistic 3D product shot. Best fit: ecommerce / hardware / app launches / Kickstarter. $100–$300 each.
- landing_page   — animated SaaS landing hero (can scrape their live URL). Best fit: SaaS / indie founders / pre-launch. $200–$600 each.
- full_stack     — bundle of all. Best fit: 100k+ creators. $1500–$3000/mo.

Message:
"""
{message}
"""

Score guidelines:
- 80–100: clear paying client posted a real brief with budget/timeline hint.
- 60–79: likely fit, details missing but the intent is obvious.
- 40–59: ambiguous — could be a question rather than a buying signal.
- 0–39: not a gig (news, spam, announcement, cold pitch from someone ELSE offering similar services = competitor).

Return ONLY valid JSON (no markdown):
{{"score": 75, "service": "clipping", "reason": "Podcaster says 'need someone to cut my 2hr episodes into TikToks, DM for budget'"}}

`service` MUST be one of: clipping, animations, thumbnails, ugc, full_stack — or null if score < 40."#,
        channel = channel,
        message = message,
    );

    let response = match crate::llm_utils::generate_text_best_effort(
        state.nvidia_nim_client.as_ref(),
        state.gemma_client.as_ref(),
        state.gemini_client.as_ref(),
        &prompt,
    ).await {
        Ok(text) => text,
        Err(_)   => return (0, "scoring failed".to_string(), None),
    };

    let cleaned = response.trim()
        .trim_start_matches("```json").trim_start_matches("```")
        .trim_end_matches("```").trim();

    let v: serde_json::Value = match serde_json::from_str(cleaned) {
        Ok(v) => v,
        Err(_) => return (0, "parse failed".to_string(), None),
    };
    let score  = v.get("score").and_then(|s| s.as_i64()).unwrap_or(0) as i32;
    let reason = v.get("reason").and_then(|r| r.as_str()).unwrap_or("").to_string();
    let svc    = v.get("service").and_then(|s| s.as_str()).map(|s| s.to_lowercase());
    let service = svc.and_then(|s| match s.as_str() {
        "clipping" | "animations" | "thumbnails" | "ugc" | "product_mockup" | "landing_page" | "full_stack" => Some(s),
        _ => None,
    });
    (score, reason, service)
}

// ============================================================================
// Market-aligned unlock pricing for delivery samples
// ============================================================================
//
// Market rates (per research + honest pricing for cold-DM impulse buys):
//   SaaS explainer videos:   $1,500-$7,000 (agency rates)
//   Product promo videos:    $500-$3,000
//   Landing page videos:     $1,000-$5,000
//
// Our pricing (AI-produced, first-touch cold DM, priced for conversion):
//   Comprehensive agent video w/ website:  $197-$497
//   Single-shot Blender animation:          $49-$97
//   Thumbnails / basic Blender:             $9-$29

pub fn unlock_price_for(service: Option<&str>, has_website: bool) -> f64 {
    if has_website {
        // Agent-generated comprehensive video (script + scenes + voiceover).
        // Full production value justifies mid-market pricing.
        match service {
            Some("full_stack")       => 497.00, // Most ambitious: long-form brand video
            Some("landing_page")     => 297.00, // Animated landing page presentation
            Some("product_mockup")   => 197.00, // Product showcase video
            Some("ugc")              => 197.00, // UGC-style promo video
            Some("animations")       => 147.00, // Branded animation package
            _                        => 197.00, // Default: scene-based branded video
        }
    } else {
        // Single-shot Blender render (faster, cheaper to produce).
        match service {
            Some("landing_page")     => 97.00,
            Some("product_mockup")   => 97.00,
            Some("animations")       => 49.00,
            Some("ugc") | Some("full_stack") => 49.00,
            Some("thumbnails")       => 19.00,
            _                        => 29.00,
        }
    }
}

// ============================================================================
// Full AI agent video generation for high-value leads
// ============================================================================

/// Max iterations on the agent video + QA loop before giving up and
/// shipping whatever we have. Each iteration is 3-8 minutes of compute,
/// so a hard cap of 5 keeps the worst-case bounded at ~40 min.
const AGENT_VIDEO_MAX_RETRIES: usize = 5;

async fn run_agent_video_for_lead(
    delivery_id: uuid::Uuid,
    website_url: &str,
    lead_name: &str,
    lead_bio: &str,
    state: Arc<AppState>,
) {
    tracing::info!("🎬 Agent video gen for lead delivery {} — URL: {}", delivery_id, website_url);

    let full_path = format!("outputs/lead_full_{}.mp4", delivery_id);
    let preview_path = format!("outputs/lead_preview_{}.mp4", delivery_id);

    let base_prompt = format!(
        r#"Create a comprehensive 3-4 minute promotional video about this business,
then cut a branded 30-60 second preview from it.

Business website: {website_url}
Business name: {lead_name}
Bio: {lead_bio}

Full pipeline (LLM-driven — you pick the exact order and tools):

PHASE 1 — Understand the business:
  - read_website_content({website_url}) to understand what they do
  - fetch_website_image({website_url}) to get hero/banner visuals

PHASE 2 — Write a comprehensive script (3-4 minutes of narration):
  - generate_video_script for a long-form explainer:
    intro → problem → solution (their product) → features → social proof → CTA

PHASE 3 — Produce the full video:
  - Multiple blender_generate_scene calls using reference_image_url for branded scenes
  - blender_generate_title_card for the intro/outro
  - blender_generate_lower_third for feature callouts
  - add_voiceover_to_video to narrate the full script with ElevenLabs TTS
  - Composite / concatenate the scenes into ONE final video at: {full_path}

PHASE 4 — Cut a branded preview for free sharing:
  - Use FFmpeg tools to trim the most engaging 30-60 second segment
  - Add a watermark overlay: "VideoSync.video — PREVIEW" diagonal text across the frame
  - Add an "Unlock full video at videosync.video/delivery/{delivery_id}" text banner
  - Save the preview at: {preview_path}

Deliverables (MUST produce both files):
  - Full clean HD video: {full_path}
  - Watermarked preview: {preview_path}

Make it professional — this is pitched to a paying client. No placeholder content.
Output file paths clearly in your final response so the delivery pipeline can find them."#,
        website_url  = website_url,
        lead_name    = lead_name,
        lead_bio     = lead_bio.chars().take(200).collect::<String>(),
        full_path    = full_path,
        preview_path = preview_path,
        delivery_id  = delivery_id,
    );
    let prompt = base_prompt.clone();

    let gemini_client = match state.video_gemini_client.as_ref().or(state.gemini_client.as_ref()) {
        Some(c) => Arc::new(c.clone()),
        None => {
            tracing::error!("No Gemini client available for agent video gen");
            let _ = sqlx::query("UPDATE deliveries SET status = 'failed', error_message = 'No LLM client available' WHERE id = $1")
                .bind(delivery_id).execute(&state.db_pool).await;
            return;
        }
    };

    let agent = crate::agent::stateful_agent::StatefulGeminiAgent::new(gemini_client);

    // Iterative retry loop: run the agent, review the output, retry with
    // accumulated feedback until the reviewer passes it or we hit the cap.
    // Each retry prepends the QA hint to the prompt so the agent can
    // adjust specifically for what the reviewer flagged.
    let mut current_prompt = prompt.clone();
    let mut best_full: Option<String>    = None;
    let mut best_preview: Option<String> = None;
    let mut best_score: i32              = -1;
    let mut best_feedback: String        = String::new();
    let mut retries_used: i32            = 0;

    for attempt in 0..AGENT_VIDEO_MAX_RETRIES {
        let session_id = format!("lead-sample-{}-try{}", delivery_id, attempt);
        tracing::info!("🎬 Agent attempt {}/{} for delivery {}", attempt + 1, AGENT_VIDEO_MAX_RETRIES, delivery_id);

        let agent_result = agent.chat(
            &current_prompt,
            &session_id,
            String::new(),
            state.clone(),
            state.job_manager.clone(),
            None,
        ).await;

        // Collect the actual output paths the agent produced
        let (full_produced, preview_produced) = locate_agent_outputs(
            &agent_result.as_deref().unwrap_or(""),
            &full_path,
            &preview_path,
        ).await;

        // Review the full video against the original brief
        let review = if let Some(ref full) = full_produced {
            crate::render_review::review_render(
                &state,
                full,
                &base_prompt,
                "agent_lead_video",
                Some(delivery_id),
            ).await
        } else {
            crate::render_review::ReviewResult {
                pass: false,
                score: 0,
                feedback: "Agent did not produce expected full video output".to_string(),
                retry_hint: Some("Produce the full video at the exact path specified in the prompt".to_string()),
            }
        };

        retries_used = attempt as i32;

        // Track the best attempt so we always ship SOMETHING even if no attempt passes
        if review.score > best_score {
            best_score    = review.score;
            best_full     = full_produced.clone();
            best_preview  = preview_produced.clone();
            best_feedback = review.feedback.clone();
        }

        if review.pass {
            tracing::info!("✅ Agent video PASSED review on attempt {} (score {})", attempt + 1, review.score);
            break;
        }

        // Not passing — prepare the next attempt with accumulated feedback
        if attempt + 1 < AGENT_VIDEO_MAX_RETRIES {
            let hint = review.retry_hint.clone().unwrap_or_else(|| review.feedback.clone());
            tracing::warn!("🔄 Retrying (score {}): {}", review.score, hint);
            current_prompt = format!(
                "PREVIOUS ATTEMPT FAILED QA REVIEW (score {}/10).\n\
                Feedback: {}\n\
                Retry hint: {}\n\n\
                Apply the feedback above, then run the full pipeline below:\n\n{}",
                review.score, review.feedback, hint, base_prompt,
            );
        } else {
            tracing::warn!("⚠️ Hit max retries ({}) for delivery {} — shipping best attempt (score {})",
                AGENT_VIDEO_MAX_RETRIES, delivery_id, best_score);
        }
    }

    // Upload whatever we ended up with. If even the best attempt produced
    // no file, fall back to the direct Blender render path.
    let Some(full_path_final) = best_full else {
        tracing::warn!("No usable output from {} attempts — falling back to direct Blender", retries_used + 1);
        crate::handlers::admin::run_delivery_job(delivery_id, state).await;
        return;
    };

    let r2 = match state.r2_client.as_ref() {
        Some(r) => r,
        None => {
            tracing::error!("R2 not configured — can't upload agent outputs");
            let _ = sqlx::query(
                "UPDATE deliveries SET status = 'failed', error_message = 'R2 not configured' WHERE id = $1"
            ).bind(delivery_id).execute(&state.db_pool).await;
            return;
        }
    };

    // Upload full clean HD video
    let full_url = match r2.upload_file(&full_path_final, &format!("deliveries/{}/full.mp4", delivery_id)).await {
        Ok(u) => u,
        Err(e) => {
            tracing::error!("Failed to upload full video: {e}");
            let _ = sqlx::query(
                "UPDATE deliveries SET status = 'failed', error_message = $1 WHERE id = $2"
            ).bind(format!("R2 upload failed: {e}")).bind(delivery_id).execute(&state.db_pool).await;
            return;
        }
    };

    // Upload preview (watermarked) — fall back to full video if preview wasn't produced
    let preview_url = if let Some(pp) = best_preview.as_ref() {
        match r2.upload_file(pp, &format!("deliveries/{}/preview.mp4", delivery_id)).await {
            Ok(u) => Some(u),
            Err(e) => {
                tracing::warn!("Preview upload failed: {e} — clients will see full video as preview");
                None
            }
        }
    } else {
        None
    };

    let qa_note: Option<String> = if best_score < 6 {
        Some(format!("QA final score {} after {} retries: {}", best_score, retries_used + 1, best_feedback))
    } else {
        None
    };

    let _ = sqlx::query(
        "UPDATE deliveries SET status='completed', output_r2_url=$1, preview_r2_url=$2,
         qa_retry_count=$3, final_qa_score=$4, error_message=$5, completed_at=NOW()
         WHERE id=$6"
    )
    .bind(&full_url)
    .bind(preview_url.as_deref())
    .bind(retries_used + 1)
    .bind(best_score)
    .bind(qa_note.as_deref())
    .bind(delivery_id)
    .execute(&state.db_pool)
    .await;

    tracing::info!("📦 Delivery {} complete — full={}, preview={:?}, score={}, retries={}",
        delivery_id, full_url, preview_url, best_score, retries_used + 1);
}

/// Find the full + preview output files the agent produced. Tries the
/// exact expected paths first, then falls back to parsing the agent's
/// response for any `outputs/*.mp4` mentions.
async fn locate_agent_outputs(
    response: &str,
    expected_full: &str,
    expected_preview: &str,
) -> (Option<String>, Option<String>) {
    let full = if tokio::fs::metadata(expected_full).await.is_ok() {
        Some(expected_full.to_string())
    } else {
        find_mp4_in_response(response, |p| !p.contains("preview"))
    };
    let preview = if tokio::fs::metadata(expected_preview).await.is_ok() {
        Some(expected_preview.to_string())
    } else {
        find_mp4_in_response(response, |p| p.contains("preview"))
    };
    (full, preview)
}

fn find_mp4_in_response(response: &str, filter: impl Fn(&str) -> bool) -> Option<String> {
    for word in response.split_whitespace() {
        let clean = word.trim_matches(|c: char| !c.is_alphanumeric() && c != '/' && c != '.' && c != '_' && c != '-');
        if clean.ends_with(".mp4") && clean.contains('/') && filter(clean) {
            return Some(clean.to_string());
        }
    }
    None
}

fn extract_output_path_from_response(response: &str) -> Option<String> {
    // Look for file paths in the agent's response
    for word in response.split_whitespace() {
        let clean = word.trim_matches(|c: char| !c.is_alphanumeric() && c != '/' && c != '.' && c != '_' && c != '-');
        if clean.ends_with(".mp4") && clean.contains('/') {
            return Some(clean.to_string());
        }
    }
    None
}

// ============================================================================
// Helpers for product_mockup + landing_page sample generation
// ============================================================================

/// Calls Gemini image-generation, saves the bytes to R2, returns a
/// pre-signed URL Blender can fetch. Returns None on any failure so the
/// caller falls back to rendering without a reference image.
async fn try_generate_image(state: &Arc<AppState>, prompt: &str) -> Option<String> {
    let gemini = state.gemini_client.as_ref()?;

    let bytes = match gemini.generate_image(prompt, Some("16:9"), None, None).await {
        Ok(b) => b,
        Err(e) => {
            tracing::warn!("try_generate_image: Gemini call failed: {}", e);
            return None;
        }
    };

    // Persist locally long enough to hand to R2's file-based upload API.
    // outputs/ is gitignored + auto-cleaned.
    let local_path = format!("outputs/sample_gen_{}.png", chrono::Utc::now().timestamp_millis());
    if let Err(e) = tokio::fs::write(&local_path, &bytes).await {
        tracing::warn!("try_generate_image: local write failed: {}", e);
        return None;
    }

    let r2 = state.r2_client.as_ref()?;
    let key = format!("sample_gen/{}.png", uuid::Uuid::new_v4());
    if let Err(e) = r2.upload(&local_path, &key).await {
        tracing::warn!("try_generate_image: R2 upload failed: {}", e);
        return None;
    }

    // Pre-signed URL valid for 7 days — long enough for Blender to render
    // AND the delivery page to display the source image as a fallback.
    match r2.presign_get(&key, 7 * 24 * 3600).await {
        Ok(url) => Some(url),
        Err(e) => {
            tracing::warn!("try_generate_image: presign_get failed: {}", e);
            None
        }
    }
}

/// Fetch a landing page URL and extract the best hero image. Looks in order:
///   1. `<meta property="og:image">` — usually the designer-chosen hero
///   2. `<meta name="twitter:image">`
///   3. First `<img>` tag with width >= 400 (best-effort regex)
///
/// Returns None if the URL is unfetchable or has no suitable image. When
/// None is returned, the caller falls back to Gemini-synthesised hero.
pub async fn fetch_landing_page_hero(url: &str) -> Option<String> {
    // Only HTTPS URLs — blocks SSRF into internal services.
    if !url.starts_with("https://") && !url.starts_with("http://") {
        return None;
    }

    let client = reqwest::Client::builder()
        .timeout(std::time::Duration::from_secs(15))
        .user_agent("Mozilla/5.0 (compatible; VideoSyncBot/1.0)")
        .redirect(reqwest::redirect::Policy::limited(5))
        .build()
        .ok()?;

    let resp = client.get(url).send().await.ok()?;
    if !resp.status().is_success() { return None; }
    let html = resp.text().await.ok()?;

    // Priority 1: og:image meta tag.
    if let Some(img) = extract_meta_content(&html, "og:image") {
        return Some(normalise_image_url(&img, url));
    }
    if let Some(img) = extract_meta_content(&html, "twitter:image") {
        return Some(normalise_image_url(&img, url));
    }
    None
}

/// Pull the `content` attribute of the first matching meta tag.
/// We tolerate both `property="og:image"` and `name="og:image"` variants.
pub fn extract_meta_content_pub(html: &str, key: &str) -> Option<String> {
    extract_meta_content(html, key)
}

fn extract_meta_content(html: &str, key: &str) -> Option<String> {
    let lower = html.to_lowercase();
    let needles = [
        format!("property=\"{}\"", key),
        format!("property='{}'", key),
        format!("name=\"{}\"", key),
        format!("name='{}'", key),
    ];
    let pos = needles.iter().find_map(|n| lower.find(n))?;
    // Look for content="..." within the same tag. Meta tags are typically
    // short so a local window works without a real HTML parser dep.
    let window_end = (pos + 500).min(html.len());
    let window = &html[pos..window_end];
    // Find content=" or content='
    let start = window.to_lowercase().find("content=")?;
    let after = &window[start + "content=".len()..];
    let quote = after.chars().next()?;
    if quote != '"' && quote != '\'' { return None; }
    let rest = &after[1..];
    let end = rest.find(quote)?;
    Some(rest[..end].to_string())
}

/// Convert a possibly-relative image URL into an absolute URL.
fn normalise_image_url(img: &str, page_url: &str) -> String {
    if img.starts_with("http://") || img.starts_with("https://") {
        return img.to_string();
    }
    if img.starts_with("//") {
        return format!("https:{}", img);
    }
    // Relative path — resolve against the page's origin.
    if let Ok(base) = url::Url::parse(page_url) {
        if let Ok(absolute) = base.join(img) {
            return absolute.to_string();
        }
    }
    img.to_string()
}

// ============================================================================
// Manual service-type override on an Instagram lead
// ============================================================================
//
// The scorer auto-picks a service_type, but the whitelisted user might
// have more context (they know the lead's actual product from reading
// their bio) and want to pitch a different service. This endpoint lets
// them overwrite. Subsequent `generate-dm` + `generate-sample` calls
// will use the overridden value.

#[derive(Debug, Deserialize)]
struct UpdateServiceTypeRequest {
    service_type: Option<String>,
}

async fn instagram_update_service_type(
    Extension(state):  Extension<Arc<AppState>>,
    Extension(claims): Extension<Claims>,
    Path(id):          Path<uuid::Uuid>,
    Json(req):         Json<UpdateServiceTypeRequest>,
) -> Json<serde_json::Value> {
    let user_id: i32 = claims.sub.parse().unwrap_or(0);

    // Empty string / None → reset to NULL so the next DM re-reads the
    // AI's default. Anything else must be one of the known service tags.
    let service: Option<String> = match req.service_type.as_deref() {
        None | Some("") => None,
        Some(s) => match s {
            "clipping" | "animations" | "thumbnails" | "ugc"
            | "product_mockup" | "landing_page" | "full_stack" => Some(s.to_string()),
            _ => return Json(json!({
                "success": false,
                "error":   format!("service_type must be one of: clipping, animations, thumbnails, ugc, product_mockup, landing_page, full_stack (got: {})", s),
            })),
        },
    };

    match sqlx::query(
        "UPDATE instagram_leads SET service_type = $1, updated_at = NOW()
         WHERE id = $2 AND user_id = $3"
    )
    .bind(service.as_deref())
    .bind(id)
    .bind(user_id)
    .execute(&state.db_pool)
    .await {
        Ok(_)  => Json(json!({"success": true, "service_type": service})),
        Err(e) => Json(json!({"success": false, "error": format!("DB error: {}", e)})),
    }
}

// ============================================================================
// Public wrapper + login endpoints for the Telegram MTProto watcher
// ============================================================================

/// Crate-public wrapper around `score_telegram_opportunity` so the
/// MTProto watcher in src/telegram_client.rs can score inbound messages
/// with the same AI pass as the manual-entry form.
pub async fn score_telegram_opportunity_public(
    state:   &Arc<AppState>,
    channel: &str,
    message: &str,
) -> (i32, String, Option<String>) {
    score_telegram_opportunity(state, channel, message).await
}

#[derive(Debug, Deserialize)]
struct TelegramLoginStartReq {
    phone: String,
}

async fn telegram_login_start(
    Extension(state): Extension<Arc<AppState>>,
    Json(req):        Json<TelegramLoginStartReq>,
) -> Json<serde_json::Value> {
    let phone = req.phone.trim().to_string();
    if phone.is_empty() {
        return Json(json!({"success": false, "error": "phone is required (include country code, e.g. +14155551234)"}));
    }
    match crate::telegram_client::login_start(&state, &phone).await {
        Ok(()) => Json(json!({
            "success": true,
            "message": "Code sent to your Telegram. POST it to /login/verify with {phone, code} within 10 minutes."
        })),
        Err(e) => Json(json!({"success": false, "error": e})),
    }
}

#[derive(Debug, Deserialize)]
struct TelegramLoginVerifyReq {
    phone: String,
    code:  String,
}

async fn telegram_login_verify(
    Extension(state): Extension<Arc<AppState>>,
    Json(req):        Json<TelegramLoginVerifyReq>,
) -> Json<serde_json::Value> {
    match crate::telegram_client::login_verify(&state, req.phone.trim(), req.code.trim()).await {
        Ok(()) => Json(json!({
            "success": true,
            "message": "Logged in. The Telegram watcher is now active."
        })),
        Err(e) => Json(json!({"success": false, "error": e})),
    }
}

async fn telegram_watcher_status(
    Extension(state): Extension<Arc<AppState>>,
) -> Json<serde_json::Value> {
    Json(crate::telegram_client::status(&state).await)
}
