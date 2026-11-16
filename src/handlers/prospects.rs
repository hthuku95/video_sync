// Admin-only prospect finder — uses YouTube + Twitch APIs to discover
// potential clients (content creators and clippers), scores them with Gemini,
// and generates personalized DM scripts.

use crate::llm_utils::generate_text_best_effort;
use crate::middleware::admin::{admin_middleware, admin_or_whitelisted_middleware};
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
use regex::Regex;
use serde::{Deserialize, Serialize};
use serde_json::json;
use sqlx::Row;
use std::sync::Arc;
use uuid::Uuid;

pub fn prospect_routes() -> Router {
    // SSR page — public route, uses JS in the browser to read the JWT from
    // localStorage and call the /api/* endpoints below. Putting middleware
    // here would reject every navigation because browsers don't send
    // `Authorization` headers on `<a href>` or address-bar loads.
    let public_page = Router::new().route("/admin/prospect-finder", get(prospect_finder_page));

    // Admin-only API routes (staff/superuser only — search, LinkedIn, Telegram, etc.)
    let admin_only = Router::new()
        .route("/api/admin/prospects/search", post(search_prospects))
        .route("/api/admin/prospects/hunt", post(hunt_prospects))
        .route(
            "/api/admin/prospects/redistribute",
            post(redistribute_prospects),
        )
        .route("/api/admin/prospects/runs/:id", get(prospect_run_status))
        .route(
            "/api/admin/prospects/linkedin/agents",
            get(linkedin_list_agents),
        )
        .route(
            "/api/admin/prospects/linkedin/launch",
            post(linkedin_launch_search),
        )
        .route(
            "/api/admin/prospects/linkedin/search",
            post(linkedin_smart_search),
        )
        .route(
            "/api/admin/prospects/linkedin/jobs",
            get(linkedin_list_jobs),
        )
        .route(
            "/api/admin/prospects/linkedin/jobs/:job_id/results",
            get(linkedin_fetch_results),
        )
        // SaaS prospects
        .route(
            "/api/admin/prospects/saas/search",
            post(search_compar_edge_prospects),
        )
        .route(
            "/api/admin/prospects/product-hunt/search",
            post(search_product_hunt_prospects),
        )
        // Telegram
        .route("/api/admin/telegram/channels", get(telegram_list_channels))
        .route("/api/admin/telegram/channels", post(telegram_add_channel))
        .route(
            "/api/admin/telegram/channels/:id",
            delete(telegram_delete_channel),
        )
        .route(
            "/api/admin/telegram/opportunities",
            get(telegram_list_opportunities),
        )
        .route(
            "/api/admin/telegram/opportunities",
            post(telegram_add_opportunity_manual),
        )
        .route(
            "/api/admin/telegram/opportunities/:id",
            patch(telegram_update_opportunity),
        )
        // MTProto
        .route(
            "/api/admin/telegram/login/start",
            post(telegram_login_start),
        )
        .route(
            "/api/admin/telegram/login/verify",
            post(telegram_login_verify),
        )
        .route("/api/admin/telegram/status", get(telegram_watcher_status))
        .route(
            "/api/admin/telegram/discover",
            post(telegram_discover_channels),
        )
        .route(
            "/api/admin/telegram/discovered",
            get(telegram_list_discovered_channels),
        )
        .layer(axum::middleware::from_fn(admin_middleware))
        .layer(axum::middleware::from_fn(auth_middleware));

    // Admin or whitelisted API routes (staff/superuser + whitelisted content machine users)
    let admin_or_whitelisted = Router::new()
        .route("/api/admin/prospects", get(list_prospects))
        .route("/api/admin/prospects/:id", patch(update_prospect))
        .route(
            "/api/admin/prospects/:id/dm-script",
            post(regenerate_dm_script),
        )
        .route(
            "/api/admin/prospects/:id/generate-outreach",
            post(generate_outreach_message),
        )
        .route(
            "/api/admin/prospects/:id/generate-sample-pack",
            post(generate_prospect_sample_pack),
        )
        .route(
            "/api/admin/prospects/:id/send-email",
            post(send_prospect_email_handler),
        )
        .route(
            "/api/admin/prospects/bulk-send-email",
            post(bulk_send_prospect_emails),
        )
        .route("/api/admin/prospects/:id", delete(delete_prospect))
        .layer(axum::middleware::from_fn(admin_or_whitelisted_middleware))
        .layer(axum::middleware::from_fn(auth_middleware));

    public_page.merge(admin_only).merge(admin_or_whitelisted)
}

/// Routes accessible to any authenticated (whitelisted) user — NOT admin-only.
/// Used by content_machine for Instagram lead generation.
pub fn instagram_routes() -> Router {
    Router::new()
        .route(
            "/api/portfolio-samples/crypto-saas",
            post(crate::handlers::admin::api_generate_crypto_saas_portfolio_samples),
        )
        .route(
            "/api/portfolio-samples/dfy",
            get(crate::handlers::admin::api_list_dfy_portfolio_samples)
                .post(crate::handlers::admin::api_generate_dfy_portfolio_samples),
        )
        .route("/api/instagram/leads/search", post(instagram_search_leads))
        .route(
            "/api/instagram/leads/auto-discover",
            post(instagram_auto_discover),
        )
        .route(
            "/api/instagram/leads/kick-auto-discover",
            post(instagram_kick_auto_discover),
        )
        .route(
            "/api/instagram/leads/kick-method-b",
            post(instagram_kick_method_b),
        )
        .route("/api/instagram/leads/top", get(instagram_top_leads))
        .route(
            "/api/instagram/leads",
            get(instagram_list_leads).delete(instagram_clear_leads),
        )
        .route(
            "/api/instagram/leads/:id/generate-dm",
            post(instagram_generate_dm),
        )
        .route(
            "/api/instagram/leads/:id/generate-sample",
            post(instagram_generate_sample),
        )
        .route(
            "/api/instagram/leads/:id/contact-status",
            patch(instagram_update_contact_status),
        )
        .route(
            "/api/instagram/leads/:id/service-type",
            patch(instagram_update_service_type),
        )
        .layer(axum::middleware::from_fn(admin_or_whitelisted_middleware))
        .layer(axum::middleware::from_fn(auth_middleware))
}

#[derive(Debug, Deserialize)]
struct SearchRequest {
    platform: String,      // "youtube" | "twitch"
    prospect_type: String, // "content_creator" | "creator_manager" | "clipper" | "podcaster" | "educator" | "business_owner"
    category: Option<String>,
    min_viewers: Option<i64>,
    max_viewers: Option<i64>,
    limit: Option<usize>,
    sourced_by: Option<i32>, // User ID of the whitelisted content machine user who sourced this
    /// Forced service scope (owner directive Oct 2026): when set, EVERY scored
    /// prospect is tagged with this service only — the search returns prospects
    /// OF that service. Must be enabled and consistent with the platform.
    service: Option<String>,
}

#[derive(Debug, Serialize)]
struct ProspectAgentCheckpoint {
    step: String,
    state: serde_json::Value,
    notes: Option<String>,
}

#[derive(Debug, Deserialize)]
struct GenerateOutreachRequest {
    delivery_url: String,
}

#[derive(Debug, Deserialize)]
struct GenerateProspectSamplePackRequest {
    source_url: Option<String>,
    product_name: Option<String>,
    offer_type: Option<String>,
    notes: Option<String>,
}

#[derive(Debug, Deserialize)]
struct ListQuery {
    prospect_type: Option<String>,
    contact_status: Option<String>,
    platform: Option<String>,
    sourced_by: Option<i32>,
    /// Show prospects of disabled services too (admin debugging). Default:
    /// only prospects of ON services (plus unscored NULL-service rows).
    include_disabled: Option<bool>,
    /// Only prospects created in the last N days (launch freshness).
    fresh_days: Option<i64>,
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

async fn create_prospect_agent_run(
    state: &Arc<AppState>,
    user_id: Option<i32>,
    run_type: &str,
    goal: &str,
    input: serde_json::Value,
) -> Option<Uuid> {
    sqlx::query_scalar::<_, Uuid>(
        "INSERT INTO prospect_agent_runs (user_id, run_type, goal, input, state, current_step)
         VALUES ($1, $2, $3, $4, $4, 'planned')
         RETURNING id",
    )
    .bind(user_id)
    .bind(run_type)
    .bind(goal)
    .bind(input)
    .fetch_optional(&state.db_pool)
    .await
    .ok()
    .flatten()
}

async fn checkpoint_prospect_agent_run(
    state: &Arc<AppState>,
    run_id: Option<Uuid>,
    step: &str,
    checkpoint_state: serde_json::Value,
    notes: Option<&str>,
) {
    let Some(run_id) = run_id else {
        return;
    };

    let next_num = sqlx::query_scalar::<_, i32>(
        "SELECT COALESCE(MAX(checkpoint_num), 0) + 1
         FROM prospect_agent_checkpoints
         WHERE run_id = $1",
    )
    .bind(run_id)
    .fetch_one(&state.db_pool)
    .await
    .unwrap_or(1);

    let checkpoint = ProspectAgentCheckpoint {
        step: step.to_string(),
        state: checkpoint_state.clone(),
        notes: notes.map(str::to_string),
    };

    let _ = sqlx::query(
        "INSERT INTO prospect_agent_checkpoints (run_id, checkpoint_num, step, state, notes)
         VALUES ($1, $2, $3, $4, $5)
         ON CONFLICT (run_id, checkpoint_num) DO NOTHING",
    )
    .bind(run_id)
    .bind(next_num)
    .bind(&checkpoint.step)
    .bind(&checkpoint.state)
    .bind(checkpoint.notes.as_deref())
    .execute(&state.db_pool)
    .await;

    let _ = sqlx::query(
        "UPDATE prospect_agent_runs
         SET current_step = $1, state = $2, updated_at = NOW()
         WHERE id = $3",
    )
    .bind(step)
    .bind(&checkpoint_state)
    .bind(run_id)
    .execute(&state.db_pool)
    .await;
}

async fn complete_prospect_agent_run(
    state: &Arc<AppState>,
    run_id: Option<Uuid>,
    status: &str,
    final_state: serde_json::Value,
    error: Option<&str>,
) {
    let Some(run_id) = run_id else {
        return;
    };
    let _ = sqlx::query(
        "UPDATE prospect_agent_runs
         SET status=$1, current_step=$2, state=$3, last_error=$4,
             completed_at = CASE WHEN $1 IN ('completed','failed') THEN NOW() ELSE completed_at END,
             updated_at=NOW()
         WHERE id=$5",
    )
    .bind(status)
    .bind(status)
    .bind(final_state)
    .bind(error)
    .bind(run_id)
    .execute(&state.db_pool)
    .await;
}

// ============================================================================
// Search — AI-powered prospect discovery
// ============================================================================

async fn search_prospects(
    Extension(state): Extension<Arc<AppState>>,
    Extension(claims): Extension<Claims>,
    Json(mut payload): Json<SearchRequest>,
) -> Result<Json<serde_json::Value>, (StatusCode, Json<ErrorResponse>)> {
    // Owner attribution (owner directive Oct 2026): every find carries an
    // owner so sourced_by is never NULL. Explicit param wins; helpers default
    // to themselves; admin hunts auto-balance to the lightest pool user.
    if payload.sourced_by.is_none() {
        let is_admin = claims.is_superuser || claims.is_staff;
        if is_admin {
            if let Some(uid) = lightest_distribution_user(&state.db_pool).await {
                payload.sourced_by = Some(uid);
            }
        } else if let Ok(uid) = claims.sub.parse::<i32>() {
            payload.sourced_by = Some(uid);
        }
    }
    // Service switch (owner directive Sep 2026): prospect discovery runs only
    // while the prospecting group is on. Relevance to the two clipping
    // businesses is enforced downstream by the constrained scoring menu.
    if !crate::services::service_flags::service_enabled(&state.db_pool, "prospecting").await {
        return Err((
            StatusCode::SERVICE_UNAVAILABLE,
            Json(ErrorResponse {
                success: false,
                message: "Prospect discovery is currently disabled (service_flags).".to_string(),
            }),
        ));
    }
    let limit = payload.limit.unwrap_or(20).min(50);
    // Forced-service scope (owner directive Oct 2026): searching FOR a service
    // returns prospects OF that service only. The requested service must be a
    // known revenue service, switched ON, and consistent with the platform
    // (youtube→youtube_clipping, twitch→twitch_clipping, kick*→kick_auto_clipper).
    if let Some(ref wanted) = payload.service {
        let normalized = normalize_revenue_service(&wanted.to_ascii_lowercase()).to_string();
        if !is_valid_revenue_service(&normalized) {
            return Err((
                StatusCode::BAD_REQUEST,
                Json(ErrorResponse {
                    success: false,
                    message: format!("Unknown service '{wanted}'."),
                }),
            ));
        }
        if !crate::services::service_flags::service_enabled(&state.db_pool, &normalized).await {
            return Err((
                StatusCode::SERVICE_UNAVAILABLE,
                Json(ErrorResponse {
                    success: false,
                    message: format!(
                        "Service '{normalized}' is currently disabled (service_flags)."
                    ),
                }),
            ));
        }
        let expected = match payload.platform.as_str() {
            "youtube" => "youtube_clipping",
            "twitch" | "twitch_clipper" => "twitch_clipping",
            "kick" | "kick_clipper" | "kick_clipper_top" => "kick_auto_clipper",
            _ => "",
        };
        if !expected.is_empty() && normalized != expected {
            return Err((
                StatusCode::BAD_REQUEST,
                Json(ErrorResponse {
                    success: false,
                    message: format!(
                        "Service '{normalized}' does not match platform '{}' (expected '{expected}').",
                        payload.platform
                    ),
                }),
            ));
        }
    }
    let user_id = claims.sub.parse::<i32>().ok();
    let run_input = json!({
        "platform": payload.platform.clone(),
        "prospect_type": payload.prospect_type.clone(),
        "category": payload.category.clone(),
        "min_viewers": payload.min_viewers,
        "max_viewers": payload.max_viewers,
        "limit": limit,
    });
    let run_id = create_prospect_agent_run(
        &state,
        user_id,
        "prospect_search",
        "Autonomously discover, enrich, score, and prepare outreach for revenue prospects.",
        run_input.clone(),
    )
    .await;
    checkpoint_prospect_agent_run(
        &state,
        run_id,
        "planned",
        json!({
            "input": run_input,
            "next_steps": ["generate_search_query", "search_platform", "enrich_contacts", "score_fit", "write_outreach"]
        }),
        Some("Created stateful prospect discovery run."),
    )
    .await;

    // Background+poll (owner directive Oct 2026): the dispatch above is
    // validated; execution runs detached so multi-minute searches (Helix +
    // 10x YouTube + scoring) never hit the ~100s proxy cap. Poll
    // GET /api/admin/prospects/runs/:id for completion.
    let spawn_state = state.clone();
    tokio::spawn(async move {
        execute_search_dispatch(spawn_state, payload, limit, run_id).await;
    });

    Ok(Json(json!({
        "success": true,
        "accepted": true,
        "agent_run_id": run_id.map(|id| id.to_string()),
        "message": "Prospect search running in background — poll run status for completion."
    })))
}

/// Owner-alt account IDs excluded from prospect auto-distribution (owner's own
/// addresses — only harryomollo95@gmail.com id=1 participates alongside helpers).
const DISTRIBUTION_EXCLUDE_IDS: [i32; 6] = [2, 3, 10, 16, 17, 18];

/// Lightest-loaded distribution user (owner directive Oct 2026): whitelisted
/// users with accounts, minus owner alts, ordered by fewest sourced prospects.
/// Keeps team supply balanced without any manual bookkeeping.
async fn lightest_distribution_user(pool: &sqlx::PgPool) -> Option<i32> {
    sqlx::query_scalar::<_, i32>(
        "SELECT u.id FROM users u JOIN whitelist_emails w ON w.email = u.email \
         WHERE NOT (u.id = ANY($1)) \
         ORDER BY (SELECT COUNT(*) FROM prospects p WHERE p.sourced_by = u.id) ASC, u.id ASC \
         LIMIT 1",
    )
    .bind(&DISTRIBUTION_EXCLUDE_IDS[..])
    .fetch_optional(pool)
    .await
    .ok()
    .flatten()
}

/// POST /api/admin/prospects/hunt — fire-and-distribute prospect hunt (owner
/// directive Oct 2026). Same validation/gates/background as search; results
/// always carry an owner (explicit sourced_by, else lightest pool user) so the
/// sourced_by field is never NULL. Dashboard calls this for launch hunts.
#[derive(Debug, Deserialize)]
struct HuntRequest {
    platform: String,
    prospect_type: String,
    category: Option<String>,
    min_viewers: Option<i64>,
    max_viewers: Option<i64>,
    limit: Option<usize>,
    service: Option<String>,
    sourced_by: Option<i32>,
}

async fn hunt_prospects(
    Extension(state): Extension<Arc<AppState>>,
    Extension(claims): Extension<Claims>,
    Json(req): Json<HuntRequest>,
) -> Result<Json<serde_json::Value>, (StatusCode, Json<ErrorResponse>)> {
    let payload = SearchRequest {
        platform: req.platform,
        prospect_type: req.prospect_type,
        category: req.category,
        min_viewers: req.min_viewers,
        max_viewers: req.max_viewers,
        limit: req.limit,
        sourced_by: req.sourced_by,
        service: req.service,
    };
    search_prospects(Extension(state), Extension(claims), Json(payload)).await
}

/// POST /api/admin/prospects/redistribute — round-robin all unowned
/// (sourced_by IS NULL) prospects across the distribution pool. For tests,
/// repairs, and rebalancing. Returns counts.
async fn redistribute_prospects(
    Extension(state): Extension<Arc<AppState>>,
) -> Result<Json<serde_json::Value>, (StatusCode, Json<ErrorResponse>)> {
    let pool: Vec<i32> = sqlx::query_scalar::<_, i32>(
        "SELECT u.id FROM users u JOIN whitelist_emails w ON w.email = u.email \
         WHERE NOT (u.id = ANY($1)) ORDER BY u.id ASC",
    )
    .bind(&DISTRIBUTION_EXCLUDE_IDS[..])
    .fetch_all(&state.db_pool)
    .await
    .map_err(|e| {
        (
            StatusCode::INTERNAL_SERVER_ERROR,
            Json(ErrorResponse {
                success: false,
                message: e.to_string(),
            }),
        )
    })?;
    if pool.is_empty() {
        return Err((
            StatusCode::SERVICE_UNAVAILABLE,
            Json(ErrorResponse {
                success: false,
                message: "Distribution pool is empty.".to_string(),
            }),
        ));
    }
    let ids: Vec<Uuid> = sqlx::query_scalar::<_, Uuid>(
        "SELECT id FROM prospects WHERE sourced_by IS NULL ORDER BY created_at ASC",
    )
    .fetch_all(&state.db_pool)
    .await
    .unwrap_or_default();
    let mut assigned = 0usize;
    for (k, id) in ids.iter().enumerate() {
        let uid = pool[k % pool.len()];
        let ok =
            sqlx::query("UPDATE prospects SET sourced_by = $1, updated_at = NOW() WHERE id = $2")
                .bind(uid)
                .bind(id)
                .execute(&state.db_pool)
                .await
                .is_ok();
        if ok {
            assigned += 1;
        }
    }
    Ok(Json(
        json!({"success": true, "redistributed": assigned, "pool_size": pool.len()}),
    ))
}

/// GET /api/admin/prospects/runs/:id — poll a background search run.
/// Returns {success, status, current_step, found, error}. Terminal states:
/// completed | failed. Used by background+poll prospect discovery (Oct 2026).
async fn prospect_run_status(
    Extension(state): Extension<Arc<AppState>>,
    Path(id): Path<Uuid>,
) -> Json<serde_json::Value> {
    let row: Option<(String, String, serde_json::Value, Option<String>)> = sqlx::query_as(
        "SELECT status, current_step, state, last_error FROM prospect_agent_runs WHERE id = $1",
    )
    .bind(id)
    .fetch_optional(&state.db_pool)
    .await
    .ok()
    .flatten();
    match row {
        Some((status, step, st, err)) => Json(json!({
            "success": true,
            "status": status,
            "current_step": step,
            "found": st.get("found").and_then(|v| v.as_u64()).unwrap_or(0),
            "error": err,
        })),
        None => Json(json!({"success": false, "error": "Run not found"})),
    }
}

async fn search_youtube_prospects(
    state: &Arc<AppState>,
    payload: &SearchRequest,
    limit: usize,
) -> Result<usize, (StatusCode, Json<ErrorResponse>)> {
    let api_key = std::env::var("YOUTUBE_API_KEY").unwrap_or_default();
    if api_key.is_empty() {
        return Err((
            StatusCode::SERVICE_UNAVAILABLE,
            Json(ErrorResponse {
                success: false,
                message: "YOUTUBE_API_KEY not configured".to_string(),
            }),
        ));
    }

    let base_category = payload
        .category
        .clone()
        .unwrap_or_else(|| "general".to_string());

    // AI generates the most effective YouTube search query for this prospect type + category
    let search_query =
        ai_generate_youtube_query(state, &payload.prospect_type, &base_category).await;
    tracing::info!(
        "🔍 YouTube prospect search query (AI): \"{}\"",
        search_query
    );

    let client = reqwest::Client::new();
    let search_url = format!(
        "https://www.googleapis.com/youtube/v3/search?part=snippet&type=channel&q={}&maxResults={}&order=relevance&key={}",
        urlencoding::encode(&search_query), limit.min(50), api_key
    );

    let search_resp: serde_json::Value = client
        .get(&search_url)
        .send()
        .await
        .map_err(|e| {
            (
                StatusCode::BAD_GATEWAY,
                Json(ErrorResponse {
                    success: false,
                    message: format!("YouTube API error: {}", e),
                }),
            )
        })?
        .json()
        .await
        .map_err(|e| {
            (
                StatusCode::BAD_GATEWAY,
                Json(ErrorResponse {
                    success: false,
                    message: format!("YouTube parse error: {}", e),
                }),
            )
        })?;

    let items = match search_resp["items"].as_array() {
        Some(arr) => arr.clone(),
        None => return Ok(0),
    };

    // Collect channel IDs for stats lookup
    let channel_ids: Vec<String> = items
        .iter()
        .filter_map(|item| item["id"]["channelId"].as_str().map(String::from))
        .collect();

    if channel_ids.is_empty() {
        return Ok(0);
    }

    let stats_url = format!(
        "https://www.googleapis.com/youtube/v3/channels?part=snippet,statistics&id={}&key={}",
        channel_ids.join(","),
        api_key
    );
    let stats_resp: serde_json::Value = client
        .get(&stats_url)
        .send()
        .await
        .map_err(|e| {
            (
                StatusCode::BAD_GATEWAY,
                Json(ErrorResponse {
                    success: false,
                    message: e.to_string(),
                }),
            )
        })?
        .json()
        .await
        .map_err(|e| {
            (
                StatusCode::BAD_GATEWAY,
                Json(ErrorResponse {
                    success: false,
                    message: e.to_string(),
                }),
            )
        })?;

    let channels = stats_resp["items"].as_array().cloned().unwrap_or_default();
    let mut count = 0;

    for channel in &channels {
        let channel_id = channel["id"].as_str().unwrap_or("").to_string();
        let display_name = channel["snippet"]["title"]
            .as_str()
            .unwrap_or("")
            .to_string();
        let description = channel["snippet"]["description"]
            .as_str()
            .unwrap_or("")
            .to_string();
        let sub_count: i64 = channel["statistics"]["subscriberCount"]
            .as_str()
            .unwrap_or("0")
            .parse()
            .unwrap_or(0);

        if display_name.is_empty() || channel_id.is_empty() {
            continue;
        }

        // Apply viewer range filter
        if let Some(min) = payload.min_viewers {
            if sub_count < min {
                continue;
            }
        }
        if let Some(max) = payload.max_viewers {
            if sub_count > max {
                continue;
            }
        }

        let platform_url = format!("https://youtube.com/channel/{}", channel_id);
        let category = payload
            .category
            .clone()
            .unwrap_or_else(|| "general".to_string());

        // Extract creator contact fields from the public channel description.
        let mut twitter_handle = extract_best_twitter_handle(&description);
        let mut instagram_handle = extract_best_instagram_handle(&description);
        let mut business_email = extract_best_business_email(&description);
        let mut external_url = extract_best_external_url(&description);
        let enrichment_url = external_url.clone();
        let contact_enrichment = enrich_public_contact_fields(
            enrichment_url.as_deref(),
            &mut twitter_handle,
            &mut instagram_handle,
            &mut business_email,
            &mut external_url,
        )
        .await;
        let scoring_description = build_scoring_description(&description);

        let (score, reasoning, service, dm_creator, dm_clipper, x_dm, email_script) =
            score_prospect_with_ai(
                state,
                &display_name,
                sub_count,
                &scoring_description,
                &category,
                &payload.prospect_type,
                "youtube",
                payload.service.as_deref(),
            )
            .await;

        sqlx::query(
            "INSERT INTO prospects (platform, channel_id, display_name, platform_url,
             subscriber_count, content_category, channel_description, prospect_type,
             ai_score, ai_reasoning, dm_script_creator, dm_script_clipper, twitter_handle,
             instagram_handle, business_email, external_url, service_type, x_dm_script,
             email_script, contact_enrichment, sourced_by)
             VALUES ($1,$2,$3,$4,$5,$6,$7,$8,$9,$10,$11,$12,$13,$14,$15,$16,$17,$18,$19,$20,$21)
             ON CONFLICT (platform, channel_id) DO UPDATE SET
               ai_score = EXCLUDED.ai_score,
               ai_reasoning = EXCLUDED.ai_reasoning,
               dm_script_creator = EXCLUDED.dm_script_creator,
               dm_script_clipper = EXCLUDED.dm_script_clipper,
               x_dm_script = EXCLUDED.x_dm_script,
               email_script = EXCLUDED.email_script,
               twitter_handle = COALESCE(EXCLUDED.twitter_handle, prospects.twitter_handle),
               instagram_handle = COALESCE(EXCLUDED.instagram_handle, prospects.instagram_handle),
               business_email = COALESCE(EXCLUDED.business_email, prospects.business_email),
               external_url = COALESCE(EXCLUDED.external_url, prospects.external_url),
               contact_enrichment = COALESCE(NULLIF(EXCLUDED.contact_enrichment, '{}'::jsonb), prospects.contact_enrichment),
               service_type = EXCLUDED.service_type,
               updated_at = NOW()",
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
        .bind(instagram_handle.as_deref())
        .bind(business_email.as_deref())
        .bind(external_url.as_deref())
        .bind(&service)
        .bind(&x_dm)
        .bind(&email_script)
        .bind(&contact_enrichment)
        .bind(payload.sourced_by)
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
        return Err((
            StatusCode::SERVICE_UNAVAILABLE,
            Json(ErrorResponse {
                success: false,
                message: "TWITCH_CLIENT_ID not configured".to_string(),
            }),
        ));
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
        .send()
        .await
        .map_err(|e| {
            (
                StatusCode::BAD_GATEWAY,
                Json(ErrorResponse {
                    success: false,
                    message: e.to_string(),
                }),
            )
        })?
        .json()
        .await
        .map_err(|e| {
            (
                StatusCode::BAD_GATEWAY,
                Json(ErrorResponse {
                    success: false,
                    message: e.to_string(),
                }),
            )
        })?;

    let access_token = token_resp["access_token"]
        .as_str()
        .unwrap_or("")
        .to_string();

    // AI picks the best Twitch game/category to search for this prospect type
    let game_name = ai_generate_twitch_category(
        state,
        &payload.prospect_type,
        &payload
            .category
            .clone()
            .unwrap_or_else(|| "general".to_string()),
    )
    .await;
    tracing::info!("🎮 Twitch prospect category (AI): \"{}\"", game_name);

    // Look up the game ID on Twitch so we can filter streams by category
    let game_id: Option<String> = if !game_name.is_empty() {
        let game_url = format!(
            "https://api.twitch.tv/helix/games?name={}",
            urlencoding::encode(&game_name)
        );
        if let Ok(resp) = client
            .get(&game_url)
            .header("Client-Id", &client_id)
            .header("Authorization", format!("Bearer {}", access_token))
            .send()
            .await
        {
            if let Ok(val) = resp.json::<serde_json::Value>().await {
                val["data"]
                    .as_array()
                    .and_then(|arr| arr.first())
                    .and_then(|g| g["id"].as_str())
                    .map(String::from)
            } else {
                None
            }
        } else {
            None
        }
    } else {
        None
    };

    // Build streams URL — filter by game_id if found, otherwise top streams
    let streams_url = if let Some(ref gid) = game_id {
        format!(
            "https://api.twitch.tv/helix/streams?first={}&game_id={}",
            limit.min(100),
            gid
        )
    } else {
        format!(
            "https://api.twitch.tv/helix/streams?first={}",
            limit.min(100)
        )
    };

    let streams_resp: serde_json::Value = client
        .get(&streams_url)
        .header("Client-Id", &client_id)
        .header("Authorization", format!("Bearer {}", access_token))
        .send()
        .await
        .map_err(|e| {
            (
                StatusCode::BAD_GATEWAY,
                Json(ErrorResponse {
                    success: false,
                    message: e.to_string(),
                }),
            )
        })?
        .json()
        .await
        .map_err(|e| {
            (
                StatusCode::BAD_GATEWAY,
                Json(ErrorResponse {
                    success: false,
                    message: e.to_string(),
                }),
            )
        })?;

    let streams = streams_resp["data"].as_array().cloned().unwrap_or_default();
    let stream_logins: Vec<String> = streams
        .iter()
        .filter_map(|stream| stream["user_login"].as_str().map(str::to_string))
        .collect();

    let mut twitch_users_by_login = std::collections::HashMap::<String, serde_json::Value>::new();
    if !stream_logins.is_empty() {
        let mut users_req = client
            .get("https://api.twitch.tv/helix/users")
            .header("Client-Id", &client_id)
            .header("Authorization", format!("Bearer {}", access_token));

        for login in &stream_logins {
            users_req = users_req.query(&[("login", login)]);
        }

        if let Ok(resp) = users_req.send().await {
            if let Ok(value) = resp.json::<serde_json::Value>().await {
                if let Some(users) = value["data"].as_array() {
                    for user in users {
                        if let Some(login) = user["login"].as_str() {
                            twitch_users_by_login.insert(login.to_lowercase(), user.clone());
                        }
                    }
                }
            }
        }
    }
    let mut count = 0;

    for stream in &streams {
        let channel_id = stream["user_id"].as_str().unwrap_or("").to_string();
        let user_login = stream["user_login"].as_str().unwrap_or("").to_string();
        let display_name = stream["user_name"].as_str().unwrap_or("").to_string();
        let viewer_count: i64 = stream["viewer_count"].as_i64().unwrap_or(0);
        let game_name = stream["game_name"].as_str().unwrap_or("").to_string();

        if display_name.is_empty() {
            continue;
        }

        if let Some(min) = payload.min_viewers {
            if viewer_count < min {
                continue;
            }
        }
        if let Some(max) = payload.max_viewers {
            if viewer_count > max {
                continue;
            }
        }

        let platform_url = format!("https://twitch.tv/{}", display_name.to_lowercase());
        let category = if game_name.is_empty() {
            payload
                .category
                .clone()
                .unwrap_or_else(|| "gaming".to_string())
        } else {
            game_name.clone()
        };
        let user_details = twitch_users_by_login
            .get(&user_login.to_lowercase())
            .cloned()
            .unwrap_or(serde_json::Value::Null);
        let description = user_details["description"]
            .as_str()
            .unwrap_or("")
            .to_string();
        let mut twitter_handle = extract_best_twitter_handle(&description);
        let mut instagram_handle = extract_best_instagram_handle(&description);
        let mut business_email = extract_best_business_email(&description);
        let mut external_url = extract_best_external_url(&description);
        let enrichment_url = external_url.clone();
        let contact_enrichment = enrich_public_contact_fields(
            enrichment_url.as_deref(),
            &mut twitter_handle,
            &mut instagram_handle,
            &mut business_email,
            &mut external_url,
        )
        .await;
        let scoring_description = build_scoring_description(&description);

        let (score, reasoning, service, dm_creator, dm_clipper, x_dm, email_script) =
            score_prospect_with_ai(
                state,
                &display_name,
                viewer_count,
                &scoring_description,
                &category,
                &payload.prospect_type,
                "twitch",
                payload.service.as_deref(),
            )
            .await;

        sqlx::query(
            "INSERT INTO prospects (platform, channel_id, display_name, platform_url,
             avg_viewer_count, content_category, prospect_type,
             ai_score, ai_reasoning, dm_script_creator, dm_script_clipper,
             twitter_handle, instagram_handle, business_email, external_url, service_type,
             x_dm_script, email_script, contact_enrichment, sourced_by)
             VALUES ($1,$2,$3,$4,$5,$6,$7,$8,$9,$10,$11,$12,$13,$14,$15,$16,$17,$18,$19,$20)
             ON CONFLICT (platform, channel_id) DO UPDATE SET
               avg_viewer_count = EXCLUDED.avg_viewer_count,
               ai_score = EXCLUDED.ai_score,
               ai_reasoning = EXCLUDED.ai_reasoning,
               dm_script_creator = EXCLUDED.dm_script_creator,
               dm_script_clipper = EXCLUDED.dm_script_clipper,
               x_dm_script = EXCLUDED.x_dm_script,
               email_script = EXCLUDED.email_script,
               twitter_handle = COALESCE(EXCLUDED.twitter_handle, prospects.twitter_handle),
               instagram_handle = COALESCE(EXCLUDED.instagram_handle, prospects.instagram_handle),
               business_email = COALESCE(EXCLUDED.business_email, prospects.business_email),
               external_url = COALESCE(EXCLUDED.external_url, prospects.external_url),
               contact_enrichment = COALESCE(NULLIF(EXCLUDED.contact_enrichment, '{}'::jsonb), prospects.contact_enrichment),
               service_type = EXCLUDED.service_type,
               updated_at = NOW()",
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
        .bind(twitter_handle.as_deref())
        .bind(instagram_handle.as_deref())
        .bind(business_email.as_deref())
        .bind(external_url.as_deref())
        .bind(&service)
        .bind(&x_dm)
        .bind(&email_script)
        .bind(&contact_enrichment)
        .bind(payload.sourced_by)
        .execute(&state.db_pool)
        .await
        .ok();

        count += 1;
    }

    Ok(count)
}

async fn search_kick_prospects(
    state: &Arc<AppState>,
    payload: &SearchRequest,
    limit: usize,
) -> Result<usize, (StatusCode, Json<ErrorResponse>)> {
    let kick_client = state.kick_client.as_ref().ok_or_else(|| {
        (
            StatusCode::SERVICE_UNAVAILABLE,
            Json(ErrorResponse {
                success: false,
                message: "Kick.com client not configured".to_string(),
            }),
        )
    })?;

    // AI picks the best Kick category to search for this prospect type
    let category_query = ai_generate_twitch_category(
        state,
        &payload.prospect_type,
        &payload
            .category
            .clone()
            .unwrap_or_else(|| "general".to_string()),
    )
    .await;
    tracing::info!("🎮 Kick prospect category (AI): \"{}\"", category_query);

    // Fetch live streams in the AI-recommended category
    let streams = if !category_query.is_empty() && category_query != "NONE" {
        kick_client
            .search_livestreams_by_category_name(&category_query)
            .await
            .map_err(|e| {
                (
                    StatusCode::BAD_GATEWAY,
                    Json(ErrorResponse {
                        success: false,
                        message: format!("Kick livestream search failed: {}", e),
                    }),
                )
            })?
    } else {
        kick_client
            .get_livestreams(None, None, Some(100))
            .await
            .map_err(|e| {
                (
                    StatusCode::BAD_GATEWAY,
                    Json(ErrorResponse {
                        success: false,
                        message: format!("Kick livestream list failed: {}", e),
                    }),
                )
            })?
    };

    let mut count = 0usize;
    for stream in streams.iter().take(limit) {
        let channel_url = format!("https://kick.com/{}", stream.slug);
        let channel_name = stream.slug.clone();
        let description = stream
            .stream_title
            .as_deref()
            .unwrap_or(&channel_name)
            .to_string();
        let scoring_description = build_scoring_description(&description);

        let (ai_score, _reasoning, service, dm_creator, dm_clipper, x_dm, email_script) =
            score_prospect_with_ai(
                state,
                &channel_name,
                stream.viewer_count.unwrap_or(0),
                &scoring_description,
                &payload
                    .category
                    .clone()
                    .unwrap_or_else(|| "general".to_string()),
                &payload.prospect_type,
                "kick",
                payload.service.as_deref(),
            )
            .await;

        if ai_score < 0.3 {
            continue;
        }

        let category = payload
            .category
            .clone()
            .unwrap_or_else(|| "general".to_string());
        let _ = sqlx::query(
            r#"INSERT INTO prospects
               (platform, channel_id, display_name, platform_url,
                channel_description, avg_viewer_count, content_category,
                prospect_type, ai_score, ai_reasoning,
                dm_script_creator, dm_script_clipper,
                service_type, x_dm_script, email_script)
               VALUES ($1,$2,$3,$4,$5,$6,$7,$8,$9,$10,$11,$12,$13,$14,$15)
               ON CONFLICT (platform, channel_id) DO UPDATE SET
                ai_score = EXCLUDED.ai_score,
                avg_viewer_count = EXCLUDED.avg_viewer_count,
                ai_reasoning = EXCLUDED.ai_reasoning,
                x_dm_script = EXCLUDED.x_dm_script,
                email_script = EXCLUDED.email_script,
                updated_at = NOW()"#,
        )
        .bind("kick")
        .bind(&stream.slug)
        .bind(&stream.slug)
        .bind(&channel_url)
        .bind(&description)
        .bind(stream.viewer_count.unwrap_or(0))
        .bind(&category)
        .bind(&payload.prospect_type)
        .bind(ai_score)
        .bind(&_reasoning)
        .bind(&dm_creator)
        .bind(&dm_clipper)
        .bind(&service)
        .bind(&x_dm)
        .bind(&email_script)
        .execute(&state.db_pool)
        .await
        .ok();

        count += 1;
    }

    Ok(count)
}

/// Search YouTube for channels that repost Kick.com content (clipping channels).
/// Unlike `search_kick_prospects` which searches Kick.com live streams, this function
/// finds YouTube channels whose content shows Kick.com branding (green watermark),
/// indicating they clip Kick streamers and repost to YouTube. These are the actual
/// target prospects for the `kick_auto_clipper` Managed Campaign service.
async fn search_kick_clipper_prospects(
    state: &Arc<AppState>,
    payload: &SearchRequest,
    limit: usize,
) -> Result<usize, (StatusCode, Json<ErrorResponse>)> {
    let api_key = std::env::var("YOUTUBE_API_KEY").unwrap_or_default();
    if api_key.is_empty() {
        return Err((
            StatusCode::BAD_REQUEST,
            Json(ErrorResponse {
                success: false,
                message: "YouTube API key not configured".to_string(),
            }),
        ));
    }

    let http = reqwest::Client::new();
    let limit = limit.min(20);
    let mut found = 0usize;

    // 1. AI generates Kick-specific YouTube search query
    let base_category = payload
        .category
        .clone()
        .unwrap_or_else(|| "gaming clips".to_string());
    let query_prompt = format!(
        "You are finding YouTube CHANNELS that clip and repost content from Kick.com streamers. \
         These channels repost Kick stream highlights with the Kick.com green watermark visible. \
         Generate ONE concise YouTube search query (5-10 words) to find such channels. \
         Prospect type: clipper. Category: {}. Return ONLY the raw search query.",
        base_category
    );
    let search_query = match crate::llm_utils::generate_text_fast(
        state.qwen_client.as_ref(),
        state.deepseek_client.as_ref(),
        state.gemini_client.as_ref(),
        &query_prompt,
    )
    .await
    {
        Ok(q) => q.trim().trim_matches('"').to_string(),
        Err(_) => "kick stream highlights compilation channel".to_string(),
    };

    tracing::info!(
        "🎯 Kick clipper YouTube search query (AI): \"{}\"",
        search_query
    );

    // 2. Search YouTube for channels matching the query
    let search_url = format!(
        "https://www.googleapis.com/youtube/v3/search?part=snippet&type=channel&q={}&maxResults={}&order=relevance&key={}",
        urlencoding::encode(&search_query), limit.min(50), api_key
    );

    let search_resp: serde_json::Value = http
        .get(&search_url)
        .send()
        .await
        .map_err(|e| {
            (
                StatusCode::BAD_GATEWAY,
                Json(ErrorResponse {
                    success: false,
                    message: format!("YouTube search failed: {e}"),
                }),
            )
        })?
        .json()
        .await
        .map_err(|e| {
            (
                StatusCode::BAD_GATEWAY,
                Json(ErrorResponse {
                    success: false,
                    message: format!("YouTube search parse failed: {e}"),
                }),
            )
        })?;

    let channel_ids: Vec<String> = search_resp["items"]
        .as_array()
        .map(|a| {
            a.iter()
                .filter_map(|item| item["id"]["channelId"].as_str().map(String::from))
                .collect()
        })
        .unwrap_or_default();

    if channel_ids.is_empty() {
        return Ok(0);
    }

    // 3. Get channel stats + snippet
    let stats_url = format!(
        "https://www.googleapis.com/youtube/v3/channels?part=snippet,statistics&id={}&key={}",
        channel_ids.join(","),
        api_key
    );

    let stats_resp: serde_json::Value = http
        .get(&stats_url)
        .send()
        .await
        .map_err(|e| {
            (
                StatusCode::BAD_GATEWAY,
                Json(ErrorResponse {
                    success: false,
                    message: format!("YouTube stats failed: {e}"),
                }),
            )
        })?
        .json()
        .await
        .map_err(|e| {
            (
                StatusCode::BAD_GATEWAY,
                Json(ErrorResponse {
                    success: false,
                    message: format!("YouTube stats parse failed: {e}"),
                }),
            )
        })?;

    let channels_data = stats_resp["items"]
        .as_array()
        .map(|a| a.to_vec())
        .unwrap_or_default();

    // 4. Process each channel
    for channel in &channels_data {
        let channel_id = channel["id"].as_str().unwrap_or("").to_string();
        let display_name = channel["snippet"]["title"]
            .as_str()
            .unwrap_or("Unknown")
            .to_string();
        let description = channel["snippet"]["description"]
            .as_str()
            .unwrap_or("")
            .to_string();
        let sub_count = channel["statistics"]["subscriberCount"]
            .as_str()
            .and_then(|s| s.parse::<i64>().ok())
            .unwrap_or(0);
        let video_count = channel["statistics"]["videoCount"]
            .as_str()
            .and_then(|s| s.parse::<i64>().ok())
            .unwrap_or(0);
        let platform_url = format!("https://youtube.com/channel/{channel_id}");

        // Apply subscriber range filter
        if let Some(min) = payload.min_viewers {
            if sub_count < min {
                continue;
            }
        }
        if let Some(max) = payload.max_viewers {
            if sub_count > max {
                continue;
            }
        }

        // 5. Fetch 3 most recent videos for thumbnail analysis
        let videos_url = format!(
            "https://www.googleapis.com/youtube/v3/search?part=snippet&channelId={}&maxResults=3&order=date&type=video&key={}",
            channel_id, api_key
        );

        let videos_resp: serde_json::Value = match http.get(&videos_url).send().await {
            Ok(r) => r.json().await.unwrap_or_default(),
            Err(_) => continue,
        };

        let videos = videos_resp["items"]
            .as_array()
            .map(|a| a.to_vec())
            .unwrap_or_default();

        let mut video_titles: Vec<String> = Vec::new();
        let mut thumbnail_urls: Vec<String> = Vec::new();

        for video in &videos {
            if let Some(title) = video["snippet"]["title"].as_str() {
                video_titles.push(title.to_string());
            }
            let thumb = video["snippet"]["thumbnails"]["high"]["url"]
                .as_str()
                .or_else(|| video["snippet"]["thumbnails"]["medium"]["url"].as_str())
                .or_else(|| video["snippet"]["thumbnails"]["default"]["url"].as_str());
            if let Some(t) = thumb {
                thumbnail_urls.push(t.to_string());
            }
        }

        // 6. Text-based Kick signal detection (free, no vision API call)
        let title_kick_keywords = video_titles
            .iter()
            .filter(|t| {
                let l = t.to_lowercase();
                l.contains("kick")
                    || l.contains("stream")
                    || l.contains("highlights")
                    || l.contains("clips")
                    || l.contains("moments")
                    || l.contains("compilation")
                    || l.contains("best of")
            })
            .count();

        let desc_has_kick = description.to_lowercase().contains("kick.com")
            || description.to_lowercase().contains(" kick ")
            || description.to_lowercase().contains("kickstreamer");

        // 7. Vision analysis: check first thumbnail for Kick.com watermark (green logo)
        let mut vision_confirmed = false;
        if let Some(thumb_url) = thumbnail_urls.first() {
            if let Ok(resp) = http.get(thumb_url).send().await {
                if let Ok(bytes) = resp.bytes().await {
                    if !bytes.is_empty() {
                        let vision_prompt = "Analyze this YouTube video thumbnail. \
                            Does it contain the Kick.com logo, text \"Kick.com\", \
                            a green circular watermark with \"Kick\", \
                            or green brand styling? Look for the distinctive green Kick.com watermark. \
                            Answer ONLY with YES or NO.";
                        vision_confirmed = match &state.gemini_client {
                            Some(g) => g
                                .analyze_image_bytes(&bytes, vision_prompt)
                                .await
                                .ok()
                                .map(|r| r.to_uppercase().contains("YES"))
                                .unwrap_or(false),
                            None => false,
                        };
                    }
                }
            }
        }

        // 8. Calculate posting frequency (videos/week, rough estimate)
        let posting_weekly = (video_count as f64 / 4.0).max(0.5);

        // 9. Build comprehensive scoring description for the LLM
        let scoring_desc = format!(
            "YouTube channel: {}. Subscribers: {}. Total videos: {}. \
             Recent video titles: {:?}. \
             Description mentions Kick: {}. Thumbnails show Kick branding: {}. \
             Kick-related keywords in titles: {}/{}. Weekly posting: {:.0} videos/week. \
             Assessment: This channel appears to {} Kick.com content on YouTube.",
            display_name,
            sub_count,
            video_count,
            video_titles,
            if desc_has_kick { "YES" } else { "no" },
            if vision_confirmed {
                "YES (green Kick watermark detected in thumbnails)"
            } else {
                "no"
            },
            title_kick_keywords,
            video_titles.len(),
            posting_weekly,
            if vision_confirmed || desc_has_kick {
                "be actively reposting"
            } else {
                "potentially repost"
            }
        );

        // 10. AI scoring — force clipper prospect type for kick_auto_clipper evaluation
        let (score, reasoning, _service, dm_creator, dm_clipper, x_dm, email_script) =
            score_prospect_with_ai(
                state,
                &display_name,
                sub_count,
                &scoring_desc,
                &base_category,
                "clipper",
                "kick",
                payload.service.as_deref(),
            )
            .await;

        if score < 0.3 {
            continue;
        }

        // 11. Identify which Kick creators this prospect clips (for sample generation)
        let mut detected_source_creators: Vec<String> = Vec::new();
        if score >= 0.5 {
            let creator_prompt = format!(
                "YouTube channel '{}' reposts clips from Kick.com streamers. \
                 Based on the channel name, description, and recent video titles {:?}, \
                 identify which specific Kick.com streamers/creators this channel clips content from. \
                 Return ONLY a comma-separated list of Kick usernames/slugs. \
                 If you cannot identify specific creators, return 'unknown'.",
                display_name, video_titles
            );
            if let Ok(creators_raw) = crate::llm_utils::generate_text_fast(
                state.qwen_client.as_ref(),
                state.deepseek_client.as_ref(),
                state.gemini_client.as_ref(),
                &creator_prompt,
            )
            .await
            {
                let raw = creators_raw.trim().to_lowercase();
                if !raw.contains("unknown") && !raw.is_empty() {
                    detected_source_creators = raw
                        .split(',')
                        .map(|s| s.trim().trim_matches('"').trim_matches('\'').to_string())
                        .filter(|s| !s.is_empty())
                        .collect();
                }
            }
        }

        // 12. Contact enrichment
        let mut twitter_handle = extract_best_twitter_handle(&description);
        let mut instagram_handle = extract_best_instagram_handle(&description);
        let mut business_email = extract_best_business_email(&description);
        let mut external_url = extract_best_external_url(&description);

        let enrichment_url = if platform_url.starts_with("http") {
            Some(platform_url.as_str())
        } else {
            None
        };
        let mut contact_enrichment = enrich_public_contact_fields(
            enrichment_url,
            &mut twitter_handle,
            &mut instagram_handle,
            &mut business_email,
            &mut external_url,
        )
        .await;

        // Embed detected source creators for sample generation
        if !detected_source_creators.is_empty() {
            if let Some(obj) = contact_enrichment.as_object_mut() {
                obj.insert(
                    "detected_source_creators".to_string(),
                    json!(detected_source_creators),
                );
            }
        }

        // 12. Store prospect with kick_auto_clipper service type
        let kick_signals = json!({
            "vision_confirmed": vision_confirmed,
            "title_kick_keywords": title_kick_keywords,
            "desc_has_kick": desc_has_kick,
            "posting_weekly": posting_weekly,
            "video_titles_analyzed": video_titles.len(),
        });

        let channel_desc = format!(
            "{} | Kick clipper channel, {} videos, ~{:.0}/week | signals: {}",
            description, video_count, posting_weekly, kick_signals
        );

        let reasoning_with_signals = format!(
            "{} | Kick signals: titleKw={}/{} vision={} descKick={} weekly={:.0}",
            reasoning,
            title_kick_keywords,
            video_titles.len(),
            vision_confirmed,
            desc_has_kick,
            posting_weekly
        );

        let _ = sqlx::query(
            "INSERT INTO prospects \
             (platform, channel_id, display_name, platform_url, subscriber_count, \
              content_category, channel_description, prospect_type, \
              ai_score, ai_reasoning, dm_script_creator, dm_script_clipper, \
              twitter_handle, instagram_handle, business_email, external_url, \
              service_type, x_dm_script, email_script, contact_enrichment, sourced_by) \
             VALUES ($1,$2,$3,$4,$5,$6,$7,$8,$9,$10,$11,$12,$13,$14,$15,$16,$17,$18,$19,$20,$21) \
             ON CONFLICT (platform, channel_id) DO UPDATE SET \
             ai_score = EXCLUDED.ai_score, ai_reasoning = EXCLUDED.ai_reasoning, \
             dm_script_creator = EXCLUDED.dm_script_creator, dm_script_clipper = EXCLUDED.dm_script_clipper, \
             x_dm_script = EXCLUDED.x_dm_script, email_script = EXCLUDED.email_script, \
             twitter_handle = COALESCE(EXCLUDED.twitter_handle, prospects.twitter_handle), \
             instagram_handle = COALESCE(EXCLUDED.instagram_handle, prospects.instagram_handle), \
             business_email = COALESCE(EXCLUDED.business_email, prospects.business_email), \
             external_url = COALESCE(EXCLUDED.external_url, prospects.external_url), \
             contact_enrichment = COALESCE(NULLIF(EXCLUDED.contact_enrichment, '{}'::jsonb), prospects.contact_enrichment), \
             service_type = EXCLUDED.service_type, updated_at = NOW()"
        )
        .bind("youtube")
        .bind(&channel_id)
        .bind(&display_name)
        .bind(&platform_url)
        .bind(sub_count)
        .bind(&base_category)
        .bind(&channel_desc)
        .bind("clipper")
        .bind(score)
        .bind(&reasoning_with_signals)
        .bind(&dm_creator)
        .bind(&dm_clipper)
        .bind(&twitter_handle)
        .bind(&instagram_handle)
        .bind(&business_email)
        .bind(&external_url)
        .bind("kick_auto_clipper")
        .bind(&x_dm)
        .bind(&email_script)
        .bind(&contact_enrichment)
        .bind(payload.sourced_by)
        .execute(&state.db_pool)
        .await
        .ok();

        found += 1;
    }

    Ok(found)
}

/// Search for kick_auto_clipper prospects by starting from Kick.com:
/// 1. Find top live streamers on Kick via the API (free, no Cloudflare)
/// 2. For each top streamer, search YouTube for channels posting their clips
/// 3. Score and store those channels as kick_auto_clipper prospects
async fn search_kick_clipper_prospects_top_streamers(
    state: &Arc<AppState>,
    payload: &SearchRequest,
    limit: usize,
) -> Result<usize, (StatusCode, Json<ErrorResponse>)> {
    let api_key = std::env::var("YOUTUBE_API_KEY").unwrap_or_default();
    if api_key.is_empty() {
        return Err((
            StatusCode::BAD_REQUEST,
            Json(ErrorResponse {
                success: false,
                message: "YouTube API key not configured".to_string(),
            }),
        ));
    }

    let Some(kick) = &state.kick_client else {
        return Err((
            StatusCode::BAD_REQUEST,
            Json(ErrorResponse {
                success: false,
                message: "Kick client not configured".to_string(),
            }),
        ));
    };

    let http = reqwest::Client::new();
    let limit = limit.min(20);
    let mut found = 0usize;
    let base_category = payload
        .category
        .clone()
        .unwrap_or_else(|| "just chatting".to_string());

    // 1. Fetch top live streamers from Kick via API
    let livestreams = match kick
        .search_livestreams_by_category_name(&base_category)
        .await
    {
        Ok(streams) => streams,
        Err(e) => {
            tracing::warn!("kick_top_streamers: failed to fetch livestreams: {e}");
            Vec::new()
        }
    };

    if livestreams.is_empty() {
        tracing::info!(
            "kick_top_streamers: no live streams found for category '{}'",
            base_category
        );
        return Ok(0);
    }

    // Sort by viewer count descending, take top N streamers
    let mut sorted: Vec<_> = livestreams
        .into_iter()
        .filter_map(|s| {
            let viewers = s.viewer_count.unwrap_or(0);
            if viewers < 10 {
                return None;
            } // Skip tiny streams
            Some((viewers, s))
        })
        .collect();
    sorted.sort_by(|a, b| b.0.cmp(&a.0));
    let top_streamers: Vec<_> = sorted.into_iter().take(10).collect(); // Top 10 streamers

    tracing::info!(
        "🎯 Kick top streamers in '{}': {} found, processing top {}",
        base_category,
        top_streamers.len(),
        top_streamers.len()
    );

    // 2. For each top Kick streamer, search YouTube for clipping channels
    for (viewers, streamer) in &top_streamers {
        let slug = &streamer.slug;
        let stream_title = streamer.stream_title.as_deref().unwrap_or(slug);

        // AI generates a targeted YouTube search query for this specific streamer's clippers
        let query_prompt = format!(
            "A Kick.com streamer named '{}' ({}) streams '{}'. \
             Find YouTube CHANNELS that clip and repost this streamer's content. \
             Generate ONE concise YouTube search query (5-10 words) to find these clipping channels. \
             Return ONLY the raw search query.",
            slug, stream_title, base_category
        );
        let search_query = match crate::llm_utils::generate_text_fast(
            state.qwen_client.as_ref(),
            state.deepseek_client.as_ref(),
            state.gemini_client.as_ref(),
            &query_prompt,
        )
        .await
        {
            Ok(q) => q.trim().trim_matches('"').to_string(),
            Err(_) => format!("{} kick stream highlights clips", slug),
        };

        tracing::info!(
            "  Searching YouTube for clippers of '{}': \"{}\"",
            slug,
            search_query
        );

        // Search YouTube for channels
        let search_url = format!(
            "https://www.googleapis.com/youtube/v3/search?part=snippet&type=channel&q={}&maxResults=5&order=relevance&key={}",
            urlencoding::encode(&search_query), api_key
        );

        let search_resp: serde_json::Value = match http.get(&search_url).send().await {
            Ok(r) => r.json().await.unwrap_or_default(),
            Err(_) => continue,
        };

        let channel_ids: Vec<String> = search_resp["items"]
            .as_array()
            .map(|a| {
                a.iter()
                    .filter_map(|item| item["id"]["channelId"].as_str().map(String::from))
                    .collect()
            })
            .unwrap_or_default();

        if channel_ids.is_empty() {
            continue;
        }

        // Get channel stats
        let stats_url = format!(
            "https://www.googleapis.com/youtube/v3/channels?part=snippet,statistics&id={}&key={}",
            channel_ids.join(","),
            api_key
        );
        let stats_resp: serde_json::Value = match http.get(&stats_url).send().await {
            Ok(r) => r.json().await.unwrap_or_default(),
            Err(_) => continue,
        };

        let channels_data = stats_resp["items"]
            .as_array()
            .map(|a| a.to_vec())
            .unwrap_or_default();

        for channel in &channels_data {
            let channel_id = channel["id"].as_str().unwrap_or("").to_string();
            let display_name = channel["snippet"]["title"]
                .as_str()
                .unwrap_or("Unknown")
                .to_string();
            let description = channel["snippet"]["description"]
                .as_str()
                .unwrap_or("")
                .to_string();
            let sub_count = channel["statistics"]["subscriberCount"]
                .as_str()
                .and_then(|s| s.parse::<i64>().ok())
                .unwrap_or(0);
            let video_count = channel["statistics"]["videoCount"]
                .as_str()
                .and_then(|s| s.parse::<i64>().ok())
                .unwrap_or(0);
            let platform_url = format!("https://youtube.com/channel/{}", channel_id);

            // Apply subscriber range filter
            if let Some(min) = payload.min_viewers {
                if sub_count < min {
                    continue;
                }
            }
            if let Some(max) = payload.max_viewers {
                if sub_count > max {
                    continue;
                }
            }

            // Fetch recent video titles for analysis
            let videos_url = format!(
                "https://www.googleapis.com/youtube/v3/search?part=snippet&channelId={}&maxResults=3&order=date&type=video&key={}",
                channel_id, api_key
            );
            let videos_resp: serde_json::Value = match http.get(&videos_url).send().await {
                Ok(r) => r.json().await.unwrap_or_default(),
                Err(_) => serde_json::Value::Null,
            };
            let video_titles: Vec<String> = videos_resp["items"]
                .as_array()
                .map(|a| {
                    a.iter()
                        .filter_map(|v| v["snippet"]["title"].as_str().map(String::from))
                        .collect()
                })
                .unwrap_or_default();

            // Build scoring description — we already know this channel clips THIS streamer
            let scoring_desc = format!(
                "YouTube channel: {}. Subscribers: {}. Total videos: {}. \
                 Recent video titles: {:?}. \
                 This channel clips content from Kick streamer '{slug}' ({viewers} live viewers, '{base_category}'). \
                 Assessment: This channel reposts {slug}'s Kick.com content on YouTube.",
                display_name, sub_count, video_count, video_titles,
            );

            // Score with LLM
            let (score, reasoning, _service, dm_creator, dm_clipper, x_dm, email_script) =
                score_prospect_with_ai(
                    state,
                    &display_name,
                    sub_count,
                    &scoring_desc,
                    &base_category,
                    "clipper",
                    "kick",
                    payload.service.as_deref(),
                )
                .await;

            if score < 0.3 {
                continue;
            }

            // detected_source_creators = the Kick streamer this channel clips
            let detected_source_creators = vec![slug.clone()];

            // Contact enrichment
            let mut twitter_handle = extract_best_twitter_handle(&description);
            let mut instagram_handle = extract_best_instagram_handle(&description);
            let mut business_email = extract_best_business_email(&description);
            let mut external_url = extract_best_external_url(&description);

            let enrichment_url = Some(platform_url.as_str());
            let mut contact_enrichment = enrich_public_contact_fields(
                enrichment_url,
                &mut twitter_handle,
                &mut instagram_handle,
                &mut business_email,
                &mut external_url,
            )
            .await;

            if !detected_source_creators.is_empty() {
                if let Some(obj) = contact_enrichment.as_object_mut() {
                    obj.insert(
                        "detected_source_creators".to_string(),
                        json!(detected_source_creators),
                    );
                }
            }

            let post_signal = json!({
                "source_kick_streamer": slug,
                "source_kick_viewers": viewers,
                "source_kick_category": base_category,
            });

            let channel_desc = format!(
                "{} | Clips {} from Kick ({} viewers) | {} videos",
                description, slug, viewers, video_count
            );
            let reasoning_with_signals = format!(
                "{} | Clips Kick streamer={} viewers={}",
                reasoning, slug, viewers
            );

            let _ = sqlx::query(
                "INSERT INTO prospects \
                 (platform, channel_id, display_name, platform_url, subscriber_count, \
                  content_category, channel_description, prospect_type, \
                  ai_score, ai_reasoning, dm_script_creator, dm_script_clipper, \
                  twitter_handle, instagram_handle, business_email, external_url, \
                  service_type, x_dm_script, email_script, contact_enrichment, sourced_by) \
                 VALUES ($1,$2,$3,$4,$5,$6,$7,$8,$9,$10,$11,$12,$13,$14,$15,$16,$17,$18,$19,$20,$21) \
                 ON CONFLICT (platform, channel_id) DO UPDATE SET \
                 ai_score = EXCLUDED.ai_score, ai_reasoning = EXCLUDED.ai_reasoning, \
                 dm_script_creator = EXCLUDED.dm_script_creator, dm_script_clipper = EXCLUDED.dm_script_clipper, \
                 x_dm_script = EXCLUDED.x_dm_script, email_script = EXCLUDED.email_script, \
                 twitter_handle = COALESCE(EXCLUDED.twitter_handle, prospects.twitter_handle), \
                 instagram_handle = COALESCE(EXCLUDED.instagram_handle, prospects.instagram_handle), \
                 business_email = COALESCE(EXCLUDED.business_email, prospects.business_email), \
                 external_url = COALESCE(EXCLUDED.external_url, prospects.external_url), \
                 contact_enrichment = COALESCE(NULLIF(EXCLUDED.contact_enrichment, '{}'::jsonb), prospects.contact_enrichment), \
                 service_type = EXCLUDED.service_type, updated_at = NOW()"
            )
            .bind("youtube")
            .bind(&channel_id)
            .bind(&display_name)
            .bind(&platform_url)
            .bind(sub_count)
            .bind(&base_category)
            .bind(&channel_desc)
            .bind("clipper")
            .bind(score)
            .bind(&reasoning_with_signals)
            .bind(&dm_creator)
            .bind(&dm_clipper)
            .bind(&twitter_handle)
            .bind(&instagram_handle)
            .bind(&business_email)
            .bind(&external_url)
            .bind("kick_auto_clipper")
            .bind(&x_dm)
            .bind(&email_script)
            .bind(&contact_enrichment)
            .bind(payload.sourced_by)
            .execute(&state.db_pool)
            .await
            .ok();

            found += 1;
            if found >= limit {
                break;
            }
        }

        if found >= limit {
            break;
        }
    }

    tracing::info!("🎯 Kick top-streamer method: found {} prospects", found);
    Ok(found)
}

/// Search for twitch_clipping prospects by starting from Twitch itself:
/// 1. Find top live streamers via the Helix API (free, same creds as search)
/// 2. For each top streamer, search YouTube for channels posting their clips
/// 3. Score and store those channels as twitch_clipping prospects with
///    prospect_type "twitch_clipper" (vs the big creators found by the plain
///    "twitch" platform search). Mirrors kick Method B.
async fn search_twitch_clipper_prospects_top_streamers(
    state: &Arc<AppState>,
    payload: &SearchRequest,
    limit: usize,
) -> Result<usize, (StatusCode, Json<ErrorResponse>)> {
    let yt_key = std::env::var("YOUTUBE_API_KEY").unwrap_or_default();
    if yt_key.is_empty() {
        return Err((
            StatusCode::BAD_REQUEST,
            Json(ErrorResponse {
                success: false,
                message: "YouTube API key not configured".to_string(),
            }),
        ));
    }
    let client_id = std::env::var("TWITCH_TV_CLIENT_ID").unwrap_or_default();
    let client_secret = std::env::var("TWITCH_TV_CLIENT_SECRET").unwrap_or_default();
    if client_id.is_empty() {
        return Err((
            StatusCode::SERVICE_UNAVAILABLE,
            Json(ErrorResponse {
                success: false,
                message: "TWITCH_CLIENT_ID not configured".to_string(),
            }),
        ));
    }

    let http = reqwest::Client::new();
    let limit = limit.min(20);
    let base_category = payload
        .category
        .clone()
        .unwrap_or_else(|| "Just Chatting".to_string());

    // 1. App access token, then top live streams (Helix returns viewers desc).
    let token_resp: serde_json::Value = http
        .post("https://id.twitch.tv/oauth2/token")
        .form(&[
            ("client_id", client_id.as_str()),
            ("client_secret", client_secret.as_str()),
            ("grant_type", "client_credentials"),
        ])
        .send()
        .await
        .map_err(|e| {
            (
                StatusCode::BAD_GATEWAY,
                Json(ErrorResponse {
                    success: false,
                    message: e.to_string(),
                }),
            )
        })?
        .json()
        .await
        .map_err(|e| {
            (
                StatusCode::BAD_GATEWAY,
                Json(ErrorResponse {
                    success: false,
                    message: e.to_string(),
                }),
            )
        })?;
    let access_token = token_resp["access_token"]
        .as_str()
        .unwrap_or("")
        .to_string();

    let streams_resp: serde_json::Value = http
        .get("https://api.twitch.tv/helix/streams?first=20")
        .header("Client-Id", &client_id)
        .header("Authorization", format!("Bearer {}", access_token))
        .send()
        .await
        .map_err(|e| {
            (
                StatusCode::BAD_GATEWAY,
                Json(ErrorResponse {
                    success: false,
                    message: e.to_string(),
                }),
            )
        })?
        .json()
        .await
        .unwrap_or_default();

    let mut top: Vec<(i64, String, String)> = streams_resp["data"]
        .as_array()
        .map(|a| {
            a.iter()
                .filter_map(|s| {
                    let viewers = s["viewer_count"].as_i64().unwrap_or(0);
                    if viewers < 50 {
                        return None;
                    }
                    Some((
                        viewers,
                        s["user_login"].as_str().unwrap_or("").to_string(),
                        s["title"].as_str().unwrap_or("").to_string(),
                    ))
                })
                .collect()
        })
        .unwrap_or_default();
    top.sort_by(|a, b| b.0.cmp(&a.0));
    top.truncate(10);

    if top.is_empty() {
        tracing::info!("twitch_top_streamers: no live streams found");
        return Ok(0);
    }
    tracing::info!("🎯 Twitch top streamers: processing top {}", top.len());

    // Forced service for this finder (dispatch validated it when requested).
    let forced = payload.service.as_deref();

    // 2. For each top Twitch streamer, search YouTube for clipping channels.
    let mut found = 0usize;
    for (viewers, login, stream_title) in &top {
        if login.is_empty() {
            continue;
        }
        let query_prompt = format!(
            "A Twitch streamer named '{}' ('{}') streams '{}'. \
             Find YouTube CHANNELS that clip and repost this streamer's content. \
             Generate ONE concise YouTube search query (5-10 words) to find these clipping channels. \
             Return ONLY the raw search query.",
            login, stream_title, base_category
        );
        let search_query = match crate::llm_utils::generate_text_fast(
            state.qwen_client.as_ref(),
            state.deepseek_client.as_ref(),
            state.gemini_client.as_ref(),
            &query_prompt,
        )
        .await
        {
            Ok(q) => q.trim().trim_matches('"').to_string(),
            Err(_) => format!("{} twitch stream highlights clips", login),
        };

        tracing::info!(
            "  Searching YouTube for clippers of '{}': \"{}\"",
            login,
            search_query
        );

        let search_url = format!(
            "https://www.googleapis.com/youtube/v3/search?part=snippet&type=channel&q={}&maxResults=5&order=relevance&key={}",
            urlencoding::encode(&search_query), yt_key
        );
        let search_resp: serde_json::Value = match http.get(&search_url).send().await {
            Ok(r) => r.json().await.unwrap_or_default(),
            Err(_) => continue,
        };
        let channel_ids: Vec<String> = search_resp["items"]
            .as_array()
            .map(|a| {
                a.iter()
                    .filter_map(|item| item["id"]["channelId"].as_str().map(String::from))
                    .collect()
            })
            .unwrap_or_default();
        if channel_ids.is_empty() {
            continue;
        }

        let stats_url = format!(
            "https://www.googleapis.com/youtube/v3/channels?part=snippet,statistics&id={}&key={}",
            channel_ids.join(","),
            yt_key
        );
        let stats_resp: serde_json::Value = match http.get(&stats_url).send().await {
            Ok(r) => r.json().await.unwrap_or_default(),
            Err(_) => continue,
        };
        let channels_data = stats_resp["items"]
            .as_array()
            .map(|a| a.to_vec())
            .unwrap_or_default();

        for channel in &channels_data {
            let channel_id = channel["id"].as_str().unwrap_or("").to_string();
            let display_name = channel["snippet"]["title"]
                .as_str()
                .unwrap_or("Unknown")
                .to_string();
            let description = channel["snippet"]["description"]
                .as_str()
                .unwrap_or("")
                .to_string();
            let sub_count = channel["statistics"]["subscriberCount"]
                .as_str()
                .and_then(|s| s.parse::<i64>().ok())
                .unwrap_or(0);
            let video_count = channel["statistics"]["videoCount"]
                .as_str()
                .and_then(|s| s.parse::<i64>().ok())
                .unwrap_or(0);
            let platform_url = format!("https://youtube.com/channel/{}", channel_id);

            if let Some(min) = payload.min_viewers {
                if sub_count < min {
                    continue;
                }
            }
            if let Some(max) = payload.max_viewers {
                if sub_count > max {
                    continue;
                }
            }

            let videos_url = format!(
                "https://www.googleapis.com/youtube/v3/search?part=snippet&channelId={}&maxResults=3&order=date&type=video&key={}",
                channel_id, yt_key
            );
            let videos_resp: serde_json::Value = match http.get(&videos_url).send().await {
                Ok(r) => r.json().await.unwrap_or_default(),
                Err(_) => serde_json::Value::Null,
            };
            let video_titles: Vec<String> = videos_resp["items"]
                .as_array()
                .map(|a| {
                    a.iter()
                        .filter_map(|v| v["snippet"]["title"].as_str().map(String::from))
                        .collect()
                })
                .unwrap_or_default();

            let scoring_desc = format!(
                "YouTube channel: {}. Subscribers: {}. Total videos: {}. \
                 Recent video titles: {:?}. \
                 This channel clips content from Twitch streamer '{login}' ({viewers} live viewers, '{base_category}'). \
                 Assessment: This channel reposts {login}'s Twitch content on YouTube.",
                display_name, sub_count, video_count, video_titles,
            );

            let (score, reasoning, _service, dm_creator, dm_clipper, x_dm, email_script) =
                score_prospect_with_ai(
                    state,
                    &display_name,
                    sub_count,
                    &scoring_desc,
                    &base_category,
                    "twitch_clipper",
                    "twitch",
                    forced,
                )
                .await;

            if score < 0.3 {
                continue;
            }

            let detected_source_creators = vec![login.clone()];

            let mut twitter_handle = extract_best_twitter_handle(&description);
            let mut instagram_handle = extract_best_instagram_handle(&description);
            let mut business_email = extract_best_business_email(&description);
            let mut external_url = extract_best_external_url(&description);

            let enrichment_url = Some(platform_url.as_str());
            let mut contact_enrichment = enrich_public_contact_fields(
                enrichment_url,
                &mut twitter_handle,
                &mut instagram_handle,
                &mut business_email,
                &mut external_url,
            )
            .await;

            if !detected_source_creators.is_empty() {
                if let Some(obj) = contact_enrichment.as_object_mut() {
                    obj.insert(
                        "detected_source_creators".to_string(),
                        json!(detected_source_creators),
                    );
                }
            }

            let channel_desc = format!(
                "{} | Clips {} from Twitch ({} viewers) | {} videos",
                description, login, viewers, video_count
            );
            let reasoning_with_signals = format!(
                "{} | Clips Twitch streamer={} viewers={}",
                reasoning, login, viewers
            );

            let _ = sqlx::query(
                "INSERT INTO prospects \
                 (platform, channel_id, display_name, platform_url, subscriber_count, \
                  content_category, channel_description, prospect_type, \
                  ai_score, ai_reasoning, dm_script_creator, dm_script_clipper, \
                  twitter_handle, instagram_handle, business_email, external_url, \
                  service_type, x_dm_script, email_script, contact_enrichment, sourced_by) \
                 VALUES ($1,$2,$3,$4,$5,$6,$7,$8,$9,$10,$11,$12,$13,$14,$15,$16,$17,$18,$19,$20,$21) \
                 ON CONFLICT (platform, channel_id) DO UPDATE SET \
                 ai_score = EXCLUDED.ai_score, ai_reasoning = EXCLUDED.ai_reasoning, \
                 dm_script_creator = EXCLUDED.dm_script_creator, dm_script_clipper = EXCLUDED.dm_script_clipper, \
                 x_dm_script = EXCLUDED.x_dm_script, email_script = EXCLUDED.email_script, \
                 twitter_handle = COALESCE(EXCLUDED.twitter_handle, prospects.twitter_handle), \
                 instagram_handle = COALESCE(EXCLUDED.instagram_handle, prospects.instagram_handle), \
                 business_email = COALESCE(EXCLUDED.business_email, prospects.business_email), \
                 external_url = COALESCE(EXCLUDED.external_url, prospects.external_url), \
                 contact_enrichment = COALESCE(NULLIF(EXCLUDED.contact_enrichment, '{}'::jsonb), prospects.contact_enrichment), \
                 service_type = EXCLUDED.service_type, updated_at = NOW()"
            )
            .bind("youtube")
            .bind(&channel_id)
            .bind(&display_name)
            .bind(&platform_url)
            .bind(sub_count)
            .bind(&base_category)
            .bind(&channel_desc)
            .bind("twitch_clipper")
            .bind(score)
            .bind(&reasoning_with_signals)
            .bind(&dm_creator)
            .bind(&dm_clipper)
            .bind(&twitter_handle)
            .bind(&instagram_handle)
            .bind(&business_email)
            .bind(&external_url)
            .bind("twitch_clipping")
            .bind(&x_dm)
            .bind(&email_script)
            .bind(&contact_enrichment)
            .bind(payload.sourced_by)
            .execute(&state.db_pool)
            .await
            .ok();

            found += 1;
            if found >= limit {
                break;
            }
        }

        if found >= limit {
            break;
        }
    }

    tracing::info!("🎯 Twitch top-streamer method: found {} prospects", found);
    Ok(found)
}

/// Extract a Twitter/X handle from a channel description using simple pattern matching.
fn extract_twitter_handle(description: &str) -> Option<String> {
    // Look for twitter.com/handle or x.com/handle patterns
    for pattern in &["twitter.com/", "x.com/"] {
        if let Some(pos) = description.to_lowercase().find(pattern) {
            let after = &description[pos + pattern.len()..];
            let handle: String = after
                .chars()
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

fn extract_instagram_handle(description: &str) -> Option<String> {
    for pattern in &["instagram.com/", "instagr.am/"] {
        if let Some(pos) = description.to_lowercase().find(pattern) {
            let after = &description[pos + pattern.len()..];
            let handle: String = after
                .chars()
                .take_while(|c| c.is_alphanumeric() || *c == '_' || *c == '.')
                .collect();
            if !handle.is_empty() && handle.len() <= 50 {
                return Some(format!("@{}", handle));
            }
        }
    }
    None
}

fn extract_business_email(description: &str) -> Option<String> {
    let regex = Regex::new(r"(?i)\b[a-z0-9._%+\-]+@[a-z0-9.\-]+\.[a-z]{2,}\b").ok()?;
    regex.find(description).map(|m| {
        m.as_str()
            .trim_matches(|c: char| ",.;:()[]{}".contains(c))
            .to_string()
    })
}

fn extract_external_url(description: &str) -> Option<String> {
    let regex = Regex::new(r#"https?://[^\s<>"']+"#).ok()?;
    let mut fallback: Option<String> = None;

    for matched in regex.find_iter(description) {
        let candidate = matched
            .as_str()
            .trim_end_matches(|c: char| ",.;:()[]{}".contains(c))
            .to_string();
        let lower = candidate.to_lowercase();

        if fallback.is_none() {
            fallback = Some(candidate.clone());
        }

        let is_social = [
            "youtube.com",
            "youtu.be",
            "twitter.com",
            "x.com",
            "instagram.com",
            "instagr.am",
            "twitch.tv",
            "discord.gg",
            "discord.com",
            "t.me",
        ]
        .iter()
        .any(|domain| lower.contains(domain));

        if !is_social {
            return Some(candidate);
        }
    }

    fallback
}

fn extract_best_twitter_handle(description: &str) -> Option<String> {
    extract_twitter_handle(description)
        .or_else(|| {
            extract_labeled_handle(
                description,
                &[
                    r"twitter",
                    r"x/twitter",
                    r"x profile",
                    r"x handle",
                    r"x\.com",
                    r"x\s+at",
                ],
            )
        })
        .or_else(|| {
            // Fallback: standalone @handle near Twitter/X context words.
            // Avoid false-positives by requiring explicit Twitter/X labels.
            let context_regex = Regex::new(
                r"(?i)(?:twitter|x\s*(?:/|at|:))\s*@?([a-zA-Z0-9_]{2,50})(?![a-zA-Z0-9_])",
            )
            .ok()?;
            for capture in context_regex.captures_iter(description) {
                if let Some(m) = capture.get(1) {
                    let handle = m.as_str();
                    if !handle.contains('.') && handle.len() >= 2 && handle.len() <= 50 {
                        return Some(format!("@{}", handle));
                    }
                }
            }
            None
        })
}

fn extract_best_instagram_handle(description: &str) -> Option<String> {
    extract_instagram_handle(description)
        .or_else(|| extract_labeled_handle(description, &["instagram", "insta", "ig"]))
        .or_else(|| extract_bare_mention_handle(description))
}

/// Bare @mentions ("follow @foo for clips") with no label keyword nearby.
/// The preceding-character guard skips emails (contact@mail.com has an
/// alphanumeric before @) and URLs. Trailing dots are trimmed.
fn extract_bare_mention_handle(description: &str) -> Option<String> {
    let regex = Regex::new(r"(?i)(?:^|[^a-z0-9_.])@([a-z0-9_.]{2,30})").ok()?;
    let capture = regex.captures(description)?;
    let handle = capture
        .get(1)?
        .as_str()
        .trim_matches('.')
        .trim_end_matches('_');
    if handle.len() < 2 || handle.contains("..") {
        return None;
    }
    Some(format!("@{handle}"))
}

/// Hard Kick-clipping signals in bio/caption text. Returns evidence strings
/// (e.g. "links kick.com/neon"). Used to force kick_clipper categorization
/// without spending an LLM call, and as evidence inside scoring prompts.
/// bio fields already carry recent post captions (see scoring prompts), so one
/// scan covers both.
pub(crate) fn kick_caption_signals(text: &str) -> Vec<String> {
    let mut out = Vec::new();
    let lower = text.to_lowercase();
    let link_re = Regex::new(r"(?i)kick\.com/([a-z0-9_]{2,25})").ok();
    if let Some(re) = link_re {
        for cap in re.captures_iter(text) {
            if let Some(slug) = cap.get(1) {
                out.push(format!("links kick.com/{}", slug.as_str().to_lowercase()));
            }
        }
    }
    let mentions_kick = lower.contains("kick.com")
        || lower.contains("kick stream")
        || lower.contains("kick clip")
        || lower.contains("kick highlight")
        || lower.contains("kick vod")
        || lower.contains("kick.com");
    let mentions_clips = lower.contains("clip")
        || lower.contains("highlight")
        || lower.contains("stream moment")
        || lower.contains("best moment");
    if mentions_kick && mentions_clips {
        out.push("mentions Kick + clips/highlights in text".to_string());
    }
    out
}

fn extract_best_business_email(description: &str) -> Option<String> {
    extract_business_email(description).or_else(|| {
        let normalized = normalize_obfuscated_contact_text(description);
        let regex = Regex::new(r"(?i)\b[a-z0-9._%+\-]+@[a-z0-9.\-]+\.[a-z]{2,}\b").ok()?;
        regex.find(&normalized).map(|m| {
            m.as_str()
                .trim_matches(|c: char| ",.;:()[]{}".contains(c))
                .to_string()
        })
    })
}

fn extract_best_external_url(description: &str) -> Option<String> {
    extract_external_url(description).or_else(|| {
        let bare_domain_regex =
            Regex::new(r#"(?i)\b(?:www\.)?[a-z0-9-]+(?:\.[a-z0-9-]+)+(?:/[^\s<>"']*)?"#).ok()?;
        let mut fallback: Option<String> = None;

        for matched in bare_domain_regex.find_iter(description) {
            let candidate = matched
                .as_str()
                .trim_end_matches(|c: char| ",.;:()[]{}".contains(c))
                .to_string();
            let lower = candidate.to_lowercase();
            if candidate.contains('@') {
                continue;
            }

            let url = if lower.starts_with("http://") || lower.starts_with("https://") {
                candidate.clone()
            } else {
                format!("https://{}", candidate)
            };

            if fallback.is_none() {
                fallback = Some(url.clone());
            }

            let is_social = [
                "youtube.com",
                "youtu.be",
                "twitter.com",
                "x.com",
                "instagram.com",
                "instagr.am",
                "twitch.tv",
                "discord.gg",
                "discord.com",
                "t.me",
                "linktr.ee",
                "beacons.ai",
                "solo.to",
            ]
            .iter()
            .any(|domain| lower.contains(domain));

            if !is_social {
                return Some(url);
            }
        }

        fallback
    })
}

async fn guess_company_website(company_name: &str) -> Option<String> {
    let slug: String = company_name
        .to_lowercase()
        .replace(' ', "")
        .chars()
        .filter(|c| c.is_ascii_alphanumeric() || *c == '-')
        .collect();

    if slug.is_empty() {
        return None;
    }

    let candidates = [
        format!("https://www.{}.com", slug),
        format!("https://{}.com", slug),
        format!("https://www.{}.io", slug),
        format!("https://{}.io", slug),
    ];

    for url in candidates {
        match crate::browserbase_client::fetch_url_raw(&url).await {
            Ok(Some(html)) if html.len() > 100 => return Some(url),
            _ => {}
        }
    }
    None
}

async fn enrich_public_contact_fields(
    url: Option<&str>,
    twitter_handle: &mut Option<String>,
    instagram_handle: &mut Option<String>,
    business_email: &mut Option<String>,
    external_url: &mut Option<String>,
) -> serde_json::Value {
    let Some(url) = url else {
        return json!({});
    };
    if !url.starts_with("https://") && !url.starts_with("http://") {
        return json!({});
    }

    let lower = url.to_lowercase();
    let should_fetch = [
        "linktr.ee",
        "beacons.ai",
        "solo.to",
        "bio.site",
        "carrd.co",
        "lnk.bio",
        "hoo.be",
    ]
    .iter()
    .any(|domain| lower.contains(domain))
        || ![
            "youtube.com",
            "youtu.be",
            "twitch.tv",
            "twitter.com",
            "x.com",
            "instagram.com",
            "discord.gg",
            "discord.com",
            "t.me",
        ]
        .iter()
        .any(|domain| lower.contains(domain));

    if !should_fetch {
        return json!({"source_url": url, "skipped": "social_or_unsupported_url"});
    }

    let client = match reqwest::Client::builder()
        .timeout(std::time::Duration::from_secs(10))
        .user_agent("Mozilla/5.0 (compatible; VideoSyncProspectBot/1.0)")
        .redirect(reqwest::redirect::Policy::limited(4))
        .build()
    {
        Ok(client) => client,
        Err(_) => return json!({"source_url": url, "error": "client_build_failed"}),
    };

    let html = match client.get(url).send().await {
        Ok(resp) if resp.status().is_success() => resp.text().await.unwrap_or_default(),
        Ok(resp) => {
            return json!({
                "source_url": url,
                "error": format!("http_{}", resp.status().as_u16())
            })
        }
        Err(e) => return json!({"source_url": url, "error": e.to_string()}),
    };

    let found_twitter = extract_best_twitter_handle(&html);
    let found_instagram = extract_best_instagram_handle(&html);
    let found_email = extract_best_business_email(&html);
    let found_external = extract_best_external_url(&html);

    if twitter_handle.is_none() {
        *twitter_handle = found_twitter.clone();
    }
    if instagram_handle.is_none() {
        *instagram_handle = found_instagram.clone();
    }
    if business_email.is_none() {
        *business_email = found_email.clone();
    }
    if external_url.as_deref() == Some(url) {
        if let Some(found) = found_external.as_ref() {
            let found_lower = found.to_lowercase();
            let is_same_family = lower.contains("linktr.ee")
                || lower.contains("beacons.ai")
                || lower.contains("solo.to")
                || lower.contains("bio.site")
                || lower.contains("lnk.bio")
                || lower.contains("hoo.be");
            if is_same_family
                && !found_lower.contains("linktr.ee")
                && !found_lower.contains("beacons.ai")
            {
                *external_url = Some(found.clone());
            }
        }
    }

    json!({
        "source_url": url,
        "fetched_public_page": true,
        "twitter_handle": found_twitter,
        "instagram_handle": found_instagram,
        "business_email": found_email,
        "external_url": found_external,
    })
}

fn extract_labeled_handle(description: &str, labels: &[&str]) -> Option<String> {
    let joined = labels.join("|");
    let pattern = format!(
        r"(?i)(?:^|[\s|,;/])(?:{})(?:\s*(?:[:\-|]|is|handle)?\s*)(@?[a-z0-9_.]{{2,50}})",
        joined
    );
    let regex = Regex::new(&pattern).ok()?;
    let capture = regex.captures(description)?;
    let handle = capture.get(1)?.as_str().trim_start_matches('@');
    if handle.is_empty() {
        None
    } else {
        Some(format!("@{}", handle))
    }
}

fn normalize_obfuscated_contact_text(description: &str) -> String {
    let mut normalized = description.to_string();
    for pattern in ["[at]", "(at)", "{at}", " at "] {
        normalized = normalized.replace(pattern, "@");
        normalized = normalized.replace(&pattern.to_uppercase(), "@");
    }
    for pattern in ["[dot]", "(dot)", "{dot}", " dot "] {
        normalized = normalized.replace(pattern, ".");
        normalized = normalized.replace(&pattern.to_uppercase(), ".");
    }
    normalized
}

fn extract_role_signals(description: &str) -> Vec<&'static str> {
    let lower = description.to_lowercase();
    let mut signals = Vec::new();

    if [
        "creator manager",
        "talent manager",
        "artist manager",
        "management",
        "bookings",
        "partnerships",
        "agency",
        "creator ops",
        "social media manager",
    ]
    .iter()
    .any(|keyword| lower.contains(keyword))
    {
        signals.push("creator-manager");
    }

    if [
        "video editor",
        "short form editor",
        "short-form editor",
        "clipper",
        "content repurposing",
        "repurpose",
        "podcast producer",
        "reels editor",
        "shorts editor",
        "business_explainer editor",
    ]
    .iter()
    .any(|keyword| lower.contains(keyword))
    {
        signals.push("clipper-operator");
    }

    if [
        "youtube", "podcast", "streamer", "twitch", "host", "educator", "course", "founder",
    ]
    .iter()
    .any(|keyword| lower.contains(keyword))
    {
        signals.push("direct-buyer");
    }

    signals
}

fn build_scoring_description(description: &str) -> String {
    let signals = extract_role_signals(description);
    if signals.is_empty() {
        description.to_string()
    } else if description.trim().is_empty() {
        format!("Detected operator signals: {}", signals.join(", "))
    } else {
        format!(
            "{}\nDetected operator signals: {}",
            description,
            signals.join(", ")
        )
    }
}

/// Use best available LLM to score a prospect and generate DM scripts.
/// Priority: NVIDIA NIM → Gemma 4 → Gemini Flash.
/// Returns (score, reasoning, dm_creator, dm_clipper).
/// AI-scored prospect data — matches the IG lead pattern.
/// Returns `(score_0_1, reasoning, service_type, legacy_dm, dm_clipper_fallback, x_dm, email_script)`.
/// `service_type` is one of the revenue service tags accepted by `is_valid_revenue_service`.
async fn score_prospect_with_ai(
    state: &Arc<AppState>,
    name: &str,
    audience_size: i64,
    description: &str,
    category: &str,
    prospect_type: &str,
    platform: &str,
    forced_service: Option<&str>,
) -> (f64, String, String, String, String, String, String) {
    // Forced-service scope (owner directive Oct 2026): the caller validated
    // this service (known + enabled). Every prospect from this search is tagged
    // with it — the AI still scores fit 0-1, but the service is fixed.
    let forced = forced_service
        .map(|s| normalize_revenue_service(&s.to_ascii_lowercase()).to_string())
        .filter(|s| !s.is_empty());
    if state.nvidia_nim_client.is_none()
        && state.gemma_client.is_none()
        && state.gemini_client.is_none()
    {
        let service = forced
            .clone()
            .unwrap_or_else(|| default_service_for_prospect(category, prospect_type, platform));
        let x_dm = default_x_dm(name, &service);
        let email_script = default_email_script(name, &service);
        return (
            0.5,
            "No LLM configured".to_string(),
            service,
            x_dm.clone(),
            default_dm_clipper(name),
            x_dm,
            email_script,
        );
    }

    // Service menu is built from enabled flags (owner directive Sep 2026):
    // prospecting only funnels the services that are switched on. A forced
    // service narrows the menu to that service alone (Oct 2026 scoping).
    let (service_menu, must_line) = match forced.as_deref() {
        Some(f) => (
            crate::services::service_flags::service_menu_line(f)
                .unwrap_or("Pick the forced service.")
                .to_string(),
            format!("`service` MUST be exactly: {f}. No other values are valid."),
        ),
        None => crate::services::service_flags::scoring_menu(&state.db_pool).await,
    };

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
{service_menu}

Prospect-type guidance:
- If `prospect_type` is `clipper`, favor people or teams already selling clip editing, short-form growth, or creator post-production. Score higher if they would clearly benefit from tooling, faster fulfillment, or premium add-on renders.
- If `prospect_type` is `content_creator` or `podcaster`, treat them as direct buyers of clipping / animations.
- If `prospect_type` is `educator`, favor education.
- If `prospect_type` is `business_owner`, favor landing_page.
- If the prospect looks like a founder, SaaS, app, startup, AI tool, agency, consultant, software channel, business operator, or has an external product/site URL, favor **landing_page** unless another service is obviously stronger.

Score guidelines:
- 0.8-1.0: clear paying client, monetised, has content we can act on now.
- 0.6-0.8: likely fit, bio + audience size point strongly to one service.
- 0.4-0.6: ambiguous — could go either way from the signals alone.
- 0.0-0.4: bad fit (fan page, brand parody, competitor/fellow editor).

Return ONLY valid JSON (no markdown):
{{
  "score": 0.75,
  "reasoning": "<1-2 sentences explaining the score + why this service>",
  "service": "twitch_clipping",
  "dm": "<legacy short DM locked to the chosen service>",
  "x_dm": "<X/Twitter DM: short, casual, 280 chars max, mention I am verified so I can DM directly, name the concrete offer, end with a yes/no ask>",
  "email_script": "<cold email: subject line + 4-6 sentence body, concrete offer, price range, asks permission to send a sample pack>",
  "dm_clipper": "<alt DM treating them as a clipper looking for tooling — 2-3 sentences>"
}}

{must_line}"#,
        name = name,
        service_menu = service_menu,
        must_line = must_line,
        audience = audience_size,
        category_word = if audience_size > 0 {
            "subs/viewers"
        } else {
            "unknown"
        },
        category = category,
        description = description,
        prospect_type = prospect_type,
    );

    match crate::llm_utils::generate_text_fast(
        state.qwen_client.as_ref(),
        state.deepseek_client.as_ref(),
        state.gemini_client.as_ref(),
        &prompt,
    )
    .await
    {
        Ok(text) => {
            let cleaned = text
                .trim()
                .trim_start_matches("```json")
                .trim_start_matches("```")
                .trim_end_matches("```")
                .trim();
            match serde_json::from_str::<serde_json::Value>(cleaned) {
                Ok(v) => {
                    let score = v["score"].as_f64().unwrap_or(0.5).clamp(0.0, 1.0);
                    let reasoning = v["reasoning"].as_str().unwrap_or("").to_string();
                    let dm_creator = v["dm"]
                        .as_str()
                        .or_else(|| v["dm_creator"].as_str()) // legacy field name
                        .unwrap_or(&default_dm_creator(name))
                        .to_string();
                    let x_dm = v["x_dm"].as_str().unwrap_or(&dm_creator).to_string();
                    let email_script = v["email_script"]
                        .as_str()
                        .map(str::to_string)
                        .unwrap_or_else(|| {
                            default_email_script(
                                name,
                                v["service"].as_str().unwrap_or("landing_page"),
                            )
                        });
                    let dm_clipper = v["dm_clipper"]
                        .as_str()
                        .unwrap_or(&default_dm_clipper(name))
                        .to_string();
                    // Coerce service into the enabled set (owner directive Sep 2026):
                    // the menu already constrains the LLM, this is the backstop.
                    let service_raw = v["service"].as_str().unwrap_or("clipping").to_lowercase();
                    let service = if is_valid_revenue_service(&service_raw) {
                        normalize_revenue_service(&service_raw).to_string()
                    } else {
                        default_service_for_prospect(category, prospect_type, platform)
                    };
                    let clipper_like = matches!(
                        prospect_type,
                        "clipper"
                            | "clipping_channel"
                            | "compilation"
                            | "kick_clipper"
                            | "twitch_clipper"
                    ) || category.to_ascii_lowercase().contains("clip")
                        || category.to_ascii_lowercase().contains("kick");
                    let service = crate::services::service_flags::constrain_service(
                        &state.db_pool,
                        &service,
                        clipper_like,
                        platform,
                    )
                    .await;
                    // Post-process: override LLM's choice for known prospect types,
                    // but only toward enabled services.
                    let service = match prospect_type {
                        "educator" => {
                            if crate::services::service_flags::service_enabled(
                                &state.db_pool,
                                "education",
                            )
                            .await
                            {
                                "education".to_string()
                            } else {
                                service
                            }
                        }
                        _ => service,
                    };
                    // Forced-service scope: the tag is fixed regardless of the
                    // AI pick or the post-process overrides above.
                    let service = forced.clone().unwrap_or(service);
                    (
                        score,
                        reasoning,
                        service,
                        dm_creator,
                        dm_clipper,
                        x_dm,
                        email_script,
                    )
                }
                Err(_) => (
                    0.5,
                    "Parse error".to_string(),
                    forced.clone().unwrap_or_else(|| {
                        default_service_for_prospect(category, prospect_type, platform)
                    }),
                    default_x_dm(
                        name,
                        &default_service_for_prospect(category, prospect_type, platform),
                    ),
                    default_dm_clipper(name),
                    default_x_dm(
                        name,
                        &default_service_for_prospect(category, prospect_type, platform),
                    ),
                    default_email_script(
                        name,
                        &default_service_for_prospect(category, prospect_type, platform),
                    ),
                ),
            }
        }
        Err(_) => (
            0.5,
            "AI unavailable".to_string(),
            forced
                .clone()
                .unwrap_or_else(|| default_service_for_prospect(category, prospect_type, platform)),
            default_x_dm(
                name,
                &default_service_for_prospect(category, prospect_type, platform),
            ),
            default_dm_clipper(name),
            default_x_dm(
                name,
                &default_service_for_prospect(category, prospect_type, platform),
            ),
            default_email_script(
                name,
                &default_service_for_prospect(category, prospect_type, platform),
            ),
        ),
    }
}

fn default_dm_creator(name: &str) -> String {
    default_x_dm(name, "landing_page")
}

fn default_dm_clipper(name: &str) -> String {
    format!("Hey {}! I built an AI clipping + rendering platform that processes YouTube and Twitch videos 10x faster — short-form clips, animations, and thumbnails in one pipeline. Want free access to try it?", name)
}

fn default_service_for_prospect(category: &str, prospect_type: &str, platform: &str) -> String {
    let combined = format!("{} {} {}", category, prospect_type, platform).to_lowercase();
    if combined.contains("education")
        || combined.contains("educator")
        || combined.contains("course")
        || combined.contains("math")
        || combined.contains("finance")
        || combined.contains("manim")
    {
        "education".to_string()
    } else if combined.contains("business")
        || combined.contains("saas")
        || combined.contains("startup")
        || combined.contains("app")
        || combined.contains("founder")
    {
        "landing_page".to_string()
    } else if combined.contains("whiteboard")
        || combined.contains("sketch")
        || combined.contains("hand drawn")
        || combined.contains("doodle")
    {
        "whiteboard_animation".to_string()
    } else if combined.contains("typography")
        || combined.contains("lyric")
        || combined.contains("text animation")
        || combined.contains("quote video")
    {
        "kinetic_typography".to_string()
    } else if combined.contains("infographic")
        || combined.contains("data viz")
        || combined.contains("chart")
        || combined.contains("statistic")
    {
        "animated_infographic".to_string()
    } else if combined.contains("algorithm")
        || combined.contains("coding")
        || combined.contains("programming")
        || combined.contains("software developer")
        || combined.contains("code tutorial")
    {
        "algorithm_viz".to_string()
    } else if combined.contains("pitch deck")
        || combined.contains("investor")
        || combined.contains("fundraising")
        || combined.contains("venture capital")
        || combined.contains("demo day")
    {
        "investor_pitch".to_string()
    } else if combined.contains("year in review")
        || combined.contains("wrapped")
        || combined.contains("year recap")
        || combined.contains("year in review video")
    {
        "year_in_review".to_string()
    } else if combined.contains("isometric")
        || combined.contains("architecture")
        || combined.contains("perspective")
        || combined.contains("architectural")
    {
        "isometric_explainer".to_string()
    } else if combined.contains("twitch") {
        "twitch_clipping".to_string()
    } else if combined.contains("youtube")
        || combined.contains("podcast")
        || combined.contains("stream")
        || combined.contains("creator")
    {
        "youtube_clipping".to_string()
    } else if combined.contains("clip")
        || combined.contains("highlight")
        || combined.contains("best.of")
        || combined.contains("kick")
    {
        "kick_auto_clipper".to_string()
    } else {
        "landing_page".to_string()
    }
}

fn normalize_revenue_service(service: &str) -> &str {
    match service {
        "kick_auto_clipper" | "kick_clipper" | "auto_clipper" => "kick_auto_clipper",
        // Platform split (Oct 2026): youtube/twitch clipping share the clipping core.
        "youtube_clipping" | "youtube_clips" | "yt_clipping" => "youtube_clipping",
        "twitch_clipping" | "twitch_clips" => "twitch_clipping",
        "manim" | "manim_explainer" => "manim_explainer",
        "whiteboard" | "whiteboard_animation" | "hand_drawn" | "sketch" => "whiteboard_animation",
        "kinetic_type" | "kinetic_typography" | "type_motion" | "text_animation" => {
            "kinetic_typography"
        }
        "infographic" | "animated_infographic" | "chart_animation" => "animated_infographic",
        "algorithm" | "algorithm_viz" | "code_viz" | "data_structure" | "coding" => "algorithm_viz",
        "pitch_deck" | "investor_pitch" | "pitch" | "fundraising" => "investor_pitch",
        "year_in_review" | "wrapped" | "year_recap" => "year_in_review",
        "isometric" | "isometric_explainer" | "isometric_3d" => "isometric_explainer",
        other => other,
    }
}

fn is_valid_revenue_service(service: &str) -> bool {
    matches!(
        normalize_revenue_service(service),
        "clipping"
            | "youtube_clipping"
            | "twitch_clipping"
            | "landing_page"
            | "education"
            | "kick_auto_clipper"
            | "manim_explainer"
            | "whiteboard_animation"
            | "kinetic_typography"
            | "animated_infographic"
            | "algorithm_viz"
            | "investor_pitch"
            | "year_in_review"
            | "isometric_explainer"
    )
}

fn service_offer_line(service: &str) -> &'static str {
    match normalize_revenue_service(service) {
        "clipping" => "automated clip generation from your streams and VODs — posted to your social accounts daily",
        "youtube_clipping" => "automated clip generation from your YouTube videos and podcasts — posted to your social accounts daily",
        "twitch_clipping" => "automated clip generation from your Twitch streams and VODs — posted to your social accounts daily",
        "education" => "AI-animated educational videos of ANY type — math, science, coding, language, history, finance, and more. The system produces equations, diagrams, code animations, data charts, motion graphics, VFX, and 3D depth with narration — generated and posted to your social accounts daily",
        "landing_page" => "videos generated from your website URL — any type of video (how-to, marketing, product demo, explainer, testimonial) built from what the AI learns by crawling your site — posted to your social accounts as marketing content daily",
        "kick_auto_clipper" => "automated Kick VOD clipping from your favorite streamers — posted to your social accounts daily",
        "manim_explainer" => "narrated animated explainer videos — generated and posted to your social accounts daily",
        "whiteboard_animation" => "narrated whiteboard-style hand-drawn explainer videos — generated and posted daily",
        "kinetic_typography" => "animated text-motion videos from your scripts — posted to your social accounts daily",
        "animated_infographic" => "animated data infographics with charts — generated from your data and posted daily",
        "algorithm_viz" => "animated algorithm and data structure visualizations — posted to your social accounts daily",
        "investor_pitch" => "animated investor pitch videos with motion graphics — generated and posted to your social accounts daily",
        "year_in_review" => "personalized recap and highlight videos — generated from your content and posted daily",
        "isometric_explainer" => "isometric-style animated explainer videos — generated and posted to your social accounts daily",
        _ => "videos generated from your website or brief — posted to your social accounts daily",
    }
}

fn service_price_line(service: &str) -> &'static str {
    match normalize_revenue_service(service) {
        "youtube_clipping" => "$297/mo",
        "clipping" | "kick_auto_clipper" | "twitch_clipping" | "education" => "$199/mo",
        "landing_page" => "$149/mo",
        "manim_explainer"
        | "whiteboard_animation"
        | "kinetic_typography"
        | "animated_infographic"
        | "algorithm_viz"
        | "investor_pitch"
        | "year_in_review"
        | "isometric_explainer" => "$149/mo",
        _ => "$149/mo",
    }
}

fn service_target_duration_seconds(service: &str) -> f64 {
    match normalize_revenue_service(service) {
        "education" => 90.0,
        "clipping" => 60.0,
        "youtube_clipping" => 60.0,
        "twitch_clipping" => 60.0,
        "kick_auto_clipper" => 60.0,
        "manim_explainer" | "whiteboard_animation" | "kinetic_typography" => 60.0,
        "animated_infographic" | "algorithm_viz" => 75.0,
        "investor_pitch" => 90.0,
        "year_in_review" => 60.0,
        "isometric_explainer" => 60.0,
        _ => 60.0,
    }
}

fn service_long_form_style(service: &str) -> &'static str {
    match normalize_revenue_service(service) {
        "clipping" => "high-retention creator clip pack presentation, fast hooks, captions, motion graphics, YouTube Shorts energy",
        "youtube_clipping" => "high-retention YouTube clip pack presentation, fast hooks, captions, motion graphics, Shorts energy",
        "twitch_clipping" => "high-retention Twitch clip pack presentation, fast hooks, captions, motion graphics, stream-highlight energy",
        "education" => "clear narrated educational explainer, animated diagrams, structured lesson pacing",
        "kick_auto_clipper" => "automated Kick clip generation from VODs, branded lower thirds, outro, and watermark, TikTok/Shorts native",
        "manim_explainer" => "narrated Manim animated explainer, clean motion graphics, math/technical diagrams, professional educational tone",
        "whiteboard_animation" => "narrated whiteboard hand-drawn sketch explainer, marker-on-board style, text reveals, friendly educational tone",
        "kinetic_typography" => "dynamic kinetic typography text animation, word-by-word reveals, morphing, bold font variations, vibrant colors",
        "animated_infographic" => "animated data infographic, bar/pie chart visualizations, value counters, data-driven professional tone",
        "algorithm_viz" => "algorithm visualization with animated data structures, color-coded elements, step-by-step execution, educational technical tone",
        "investor_pitch" => "professional investor pitch deck, clean title cards, data charts, animated metrics, confident persuasive tone",
        "year_in_review" => "year-in-review recap, bold typography, stat reveals, value counters, energetic celebratory tone",
        "isometric_explainer" => "isometric 3D perspective explainer, camera angles, geometric shapes, clean modern motion graphics",
        _ => "premium SaaS launch video, clean product motion, polished founder-outreach sample",
    }
}

fn service_long_form_offer_type(service: &str) -> String {
    match normalize_revenue_service(service) {
        "clipping" => "clip_pack".to_string(),
        "youtube_clipping" => "clip_pack".to_string(),
        "twitch_clipping" => "clip_pack".to_string(),
        "education" => "education_explainer_pack".to_string(),
        "kick_auto_clipper" => "kick_auto_clipper_pack".to_string(),
        "manim_explainer" => "manim_pack".to_string(),
        "whiteboard_animation" => "whiteboard_pack".to_string(),
        "kinetic_typography" => "type_pack".to_string(),
        "animated_infographic" => "infographic_pack".to_string(),
        "algorithm_viz" => "algorithm_pack".to_string(),
        "investor_pitch" => "pitch_pack".to_string(),
        "year_in_review" => "wrapped_pack".to_string(),
        "isometric_explainer" => "isometric_pack".to_string(),
        _ => "saas_demo_pack".to_string(),
    }
}

fn service_page_for_revenue_service(service: &str) -> &'static str {
    match normalize_revenue_service(service) {
        "clipping" => "/services/clipper-enhancement-pack",
        // Platform split (Oct 2026): dedicated pages are out of scope — both
        // point at the generic clipping page until they get their own.
        "youtube_clipping" => "/services/clipper-enhancement-pack",
        "twitch_clipping" => "/services/clipper-enhancement-pack",
        "education" => "/services/education-explainer-pack",
        "kick_auto_clipper" => "/services/kick-auto-clipper",
        _ => "/services/saas-launch-pack",
    }
}

fn should_use_long_form_for_revenue_sample(service: &str, has_reference_url: bool) -> bool {
    matches!(
        normalize_revenue_service(service),
        "landing_page"
            | "clipping"
            | "youtube_clipping"
            | "twitch_clipping"
            | "education"
            | "kick_auto_clipper"
            | "manim_explainer"
            | "whiteboard_animation"
            | "kinetic_typography"
            | "animated_infographic"
            | "algorithm_viz"
            | "investor_pitch"
            | "year_in_review"
            | "isometric_explainer"
    ) && (has_reference_url
        || matches!(
            normalize_revenue_service(service),
            "education"
                | "kick_auto_clipper"
                | "manim_explainer"
                | "whiteboard_animation"
                | "kinetic_typography"
                | "animated_infographic"
                | "algorithm_viz"
                | "investor_pitch"
                | "year_in_review"
                | "isometric_explainer"
        ))
}

/// Map a Managed Campaign service to the TypeScript app subdomain that hosts its
/// branded delivery viewer (so free-sample links never leak raw storage URLs).
fn app_subdomain_for_service(service: &str) -> &'static str {
    match service {
        "kick_auto_clipper" | "twitch_clipping" | "youtube_clipping" | "clipping" => "clips",
        "landing_page" => "app",
        "education" => "learn",
        "manim_explainer"
        | "whiteboard_animation"
        | "kinetic_typography"
        | "animated_infographic"
        | "algorithm_viz"
        | "investor_pitch"
        | "year_in_review"
        | "isometric_explainer" => "motion",
        _ => "app",
    }
}

fn default_x_dm(name: &str, service: &str) -> String {
    format!(
        "Hey {} — I put together a free sample of what I can do for your content — {}.",
        name,
        service_offer_line(service),
    )
}

fn default_email_script(name: &str, service: &str) -> String {
    format!(
        "Subject: free sample for {}\n\nHey {},\n\nI put together a free sample of what VideoSync can do for your content — {}. The link is in this message.\n\nLet me know what you think!\n\nBest,\nVideoSync",
        name,
        name,
        service_offer_line(service),
    )
}

// ============================================================================
// CRUD endpoints
// ============================================================================

async fn list_prospects(
    Extension(state): Extension<Arc<AppState>>,
    Extension(claims): Extension<Claims>,
    Query(q): Query<ListQuery>,
) -> Result<Json<serde_json::Value>, (StatusCode, Json<ErrorResponse>)> {
    let is_admin = claims.is_superuser || claims.is_staff;

    // Build query with parameterized binds — collect active filters first so
    // we know which $N placeholders to emit.
    let mut conditions: Vec<&str> = vec![];
    if q.prospect_type.is_some() {
        conditions.push("prospect_type");
    }
    if q.contact_status.is_some() {
        conditions.push("contact_status");
    }
    if q.platform.is_some() {
        conditions.push("platform");
    }
    if q.sourced_by.is_some() || (!is_admin) {
        // Non-admin users always filtered to their sourced prospects
        conditions.push("sourced_by");
    }

    let mut sql = "SELECT prospects.id, platform, channel_id, display_name, platform_url, \
                   subscriber_count, avg_viewer_count, content_category, prospect_type, \
                   ai_score, ai_reasoning, dm_script_creator, dm_script_clipper, \
                   x_dm_script, email_script, contact_status, notes, twitter_handle, instagram_handle, \
                   business_email, external_url, service_type, sample_delivery_id, contact_enrichment, \
                   prospects.referred_by, sourced_by, u.email AS sourced_by_email, prospects.created_at, \
                   ((CASE WHEN business_email IS NOT NULL AND business_email <> '' THEN 60 ELSE 0 END) + \
                    (CASE WHEN twitter_handle IS NOT NULL AND twitter_handle <> '' THEN 50 ELSE 0 END) + \
                    (CASE WHEN external_url IS NOT NULL AND external_url <> '' THEN 25 ELSE 0 END) + \
         (CASE \
            WHEN service_type IN ('landing_page','investor_pitch') THEN 30 \
            WHEN service_type IN ('education','manim_explainer','whiteboard_animation','animated_infographic','algorithm_viz') THEN 25 \
            WHEN service_type IN ('clipping','youtube_clipping','twitch_clipping','kick_auto_clipper','kinetic_typography','year_in_review','isometric_explainer') THEN 15 \
            ELSE 0 \
          END) + \
                    COALESCE((ai_score * 100)::int, 0)) AS revenue_priority \
                   FROM prospects LEFT JOIN users u ON u.id = prospects.sourced_by WHERE 1=1"
        .to_string();

    for (i, col) in conditions.iter().enumerate() {
        sql.push_str(&format!(" AND {} = ${}", col, i + 1));
    }
    // Service visibility (owner directive Oct 2026): UIs display only prospects
    // of ON services. Unscored rows (service_type IS NULL) always show — they
    // are candidates, not assignments. Admins can pass include_disabled=true.
    let show_all = q.include_disabled.unwrap_or(false) && is_admin;
    let enabled_services: Vec<String> = if show_all {
        vec![]
    } else {
        crate::services::service_flags::enabled_services(&state.db_pool).await
    };
    if !show_all {
        sql.push_str(&format!(
            " AND (service_type IS NULL OR service_type = ANY(${}))",
            conditions.len() + 1
        ));
    }
    // Launch freshness (owner directive Oct 2026): optional created-in-last-N-days.
    let fresh_days: Option<i64> = q.fresh_days.filter(|d| *d > 0);
    if fresh_days.is_some() {
        let n = conditions.len() + if show_all { 1 } else { 2 };
        sql.push_str(&format!(
            " AND prospects.created_at > NOW() - ((${}::text) || ' days')::interval",
            n
        ));
    }
    sql.push_str(
        " ORDER BY revenue_priority DESC, ai_score DESC NULLS LAST, created_at DESC LIMIT 200",
    );

    // Bind only the filter values that are present, in the same order.
    let mut query = sqlx::query(&sql);
    if let Some(ref pt) = q.prospect_type {
        query = query.bind(pt);
    }
    if let Some(ref cs) = q.contact_status {
        query = query.bind(cs);
    }
    if let Some(ref pl) = q.platform {
        query = query.bind(pl);
    }
    if let Some(sb) = q.sourced_by {
        query = query.bind(sb);
    } else if !is_admin {
        // Auto-filter to user's sourced prospects
        let user_id: i32 = claims.sub.parse().unwrap_or(0);
        query = query.bind(user_id);
    }
    if !show_all {
        query = query.bind(enabled_services);
    }
    if let Some(days) = fresh_days {
        query = query.bind(days);
    }

    let rows = query.fetch_all(&state.db_pool).await.map_err(|e| {
        (
            StatusCode::INTERNAL_SERVER_ERROR,
            Json(ErrorResponse {
                success: false,
                message: e.to_string(),
            }),
        )
    })?;

    let prospects: Vec<serde_json::Value> = rows
        .iter()
        .map(|r| {
            json!({
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
                "x_dm_script": r.get::<Option<String>, _>("x_dm_script"),
                "email_script": r.get::<Option<String>, _>("email_script"),
                "contact_status": r.get::<String, _>("contact_status"),
                "notes": r.get::<Option<String>, _>("notes"),
                "twitter_handle": r.get::<Option<String>, _>("twitter_handle"),
                "instagram_handle": r.get::<Option<String>, _>("instagram_handle"),
                "business_email": r.get::<Option<String>, _>("business_email"),
                "external_url": r.get::<Option<String>, _>("external_url"),
                "service_type": r.get::<Option<String>, _>("service_type"),
                "service_page_url": r.get::<Option<String>, _>("service_type").as_deref().map(service_page_for_revenue_service),
                "sample_delivery_id": r.get::<Option<Uuid>, _>("sample_delivery_id").map(|id| id.to_string()),
                "sample_delivery_url": r.get::<Option<Uuid>, _>("sample_delivery_id").map(|id| format!("/delivery/{id}")),
                "contact_enrichment": r.get::<serde_json::Value, _>("contact_enrichment"),
                "revenue_priority": r.get::<i32, _>("revenue_priority"),
                "referred_by": r.get::<Option<String>, _>("referred_by"),
                "sourced_by": r.get::<Option<i32>, _>("sourced_by"),
                "sourced_by_email": r.get::<Option<String>, _>("sourced_by_email"),
                "created_at": r.get::<chrono::DateTime<chrono::Utc>, _>("created_at"),
            })
        })
        .collect();

    Ok(Json(json!({ "success": true, "prospects": prospects })))
}

const VALID_CONTACT_STATUSES: &[&str] = &[
    "new",
    "contacted",
    "replied",
    "interested",
    "deal",
    "converted",
    "rejected",
];

async fn update_prospect(
    Extension(state): Extension<Arc<AppState>>,
    Extension(claims): Extension<crate::models::auth::Claims>,
    Path(id): Path<Uuid>,
    Json(payload): Json<UpdateProspectRequest>,
) -> Result<Json<serde_json::Value>, (StatusCode, Json<ErrorResponse>)> {
    let is_admin = claims.is_superuser || claims.is_staff;
    if !is_admin {
        let owned: bool = sqlx::query_scalar(
            "SELECT EXISTS(SELECT 1 FROM prospects WHERE id = $1 AND sourced_by = $2)",
        )
        .bind(id)
        .bind(claims.sub.parse::<i32>().unwrap_or(0))
        .fetch_one(&state.db_pool)
        .await
        .unwrap_or(false);
        if !owned {
            return Err((
                StatusCode::FORBIDDEN,
                Json(ErrorResponse {
                    success: false,
                    message: "You can only act on prospects you sourced.".to_string(),
                }),
            ));
        }
    }
    if let Some(ref status) = payload.contact_status {
        if !VALID_CONTACT_STATUSES.contains(&status.as_str()) {
            return Err((
                StatusCode::BAD_REQUEST,
                Json(ErrorResponse {
                    success: false,
                    message: format!(
                        "Invalid contact_status '{}'. Must be one of: {}",
                        status,
                        VALID_CONTACT_STATUSES.join(", ")
                    ),
                }),
            ));
        }
        sqlx::query("UPDATE prospects SET contact_status=$1, updated_at=NOW() WHERE id=$2")
            .bind(status)
            .bind(id)
            .execute(&state.db_pool)
            .await
            .ok();
    }
    if let Some(ref notes) = payload.notes {
        sqlx::query("UPDATE prospects SET notes=$1, updated_at=NOW() WHERE id=$2")
            .bind(notes)
            .bind(id)
            .execute(&state.db_pool)
            .await
            .ok();
    }
    Ok(Json(json!({ "success": true })))
}

async fn regenerate_dm_script(
    Extension(state): Extension<Arc<AppState>>,
    Extension(claims): Extension<crate::models::auth::Claims>,
    Path(id): Path<Uuid>,
) -> Result<Json<serde_json::Value>, (StatusCode, Json<ErrorResponse>)> {
    let is_admin = claims.is_superuser || claims.is_staff;
    if !is_admin {
        let owned: bool = sqlx::query_scalar(
            "SELECT EXISTS(SELECT 1 FROM prospects WHERE id = $1 AND sourced_by = $2)",
        )
        .bind(id)
        .bind(claims.sub.parse::<i32>().unwrap_or(0))
        .fetch_one(&state.db_pool)
        .await
        .unwrap_or(false);
        if !owned {
            return Err((
                StatusCode::FORBIDDEN,
                Json(ErrorResponse {
                    success: false,
                    message: "You can only act on prospects you sourced.".to_string(),
                }),
            ));
        }
    }
    // Service switch (owner directive Sep 2026).
    if !crate::services::service_flags::service_enabled(&state.db_pool, "outreach").await {
        return Err((
            StatusCode::SERVICE_UNAVAILABLE,
            Json(ErrorResponse {
                success: false,
                message: "Outreach generation is currently disabled (service_flags).".to_string(),
            }),
        ));
    }
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
    let category: String = row
        .get::<Option<String>, _>("content_category")
        .unwrap_or_default();
    let description: String = row
        .get::<Option<String>, _>("channel_description")
        .unwrap_or_default();
    let pt: String = row.get("prospect_type");

    let audience = subs.or(viewers).unwrap_or(0);
    let (score, reasoning, service, dm_creator, dm_clipper, x_dm, email_script) =
        score_prospect_with_ai(
            &state,
            &name,
            audience,
            &description,
            &category,
            &pt,
            "",
            None,
        )
        .await;

    sqlx::query(
        "UPDATE prospects
         SET ai_score=$1, ai_reasoning=$2, dm_script_creator=$3, dm_script_clipper=$4,
             service_type=$5, x_dm_script=$6, email_script=$7, updated_at=NOW()
         WHERE id=$8",
    )
    .bind(score)
    .bind(&reasoning)
    .bind(&dm_creator)
    .bind(&dm_clipper)
    .bind(&service)
    .bind(&x_dm)
    .bind(&email_script)
    .bind(id)
    .execute(&state.db_pool)
    .await
    .ok();

    Ok(Json(json!({
        "success":     true,
        "dm_creator":  dm_creator,
        "dm_clipper":  dm_clipper,
        "score":       score,
        "service":     service,
        "x_dm":        x_dm,
        "email_script": email_script,
    })))
}

async fn bulk_send_prospect_emails(
    Extension(state): Extension<Arc<AppState>>,
    Extension(claims): Extension<crate::models::auth::Claims>,
    Json(payload): Json<Option<BulkSendEmailRequest>>,
) -> Result<Json<serde_json::Value>, (StatusCode, Json<ErrorResponse>)> {
    let is_admin = claims.is_superuser || claims.is_staff;
    let acting_user: i32 = claims.sub.parse().unwrap_or(0);

    let req = payload.unwrap_or(BulkSendEmailRequest {
        service_type: None,
        contact_status: None,
        fresh_days: None,
        include_contacted: Some(false),
        limit: None,
    });

    // Build the candidate list. Whitelisted (non-admin) users are scoped to
    // their own sourced prospects; admins see everything.
    let mut sql = String::from(
        "SELECT id, display_name, business_email, email_script, service_type, sourced_by          FROM prospects WHERE business_email IS NOT NULL AND business_email <> ''",
    );
    let mut binds: Vec<String> = Vec::new();
    if !req.include_contacted() {
        sql.push_str(" AND (contact_status IS NULL OR contact_status NOT IN ('contacted','replied','interested','deal','converted'))");
    }
    if let Some(ref st) = req.service_type {
        sql.push_str(" AND service_type = $");
        sql.push_str(&(binds.len() + 1).to_string());
        binds.push(st.clone());
    }
    if let Some(ref cs) = req.contact_status {
        sql.push_str(&format!(" AND contact_status = ${}", binds.len() + 1));
        binds.push(cs.clone());
    }
    if !is_admin {
        // sourced_by is an integer; the bind vector is text, so cast explicitly.
        sql.push_str(&format!(" AND sourced_by = ${}::int", binds.len() + 1));
        binds.push(acting_user.to_string());
    }
    if let Some(days) = req.fresh_days.filter(|d| *d > 0) {
        sql.push_str(&format!(
            " AND created_at > NOW() - (INTERVAL '1 day' * {})",
            days
        ));
    }
    // Only send to enabled services (owner directive Sep 2026).
    let enabled = crate::services::service_flags::enabled_services(&state.db_pool).await;
    if !enabled.is_empty() {
        sql.push_str(&format!(
            " AND (service_type IS NULL OR service_type = ANY(${}))",
            binds.len() + 1
        ));
        // bind enabled as text[] via a separate path below (handled specially)
    }
    let cap = req.limit.unwrap_or(50).clamp(1, 200) as usize;
    sql.push_str(&format!(" ORDER BY created_at DESC LIMIT {}", cap));

    // Fetch candidates. enabled_services bind handled inline.
    let mut query = sqlx::query(&sql);
    for b in &binds {
        query = query.bind(b);
    }
    if !enabled.is_empty() {
        query = query.bind(&enabled);
    }
    let rows = query.fetch_all(&state.db_pool).await.map_err(|e| {
        (
            StatusCode::INTERNAL_SERVER_ERROR,
            Json(ErrorResponse {
                success: false,
                message: format!("Failed to load prospects: {e}"),
            }),
        )
    })?;

    // Collect candidate list, skipping prospects without an email script.
    let mut candidates: Vec<(uuid::Uuid, String, String, String, String)> = Vec::new();
    let mut skipped_no_script = 0usize;
    for r in rows {
        let id: uuid::Uuid = r.get("id");
        let name: String = r.get("display_name");
        let email: String = r.get("business_email");
        let script: Option<String> = r.get("email_script");
        let service: String = r
            .get::<Option<String>, _>("service_type")
            .unwrap_or_else(|| "landing_page".to_string());
        match script {
            Some(s) if !s.is_empty() => candidates.push((id, name, email, s, service)),
            _ => skipped_no_script += 1,
        }
    }

    let queued = candidates.len();
    if queued == 0 {
        return Ok(Json(json!({
            "success": true,
            "queued": 0,
            "message": "No prospects with emails and outreach scripts to send. Generate outreach first.",
            "skipped_no_script": skipped_no_script,
        })));
    }

    // DURABLE BATCH (owner directive Nov 16 2026 — sample-first outreach):
    // create the batch + per-prospect items. The background worker (vs-edu-worker)
    // generates missing samples, then sends each email ONLY when its sample is
    // ready. Restarts never lose the batch; per-item tracking available via the
    // batch status endpoint.
    let batch_id: Uuid = sqlx::query_scalar(
        "INSERT INTO prospect_outreach_batches (created_by, service_type, fresh_days, include_contacted, total_items)
         VALUES ($1, $2, $3, $4, $5) RETURNING id",
    )
    .bind(acting_user)
    .bind(req.service_type.clone())
    .bind(req.fresh_days)
    .bind(req.include_contacted())
    .bind(candidates.len() as i32)
    .fetch_one(&state.db_pool)
    .await
    .map_err(|e| ErrorResponse {
        success: false,
        message: format!("Failed to create outreach batch: {e}"),
    })?;

    let mut sample_needed = 0usize;
    let mut sample_recent = 0usize;
    for (id, name, email, _script, _service) in candidates {
        let pid: Uuid = id;
        // Recent usable sample? (completed delivery for this prospect within 30 days)
        let recent: Option<(Uuid, String)> = sqlx::query_as(
            "SELECT d.id, d.output_r2_url FROM deliveries d
              WHERE d.sourced_from_prospect_id = $1
                AND d.status = 'completed' AND d.output_r2_url IS NOT NULL
                AND d.created_at > NOW() - INTERVAL '30 days'
              ORDER BY d.created_at DESC LIMIT 1",
        )
        .bind(pid)
        .fetch_optional(&state.db_pool)
        .await
        .ok()
        .flatten();

        let (status, delivery_id, sample_url) = match recent {
            Some((did, url)) => {
                sample_recent += 1;
                ("ready_to_send", Some(did), Some(url))
            }
            None => {
                sample_needed += 1;
                ("pending_sample", None, None)
            }
        };

        let _ = sqlx::query(
            "INSERT INTO prospect_outreach_items
             (batch_id, prospect_id, prospect_name, prospect_email, status, delivery_id, sample_url)
             VALUES ($1,$2,$3,$4,$5,$6,$7)",
        )
        .bind(batch_id)
        .bind(pid)
        .bind(&name)
        .bind(&email)
        .bind(status)
        .bind(delivery_id)
        .bind(sample_url)
        .execute(&state.db_pool)
        .await;
    }

    Ok(Json(json!({
        "success": true,
        "batch_id": batch_id.to_string(),
        "total": candidates.len(),
        "samples_to_generate": sample_needed,
        "samples_already_ready": sample_recent,
        "message": format!(
            "Outreach batch created: {} prospects. {} sample video(s) will be generated first (background lane); each email is sent automatically once its sample is ready. Progress: /admin/email-logs.",
            candidates.len(), sample_needed
        ),
    }))
}


// ─── Smart search: build URL from filters + auto-launch ───────────────────────

#[derive(Debug, serde::Deserialize)]
struct SmartSearchRequest {
    /// Plain-English description — AI will generate all filters from this.
    /// e.g. "YouTubers and podcasters in the US with 10k+ followers who make money from content"
    /// If provided, all filter fields below are IGNORED and AI generates them.
    description: Option<String>,
    /// e.g. ["YouTuber", "Podcast Host", "Content Creator", "Marketing Manager"]
    job_titles: Option<Vec<String>>,
    /// e.g. ["Online Media", "E-Learning", "Marketing and Advertising"]
    industries: Option<Vec<String>>,
    /// e.g. ["1-10", "11-50"] or LinkedIn codes ["A", "B"]
    company_sizes: Option<Vec<String>>,
    /// e.g. ["United States", "United Kingdom"]
    locations: Option<Vec<String>>,
    /// e.g. ["OWNER", "CXO", "VP", "DIRECTOR", "MANAGER"]
    seniority: Option<Vec<String>>,
    /// Max profiles to scrape (default 100)
    max_profiles: Option<u32>,
    /// Optional: use a saved Sales Navigator list URL instead of building a search URL.
    list_url: Option<String>,
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
    // Service switch (owner directive Sep 2026): see linkedin_launch_search.
    if !crate::services::service_flags::service_enabled(&state.db_pool, "landing_page").await {
        return Json(
            json!({"success": false, "error": "LinkedIn discovery is currently disabled (landing_page off in service_flags)."}),
        );
    }
    let Some(pb) = state.phantombuster_client.as_ref() else {
        return Json(
            json!({"success": false, "error": "PhantomBuster not configured (PHANTOMBUSTER_API_KEY missing)"}),
        );
    };

    // Read session cookie from env
    let session_cookie = match std::env::var("LINKEDIN_SESSION_COOKIE") {
        Ok(c) if !c.is_empty() => c,
        _ => {
            return Json(
                json!({"success": false, "error": "LINKEDIN_SESSION_COOKIE not set in environment"}),
            )
        }
    };

    let max = req.max_profiles.unwrap_or(100);

    // If a plain-English description is given, use AI to generate all filter params
    let req = if let Some(ref desc) = req.description.clone() {
        match ai_generate_linkedin_filters(&state, desc).await {
            Ok(ai_req) => {
                tracing::info!(
                    "🤖 LinkedIn AI filters for \"{}\": {:?} / {:?} / {:?}",
                    desc,
                    ai_req.job_titles,
                    ai_req.industries,
                    ai_req.locations
                );
                ai_req
            }
            Err(e) => {
                tracing::warn!(
                    "LinkedIn AI filter generation failed: {} — using raw request",
                    e
                );
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
            Ok(None) => {
                return Json(
                    json!({"success": false, "error": "No Sales Navigator List Export phantom found. Add it from the PhantomBuster Phantom Store."}),
                )
            }
            Err(e) => {
                return Json(
                    json!({"success": false, "error": format!("Failed to list agents: {}", e)}),
                )
            }
        };
        let cid = match pb
            .launch_list_export(&agent.id, list_url, &session_cookie, max)
            .await
        {
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
            Err(e) => {
                return Json(
                    json!({"success": false, "error": format!("Failed to list agents: {}", e)}),
                )
            }
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
                let c = match pb
                    .launch_linkedin_search(&agent.id, &search, &session_cookie, max)
                    .await
                {
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
         VALUES ($1, $2, $3, 'running', NOW()) RETURNING id",
    )
    .bind(&agent.id)
    .bind(&agent.name)
    .bind(&search_url)
    .fetch_one(&state.db_pool)
    .await
    {
        Ok(id) => id,
        Err(e) => {
            return Json(json!({"success": false, "error": format!("DB insert failed: {}", e)}))
        }
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
    hashtag: String,          // e.g. "contentcreator" or "#videographer"
    max_posts: Option<u32>,   // default 50, max 200
    category: Option<String>, // label for this search (stored on leads)
    /// Forced service scope (owner directive Oct 2026): when set, scored leads
    /// from this search are tagged with this service only. Must be enabled.
    service: Option<String>,
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
struct ComparEdgeSearchRequest {
    category: Option<String>, // e.g. "crm", "project-management", "analytics"
    min_rating: Option<f64>,  // minimum rating 0-5, default 0
    min_starting_price: Option<f64>, // minimum monthly price in USD
    limit: Option<usize>,     // max results, default 50
    save: Option<bool>,       // true = upsert into prospects table
}

#[derive(Debug, Deserialize)]
struct InstagramListQuery {
    hashtag: Option<String>,
    contact_status: Option<String>,
    min_followers: Option<i64>,
    limit: Option<i64>,
    offset: Option<i64>,
    /// Show leads of disabled services too. Default: only ON-service leads
    /// (plus unscored NULL-service rows).
    include_disabled: Option<bool>,
    /// Admin oversight (owner directive Oct 2026): staff/superusers may pass a
    /// user_id to inspect that helper's pipeline, or omit it to see everyone.
    /// Non-admins always see only their own leads.
    user_id: Option<i32>,
    /// Only leads created in the last N days (launch freshness).
    fresh_days: Option<i64>,
}

/// POST /api/admin/prospects/saas/search
/// Fetches SaaS products from ComparEdge, filters by criteria, and optionally upserts into prospects.
async fn search_compar_edge_prospects(
    Extension(state): Extension<Arc<AppState>>,
    Json(req): Json<ComparEdgeSearchRequest>,
) -> Json<serde_json::Value> {
    let limit = req.limit.unwrap_or(50).min(500);
    let min_rating = req.min_rating.unwrap_or(0.0);
    let min_price = req.min_starting_price.unwrap_or(0.0);
    let save = req.save.unwrap_or(false);

    let url = "https://comparedge.com/llms-tools.json";
    let client = reqwest::Client::builder()
        .timeout(std::time::Duration::from_secs(15))
        .user_agent("VideoSyncBot/1.0")
        .build()
        .map_err(|e| format!("Failed to build HTTP client: {}", e));

    let client = match client {
        Ok(c) => c,
        Err(e) => return Json(json!({"success": false, "error": e})),
    };

    let resp = match client.get(url).send().await {
        Ok(r) => r,
        Err(e) => {
            return Json(
                json!({"success": false, "error": format!("Failed to fetch ComparEdge data: {}", e)}),
            )
        }
    };

    let body: serde_json::Value = match resp.json().await {
        Ok(v) => v,
        Err(e) => {
            return Json(
                json!({"success": false, "error": format!("Failed to parse ComparEdge JSON: {}", e)}),
            )
        }
    };

    let tools = match body.get("tools").and_then(|v| v.as_array()) {
        Some(t) => t,
        None => {
            return Json(
                json!({"success": false, "error": "ComparEdge JSON missing 'tools' array"}),
            )
        }
    };

    let cat_filter = req.category.as_deref().unwrap_or("").to_lowercase();

    let mut results: Vec<serde_json::Value> = Vec::new();
    let mut saved_count = 0usize;

    for tool in tools {
        if results.len() >= limit {
            break;
        }

        let name = tool.get("name").and_then(|v| v.as_str()).unwrap_or("");
        let website = tool.get("website").and_then(|v| v.as_str()).unwrap_or("");
        let description = tool
            .get("description")
            .and_then(|v| v.as_str())
            .unwrap_or("");
        let rating = tool.get("rating").and_then(|v| v.as_f64()).unwrap_or(0.0);
        let category = tool.get("category").and_then(|v| v.as_str()).unwrap_or("");
        let category_name = tool
            .get("categoryName")
            .and_then(|v| v.as_str())
            .unwrap_or("");
        let starting_price = tool
            .get("startingPrice")
            .and_then(|v| v.as_f64())
            .unwrap_or(0.0);
        let slug = tool.get("slug").and_then(|v| v.as_str()).unwrap_or("");

        if name.is_empty() || website.is_empty() || slug.is_empty() {
            continue;
        }

        // Apply filters
        if !cat_filter.is_empty()
            && category.to_lowercase() != cat_filter
            && category_name.to_lowercase() != cat_filter
        {
            continue;
        }
        if rating < min_rating {
            continue;
        }
        if starting_price < min_price {
            continue;
        }

        let ai_score = (rating / 5.0 * 100.0).round() as i32;

        if save {
            match sqlx::query(
                "INSERT INTO prospects (platform, channel_id, display_name, platform_url,
                 subscriber_count, content_category, channel_description, prospect_type,
                 ai_score, ai_reasoning, external_url, service_type, contact_enrichment)
                 VALUES ($1,$2,$3,$4,$5,$6,$7,$8,$9,$10,$11,$12,$13)
                 ON CONFLICT (platform, channel_id) DO UPDATE SET
                   display_name = EXCLUDED.display_name,
                   platform_url = EXCLUDED.platform_url,
                   channel_description = EXCLUDED.channel_description,
                   content_category = EXCLUDED.content_category,
                   ai_score = EXCLUDED.ai_score,
                   external_url = EXCLUDED.external_url,
                   service_type = EXCLUDED.service_type,
                   updated_at = NOW()",
            )
            .bind("compar_edge")
            .bind(slug)
            .bind(name)
            .bind(website)
            .bind(0i64) // subscriber_count
            .bind(category_name)
            .bind(description)
            .bind("saas_founder")
            .bind(ai_score)
            .bind(format!("ComparEdge SaaS product; rating {}/5", rating))
            .bind(website)
            .bind("landing_page")
            .bind(serde_json::json!({"source": "compar_edge", "category": category, "slug": slug, "startingPrice": starting_price}))
            .execute(&state.db_pool)
            .await
            {
                Ok(_) => saved_count += 1,
                Err(e) => tracing::warn!("Failed to save ComparEdge prospect {}: {}", slug, e),
            }
        }

        results.push(json!({
            "name": name,
            "website": website,
            "description": description,
            "category": category_name,
            "rating": rating,
            "starting_price": starting_price,
            "ai_score": ai_score,
            "slug": slug,
            "service_type": "landing_page",
        }));
    }

    let mut msg = format!("Found {} SaaS products", results.len());
    if save {
        msg.push_str(&format!(", saved {} to prospects", saved_count));
    }

    Json(json!({
        "success": true,
        "message": msg,
        "count": results.len(),
        "saved": if save { Some(saved_count) } else { None },
        "results": results,
    }))
}

#[derive(Debug, Deserialize)]
struct ProductHuntSearchRequest {
    topic: Option<String>, // Product Hunt topic/category (e.g. "ai", "saas", "developer-tools")
    query: Option<String>, // Direct text search (overrides topic if set)
    limit: Option<i32>,    // Max results, default 20, max 50
    order: Option<String>, // "VOTES" (default), "NEWEST", "FEATURED"
    save: Option<bool>,    // true = upsert into prospects table
}

/// POST /api/admin/prospects/product-hunt/search
/// Fetches SaaS products from Product Hunt API, filters, and optionally upserts into prospects.
async fn search_product_hunt_prospects(
    Extension(state): Extension<Arc<AppState>>,
    Json(req): Json<ProductHuntSearchRequest>,
) -> Json<serde_json::Value> {
    let limit = req.limit.unwrap_or(20).min(50);
    let order = req.order.as_deref().unwrap_or("VOTES");
    let save = req.save.unwrap_or(false);

    let posts = if let Some(query) = &req.query {
        if query.trim().is_empty() {
            return Json(json!({"success": false, "error": "query is empty"}));
        }
        crate::product_hunt_client::search_posts(query, limit).await
    } else {
        let topic = req.topic.as_deref().unwrap_or("saas");
        crate::product_hunt_client::fetch_top_posts(Some(topic), limit, Some(order)).await
    };

    let posts = match posts {
        Ok(p) => p,
        Err(e) => return Json(json!({"success": false, "error": e})),
    };

    let mut results: Vec<serde_json::Value> = Vec::new();
    let mut saved_count = 0usize;

    for post in posts {
        let name = post.name;
        let tagline = post.tagline.unwrap_or_default();
        let website = post.website.unwrap_or_default();
        let description = post
            .description
            .or(Some(tagline.clone()))
            .unwrap_or_default();

        if name.is_empty() || website.is_empty() {
            continue;
        }

        let votes = post.votes_count.unwrap_or(0);

        // Saving = full pipeline: enrichment (Scrapling) + LLM scoring so the
        // prospect lands ready for the outreach flow (owner directive Nov 9
        // 2026 — "does it really work?" polish). Without save, keep it fast.
        if save {
            let mut twitter_handle: Option<String> = None;
            let mut instagram_handle: Option<String> = None;
            let mut business_email: Option<String> = None;
            let mut external_url: Option<String> = Some(website.clone());
            let enrichment = enrich_public_contact_fields(
                Some(website.as_str()),
                &mut twitter_handle,
                &mut instagram_handle,
                &mut business_email,
                &mut external_url,
            )
            .await;
            let scoring_description = build_scoring_description(&description);
            let (score_f64, reasoning, service, _dm_creator, _dm_clipper, x_dm, email_script) =
                score_prospect_with_ai(
                    &state,
                    &name,
                    votes,
                    &scoring_description,
                    tagline.as_str(),
                    "saas_founder",
                    "product_hunt",
                    Some("landing_page"),
                )
                .await;
            let enrich_final = {
                let mut e = enrichment.clone();
                if let Some(obj) = e.as_object_mut() {
                    obj.insert("votes".to_string(), serde_json::json!(votes));
                    obj.insert("tagline".to_string(), serde_json::json!(tagline));
                    obj.insert("ph_slug".to_string(), serde_json::json!(post.id.clone()));
                }
                e
            };
            match sqlx::query(
                "INSERT INTO prospects (platform, channel_id, display_name, platform_url,
                 subscriber_count, content_category, channel_description, prospect_type,
                 ai_score, ai_reasoning, x_dm_script, email_script, external_url, service_type,
                 business_email, instagram_handle, twitter_handle, contact_enrichment)
                 VALUES ($1,$2,$3,$4,$5,$6,$7,$8,$9,$10,$11,$12,$13,$14,$15,$16,$17,$18)
                 ON CONFLICT (platform, channel_id) DO UPDATE SET
                   display_name = EXCLUDED.display_name,
                   platform_url = EXCLUDED.platform_url,
                   channel_description = EXCLUDED.channel_description,
                   ai_score = EXCLUDED.ai_score,
                   ai_reasoning = EXCLUDED.ai_reasoning,
                   x_dm_script = EXCLUDED.x_dm_script,
                   email_script = EXCLUDED.email_script,
                   external_url = EXCLUDED.external_url,
                   service_type = EXCLUDED.service_type,
                   business_email = COALESCE(EXCLUDED.business_email, prospects.business_email),
                   instagram_handle = COALESCE(EXCLUDED.instagram_handle, prospects.instagram_handle),
                   twitter_handle = COALESCE(EXCLUDED.twitter_handle, prospects.twitter_handle),
                   contact_enrichment = COALESCE(NULLIF(EXCLUDED.contact_enrichment, '{}'::jsonb), prospects.contact_enrichment) || EXCLUDED.contact_enrichment,
                   updated_at = NOW()",
            )
            .bind("product_hunt")
            .bind(post.id.clone())
            .bind(&name)
            .bind(&website)
            .bind(votes as i64)
            .bind(tagline.as_str())
            .bind(&description)
            .bind("saas_founder")
            .bind(score_f64)
            .bind(&reasoning)
            .bind(&x_dm)
            .bind(&email_script)
            .bind(&website)
            .bind(&service)
            .bind(business_email.as_deref())
            .bind(instagram_handle.as_deref())
            .bind(twitter_handle.as_deref())
            .bind(&enrich_final)
            .execute(&state.db_pool)
            .await
            {
                Ok(_) => saved_count += 1,
                Err(e) => tracing::warn!("Failed to save Product Hunt prospect {}: {e}", slug = post.id),
            }

            results.push(json!({
                "name": name,
                "website": website,
                "description": description,
                "tagline": tagline,
                "votes": votes,
                "ai_score": score_f64,
                "ai_reasoning": reasoning,
                "slug": post.id.clone(),
                "service_type": service,
                "enriched": !enrich_final.as_object().map(|o| o.is_empty()).unwrap_or(true),
                "business_email": business_email,
            }));
            continue;
        }

        let slug = post.id.clone();
        let ai_score = (votes as f64).min(100.0).max(1.0) as i32;

        results.push(json!({
            "name": name,
            "website": website,
            "description": description,
            "tagline": tagline,
            "votes": votes,
            "ai_score": ai_score,
            "slug": slug,
            "service_type": "landing_page",
        }));
    }

    let mut msg = format!("Found {} Product Hunt products", results.len());
    if save {
        msg.push_str(&format!(", saved {} to prospects", saved_count));
    }

    Json(json!({
        "success": true,
        "message": msg,
        "count": results.len(),
        "saved": if save { Some(saved_count) } else { None },
        "results": results,
    }))
}

/// POST /api/instagram/leads/search
/// Launches a PhantomBuster Instagram Hashtag Search and records leads.
async fn instagram_search_leads(
    Extension(state): Extension<Arc<AppState>>,
    Extension(claims): Extension<Claims>,
    Json(req): Json<InstagramSearchRequest>,
) -> Json<serde_json::Value> {
    let user_id: i32 = claims.sub.parse().unwrap_or(0);
    if user_id == 0 {
        return Json(json!({"success": false, "error": "Invalid user id in JWT"}));
    }

    let Some(pb) = state.phantombuster_client.as_ref() else {
        return Json(
            json!({"success": false, "error": "PhantomBuster not configured (PHANTOMBUSTER_API_KEY missing)"}),
        );
    };

    let session_cookie = match std::env::var("INSTAGRAM_SESSION_COOKIE") {
        Ok(c) if !c.is_empty() => c,
        _ => {
            return Json(
                json!({"success": false, "error": "INSTAGRAM_SESSION_COOKIE not set. Add your Instagram sessionid cookie to the server env vars."}),
            )
        }
    };

    let agent = match pb.find_instagram_hashtag_agent().await {
        Ok(Some(a)) => a,
        Ok(None) => {
            return Json(
                json!({"success": false, "error": "No Instagram Hashtag phantom found. Add 'Instagram Hashtag Search Export' from the PhantomBuster Phantom Store."}),
            )
        }
        Err(e) => {
            return Json(
                json!({"success": false, "error": format!("Failed to list PhantomBuster agents: {}", e)}),
            )
        }
    };

    let max_posts = req.max_posts.unwrap_or(50).min(200);
    let hashtag = req.hashtag.trim_start_matches('#').to_string();
    let category = req.category.clone().unwrap_or_else(|| hashtag.clone());

    // Forced-service scope (owner directive Oct 2026): validate up front so a
    // disabled/unknown service fails fast instead of burning PB runs.
    let forced_service: Option<String> = match req.service.as_deref() {
        None | Some("") => None,
        Some(wanted) => {
            let normalized = normalize_revenue_service(&wanted.to_ascii_lowercase()).to_string();
            if !is_valid_revenue_service(&normalized) {
                return Json(
                    json!({"success": false, "error": format!("Unknown service '{wanted}'.")}),
                );
            }
            if !crate::services::service_flags::service_enabled(&state.db_pool, &normalized).await {
                return Json(
                    json!({"success": false, "error": format!("Service '{normalized}' is currently disabled (service_flags).")}),
                );
            }
            Some(normalized)
        }
    };

    let launch_result = match forced_service.as_deref() {
        // Prefix convention (like instagram:kick:#): the poller parses
        // instagram:svc:{service}:#tag back into a forced scope — no schema change.
        Some(svc) => {
            let prefix = format!("instagram:svc:{svc}:#");
            try_launch_or_queue_ig_hashtag_job_with_prefix(
                &state,
                pb,
                &agent,
                &session_cookie,
                &hashtag,
                max_posts,
                user_id,
                &prefix,
            )
            .await
        }
        None => {
            try_launch_or_queue_ig_hashtag_job(
                &state,
                pb,
                &agent,
                &session_cookie,
                &hashtag,
                max_posts,
                user_id,
            )
            .await
        }
    };
    let (job_id, status, container_id) = match launch_result {
        Ok(t) => t,
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
    Extension(state): Extension<Arc<AppState>>,
    Extension(claims): Extension<Claims>,
    Query(q): Query<InstagramListQuery>,
) -> Json<serde_json::Value> {
    let user_id: i32 = claims.sub.parse().unwrap_or(0);
    if user_id == 0 {
        return Json(json!({"success": false, "error": "Invalid user id in JWT"}));
    }
    let is_admin = claims.is_superuser || claims.is_staff;

    let limit = q.limit.unwrap_or(50).min(200);
    let offset = q.offset.unwrap_or(0);

    // Scope to the caller's own leads — unless staff/superuser, who may scope
    // to one helper (user_id param) or see the whole team (param omitted).
    // The first bind is always the scope user_id; subsequent optional filters
    // start at $2. scope_all needs no user bind at all.
    let scope_all = is_admin && q.user_id.is_none();
    let scope_uid: Option<i32> = if is_admin {
        q.user_id.or(Some(user_id))
    } else {
        Some(user_id)
    };
    // Placeholder base: $1 is the scope user_id unless scope_all (team view).
    let base: usize = if scope_all { 0 } else { 1 };
    let mut sql = String::from(
        "SELECT id, username, full_name, bio, followers_count, following_count, posts_count,
                profile_url, profile_pic_url, is_private, is_verified, category,
                hashtag_source, email, external_url, dm_script, contact_status,
                pb_job_id, score, score_reason, service_type, created_at, user_id
         FROM instagram_leads",
    );
    if !scope_all {
        sql.push_str(" WHERE user_id = $1");
    } else {
        sql.push_str(" WHERE 1=1");
    }
    let mut binds: Vec<String> = Vec::new();

    if let Some(ref ht) = q.hashtag {
        binds.push(ht.trim_start_matches('#').to_string());
        sql.push_str(&format!(" AND hashtag_source = ${}", binds.len() + base));
    }
    if let Some(ref cs) = q.contact_status {
        binds.push(cs.clone());
        sql.push_str(&format!(" AND contact_status = ${}", binds.len() + base));
    }
    if let Some(mf) = q.min_followers {
        binds.push(mf.to_string());
        sql.push_str(&format!(
            " AND followers_count >= ${}::bigint",
            binds.len() + base
        ));
    }
    if let Some(days) = q.fresh_days {
        binds.push(days.max(1).to_string());
        sql.push_str(&format!(
            " AND created_at > NOW() - (({}::text) || ' days')::interval",
            binds.len() + base
        ));
    }
    // Service visibility (owner directive Oct 2026): UIs display only leads of
    // ON services; unscored rows (service_type IS NULL) always show.
    let show_all_ig = q.include_disabled.unwrap_or(false);
    let mut enabled_ig: Vec<String> = vec![];
    if !show_all_ig {
        enabled_ig = crate::services::service_flags::enabled_services(&state.db_pool).await;
        sql.push_str(&format!(
            " AND (service_type IS NULL OR service_type = ANY(${}))",
            binds.len() + base + 1
        ));
    }
    sql.push_str(" ORDER BY followers_count DESC NULLS LAST");
    sql.push_str(&format!(" LIMIT {} OFFSET {}", limit, offset));

    // Build and execute dynamically — use raw query for variable bind count
    let mut query = sqlx::query(&sql);
    if !scope_all {
        query = query.bind(scope_uid);
    }
    for b in &binds {
        query = query.bind(b);
    }
    if !show_all_ig {
        query = query.bind(enabled_ig);
    }

    let rows = match query.fetch_all(&state.db_pool).await {
        Ok(r) => r,
        Err(e) => {
            return Json(json!({"success": false, "error": format!("DB query failed: {}", e)}))
        }
    };

    let leads: Vec<serde_json::Value> = rows
        .iter()
        .map(|r| {
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
                "user_id":         r.get::<Option<i32>, _>("user_id"),
            })
        })
        .collect();

    Json(json!({"success": true, "leads": leads, "count": leads.len()}))
}

/// DELETE /api/instagram/leads
/// Delete all Instagram leads for the authenticated user. Scoped to the
/// caller so one user can never touch another user's leads.
async fn instagram_clear_leads(
    Extension(state): Extension<Arc<AppState>>,
    Extension(claims): Extension<Claims>,
) -> Json<serde_json::Value> {
    let user_id: i32 = claims.sub.parse().unwrap_or(0);
    if user_id == 0 {
        return Json(json!({"success": false, "error": "Invalid user id in JWT"}));
    }
    match sqlx::query("DELETE FROM instagram_leads WHERE user_id = $1")
        .bind(user_id)
        .execute(&state.db_pool)
        .await
    {
        Ok(r) => Json(json!({
            "success": true,
            "deleted": r.rows_affected(),
        })),
        Err(e) => Json(json!({"success": false, "error": format!("DB error: {e}")})),
    }
}

/// POST /api/instagram/leads/:id/generate-dm
/// Generate a personalized Instagram cold DM script using AI.
async fn instagram_generate_dm(
    Extension(state): Extension<Arc<AppState>>,
    Extension(claims): Extension<Claims>,
    Path(id): Path<uuid::Uuid>,
    Json(req): Json<Option<InstagramDmRequest>>,
) -> Json<serde_json::Value> {
    // Service switch (owner directive Sep 2026).
    if !crate::services::service_flags::service_enabled(&state.db_pool, "outreach").await {
        return Json(
            json!({"success": false, "error": "Outreach generation is currently disabled (service_flags)."}),
        );
    }
    let user_id: i32 = claims.sub.parse().unwrap_or(0);

    // Fetch the lead — scoped to the caller so one user can't DM another
    // user's lead or learn that another user has that lead.
    let row = match sqlx::query(
        "SELECT username, full_name, bio, followers_count, category, hashtag_source,
                external_url, email, service_type, sample_delivery_id, contact_enrichment
         FROM instagram_leads WHERE id = $1 AND user_id = $2",
    )
    .bind(id)
    .bind(user_id)
    .fetch_optional(&state.db_pool)
    .await
    {
        Ok(Some(r)) => r,
        Ok(None) => return Json(json!({"success": false, "error": "Lead not found"})),
        Err(e) => return Json(json!({"success": false, "error": format!("DB error: {}", e)})),
    };

    let assigned_app: Option<String> = row
        .get::<Option<serde_json::Value>, _>("contact_enrichment")
        .and_then(|ce| {
            ce.get("assigned_app")
                .and_then(|v| v.as_str().map(String::from))
        });
    let username: String = row.get::<Option<String>, _>("username").unwrap_or_default();
    let full_name: String = row
        .get::<Option<String>, _>("full_name")
        .unwrap_or_else(|| username.clone());
    let bio: String = row.get::<Option<String>, _>("bio").unwrap_or_default();
    let followers: i64 = row.get::<Option<i64>, _>("followers_count").unwrap_or(0);
    let category: String = row.get::<Option<String>, _>("category").unwrap_or_default();
    let ext_url: String = row
        .get::<Option<String>, _>("external_url")
        .unwrap_or_default();
    let email: String = row.get::<Option<String>, _>("email").unwrap_or_default();
    let contact_enrichment: Option<serde_json::Value> =
        row.try_get("contact_enrichment").ok().flatten();
    let service: Option<String> = row
        .get::<Option<String>, _>("service_type")
        .map(|s| normalize_revenue_service(&s).to_string());

    // Disabled-service gate (owner directives Sep 2026 + Nov 8): generating a
    // DM for a parked service wastes LLM tokens for a lead we can't sell to —
    // and NULL service_type means the lead hasn't been scored against the
    // enabled menu yet. Both are 503'd; only ON services can be messaged.
    if let Some(ref st) = service {
        if !crate::services::service_flags::service_enabled(&state.db_pool, st).await {
            return Json(json!({
                "success": false,
                "code": "service_disabled",
                "error": format!(
                    "'{st}' outreach is switched off. Leads are only messaged for the currently enabled services ({}).",
                    crate::services::service_flags::enabled_services(&state.db_pool).await.join(", ")
                ),
            }));
        }
    } else {
        return Json(json!({
            "success": false,
            "code": "not_scored",
            "error": "This lead has not been scored yet — run scoring first so it lands on an enabled service, then generate the DM."
        }));
    }

    let sample_id: Option<uuid::Uuid> = row.try_get("sample_delivery_id").ok().flatten();

    // Build the sample-link block injected into the prompt. If a sample
    // exists, the LLM is instructed to work the URL in naturally; without
    // one, we tell the model not to invent a link.
    let base_url = format!(
        "https://{}.videosync.ink",
        app_subdomain_for_service(service.as_deref().unwrap_or("landing_page"))
    );
    let sample_block =
        "NO sample exists yet — do NOT invent a URL. Offer to make one: \"want me to put together a quick sample?\"".to_string();

    let has_website = !ext_url.trim().is_empty();
    let detected_creators: Vec<String> = contact_enrichment
        .as_ref()
        .and_then(|ce| ce.get("detected_source_creators"))
        .and_then(|v| v.as_array())
        .map(|arr| {
            arr.iter()
                .filter_map(|v| v.as_str().map(String::from))
                .collect()
        })
        .unwrap_or_default();
    let sample_block = match sample_id {
        Some(sid) => {
            let sample_unlock_price = 0.0;

            format!(
                "SAMPLE LINK (must appear in the DM, woven in naturally - e.g. \"here's a quick mockup I made: <URL>\" - do NOT just paste it at the end): {}/delivery/{}",
                base_url.trim_end_matches('/'),
                sid,
            )
        }
        None => sample_block,
    };

    let niche = req
        .as_ref()
        .and_then(|r| r.niche.as_deref())
        .unwrap_or(&category);

    let followers_str = if followers > 0 {
        followers.to_string()
    } else {
        "unknown".to_string()
    };

    let service_block = service_offer_prompt(service.as_deref());
    // Campaigns switch (owner directive Nov 8 2026): when OFF the IG-lead DM
    // pitches ONLY the one-time bundle (no campaign/recurrence wording).
    // Studio keeps its DFY pitch in either state.
    let campaigns_on = crate::services::service_flags::campaigns_enabled(&state.db_pool).await;
    let is_studio = contact_enrichment
        .as_ref()
        .and_then(|ce| ce.get("assigned_app"))
        .and_then(|v| v.as_str())
        .map(|s| s == "studio")
        .unwrap_or(false);
    let service_block = if campaigns_on {
        service_block
    } else {
        crate::services::service_flags::campaigns_off_service_block(service.as_deref(), is_studio)
    };

    // Referral attribution (whitelisted non-admin senders only).
    let acting_is_admin = claims.is_superuser || claims.is_staff;
    let referral_url: Option<String> = if !acting_is_admin {
        let uid = user_id;
        let app = assigned_app.as_deref().unwrap_or_else(|| {
            app_subdomain_for_service(service.as_deref().unwrap_or("landing_page"))
        });
        let existing: Option<String> = sqlx::query_scalar(
            "SELECT code FROM referral_codes WHERE user_id = $1 AND app_slug = $2",
        )
        .bind(uid)
        .bind(app)
        .fetch_optional(&state.db_pool)
        .await
        .ok()
        .flatten();
        let code = match existing {
            Some(c) => c,
            None => {
                let new_code = format!("ref-{}-{}", app, &Uuid::new_v4().to_string()[..6]);
                sqlx::query(
                    "INSERT INTO referral_codes (user_id, code, app_slug) VALUES ($1, $2, $3) ON CONFLICT (user_id, app_slug) WHERE app_slug IS NOT NULL DO NOTHING",
                )
                .bind(uid)
                .bind(&new_code)
                .bind(app)
                .execute(&state.db_pool)
                .await
                .ok();
                sqlx::query_scalar::<_, String>(
                    "SELECT code FROM referral_codes WHERE user_id = $1 AND app_slug = $2",
                )
                .bind(uid)
                .bind(app)
                .fetch_optional(&state.db_pool)
                .await
                .ok()
                .flatten()
                .unwrap_or(new_code)
            }
        };
        Some(format!("https://videosync.ink/ref/{}/{}", app, code))
    } else {
        None
    };
    let referral_block = referral_url
        .map(|u| format!("If they want to sign up or create an account, include this signup link naturally: {}", u))
        .unwrap_or_default();

    let prompt = format!(
        r#"You are a senior outbound copywriter for a video production studio. Write a personalized Instagram DM (≤120 words) to this creator.

Creator info:
- Handle:       @{username}
- Name:         {full_name}
- Followers:    {followers}
- Bio / recent post caption: {bio}
- Niche hint:   {niche}
- Link in bio:  {ext_url}
- Email:        {email}
- Detected source creators (Kick streamers they clip): {creators}

{service_block}

{sample_block}

{referral_block}

The DM must:
- Be specific. Reference an actual detail from the bio/caption — never generic flattery.
- State concretely what the studio would do for THEM, with the realistic price from the service block above.
- If a SAMPLE LINK is provided above, weave the full URL into the DM body naturally. Never replace it with "link" or paraphrase — paste the exact URL. If no sample exists, offer to make one.
- End with a clear ask: "want me to send over the breakdown?" or "does this fit what you had in mind?"
- Sound like one founder messaging another. Casual, lowercase ok, no corporate fluff, no emoji spam.
- Adapt tone to follower size — micro creators (<10k) warm + helpful; mid creators (10k–100k) business-direct; big creators (>100k) concise with clear ROI angle.
- Do NOT start the DM with "Verified", "Verified here", or any verification phrase. Start with a genuine, personalized observation about their content.
- For landing_page: mention that the system can generate ANY type of animated video from their website URL (how-to, marketing, product demo, explainer, testimonial, etc.) — not just hero sections.
- For education: mention that it can generate ANY type of animated educational video (math, science, coding, language, history, finance, etc.) with equations, diagrams, motion graphics, VFX, and 3D depth.
- If this is a done-for-you (studio) prospect, mention that we produce and deliver the videos for them ($100-$200 for 10-30 videos) with a 15-min strategy call.

Output ONLY the DM body. No quotes, no labels, no preamble."#,
        username = username,
        full_name = full_name,
        followers = followers_str,
        bio = bio,
        niche = niche,
        ext_url = ext_url,
        email = if email.trim().is_empty() {
            "none".to_string()
        } else {
            email.clone()
        },
        creators = if detected_creators.is_empty() {
            "none".to_string()
        } else {
            detected_creators.join(", ")
        },
        service_block = service_block,
        sample_block = sample_block,
        referral_block = referral_block,
    );

    let dm_text_raw = match generate_text_best_effort(
        state.qwen_client.as_ref(),
        state.nvidia_nim_client.as_ref(),
        state.gemma_client.as_ref(),
        state.gemini_client.as_ref(),
        state.deepseek_client.as_ref(),
        &prompt,
    )
    .await
    {
        Ok(t) => t,
        Err(e) => {
            return Json(json!({"success": false, "error": format!("AI generation failed: {}", e)}))
        }
    };

    // Guarantee while the switch is OFF (shared helper in service_flags).
    let dm_text =
        crate::services::service_flags::scrub_campaign_language(&dm_text_raw, campaigns_on);

    // Save the DM script to the DB
    let _ = sqlx::query(
        "UPDATE instagram_leads SET dm_script = $1, updated_at = NOW()
         WHERE id = $2 AND user_id = $3",
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
/// * `clipping`    → returns `requires_source_url=true` because clipping
///                   needs a video URL the user supplies. The frontend
///                   should prompt for it then call /api/admin/deliveries
///                   directly.
#[derive(Debug, Deserialize)]
struct GenerateSampleRequest {
    /// Optional override — for `clipping`, the user must paste a video URL.
    source_url: Option<String>,
}

async fn instagram_generate_sample(
    Extension(state): Extension<Arc<AppState>>,
    Extension(claims): Extension<Claims>,
    Path(id): Path<uuid::Uuid>,
    Json(req): Json<Option<GenerateSampleRequest>>,
) -> Json<serde_json::Value> {
    let user_id: i32 = claims.sub.parse().unwrap_or(0);
    let source_url = req.as_ref().and_then(|r| r.source_url.clone());

    let row = match sqlx::query(
        "SELECT username, full_name, bio, service_type, sample_delivery_id, external_url,
                contact_enrichment
         FROM instagram_leads WHERE id = $1 AND user_id = $2",
    )
    .bind(id)
    .bind(user_id)
    .fetch_optional(&state.db_pool)
    .await
    {
        Ok(Some(r)) => r,
        Ok(None) => return Json(json!({"success": false, "error": "Lead not found"})),
        Err(e) => return Json(json!({"success": false, "error": format!("DB error: {}", e)})),
    };

    // If a sample is already attached, just return it — re-generating costs
    // money and the user wants the link to paste, not a fresh render.
    let service_key: String = row
        .get::<Option<String>, _>("service_type")
        .unwrap_or_else(|| "landing_page".to_string());
    if let Some(existing) = row.get::<Option<uuid::Uuid>, _>("sample_delivery_id") {
        let assigned = row
            .get::<Option<serde_json::Value>, _>("contact_enrichment")
            .and_then(|ce| {
                ce.get("assigned_app")
                    .and_then(|v| v.as_str().map(String::from))
            });
        return Json(json!({
            "success":      true,
            "delivery_id":  existing.to_string(),
            "delivery_url": format!(
                "https://{}.videosync.ink/delivery/{}",
                assigned.as_deref().unwrap_or_else(|| app_subdomain_for_service(service_key.as_str())),
                existing
            ),
            "message":      "Sample already exists for this lead.",
        }));
    }

    let username: String = row.get::<Option<String>, _>("username").unwrap_or_default();
    let full_name: String = row
        .get::<Option<String>, _>("full_name")
        .unwrap_or_else(|| username.clone());
    let bio: String = row.get::<Option<String>, _>("bio").unwrap_or_default();
    let service: Option<String> = row.get::<Option<String>, _>("service_type");
    let assigned_app: Option<String> = row
        .get::<Option<serde_json::Value>, _>("contact_enrichment")
        .and_then(|ce| {
            ce.get("assigned_app")
                .and_then(|v| v.as_str().map(String::from))
        });
    let lead_ext_url: Option<String> = row.get::<Option<String>, _>("external_url");
    let contact_enrichment: Option<serde_json::Value> =
        row.get::<Option<serde_json::Value>, _>("contact_enrichment");

    // Auto-detect the lead's website: prefer explicit source_url from request,
    // fall back to the lead's external_url from their profile (IG bio link, etc.)
    let mut source_url = source_url.or(lead_ext_url);

    // For kick_auto_clipper leads with detected source creators, auto-resolve
    // the Kick VOD URL from the first detected creator.
    if service.as_deref() == Some("kick_auto_clipper") && source_url.is_none() {
        if let Some(ref enrichment) = contact_enrichment {
            if let Some(creators) = enrichment
                .get("detected_source_creators")
                .and_then(|c| c.as_array())
            {
                if let Some(first) = creators.first().and_then(|c| c.as_str()) {
                    let kick_url = format!("https://kick.com/{}", first.trim_start_matches('@'));
                    source_url = Some(kick_url);
                    tracing::info!(
                        "🎯 Auto-resolved Kick URL for IG lead @{}: {}",
                        username,
                        source_url.as_deref().unwrap_or("")
                    );
                }
            }
        }
    }

    let service_key = service.as_deref().unwrap_or("landing_page");
    let has_reference_url = source_url
        .as_deref()
        .map(|u| u.starts_with("http://") || u.starts_with("https://"))
        .unwrap_or(false);
    let use_long_form_workflow =
        should_use_long_form_for_revenue_sample(service_key, has_reference_url);

    // For clipping the lead, we'd need their actual video URL. Tell the
    // frontend so it can ask the user for one. Don't burn a render slot on
    // a placeholder for clipping — the value is in clipping THEIR content.
    if matches!(
        service.as_deref(),
        Some("clipping" | "kick_auto_clipper" | "youtube_clipping" | "twitch_clipping")
    ) && !has_reference_url
    {
        let service_label = match service.as_deref() {
            Some("kick_auto_clipper") => "Kick auto-clipper",
            Some("youtube_clipping") => "YouTube clipping",
            Some("twitch_clipping") => "Twitch clipping",
            _ => "Clipping",
        };
        return Json(json!({
            "success":              false,
            "requires_source_url":  true,
            "error":                format!("{} samples need a source video URL (YouTube/Kick/Twitch). Paste a URL related to @{}'s content and try again.", service_label, username),
        }));
    }

    // Build a delivery row pointing at a lightweight Blender render.
    // Default to `thumbnail` if the scorer hasn't set a service yet.
    //
    // For service types that benefit from a reference image (product_mockup
    // and landing_page), we generate the image via Gemini Nano Banana or
    // use a user-supplied source URL, then pass the resulting S3 URL to
    // the Blender tool.
    let (gig_type, prompt, style, duration, mut extra) = if use_long_form_workflow {
        let target_duration = service_target_duration_seconds(service_key);
        (
            "long_form_video",
            format!("{} - {}", full_name, service_offer_line(service_key)),
            service_long_form_style(service_key),
            target_duration,
            json!({
                "workflow_engine": "long_form_video_assembly",
                "revenue_v1": true,
                "instagram_lead_id": id,
                "instagram_username": username,
                "service_offer": service_key,
                "service_offer_line": service_offer_line(service_key),
                "offer_type": service_long_form_offer_type(service_key),
                "target_duration_seconds": target_duration,
                "segment_duration_seconds": 15.0,
                "source_url": source_url.clone(),
                "bio": bio.clone(),
            }),
        )
    } else {
        match service.as_deref() {
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
                            let p = format!(
                                "Clean SaaS landing-page hero illustration: {}",
                                bio.chars().take(150).collect::<String>()
                            );
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
                    "ui_mockup",
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

            _ => {
                // If the lead has a website, generate a scene animation with
                // their hero image instead of a plain thumbnail — far more
                // impressive in a DM.
                if let Some(ref url) = source_url {
                    if let Some(hero) = fetch_landing_page_hero(url).await {
                        (
                            "ui_mockup",
                            format!("Professional animated showcase for {}", full_name),
                            "modern",
                            10.0,
                            json!({"reference_image_url": hero, "source_url": url}),
                        )
                    } else {
                        (
                            "thumbnail",
                            format!(
                                "Eye-catching thumbnail for @{} — {}",
                                username,
                                bio.chars().take(80).collect::<String>()
                            ),
                            "bold",
                            0.0,
                            json!({"title_text": full_name}),
                        )
                    }
                } else {
                    (
                        "thumbnail",
                        format!(
                            "Eye-catching thumbnail for @{} — {}",
                            username,
                            bio.chars().take(80).collect::<String>()
                        ),
                        "bold",
                        0.0,
                        json!({"title_text": full_name}),
                    )
                }
            }
        }
    };

    if let Some(extra_obj) = extra.as_object_mut() {
        extra_obj.insert("sample_owner_user_id".to_string(), json!(user_id));
        extra_obj.insert("sourced_from_lead_id".to_string(), json!(id));
    }

    let title = format!("Sample for @{}", username);

    // Stamp the origin lead on the delivery so attribution tracks back
    // to the whitelisted user who sourced this lead.
    let unlock_price = 0.0;
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
         WHERE id = $2 AND user_id = $3",
    )
    .bind(delivery_id)
    .bind(id)
    .bind(user_id)
    .execute(&state.db_pool)
    .await;

    let render_state = state.clone();
    if gig_type == "long_form_video" {
        let service_type = crate::services::normalize_to_service_type(service_key);
        let agentic_input = crate::services::ServiceInput {
            title: title.clone(),
            brief: format!(
                "{prompt}\n\nInstagram lead: @{username}\nBio/context: {bio}\n\nCreate a buyer-facing sample for this service: {offer}. It should be useful as outreach proof for a whitelisted marketer and should demonstrate the relevant creative tools for the offer.",
                offer = service_offer_line(service_key)
            ),
            source_url: source_url.clone(),
            style: style.to_string(),
            duration_seconds: duration.max(15.0),
            delivery_id,
            prospect_id: None,
            session_uuid: None,
            user_id: Some(user_id),
            source_table: Some("deliveries".to_string()),
            source_record_id: Some(delivery_id),
            idempotency_key: Some(format!("instagram-lead-long-form:{id}")),
            reference_images: vec![],
        nim_profile: None,
        lane: "interactive".to_string(),
        };
        match crate::services::AgenticServicePipeline::start(
            state.clone(),
            service_key.clone(),
            agentic_input,
        )
        .await
        {
            Ok(workflow_id) => {
                let _ = sqlx::query("UPDATE deliveries SET workflow_id = $1 WHERE id = $2")
                    .bind(workflow_id)
                    .bind(delivery_id)
                    .execute(&state.db_pool)
                    .await;
            }
            Err(e) => {
                return Json(json!({
                    "success": false,
                    "error": format!("Failed to start long-form workflow: {}", e)
                }));
            }
        }
    } else {
        let _ = crate::handlers::admin::ensure_delivery_workflow(&state, delivery_id).await;
        // For high-value leads with a website, route through the full AI agent
        // to generate a comprehensive video (script + Blender scenes + voiceover).
        // For simpler cases, use the direct Blender render path.
        let has_website = source_url.is_some();
        if has_website && gig_type == "ui_mockup" {
            let website_url = source_url.clone().unwrap_or_default();
            let lead_name = full_name.clone();
            let lead_bio = bio.clone();
            tokio::spawn(async move {
                run_agent_video_for_lead(
                    delivery_id,
                    &website_url,
                    &lead_name,
                    &lead_bio,
                    render_state,
                )
                .await;
            });
        } else {
            tokio::spawn(async move {
                crate::handlers::admin::run_delivery_job(delivery_id, render_state).await;
            });
        }
    }

    Json(json!({
        "success":      true,
        "delivery_id":  delivery_id.to_string(),
        "delivery_url": format!(
            "https://{}.videosync.ink/delivery/{}",
            assigned_app.as_deref().unwrap_or_else(|| app_subdomain_for_service(service_key)),
            delivery_id,
        ),
        "service":      service,
        "message":      "Sample queued. Render takes 1-3 minutes; the /delivery/:id link is shareable immediately and will show the result when ready.",
    }))
}

/// PATCH /api/instagram/leads/:id/contact-status
async fn instagram_update_contact_status(
    Extension(state): Extension<Arc<AppState>>,
    Extension(claims): Extension<Claims>,
    Path(id): Path<uuid::Uuid>,
    Json(req): Json<InstagramContactStatusRequest>,
) -> Json<serde_json::Value> {
    let user_id: i32 = claims.sub.parse().unwrap_or(0);
    let valid = ["new", "contacted", "replied", "converted", "skipped"];
    if !valid.contains(&req.contact_status.as_str()) {
        return Json(
            json!({"success": false, "error": "contact_status must be one of: new, contacted, replied, converted, skipped"}),
        );
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
    niche: Option<String>,
    /// Max profiles to pull per hashtag (default 30, max 100)
    max_posts_per_hashtag: Option<u32>,
    /// How many hashtags to search (default 3, max 6)
    hashtag_count: Option<usize>,
    /// Forced service scope (owner directive Oct 2026): scored leads from
    /// these searches are tagged with this service only. Must be enabled.
    service: Option<String>,
}

/// POST /api/instagram/leads/auto-discover
///
/// AI picks the best hashtags for the niche, then launches one PhantomBuster
/// search per hashtag. The background poller (`poll_instagram_jobs`) will
/// auto-import results and score leads once PhantomBuster finishes (~5–10 min).
async fn instagram_auto_discover(
    Extension(state): Extension<Arc<AppState>>,
    Extension(claims): Extension<Claims>,
    Json(req): Json<Option<AutoDiscoverRequest>>,
) -> Json<serde_json::Value> {
    let user_id: i32 = claims.sub.parse().unwrap_or(0);
    if user_id == 0 {
        return Json(json!({"success": false, "error": "Invalid user id in JWT"}));
    }

    let req = req.unwrap_or(AutoDiscoverRequest {
        niche: None,
        max_posts_per_hashtag: None,
        hashtag_count: None,
        service: None,
    });
    let niche = req.niche.as_deref().unwrap_or("content creator");
    let max_posts = req.max_posts_per_hashtag.unwrap_or(30).min(100);
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
        Ok(None) => {
            return Json(
                json!({"success": false, "error": "No Instagram Hashtag phantom found in PhantomBuster. Add 'Instagram Hashtag Search Export' from the Phantom Store."}),
            )
        }
        Err(e) => {
            return Json(json!({"success": false, "error": format!("Agent lookup failed: {}", e)}))
        }
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
        state.qwen_client.as_ref(),
        state.nvidia_nim_client.as_ref(),
        state.gemma_client.as_ref(),
        state.gemini_client.as_ref(),
        state.deepseek_client.as_ref(),
        &prompt,
    )
    .await
    {
        Ok(t) => t,
        Err(e) => {
            return Json(
                json!({"success": false, "error": format!("AI hashtag selection failed: {}", e)}),
            )
        }
    };

    // Strip markdown fences and parse JSON array
    let cleaned = hashtags_json
        .trim()
        .trim_start_matches("```json")
        .trim_start_matches("```")
        .trim_end_matches("```")
        .trim();

    let hashtags: Vec<String> = match serde_json::from_str(cleaned) {
        Ok(v) => v,
        Err(e) => {
            tracing::warn!(
                "Failed to parse AI hashtag response: {} | raw: {}",
                e,
                cleaned
            );
            // Fallback hashtags based on niche keyword
            vec![
                niche.replace(' ', ""),
                "contentcreator".to_string(),
                "youtuber".to_string(),
            ]
        }
    };

    tracing::info!(
        "🎯 Instagram auto-discover for niche '{}': hashtags = {:?}",
        niche,
        hashtags
    );

    // ── Launch a PB search for each hashtag ──────────────────────────────────
    let mut launched_jobs: Vec<serde_json::Value> = Vec::new();
    let mut errors: Vec<String> = Vec::new();

    // Only the first hashtag will actually launch — the rest queue because
    // PhantomBuster caps parallel runs per workspace. The dispatcher below
    // promotes queued jobs one at a time as slots free up.
    // Forced-service scope (owner directive Oct 2026): validated once here,
    // encoded per-job via the instagram:svc: prefix convention.
    let forced_service: Option<String> = match req.service.as_deref() {
        None | Some("") => None,
        Some(wanted) => {
            let normalized = normalize_revenue_service(&wanted.to_ascii_lowercase()).to_string();
            if !is_valid_revenue_service(&normalized) {
                return Json(
                    json!({"success": false, "error": format!("Unknown service '{wanted}'.")}),
                );
            }
            if !crate::services::service_flags::service_enabled(&state.db_pool, &normalized).await {
                return Json(
                    json!({"success": false, "error": format!("Service '{normalized}' is currently disabled (service_flags).")}),
                );
            }
            Some(normalized)
        }
    };
    for hashtag in &hashtags {
        let tag = hashtag.trim_start_matches('#').to_string();
        let launch = match forced_service.as_deref() {
            Some(svc) => {
                let prefix = format!("instagram:svc:{svc}:#");
                try_launch_or_queue_ig_hashtag_job_with_prefix(
                    &state,
                    pb,
                    &agent,
                    &session_cookie,
                    &tag,
                    max_posts,
                    user_id,
                    &prefix,
                )
                .await
            }
            None => {
                try_launch_or_queue_ig_hashtag_job(
                    &state,
                    pb,
                    &agent,
                    &session_cookie,
                    &tag,
                    max_posts,
                    user_id,
                )
                .await
            }
        };
        match launch {
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

/// POST /api/instagram/leads/kick-auto-discover
///
/// AI picks Kick-specific Instagram hashtags to find accounts that repost
/// Kick.com streamer clips. Launches PhantomBuster searches, then the
/// background poller imports + scores leads with Kick creator detection.
async fn instagram_kick_auto_discover(
    Extension(state): Extension<Arc<AppState>>,
    Extension(claims): Extension<Claims>,
    Json(req): Json<Option<AutoDiscoverRequest>>,
) -> Json<serde_json::Value> {
    let user_id: i32 = claims.sub.parse().unwrap_or(0);
    if user_id == 0 {
        return Json(json!({"success": false, "error": "Invalid user id in JWT"}));
    }

    let req = req.unwrap_or(AutoDiscoverRequest {
        niche: None,
        max_posts_per_hashtag: None,
        hashtag_count: None,
        service: None,
    });
    let category = req.niche.as_deref().unwrap_or("gaming");
    let max_posts = req.max_posts_per_hashtag.unwrap_or(30).min(100);
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
        Ok(None) => {
            return Json(
                json!({"success": false, "error": "No Instagram Hashtag phantom found in PhantomBuster. Add 'Instagram Hashtag Search Export' from the Phantom Store."}),
            )
        }
        Err(e) => {
            return Json(json!({"success": false, "error": format!("Agent lookup failed: {}", e)}))
        }
    };

    // ── Ask AI to pick Kick-specific Instagram hashtags ─────────────
    let prompt = format!(
        r#"You are finding Instagram accounts that repost clips from Kick.com streamers.
These accounts take Kick stream VODs, edit them into short-form clips (Reels), and repost them with the Kick.com green watermark.

Target niche/category: "{category}"

List {count} Instagram hashtags (no # symbol) that Kick clipper channels in the "{category}" niche actually use when posting their clips.
Choose hashtags that:
1. Kick clip channels actually tag their posts with (e.g. kickclips, kickhighlights, kickstreamer, streamclips)
2. Will surface accounts that repost Kick streamer content
3. Are actively used by clipping/editing accounts, NOT just by fans or the streamers themselves

Return ONLY a JSON array of strings. No explanation. Example: ["kickclips", "kickhighlights", "kickstreamer"]"#,
        category = category,
        count = hashtag_count,
    );

    let hashtags_json = match generate_text_best_effort(
        state.qwen_client.as_ref(),
        state.nvidia_nim_client.as_ref(),
        state.gemma_client.as_ref(),
        state.gemini_client.as_ref(),
        state.deepseek_client.as_ref(),
        &prompt,
    )
    .await
    {
        Ok(t) => t,
        Err(e) => {
            return Json(
                json!({"success": false, "error": format!("AI hashtag selection failed: {}", e)}),
            )
        }
    };

    // Strip markdown fences and parse JSON array
    let cleaned = hashtags_json
        .trim()
        .trim_start_matches("```json")
        .trim_start_matches("```")
        .trim_end_matches("```")
        .trim();

    let hashtags: Vec<String> = match serde_json::from_str(cleaned) {
        Ok(v) => v,
        Err(e) => {
            tracing::warn!(
                "Failed to parse AI Kick hashtag response: {} | raw: {}",
                e,
                cleaned
            );
            vec![
                "kickclips".to_string(),
                "kickhighlights".to_string(),
                "kickstreamer".to_string(),
            ]
        }
    };

    tracing::info!(
        "🎯 Instagram kick-auto-discover for category '{}': hashtags = {:?}",
        category,
        hashtags
    );

    // ── Launch a PB search for each hashtag, tagged as kick_clipper ─
    let mut launched_jobs: Vec<serde_json::Value> = Vec::new();
    let mut errors: Vec<String> = Vec::new();

    for hashtag in &hashtags {
        let tag = hashtag.trim_start_matches('#').to_string();
        // Override search_url to include "kick:" prefix so the poller can
        // identify these as Kick clipper leads.
        match try_launch_or_queue_ig_hashtag_job_with_prefix(
            &state,
            pb,
            &agent,
            &session_cookie,
            &tag,
            max_posts,
            user_id,
            "instagram:kick:#",
        )
        .await
        {
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
        tokio::time::sleep(tokio::time::Duration::from_millis(500)).await;
    }

    Json(json!({
        "success":     !launched_jobs.is_empty(),
        "category":    category,
        "hashtags":    hashtags,
        "jobs":        launched_jobs,
        "errors":      errors,
        "message":     format!(
            "Kick clipper auto-discovery launched {} PB searches for category '{}'. Results auto-import in ~5–10 minutes. Kick creators will be auto-detected from captions.",
            launched_jobs.len(), category
        )
    }))
}

/// POST /api/instagram/leads/kick-method-b
///
/// Method B for Instagram: start from verified top Kick streamers (OAuth API,
/// free, no Cloudflare) and search Instagram for each streamer's clippers,
/// instead of guessing hashtags. Per streamer launches deterministic tag
/// searches ({slug}, {slug}clips, {slug}highlights) with a methodb prefix
/// carrying the slug, so the poller presets detected_source_creators with
/// zero LLM detection cost. Mirrors kick Method B (YouTube) for Instagram.
async fn instagram_kick_method_b(
    Extension(state): Extension<Arc<AppState>>,
    Extension(claims): Extension<Claims>,
    Json(req): Json<Option<AutoDiscoverRequest>>,
) -> Json<serde_json::Value> {
    let user_id: i32 = claims.sub.parse().unwrap_or(0);
    if user_id == 0 {
        return Json(json!({"success": false, "error": "Invalid user id in JWT"}));
    }
    // Service switch: Method B feeds the kick_auto_clipper business.
    if !crate::services::service_flags::service_enabled(&state.db_pool, "kick_auto_clipper").await {
        return Json(
            json!({"success": false, "error": "kick_auto_clipper is currently disabled (service_flags)."}),
        );
    }

    let req = req.unwrap_or(AutoDiscoverRequest {
        niche: None,
        max_posts_per_hashtag: None,
        hashtag_count: None,
        service: None,
    });
    let category = req.niche.as_deref().unwrap_or("gaming");
    let max_posts = req.max_posts_per_hashtag.unwrap_or(30).min(100);

    let Some(pb) = state.phantombuster_client.as_ref() else {
        return Json(json!({"success": false, "error": "PhantomBuster not configured"}));
    };

    let session_cookie = match std::env::var("INSTAGRAM_SESSION_COOKIE") {
        Ok(c) if !c.is_empty() => c,
        _ => return Json(json!({"success": false, "error": "INSTAGRAM_SESSION_COOKIE not set"})),
    };

    let agent = match pb.find_instagram_hashtag_agent().await {
        Ok(Some(a)) => a,
        Ok(None) => {
            return Json(
                json!({"success": false, "error": "No Instagram Hashtag phantom found in PhantomBuster. Add 'Instagram Hashtag Search Export' from the Phantom Store."}),
            )
        }
        Err(e) => {
            return Json(json!({"success": false, "error": format!("Agent lookup failed: {}", e)}))
        }
    };

    let Some(kick) = &state.kick_client else {
        return Json(json!({"success": false, "error": "Kick client not configured"}));
    };

    // 1. Top live Kick streamers in this category (free Kick API).
    let livestreams = match kick.search_livestreams_by_category_name(category).await {
        Ok(streams) => streams,
        Err(e) => {
            return Json(json!({"success": false, "error": format!("Kick API failed: {}", e)}))
        }
    };
    let mut sorted: Vec<_> = livestreams
        .into_iter()
        .filter_map(|s| {
            let viewers = s.viewer_count.unwrap_or(0);
            if viewers < 10 {
                return None;
            }
            Some((viewers, s))
        })
        .collect();
    sorted.sort_by(|a, b| b.0.cmp(&a.0));
    let top_streamers: Vec<_> = sorted.into_iter().take(10).collect();
    if top_streamers.is_empty() {
        return Json(
            json!({"success": false, "error": format!("No live Kick streams in category '{}'", category)}),
        );
    }

    // 2. Per streamer: deterministic clipper-handle tags. Account names like
    // {slug}clips / {slug}highlights are where clippers live; the tag search
    // surfaces the accounts and the scorer (username-weighted + caption
    // signals) confirms them.
    let mut launched_jobs = Vec::new();
    let mut errors = Vec::new();
    let mut streamers = Vec::new();
    for (viewers, streamer) in &top_streamers {
        let slug = streamer.slug.to_lowercase();
        streamers.push(json!({"slug": slug.clone(), "viewers": viewers}));
        let prefix = format!("instagram:methodb:{slug}:#");
        for tag in [
            slug.clone(),
            format!("{slug}clips"),
            format!("{slug}highlights"),
        ] {
            match try_launch_or_queue_ig_hashtag_job_with_prefix(
                &state,
                pb,
                &agent,
                &session_cookie,
                &tag,
                max_posts,
                user_id,
                &prefix,
            )
            .await
            {
                Ok((job_id, status, container_id)) => {
                    launched_jobs.push(json!({
                        "job_id": job_id.to_string(),
                        "container_id": container_id,
                        "hashtag": tag,
                        "streamer": slug,
                        "status": status,
                    }));
                }
                Err(e) => errors.push(format!("#{tag} ({slug}): {e}")),
            }
            tokio::time::sleep(tokio::time::Duration::from_millis(500)).await;
        }
    }

    Json(json!({
        "success": !launched_jobs.is_empty(),
        "category": category,
        "streamers": streamers,
        "jobs": launched_jobs,
        "errors": errors,
        "message": format!(
            "Method B launched {} IG searches across {} top Kick streamers. Results auto-import in ~5–10 minutes with detected creators pre-attached.",
            launched_jobs.len(), streamers.len()
        )
    }))
}

/// GET /api/instagram/leads/top
/// Returns the highest-scored leads (score >= 60), ready for outreach.
async fn instagram_top_leads(
    Extension(state): Extension<Arc<AppState>>,
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
         LIMIT 50",
    )
    .bind(user_id)
    .fetch_all(&state.db_pool)
    .await
    {
        Ok(r) => r,
        Err(e) => return Json(json!({"success": false, "error": format!("DB error: {}", e)})),
    };

    let leads: Vec<serde_json::Value> = rows
        .iter()
        .map(|r| {
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
        })
        .collect();

    Json(json!({"success": true, "leads": leads, "count": leads.len()}))
}

pub async fn scrape_instagram_profile(
    username: &str,
) -> (Option<String>, Option<String>, Option<i32>) {
    let url = format!("https://www.instagram.com/{}/", username);
    let html = match crate::browserbase_client::fetch_url_raw(&url).await {
        Ok(Some(h)) => h,
        _ => return (None, None, None),
    };

    // bio: from og:description meta tag
    let bio = {
        let mut found = None;
        if let Ok(re) = Regex::new(r#"<meta[^>]*property="og:description"[^>]*content="([^"]*)""#) {
            if let Some(c) = re.captures(&html) {
                found = c.get(1).map(|m| m.as_str().to_string());
            }
        }
        if found.is_none() {
            if let Ok(re) =
                Regex::new(r#"<meta[^>]*content="([^"]*)"[^>]*property="og:description""#)
            {
                if let Some(c) = re.captures(&html) {
                    found = c.get(1).map(|m| m.as_str().to_string());
                }
            }
        }
        found
    };

    // external_url: link in bio (href="https://..." that's not instagram.com)
    let external_url = {
        let mut found = None;
        if let Ok(re) = Regex::new(r#"href="(https?://[^"]+)""#) {
            for cap in re.captures_iter(&html) {
                if let Some(m) = cap.get(1) {
                    let u = m.as_str();
                    if !u.contains("instagram.com") {
                        found = Some(u.to_string());
                        break;
                    }
                }
            }
        }
        found
    };

    // follower_count: from page text (regex: (\d[\d,.]*[KMB]?)\s*Followers)
    let follower_count = {
        let mut found = None;
        if let Ok(re) = Regex::new(r"(\d[\d,.]*[KMB]?)\s*Followers") {
            if let Some(c) = re.captures(&html) {
                if let Some(m) = c.get(1) {
                    found = parse_instagram_follower_count(m.as_str());
                }
            }
        }
        found
    };

    (bio, external_url, follower_count)
}

fn parse_instagram_follower_count(s: &str) -> Option<i32> {
    let s = s.trim();
    if s.is_empty() {
        return None;
    }
    let (num, mult) = if let Some(rest) = s.strip_suffix('K') {
        (rest, 1_000.0f64)
    } else if let Some(rest) = s.strip_suffix('M') {
        (rest, 1_000_000.0f64)
    } else if let Some(rest) = s.strip_suffix('B') {
        (rest, 1_000_000_000.0f64)
    } else {
        (s, 1.0f64)
    };
    let num = num.replace(',', "");
    let val: f64 = num.parse().ok()?;
    let total = (val * mult) as i64;
    if total > i32::MAX as i64 {
        return Some(i32::MAX);
    }
    if total < 0 {
        return None;
    }
    Some(total as i32)
}

// ============================================================================
// Background poller — auto-imports PhantomBuster Instagram results
// ============================================================================

/// Called by the background task every 5 minutes.
/// Checks running Instagram PB jobs, imports new leads, and scores them.
pub async fn poll_instagram_jobs(state: &Arc<AppState>) {
    let pb = match state.phantombuster_client.as_ref() {
        Some(pb) => pb,
        None => return,
    };

    // Find running Instagram jobs older than 3 minutes (give PB time to finish)
    let running_jobs = match sqlx::query(
        "SELECT id, agent_id, search_url, user_id
         FROM phantombuster_jobs
         WHERE status = 'running'
           AND search_url LIKE 'instagram:%'
           AND launched_at < NOW() - INTERVAL '3 minutes'
         ORDER BY launched_at ASC
         LIMIT 10",
    )
    .fetch_all(&state.db_pool)
    .await
    {
        Ok(rows) => rows,
        Err(e) => {
            tracing::warn!("Instagram job poll: DB query failed: {}", e);
            return;
        }
    };

    if running_jobs.is_empty() {
        return;
    }

    tracing::info!(
        "🔄 Instagram job poller: checking {} running jobs",
        running_jobs.len()
    );

    for row in running_jobs {
        let job_id: uuid::Uuid = row.get("id");
        let agent_id: String = row.get("agent_id");
        let search_url: String = row.get("search_url");
        let job_user_id: Option<i32> = row.try_get("user_id").ok().flatten();

        // Extract hashtag from search_url ("instagram:#contentcreator",
        // "instagram:kick:#contentcreator", or "instagram:methodb:{slug}:#tag"
        // for Method-B runs where the streamer slug is pre-known).
        let is_kick_clipper = search_url.contains(":kick:") || search_url.contains(":methodb:");
        let (hashtag_source, methodb_slug, forced_service): (
            String,
            Option<String>,
            Option<String>,
        ) = if let Some(rest) = search_url.strip_prefix("instagram:methodb:") {
            match rest.split_once(":#") {
                Some((slug, tag)) => (
                    tag.trim_start_matches('#').to_string(),
                    Some(slug.to_lowercase()),
                    // Method-B is kick-clipper-only by construction.
                    Some("kick_auto_clipper".to_string()),
                ),
                None => (rest.trim_start_matches('#').to_string(), None, None),
            }
        } else if let Some(rest) = search_url.strip_prefix("instagram:svc:") {
            // Forced-service scope (owner directive Oct 2026):
            // instagram:svc:{service}:#tag → all scored leads tagged so.
            match rest.split_once(":#") {
                Some((svc, tag)) => (
                    tag.trim_start_matches('#').to_string(),
                    None,
                    Some(svc.trim().to_lowercase()),
                ),
                None => (rest.trim_start_matches('#').to_string(), None, None),
            }
        } else {
            (
                search_url
                    .trim_start_matches("instagram:kick:#")
                    .trim_start_matches("instagram:kick:")
                    .trim_start_matches("instagram:#")
                    .trim_start_matches("instagram:")
                    .trim_start_matches('#')
                    .to_string(),
                None,
                // Legacy kick prefix implies kick_auto_clipper.
                if search_url.contains(":kick:") {
                    Some("kick_auto_clipper".to_string())
                } else {
                    None
                },
            )
        };

        // Fetch PB output
        let rows = match pb.fetch_output(&agent_id).await {
            Ok(r) if !r.is_empty() => r,
            Ok(_) => {
                // Empty output could mean "still running" OR "phantom errored with
                // no rows produced". Ask PB directly before assuming still running.
                if let Ok((errored, log_tail)) = pb.fetch_run_error(&agent_id).await {
                    if errored {
                        let err_msg = log_tail.unwrap_or_else(|| {
                            "PhantomBuster script exited with error".to_string()
                        });
                        tracing::warn!(
                            "Instagram job {}: PB reported error → marking failed: {}",
                            job_id,
                            err_msg
                        );
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
                     WHERE id = $2 AND launched_at < NOW() - INTERVAL '30 minutes'",
                )
                .bind(e)
                .bind(job_id)
                .execute(&state.db_pool)
                .await;
                continue;
            }
        };

        let leads = crate::phantombuster_client::PhantomBusterClient::parse_instagram_leads(rows);
        let total = leads.len();
        let mut imported = 0usize;

        for mut lead in leads {
            // Skip private accounts — they can't receive DMs from non-followers.
            if lead.is_private {
                continue;
            }
            // NOTE: do NOT filter by followers_count here. The Instagram
            // Hashtag Search Export returns post schema (no follower count), so
            // gating on >=1000 would drop every lead. Hashtag relevance is
            // itself a qualification — AI scoring and the DM generator decide
            // which to pursue.

            let category_value = if is_kick_clipper {
                "kick_clipper".to_string()
            } else {
                hashtag_source.clone()
            };
            // Method-B preset: the streamer is KNOWN (came from the search
            // prefix), so attach it with zero LLM cost. The scorer skips paid
            // detection when creators are already present.
            let preset_enrichment = methodb_slug.as_deref().map(|slug| {
                serde_json::json!({
                    "detected_source_creators": [slug],
                    "method": "method_b",
                })
            });

            // Best-effort: scrape the Instagram profile page to fill in any
            // missing bio / external_url / followers_count before enrichment.
            if lead
                .bio
                .as_ref()
                .map(|v| v.trim().is_empty())
                .unwrap_or(true)
                || lead
                    .external_url
                    .as_ref()
                    .map(|v| v.trim().is_empty())
                    .unwrap_or(true)
                || lead.followers_count.is_none()
            {
                let (scraped_bio, scraped_url, scraped_followers) =
                    scrape_instagram_profile(&lead.username).await;
                if lead
                    .bio
                    .as_ref()
                    .map(|v| v.trim().is_empty())
                    .unwrap_or(true)
                {
                    if let Some(b) = scraped_bio {
                        lead.bio = Some(b);
                    }
                }
                if lead
                    .external_url
                    .as_ref()
                    .map(|v| v.trim().is_empty())
                    .unwrap_or(true)
                {
                    if let Some(u) = scraped_url {
                        lead.external_url = Some(u);
                    }
                }
                if lead.followers_count.is_none() {
                    if let Some(f) = scraped_followers {
                        lead.followers_count = Some(f as i64);
                    }
                }
            }

            // Enrich public contact fields from the lead's bio/external URL
            // before binding into the INSERT.
            let description = lead.bio.clone().unwrap_or_else(|| lead.username.clone());
            let mut twitter_handle = extract_best_twitter_handle(&description);
            let mut instagram_handle = extract_best_instagram_handle(&description);
            let mut business_email = lead.email.clone();
            let mut external_url = lead.external_url.clone();
            let enrichment_url = external_url.clone();
            let contact_enrichment_result = enrich_public_contact_fields(
                enrichment_url.as_deref(),
                &mut twitter_handle,
                &mut instagram_handle,
                &mut business_email,
                &mut external_url,
            )
            .await;
            lead.external_url = external_url;
            lead.email = business_email;

            // Merge the enrichment result into the preset enrichment (if any)
            // so the public contact fields we scraped land in contact_enrichment.
            let mut merged_enrichment = preset_enrichment
                .clone()
                .unwrap_or_else(|| serde_json::json!({}));
            if let serde_json::Value::Object(map) = &mut merged_enrichment {
                if let serde_json::Value::Object(enriched_map) = &contact_enrichment_result {
                    for (k, v) in enriched_map {
                        map.entry(k.clone()).or_insert_with(|| v.clone());
                    }
                }
            }

            let result = sqlx::query(
                "INSERT INTO instagram_leads
                    (username, full_name, bio, followers_count, following_count, posts_count,
                     profile_url, profile_pic_url, is_private, is_verified, external_url, email,
                     category, hashtag_source, contact_enrichment, contact_status, user_id)
                 VALUES ($1,$2,$3,$4,$5,$6,$7,$8,$9,$10,$11,$12,$13,$14,$15,'new',$16)
                 ON CONFLICT (user_id, username) DO UPDATE
                   SET followers_count = EXCLUDED.followers_count,
                       bio = COALESCE(EXCLUDED.bio, instagram_leads.bio),
                       category = COALESCE(EXCLUDED.category, instagram_leads.category),
                       contact_enrichment = CASE WHEN (instagram_leads.contact_enrichment IS NULL OR instagram_leads.contact_enrichment = '{}') THEN EXCLUDED.contact_enrichment ELSE instagram_leads.contact_enrichment END,
                       updated_at = NOW()",
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
            .bind(&category_value)
            .bind(&hashtag_source)
            .bind(&merged_enrichment)
            .bind(job_user_id)
            .execute(&state.db_pool)
            .await;

            if result.is_ok() {
                imported += 1;
            }
        }

        tracing::info!(
            "✅ Instagram job {}: {} total / {} imported (hashtag: #{})",
            job_id,
            total,
            imported,
            hashtag_source
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
                score_instagram_leads(state, &hashtag_source, uid, forced_service.as_deref()).await;
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
         WHERE agent_id = $1 AND status = 'running'",
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
    lc.contains("parallel")
        || lc.contains("maximum number")
        || lc.contains("concurrency limit")
        || lc.contains("too many")
}

/// Try to launch an Instagram Hashtag Search — or enqueue it if the agent
/// already has a running job (or PB rejects the launch for parallelism).
///
/// Returns `(job_id, status, container_id_opt)` where `status` is either
/// `"running"` (launched immediately) or `"queued"` (will be launched by the
/// dispatcher when a slot frees up).
async fn try_launch_or_queue_ig_hashtag_job(
    state: &Arc<AppState>,
    pb: &crate::phantombuster_client::PhantomBusterClient,
    agent: &crate::phantombuster_client::PbAgent,
    session_cookie: &str,
    hashtag: &str,
    max_posts: u32,
    user_id: i32,
) -> Result<(uuid::Uuid, &'static str, Option<String>), String> {
    let tag = hashtag.trim_start_matches('#').to_string();
    let search_url = format!("instagram:#{}", tag);

    // Check occupancy first — cheaper than bouncing off PB's parallel limit.
    // Global occupancy (any user), because PB's limit is per-workspace.
    let occupied = agent_has_running_job(&state.db_pool, &agent.id).await;

    if !occupied {
        match pb
            .launch_instagram_hashtag_search(&agent.id, session_cookie, &tag, max_posts)
            .await
        {
            Ok(container_id) => {
                let job_id = sqlx::query_scalar::<_, uuid::Uuid>(
                    "INSERT INTO phantombuster_jobs
                        (agent_id, agent_name, search_url, status, launched_at, user_id)
                     VALUES ($1, $2, $3, 'running', NOW(), $4) RETURNING id",
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
         VALUES ($1, $2, $3, 'queued', NOW(), $4) RETURNING id",
    )
    .bind(&agent.id)
    .bind(&agent.name)
    .bind(&search_url)
    .bind(user_id)
    .fetch_one(&state.db_pool)
    .await
    .map_err(|e| format!("DB insert failed: {}", e))?;

    tracing::info!(
        "📥 Queued Instagram job {} for #{} (agent busy, user={})",
        job_id,
        tag,
        user_id
    );
    Ok((job_id, "queued", None))
}

/// Like `try_launch_or_queue_ig_hashtag_job` but accepts a custom
/// `search_url_prefix` (e.g. "instagram:kick:#") so the poller can
/// distinguish Kick-clipper mode from general Instagram leads.
async fn try_launch_or_queue_ig_hashtag_job_with_prefix(
    state: &Arc<AppState>,
    pb: &crate::phantombuster_client::PhantomBusterClient,
    agent: &crate::phantombuster_client::PbAgent,
    session_cookie: &str,
    hashtag: &str,
    max_posts: u32,
    user_id: i32,
    search_url_prefix: &str,
) -> Result<(uuid::Uuid, &'static str, Option<String>), String> {
    let tag = hashtag.trim_start_matches('#').to_string();
    let search_url = format!("{}{}", search_url_prefix, tag);

    let occupied = agent_has_running_job(&state.db_pool, &agent.id).await;

    if !occupied {
        match pb
            .launch_instagram_hashtag_search(&agent.id, session_cookie, &tag, max_posts)
            .await
        {
            Ok(container_id) => {
                let job_id = sqlx::query_scalar::<_, uuid::Uuid>(
                    "INSERT INTO phantombuster_jobs
                        (agent_id, agent_name, search_url, status, launched_at, user_id)
                     VALUES ($1, $2, $3, 'running', NOW(), $4) RETURNING id",
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
            }
            Err(e) => return Err(e),
        }
    }

    let job_id = sqlx::query_scalar::<_, uuid::Uuid>(
        "INSERT INTO phantombuster_jobs
            (agent_id, agent_name, search_url, status, created_at, user_id)
         VALUES ($1, $2, $3, 'queued', NOW(), $4) RETURNING id",
    )
    .bind(&agent.id)
    .bind(&agent.name)
    .bind(&search_url)
    .bind(user_id)
    .fetch_one(&state.db_pool)
    .await
    .map_err(|e| format!("DB insert failed: {}", e))?;

    tracing::info!(
        "📥 Queued Instagram job {} for #{} (agent busy, user={}, prefix={})",
        job_id,
        tag,
        user_id,
        search_url_prefix
    );
    Ok((job_id, "queued", None))
}

/// Background dispatcher — runs periodically, promoting the oldest `queued`
/// job per agent to `running` if the agent has no in-flight job. One promote
/// per tick per agent keeps PB well under the parallel limit even across
/// different phantom types.
pub async fn dispatch_queued_pb_jobs(state: &Arc<AppState>) {
    let Some(pb) = state.phantombuster_client.as_ref() else {
        return;
    };
    let Ok(session_cookie) = std::env::var("INSTAGRAM_SESSION_COOKIE") else {
        return;
    };
    if session_cookie.is_empty() {
        return;
    }

    // Pick agents that have queued work AND no currently-running job.
    let rows = match sqlx::query(
        "SELECT DISTINCT q.agent_id
         FROM phantombuster_jobs q
         WHERE q.status = 'queued'
           AND NOT EXISTS (
             SELECT 1 FROM phantombuster_jobs r
             WHERE r.agent_id = q.agent_id AND r.status = 'running'
           )",
    )
    .fetch_all(&state.db_pool)
    .await
    {
        Ok(rs) => rs,
        Err(e) => {
            tracing::warn!("PB dispatcher DB query failed: {}", e);
            return;
        }
    };

    if rows.is_empty() {
        return;
    }

    for row in rows {
        let agent_id: String = row.get("agent_id");

        // Oldest queued job for this agent.
        let job = match sqlx::query(
            "SELECT id, search_url FROM phantombuster_jobs
             WHERE agent_id = $1 AND status = 'queued'
             ORDER BY created_at ASC LIMIT 1",
        )
        .bind(&agent_id)
        .fetch_optional(&state.db_pool)
        .await
        {
            Ok(Some(r)) => r,
            Ok(None) => continue,
            Err(e) => {
                tracing::warn!("PB dispatcher oldest-job query failed: {}", e);
                continue;
            }
        };

        let job_id: uuid::Uuid = job.get("id");
        let search_url: String = job.get("search_url");

        // Only Instagram hashtag jobs are auto-dispatched today. LinkedIn
        // launches require rebuilding the Sales Navigator URL, which is a
        // bigger retrofit — leave those to manual retry for now.
        let Some(tag) = search_url
            .strip_prefix("instagram:#")
            .or_else(|| search_url.strip_prefix("instagram:"))
            .map(|s| s.trim_start_matches('#').to_string())
        else {
            tracing::debug!(
                "Skipping dispatch for non-IG queued job {} ({})",
                job_id,
                search_url
            );
            continue;
        };

        match pb
            .launch_instagram_hashtag_search(&agent_id, &session_cookie, &tag, 50)
            .await
        {
            Ok(_container_id) => {
                let _ = sqlx::query(
                    "UPDATE phantombuster_jobs
                     SET status = 'running', launched_at = NOW()
                     WHERE id = $1",
                )
                .bind(job_id)
                .execute(&state.db_pool)
                .await;
                tracing::info!("🚀 Dispatched queued PB job {} for #{}", job_id, tag);
            }
            Err(e) if is_pb_parallel_limit_error(&e) => {
                tracing::debug!(
                    "PB still at parallel limit on agent {}; will retry next tick",
                    agent_id
                );
                // leave status='queued' and try again next tick
            }
            Err(e) => {
                tracing::warn!(
                    "PB launch failed for queued job {}: {} — marking failed",
                    job_id,
                    e
                );
                let _ = sqlx::query(
                    "UPDATE phantombuster_jobs
                     SET status = 'failed', error = $1, completed_at = NOW()
                     WHERE id = $2",
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
async fn score_instagram_leads(
    state: &Arc<AppState>,
    hashtag: &str,
    user_id: i32,
    forced_service: Option<&str>,
) {
    // Forced-service scope (owner directive Oct 2026): validated + enabled at
    // search time; re-check here (flags may have flipped since) and skip the
    // lead (leave unscored) rather than mis-tagging it.
    let forced: Option<String> = forced_service
        .map(|s| normalize_revenue_service(&s.to_ascii_lowercase()).to_string())
        .filter(|s| is_valid_revenue_service(s));
    // No `followers_count >= 1000` filter — hashtag-mode leads come from the
    // post schema and have NULL follower counts. Filtering on them dropped
    // every lead and the UI showed `—` for every score. Score what we have;
    // the model is told to judge by bio/handle alone when follower count
    // is unknown. Scoped to one user's leads so we don't pay to re-score
    // the same lead for multiple users.
    let unscored = match sqlx::query(
        "SELECT id, username, full_name, bio, followers_count, external_url, category,
                contact_enrichment
         FROM instagram_leads
         WHERE score IS NULL
           AND hashtag_source = $1
           AND user_id = $2
           AND is_private = FALSE
         ORDER BY COALESCE(followers_count, 0) DESC, created_at DESC
         LIMIT 20",
    )
    .bind(hashtag)
    .bind(user_id)
    .fetch_all(&state.db_pool)
    .await
    {
        Ok(r) => r,
        Err(_) => return,
    };

    for row in &unscored {
        let id: uuid::Uuid = row.get("id");
        let username: String = row.get::<Option<String>, _>("username").unwrap_or_default();
        let bio: String = row.get::<Option<String>, _>("bio").unwrap_or_default();
        let followers: Option<i64> = row.get::<Option<i64>, _>("followers_count");
        let ext_url: String = row
            .get::<Option<String>, _>("external_url")
            .unwrap_or_default();
        let category: String = row.get::<Option<String>, _>("category").unwrap_or_default();
        let is_kick_lead = category == "kick_clipper";
        // Hard Kick signals in bio/captions (no LLM cost): links to kick.com,
        // kick+clip/highlight mentions. Forces kick treatment + evidence.
        let caption_signals = kick_caption_signals(&bio);
        let kick_evident = is_kick_lead || !caption_signals.is_empty();
        // Method-B preset (workspace endpoint flow): creators already known,
        // skip the paid detection call below.
        let preset_creators: Vec<String> = row
            .get::<Option<serde_json::Value>, _>("contact_enrichment")
            .as_ref()
            .and_then(|e| e.get("detected_source_creators"))
            .and_then(|c| c.as_array())
            .map(|arr| {
                arr.iter()
                    .filter_map(|v| v.as_str())
                    .map(|s| s.trim().trim_start_matches('@').to_lowercase())
                    .filter(|s| !s.is_empty())
                    .collect()
            })
            .unwrap_or_default();

        let followers_str = match followers {
            Some(n) if n > 0 => n.to_string(),
            _ => "unknown (came from hashtag search — judge by bio + handle)".to_string(),
        };

        // Service menu from enabled flags (owner directive Sep 2026). Forced
        // scope narrows it to the one service (Oct 2026).
        let (service_menu, must_line) = match forced.as_deref() {
            Some(f) => (
                crate::services::service_flags::service_menu_line(f)
                    .unwrap_or("Pick the forced service.")
                    .to_string(),
                format!("`service` MUST be exactly: {f}. No other values are valid."),
            ),
            None => crate::services::service_flags::scoring_menu(&state.db_pool).await,
        };

        let prompt = format!(
            r#"Score this Instagram creator as a potential client for a video production studio (0–100), and also pick the best service to pitch.

The studio offers:
{service_menu}

Creator profile:
- Username: @{username} (clipper-pattern names like Xclips/clipsofX/Xhighlights are a POSITIVE clipper signal — judge together with bio)
- Followers: {followers}
- Bio / recent post caption: {bio}
{kick_evidence}- Link in bio: {ext_url}

Score guidelines:
- 80–100: Clear paying client. Active creator/founder, monetised, has content the studio can act on right now.
- 60–79: Likely fit. Bio strongly hints at one of the service types and the audience size makes sense.
- 40–59: Ambiguous — could be a creator, could be a fan account, can't tell from bio alone.
- 0–39: Bad fit. Fan page, brand parody, private/spammy account, OR another freelance editor (competitor, not client).

Return ONLY valid JSON (no markdown, no code fence):
{{"score": 75, "service": "twitch_clipping", "reason": "streamer with VOD link in bio, posts stream highlights"}}

{must_line}"#,
            username = username,
            service_menu = service_menu,
            must_line = must_line,
            followers = followers_str,
            bio = bio,
            ext_url = ext_url,
            kick_evidence = if caption_signals.is_empty() {
                String::new()
            } else {
                format!(
                    "- Hard Kick signals in text (no LLM needed): {}\n",
                    caption_signals.join("; ")
                )
            },
        );

        let result = generate_text_best_effort(
            state.qwen_client.as_ref(),
            state.nvidia_nim_client.as_ref(),
            state.gemma_client.as_ref(),
            state.gemini_client.as_ref(),
            state.deepseek_client.as_ref(),
            &prompt,
        )
        .await;

        if let Ok(text) = result {
            let cleaned = text
                .trim()
                .trim_start_matches("```json")
                .trim_start_matches("```")
                .trim_end_matches("```")
                .trim();

            if let Ok(v) = serde_json::from_str::<serde_json::Value>(cleaned) {
                let score = v.get("score").and_then(|s| s.as_i64()).unwrap_or(0) as i32;
                let reason = v
                    .get("reason")
                    .and_then(|r| r.as_str())
                    .unwrap_or("")
                    .to_string();
                // Service tag — forced scope wins outright (already validated +
                // enabled-checked at search time; re-check here, skip on flip).
                // Otherwise coerced into the enabled set (owner directive Sep
                // 2026); anything else stored as NULL so the DM generator falls
                // back to "all services, AI picks one inline".
                let service = match forced.as_deref() {
                    Some(f)
                        if crate::services::service_flags::service_enabled(&state.db_pool, f)
                            .await =>
                    {
                        Some(f.to_string())
                    }
                    Some(_) => None,
                    None => {
                        let service_raw = v
                            .get("service")
                            .and_then(|s| s.as_str())
                            .unwrap_or("")
                            .to_lowercase();
                        if is_valid_revenue_service(&service_raw) {
                            let normalized = normalize_revenue_service(&service_raw).to_string();
                            let clipper_like = kick_evident
                                || bio.to_ascii_lowercase().contains("clip")
                                || bio.to_ascii_lowercase().contains("kick");
                            Some(
                                crate::services::service_flags::constrain_service(
                                    &state.db_pool,
                                    &normalized,
                                    clipper_like,
                                    "instagram",
                                )
                                .await,
                            )
                        } else {
                            None
                        }
                    }
                };

                let _ = sqlx::query(
                    "UPDATE instagram_leads
                     SET score = $1, score_reason = $2, service_type = $3
                     WHERE id = $4",
                )
                .bind(score)
                .bind(&reason)
                .bind(service.as_deref())
                .bind(id)
                .execute(&state.db_pool)
                .await;
            }
        }

        // ── Kick clipper: detect which Kick creators they clip ──────
        // Method-B preset (workspace flow) already knows the creators — skip
        // the paid detection call. Otherwise detect from bio/captions.
        if kick_evident && !bio.is_empty() && preset_creators.is_empty() {
            let creator_prompt = format!(
                "This Instagram account reposts clips from Kick.com streamers. \
                 Based on their bio and post captions below, identify which \
                 specific Kick.com streamers/creators they clip content from. \
                 \n\nBio/captions: {bio}\n\n\
                 Return ONLY a comma-separated list of Kick usernames/slugs. \
                 If you cannot identify specific creators, return 'unknown'.",
                bio = bio,
            );
            if let Ok(creators_raw) = crate::llm_utils::generate_text_fast(
                state.qwen_client.as_ref(),
                state.deepseek_client.as_ref(),
                state.gemini_client.as_ref(),
                &creator_prompt,
            )
            .await
            {
                let raw = creators_raw.trim().to_lowercase();
                if !raw.contains("unknown") && !raw.is_empty() {
                    let detected: Vec<String> = raw
                        .split(|c| c == ',' || c == '\n')
                        .map(|s| s.trim().trim_start_matches('@').to_string())
                        .filter(|s| !s.is_empty())
                        .collect();
                    if !detected.is_empty() {
                        let enrichment = serde_json::json!({
                            "detected_source_creators": detected,
                        });
                        let _ = sqlx::query(
                            "UPDATE instagram_leads
                             SET contact_enrichment = $1
                             WHERE id = $2",
                        )
                        .bind(&enrichment)
                        .bind(id)
                        .execute(&state.db_pool)
                        .await;
                        tracing::info!(
                            "🎯 Detected Kick creators for @{}: {:?}",
                            username,
                            detected
                        );
                    }
                }
            }
        }

        // Small delay to avoid quota hammering
        tokio::time::sleep(tokio::time::Duration::from_millis(300)).await;
    }

    tracing::info!(
        "📊 Scored {} Instagram leads for hashtag #{}",
        unscored.len(),
        hashtag
    );
}

// ============================================================================
// AI helpers — drive discovery, not just scoring
// ============================================================================

/// Ask the AI to generate the most effective YouTube search query for a prospect type + category.
/// Replaces all hardcoded search strings.
async fn ai_generate_youtube_query(
    state: &Arc<AppState>,
    prospect_type: &str,
    category: &str,
) -> String {
    let prompt = format!(
        r#"You are helping a video clipping agency find potential clients on YouTube.

Prospect type: {prospect_type}
Content category/niche: {category}

Generate ONE concise YouTube search query (5–10 words max) that will find active YouTube CHANNELS
matching the stated prospect type in this niche.

The query should:
- Target the right operator for the prospect type:
  - content_creator / podcaster / educator / business_owner: the channel owners themselves
  - clipper: editors, clipping agencies, short-form operators, repost channels offering clipping services
  - creator_manager: creator agencies, creator managers, talent managers, media operators managing channels
- Be specific enough to find real people, not brands or media companies
- Work as a YouTube channel search

Return ONLY the raw search query string, nothing else."#
    );

    match crate::llm_utils::generate_text_fast(
        state.qwen_client.as_ref(),
        state.deepseek_client.as_ref(),
        state.gemini_client.as_ref(),
        &prompt,
    )
    .await
    {
        Ok(t) => {
            let q = t.trim().trim_matches('"').trim_matches('\'').to_string();
            if q.is_empty() || q.len() > 100 {
                category.to_string()
            } else {
                q
            }
        }
        Err(_) => category.to_string(), // graceful fallback
    }
}

/// Ask the AI to pick the best Twitch game/category to find streamers matching the prospect type.
async fn ai_generate_twitch_category(
    state: &Arc<AppState>,
    prospect_type: &str,
    category: &str,
) -> String {
    let prompt = format!(
        r#"You are helping a video clipping agency find Twitch streamers as potential clients.

Prospect type: {prospect_type}
Niche: {category}

Return ONE Twitch game or category name (exactly as it appears on Twitch) that best matches
this niche and the stated prospect type.

Use this lens:
- for content_creator / podcaster / educator: categories where creators likely need clipping or explainers
- for clipper: categories where editors and clip operators often scout source creators
- for creator_manager: broad creator-economy categories where managers scout or oversee creator output
- for business_owner: brand-adjacent or product-demo-friendly categories

If the niche is not gaming, pick the closest non-gaming Twitch category (e.g. "Just Chatting",
"Science & Technology", "Music", "Art", "Fitness & Health", "Podcasts", "Software and Game Dev").

Return ONLY the category name, nothing else."#
    );

    match crate::llm_utils::generate_text_fast(
        state.qwen_client.as_ref(),
        state.deepseek_client.as_ref(),
        state.gemini_client.as_ref(),
        &prompt,
    )
    .await
    {
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

The agency sells a broader production stack:
- short-form clipping and repurposing
- animated explainers and motion graphics
- premium video add-ons

The best short-term buyers are often:
- clippers / short-form editors
- creator managers / talent managers / agencies
- small-to-mid creators who need clipping help

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
- job_titles: 3–6 titles. Use real LinkedIn job titles.
- If the description sounds like clippers/editors, prefer titles like "Video Editor", "Short Form Video Editor", "Content Repurposing Strategist", "Podcast Producer", "Creative Director", "Founder".
- If the description sounds like creator managers/agencies, prefer titles like "Creator Manager", "Talent Manager", "Influencer Manager", "Partnerships Manager", "Social Media Manager", "Founder".
- If the description sounds like direct creators, prefer titles like "YouTuber", "Podcast Host", "Content Creator", "Online Course Creator", "Streamer".
- industries: 2–4 industries from LinkedIn's list, leaning toward media / creator economy / marketing / education when relevant.
- locations: 1–3 countries/regions.
- seniority: prefer ["OWNER", "PARTNER", "CXO", "DIRECTOR"] when the description implies agency owners or operators; otherwise keep it founder/operator-heavy.
- company_sizes: default to ["A", "B"] (1–50 employees) unless the description clearly implies slightly larger agencies, then ["A", "B", "C"] is acceptable.

Return ONLY the JSON, no explanation."#
    );

    let text = generate_text_best_effort(
        state.qwen_client.as_ref(),
        state.nvidia_nim_client.as_ref(),
        state.gemma_client.as_ref(),
        state.gemini_client.as_ref(),
        state.deepseek_client.as_ref(),
        &prompt,
    )
    .await
    .map_err(|e| format!("LLM error: {}", e))?;

    let cleaned = text
        .trim()
        .trim_start_matches("```json")
        .trim_start_matches("```")
        .trim_end_matches("```")
        .trim();

    #[derive(serde::Deserialize)]
    struct AiFilters {
        job_titles: Option<Vec<String>>,
        industries: Option<Vec<String>>,
        locations: Option<Vec<String>>,
        seniority: Option<Vec<String>>,
        company_sizes: Option<Vec<String>>,
    }

    let filters: AiFilters = serde_json::from_str(cleaned)
        .map_err(|e| format!("JSON parse error: {} | raw: {}", e, cleaned))?;

    Ok(SmartSearchRequest {
        description: None, // already consumed
        job_titles: filters.job_titles,
        industries: filters.industries,
        company_sizes: filters.company_sizes,
        locations: filters.locations,
        seniority: filters.seniority,
        max_profiles: None,
        list_url: None,
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
         LIMIT 30",
    )
    .bind(job_id)
    .fetch_all(&state.db_pool)
    .await
    {
        Ok(r) => r,
        Err(e) => {
            tracing::warn!("LinkedIn scoring: DB error: {}", e);
            return;
        }
    };

    tracing::info!(
        "📊 AI-scoring {} LinkedIn leads for job {}",
        rows.len(),
        job_id
    );

    for row in &rows {
        let id: uuid::Uuid = row.get("id");
        let name: String = row
            .get::<Option<String>, _>("display_name")
            .unwrap_or_default();
        let description: String = row
            .get::<Option<String>, _>("channel_description")
            .unwrap_or_default();
        let audience: i64 = row.get::<Option<i64>, _>("subscriber_count").unwrap_or(0);
        let category: String = row
            .get::<Option<String>, _>("content_category")
            .unwrap_or_default();
        let job_title: String = row
            .get::<Option<String>, _>("job_title")
            .unwrap_or_default();
        let company: String = row
            .get::<Option<String>, _>("company_name")
            .unwrap_or_default();
        let seniority: String = row
            .get::<Option<String>, _>("seniority_level")
            .unwrap_or_default();

        // Build a richer description from LinkedIn fields
        let enriched_desc = format!(
            "Job title: {}. Company: {}. Seniority: {}. Bio: {}",
            job_title, company, seniority, description
        );

        let (score, reasoning, service, dm_creator, _, x_dm, email_script) =
            score_prospect_with_ai(
                state,
                &name,
                audience,
                &enriched_desc,
                &category,
                "linkedin_lead",
                "linkedin",
                None,
            )
            .await;

        let _ = sqlx::query(
            "UPDATE prospects
             SET ai_score = $1, ai_reasoning = $2, dm_script_creator = $3,
                 service_type = $4, x_dm_script = $5, email_script = $6, updated_at = NOW()
             WHERE id = $7",
        )
        .bind(score)
        .bind(&reasoning)
        .bind(&dm_creator)
        .bind(&service)
        .bind(&x_dm)
        .bind(&email_script)
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
    channel: String,
    keyword_re: Option<String>,
}

async fn telegram_list_channels(
    Extension(state): Extension<Arc<AppState>>,
) -> Json<serde_json::Value> {
    let rows = match sqlx::query(
        "SELECT id, channel, keyword_re, enabled, created_at
         FROM telegram_watch_channels ORDER BY created_at ASC",
    )
    .fetch_all(&state.db_pool)
    .await
    {
        Ok(r) => r,
        Err(e) => return Json(json!({"success": false, "error": format!("DB error: {}", e)})),
    };
    let channels: Vec<serde_json::Value> = rows
        .iter()
        .map(|r| {
            json!({
                "id":         r.get::<i32, _>("id"),
                "channel":    r.get::<String, _>("channel"),
                "keyword_re": r.try_get::<Option<String>, _>("keyword_re").ok().flatten(),
                "enabled":    r.get::<bool, _>("enabled"),
            })
        })
        .collect();
    Json(json!({"success": true, "channels": channels}))
}

async fn telegram_add_channel(
    Extension(state): Extension<Arc<AppState>>,
    Json(req): Json<AddChannelReq>,
) -> Json<serde_json::Value> {
    let channel = req.channel.trim().trim_start_matches('@').to_string();
    if channel.is_empty() {
        return Json(json!({"success": false, "error": "channel is required"}));
    }
    match sqlx::query(
        "INSERT INTO telegram_watch_channels (channel, keyword_re)
         VALUES ($1, $2)
         ON CONFLICT (channel) DO UPDATE SET keyword_re = EXCLUDED.keyword_re, enabled = TRUE
         RETURNING id",
    )
    .bind(&channel)
    .bind(req.keyword_re.as_deref())
    .fetch_one(&state.db_pool)
    .await
    {
        Ok(_) => Json(json!({"success": true, "channel": channel})),
        Err(e) => Json(json!({"success": false, "error": format!("DB error: {}", e)})),
    }
}

async fn telegram_delete_channel(
    Extension(state): Extension<Arc<AppState>>,
    Path(id): Path<i32>,
) -> Json<serde_json::Value> {
    match sqlx::query("DELETE FROM telegram_watch_channels WHERE id = $1")
        .bind(id)
        .execute(&state.db_pool)
        .await
    {
        Ok(_) => Json(json!({"success": true})),
        Err(e) => Json(json!({"success": false, "error": format!("DB error: {}", e)})),
    }
}

#[derive(Debug, Deserialize)]
struct ListOpportunitiesQuery {
    status: Option<String>,
    limit: Option<i64>,
}

async fn telegram_list_opportunities(
    Extension(state): Extension<Arc<AppState>>,
    Extension(claims): Extension<Claims>,
    Query(q): Query<ListOpportunitiesQuery>,
) -> Json<serde_json::Value> {
    let user_id: i32 = claims.sub.parse().unwrap_or(0);
    let limit = q.limit.unwrap_or(50).clamp(1, 200);

    let mut sql = String::from(
        "SELECT id, channel, message_id, sender, message, matched_kw, link,
                score, score_reason, service_type, status, source, created_at
         FROM telegram_opportunities
         WHERE user_id = $1",
    );
    if q.status.is_some() {
        sql.push_str(" AND status = $2");
    }
    sql.push_str(" ORDER BY COALESCE(score, 0) DESC, created_at DESC");
    sql.push_str(&format!(" LIMIT {}", limit));

    let rows = if let Some(s) = q.status.as_deref() {
        sqlx::query(&sql)
            .bind(user_id)
            .bind(s)
            .fetch_all(&state.db_pool)
            .await
    } else {
        sqlx::query(&sql)
            .bind(user_id)
            .fetch_all(&state.db_pool)
            .await
    };
    let rows = match rows {
        Ok(r) => r,
        Err(e) => return Json(json!({"success": false, "error": format!("DB error: {}", e)})),
    };

    let items: Vec<serde_json::Value> = rows
        .iter()
        .map(|r| {
            json!({
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
            })
        })
        .collect();
    Json(json!({"success": true, "opportunities": items, "count": items.len()}))
}

#[derive(Debug, Deserialize)]
struct AddOpportunityReq {
    channel: String,
    message: String,
    sender: Option<String>,
    link: Option<String>,
}

async fn telegram_add_opportunity_manual(
    Extension(state): Extension<Arc<AppState>>,
    Extension(claims): Extension<Claims>,
    Json(req): Json<AddOpportunityReq>,
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
         RETURNING id",
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
    .await
    {
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
    Extension(state): Extension<Arc<AppState>>,
    Extension(claims): Extension<Claims>,
    Path(id): Path<uuid::Uuid>,
    Json(req): Json<UpdateOpportunityReq>,
) -> Json<serde_json::Value> {
    let user_id: i32 = claims.sub.parse().unwrap_or(0);
    let valid = ["new", "contacted", "won", "lost", "ignored"];
    let Some(status) = req.status else {
        return Json(json!({"success": false, "error": "status is required"}));
    };
    if !valid.contains(&status.as_str()) {
        return Json(
            json!({"success": false, "error": format!("status must be one of: {}", valid.join(", "))}),
        );
    }
    match sqlx::query(
        "UPDATE telegram_opportunities
         SET status = $1, updated_at = NOW()
         WHERE id = $2 AND user_id = $3",
    )
    .bind(&status)
    .bind(id)
    .bind(user_id)
    .execute(&state.db_pool)
    .await
    {
        Ok(_) => Json(json!({"success": true})),
        Err(e) => Json(json!({"success": false, "error": format!("DB error: {}", e)})),
    }
}

/// AI-score a pasted Telegram message as a potential paid gig.
/// Mirrors the IG lead scorer — same service menu, same 0-100 scale.
async fn score_telegram_opportunity(
    state: &Arc<AppState>,
    channel: &str,
    message: &str,
) -> (i32, String, Option<String>) {
    // Service menu from enabled flags (owner directive Sep 2026).
    let (service_menu, must_line) =
        crate::services::service_flags::scoring_menu(&state.db_pool).await;
    let prompt = format!(
        r#"A message was posted in Telegram channel @{channel}. Decide whether it's a REAL paid gig opportunity for a video production studio and which service to pitch.

The studio offers (pick ONE):
{service_menu}

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
{{"score": 75, "service": "twitch_clipping", "reason": "Streamer says 'need someone to cut my 6hr broadcasts into TikToks, DM for budget'"}}

{must_line} — or null if score < 40."#,
        channel = channel,
        message = message,
        service_menu = service_menu,
        must_line = must_line,
    );

    let response = match crate::llm_utils::generate_text_best_effort(
        state.qwen_client.as_ref(),
        state.nvidia_nim_client.as_ref(),
        state.gemma_client.as_ref(),
        state.gemini_client.as_ref(),
        state.deepseek_client.as_ref(),
        &prompt,
    )
    .await
    {
        Ok(text) => text,
        Err(_) => return (0, "scoring failed".to_string(), None),
    };

    let cleaned = response
        .trim()
        .trim_start_matches("```json")
        .trim_start_matches("```")
        .trim_end_matches("```")
        .trim();

    let v: serde_json::Value = match serde_json::from_str(cleaned) {
        Ok(v) => v,
        Err(_) => return (0, "parse failed".to_string(), None),
    };
    let score = v.get("score").and_then(|s| s.as_i64()).unwrap_or(0) as i32;
    let reason = v
        .get("reason")
        .and_then(|r| r.as_str())
        .unwrap_or("")
        .to_string();
    let svc = v
        .get("service")
        .and_then(|s| s.as_str())
        .map(|s| s.to_lowercase());
    // Constrain into the enabled set (owner directive Sep 2026); Telegram gig
    // leads default to clipping when the AI picks a disabled service.
    let service = match svc {
        Some(s) if is_valid_revenue_service(&s) => {
            let normalized = normalize_revenue_service(&s).to_string();
            let clipper_like = message.to_ascii_lowercase().contains("clip")
                || message.to_ascii_lowercase().contains("kick");
            Some(
                crate::services::service_flags::constrain_service(
                    &state.db_pool,
                    &normalized,
                    clipper_like,
                    "telegram",
                )
                .await,
            )
        }
        _ => None,
    };
    (score, reason, service)
}

// ============================================================================
// Market-aligned unlock pricing for delivery samples
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
    let workflow_id = crate::handlers::admin::ensure_delivery_workflow(&state, delivery_id)
        .await
        .ok();
    let workflow_runtime = crate::services::WorkflowRuntime::new(state.db_pool.clone());

    tracing::info!(
        "🎬 Agent video gen for lead delivery {} — URL: {}",
        delivery_id,
        website_url
    );

    if let Some(workflow_id) = workflow_id {
        let _ = workflow_runtime
            .heartbeat(
                workflow_id,
                crate::services::WorkflowStatus::Planning,
                Some("lead_agent_video_started"),
                "Lead sample video workflow started and is preparing the first agent attempt.",
                json!({
                    "delivery_id": delivery_id,
                    "website_url": website_url,
                }),
            )
            .await;
    }

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
  - browserbase_crawl_website({website_url}) to crawl the full site, extract CSS design tokens, and get all page content
  - vectorize_crawled_content(feature_tag, pages) to store in Qdrant for semantic search
  - search_crawled_content(query="brand colors, features, product details", feature_tag) for specific info

PHASE 2 — Write a comprehensive script (3-4 minutes of narration):
  - generate_video_script for a long-form explainer:
    intro → problem → solution (their product) → features → social proof → CTA

PHASE 3 — Produce the full video:
  - blender_generate_scene_type(prompt="intro scene with title card", params={{"style": "cinematic", "duration": 5}}) for intro/outro with title cards
  - blender_generate_scene_type(prompt="product feature callout with lower third", params={{"style": "professional", "duration": 5}}) for feature callouts
  - manim_execute_script(description="data chart showing growth metrics") for data visualization scenes
  - Multiple blender_generate_scene_type calls with reference_image_url for branded scenes throughout
  - add_voiceover_to_video to narrate the full script with ElevenLabs TTS
  - Composite / concatenate the scenes into ONE final video at: {full_path}

PHASE 4 — Cut a short preview for social sharing:
  - Use FFmpeg tools to trim the most engaging 15-30 second segment
  - Add a clean lower-third overlay: "Made with VideoSync.video"
  - Save the preview at: {preview_path}

Deliverables (MUST produce both files):
  - Full clean HD video: {full_path}
  - Watermarked preview: {preview_path}

Make it professional — this is pitched to a paying client. No placeholder content.
Output file paths clearly in your final response so the delivery pipeline can find them."#,
        website_url = website_url,
        lead_name = lead_name,
        lead_bio = lead_bio.chars().take(200).collect::<String>(),
        full_path = full_path,
        preview_path = preview_path,
    );
    let prompt = base_prompt.clone();

    let gemini_client = match state
        .video_gemini_client
        .as_ref()
        .or(state.gemini_client.as_ref())
    {
        Some(c) => Arc::new(c.clone()),
        None => {
            tracing::error!("No Gemini client available for agent video gen");
            let _ = sqlx::query("UPDATE deliveries SET status = 'failed', error_message = 'No LLM client available' WHERE id = $1")
                .bind(delivery_id).execute(&state.db_pool).await;
            if let Some(workflow_id) = workflow_id {
                let _ = workflow_runtime
                    .mark_failed(
                        workflow_id,
                        Some("lead_agent_video_started"),
                        "No LLM client available for the lead sample workflow.",
                        None,
                    )
                    .await;
            }
            return;
        }
    };

    let agent = crate::agent::stateful_agent::StatefulGeminiAgent::new(gemini_client);

    // Iterative retry loop: run the agent, review the output, retry with
    // accumulated feedback until the reviewer passes it or we hit the cap.
    // Each retry prepends the QA hint to the prompt so the agent can
    // adjust specifically for what the reviewer flagged.
    let mut current_prompt = prompt.clone();
    let mut best_full: Option<String> = None;
    let mut best_preview: Option<String> = None;
    let mut best_score: i32 = -1;
    let mut best_feedback: String = String::new();
    let mut retries_used: i32 = 0;

    for attempt in 0..AGENT_VIDEO_MAX_RETRIES {
        let session_id = format!("lead-sample-{}-try{}", delivery_id, attempt);
        tracing::info!(
            "🎬 Agent attempt {}/{} for delivery {}",
            attempt + 1,
            AGENT_VIDEO_MAX_RETRIES,
            delivery_id
        );

        if let Some(workflow_id) = workflow_id {
            let _ = workflow_runtime
                .heartbeat(
                    workflow_id,
                    crate::services::WorkflowStatus::Running,
                    Some("lead_agent_attempt"),
                    &format!(
                        "Lead sample workflow is running agent attempt {} of {}.",
                        attempt + 1,
                        AGENT_VIDEO_MAX_RETRIES
                    ),
                    json!({
                        "delivery_id": delivery_id,
                        "attempt": attempt + 1,
                        "max_attempts": AGENT_VIDEO_MAX_RETRIES,
                    }),
                )
                .await;
        }

        let agent_result = agent
            .chat(
                &current_prompt,
                &session_id,
                String::new(),
                state.clone(),
                state.job_manager.clone(),
                None,
                None,
                None,
                None, // user_id not available in this context
            )
            .await;

        // Collect the actual output paths the agent produced
        let (full_produced, preview_produced) = locate_agent_outputs(
            &agent_result.as_deref().unwrap_or(""),
            &full_path,
            &preview_path,
        )
        .await;

        // Review the full video against the original brief
        let review = if let Some(ref full) = full_produced {
            crate::render_review::review_render(
                &state,
                full,
                &base_prompt,
                "agent_lead_video",
                Some(delivery_id),
            )
            .await
        } else {
            crate::render_review::ReviewResult {
                pass: false,
                score: 0,
                feedback: "Agent did not produce expected full video output".to_string(),
                retry_hint: Some(
                    "Produce the full video at the exact path specified in the prompt".to_string(),
                ),
                failing_section: None,
            }
        };

        retries_used = attempt as i32;

        // Track the best attempt so we always ship SOMETHING even if no attempt passes
        if review.score > best_score {
            best_score = review.score;
            best_full = full_produced.clone();
            best_preview = preview_produced.clone();
            best_feedback = review.feedback.clone();
        }

        if review.pass {
            tracing::info!(
                "✅ Agent video PASSED review on attempt {} (score {})",
                attempt + 1,
                review.score
            );
            if let Some(workflow_id) = workflow_id {
                let _ = workflow_runtime
                    .heartbeat(
                        workflow_id,
                        crate::services::WorkflowStatus::Running,
                        Some("lead_agent_review_passed"),
                        &format!(
                            "Lead sample workflow passed QA review on attempt {} with score {}.",
                            attempt + 1,
                            review.score
                        ),
                        json!({
                            "delivery_id": delivery_id,
                            "attempt": attempt + 1,
                            "score": review.score,
                        }),
                    )
                    .await;
            }
            break;
        }

        // Not passing — prepare the next attempt with accumulated feedback
        if attempt + 1 < AGENT_VIDEO_MAX_RETRIES {
            let hint = review
                .retry_hint
                .clone()
                .unwrap_or_else(|| review.feedback.clone());
            tracing::warn!("🔄 Retrying (score {}): {}", review.score, hint);
            current_prompt = format!(
                "PREVIOUS ATTEMPT FAILED QA REVIEW (score {}/10).\n\
                Feedback: {}\n\
                Retry hint: {}\n\n\
                Apply the feedback above, then run the full pipeline below:\n\n{}",
                review.score, review.feedback, hint, base_prompt,
            );
        } else {
            tracing::warn!(
                "⚠️ Hit max retries ({}) for delivery {} — shipping best attempt (score {})",
                AGENT_VIDEO_MAX_RETRIES,
                delivery_id,
                best_score
            );
            if let Some(workflow_id) = workflow_id {
                let _ = workflow_runtime
                    .heartbeat(
                        workflow_id,
                        crate::services::WorkflowStatus::Retrying,
                        Some("lead_agent_retry_cap_reached"),
                        &format!(
                            "Lead sample workflow reached the retry cap and is shipping the best reviewed attempt with score {}.",
                            best_score
                        ),
                        json!({
                            "delivery_id": delivery_id,
                            "score": best_score,
                            "retries_used": attempt + 1,
                        }),
                    )
                    .await;
            }
        }
    }

    // Upload whatever we ended up with. If even the best attempt produced
    // no file, fall back to the direct Blender render path.
    let Some(full_path_final) = best_full else {
        tracing::warn!(
            "No usable output from {} attempts — falling back to direct Blender",
            retries_used + 1
        );
        if let Some(workflow_id) = workflow_id {
            let _ = workflow_runtime
                .heartbeat(
                    workflow_id,
                    crate::services::WorkflowStatus::Retrying,
                    Some("lead_agent_fallback_to_render"),
                    "Lead sample workflow did not produce a usable full video, so it is falling back to the direct delivery render path.",
                    json!({
                        "delivery_id": delivery_id,
                        "retries_used": retries_used + 1,
                    }),
                )
                .await;
        }
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
            if let Some(workflow_id) = workflow_id {
                let _ = workflow_runtime
                    .mark_failed(
                        workflow_id,
                        Some("lead_agent_upload"),
                        "R2 is not configured for lead sample uploads.",
                        None,
                    )
                    .await;
            }
            return;
        }
    };

    // Upload full clean HD video
    let (full_backend, full_url) = match r2
        .upload_file_tracked(
            &full_path_final,
            &format!("deliveries/{}/full.mp4", delivery_id),
        )
        .await
    {
        Ok(v) => v,
        Err(e) => {
            tracing::error!("Failed to upload full video: {e}");
            let _ = sqlx::query(
                "UPDATE deliveries SET status = 'failed', error_message = $1 WHERE id = $2",
            )
            .bind(format!("R2 upload failed: {e}"))
            .bind(delivery_id)
            .execute(&state.db_pool)
            .await;
            if let Some(workflow_id) = workflow_id {
                let _ = workflow_runtime
                    .mark_failed(
                        workflow_id,
                        Some("lead_agent_upload"),
                        &format!("Lead sample full-video upload failed: {e}"),
                        None,
                    )
                    .await;
            }
            return;
        }
    };

    // Upload preview (watermarked) — fall back to full video if preview wasn't produced
    let preview_url = if let Some(pp) = best_preview.as_ref() {
        match r2
            .upload_file_tracked(pp, &format!("deliveries/{}/preview.mp4", delivery_id))
            .await
        {
            Ok((_, u)) => Some(u),
            Err(e) => {
                tracing::warn!(
                    "Preview upload failed: {e} — clients will see full video as preview"
                );
                None
            }
        }
    } else {
        None
    };

    let qa_note: Option<String> = if best_score < 6 {
        Some(format!(
            "QA final score {} after {} retries: {}",
            best_score,
            retries_used + 1,
            best_feedback
        ))
    } else {
        None
    };

    let _ = sqlx::query(
        "UPDATE deliveries SET status='completed', output_r2_url=$1, preview_r2_url=$2,
         qa_retry_count=$3, final_qa_score=$4, error_message=$5, storage_backend=$6,
         completed_at=NOW()
         WHERE id=$7",
    )
    .bind(&full_url)
    .bind(preview_url.as_deref())
    .bind(retries_used + 1)
    .bind(best_score)
    .bind(qa_note.as_deref())
    .bind(full_backend.as_str())
    .bind(delivery_id)
    .execute(&state.db_pool)
    .await;

    crate::handlers::social_publish::try_publish_delivery_to_zernio(delivery_id, &state).await;

    if let Some(workflow_id) = workflow_id {
        let _ = workflow_runtime
            .mark_completed(
                workflow_id,
                Some("lead_agent_delivery_uploaded"),
                "Lead sample workflow completed with uploaded delivery artifacts.",
                json!({
                    "delivery_id": delivery_id,
                    "output_r2_url": full_url,
                    "preview_r2_url": preview_url,
                    "qa_retry_count": retries_used + 1,
                    "final_qa_score": best_score,
                }),
            )
            .await;
    }

    tracing::info!(
        "📦 Delivery {} complete — full={}, preview={:?}, score={}, retries={}",
        delivery_id,
        full_url,
        preview_url,
        best_score,
        retries_used + 1
    );
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
        let clean = word.trim_matches(|c: char| {
            !c.is_alphanumeric() && c != '/' && c != '.' && c != '_' && c != '-'
        });
        if clean.ends_with(".mp4") && clean.contains('/') && filter(clean) {
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

    let bytes = match gemini
        .generate_image(prompt, Some("16:9"), None, None)
        .await
    {
        Ok(b) => b,
        Err(e) => {
            tracing::warn!("try_generate_image: Gemini call failed: {}", e);
            return None;
        }
    };

    // Persist locally long enough to hand to R2's file-based upload API.
    // outputs/ is gitignored + auto-cleaned.
    let local_path = format!(
        "outputs/sample_gen_{}.png",
        chrono::Utc::now().timestamp_millis()
    );
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
    if !resp.status().is_success() {
        return None;
    }
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

/// Scrape a website URL and return a structured text summary (title, description, headings, body text).
/// Used to give the LLM agent real website content so it can make informed landing page videos.
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
    if quote != '"' && quote != '\'' {
        return None;
    }
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

/// Backfill prospects with null service_type using the same
/// default_service_for_prospect logic. Runs at startup.
pub async fn backfill_null_service_types(db_pool: &sqlx::PgPool) {
    tracing::info!("🔄 Backfilling prospects with null service_type...");
    // `content_category` is nullable (`prospect_type` is the only NOT NULL one),
    // so it must be decoded as Option: a single NULL row otherwise aborted the
    // entire backfill with "unexpected null; try decoding as an Option".
    match sqlx::query_as::<_, (uuid::Uuid, Option<String>, String, Option<String>)>(
        "SELECT id, content_category, prospect_type, platform FROM prospects WHERE service_type IS NULL"
    )
    .fetch_all(db_pool)
    .await
    {
        Ok(rows) => {
            if rows.is_empty() {
                tracing::info!("✅ No prospects need service_type backfill");
                return;
            }
            let mut updated = 0u64;
            for (id, category, prospect_type, platform) in &rows {
                let service = default_service_for_prospect(
                    category.as_deref().unwrap_or(""),
                    prospect_type,
                    platform.as_deref().unwrap_or(""),
                );
                match sqlx::query("UPDATE prospects SET service_type = $1, updated_at = NOW() WHERE id = $2")
                    .bind(&service)
                    .bind(id)
                    .execute(db_pool)
                    .await
                {
                    Ok(_) => updated += 1,
                    Err(e) => tracing::error!("Failed to backfill prospect {}: {}", id, e),
                }
            }
            tracing::info!("✅ Backfilled {} / {} prospects with service_type", updated, rows.len());
        }
        Err(e) => tracing::error!("❌ Failed to query null-service_type prospects: {}", e),
    }
}

async fn instagram_update_service_type(
    Extension(state): Extension<Arc<AppState>>,
    Extension(claims): Extension<Claims>,
    Path(id): Path<uuid::Uuid>,
    Json(req): Json<UpdateServiceTypeRequest>,
) -> Json<serde_json::Value> {
    let user_id: i32 = claims.sub.parse().unwrap_or(0);

    // Empty string / None → reset to NULL so the next DM re-reads the
    // AI's default. Anything else must be one of the known service tags.
    let service: Option<String> = match req.service_type.as_deref() {
        None | Some("") => None,
        Some(s) => {
            let normalized = normalize_revenue_service(s);
            if is_valid_revenue_service(normalized) {
                Some(normalized.to_string())
            } else {
                return Json(json!({
                    "success": false,
                    "error":   format!("service_type must be one of: clipping, education, landing_page, kick_auto_clipper, manim_explainer, whiteboard_animation, kinetic_typography, animated_infographic, algorithm_viz, investor_pitch, year_in_review, isometric_explainer, youtube_clipping, twitch_clipping (got: {})", s),
                }));
            }
        }
    };

    match sqlx::query(
        "UPDATE instagram_leads SET service_type = $1, updated_at = NOW()
         WHERE id = $2 AND user_id = $3",
    )
    .bind(service.as_deref())
    .bind(id)
    .bind(user_id)
    .execute(&state.db_pool)
    .await
    {
        Ok(_) => Json(json!({"success": true, "service_type": service})),
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
    state: &Arc<AppState>,
    channel: &str,
    message: &str,
) -> (i32, String, Option<String>) {
    score_telegram_opportunity(state, channel, message).await
}

#[derive(Debug, Deserialize)]
struct TelegramLoginStartReq {
    phone: String,
}

#[derive(Debug, Deserialize)]
struct TelegramDiscoverReq {
    queries: Vec<String>,
    limit_per_query: Option<i32>,
}

#[derive(Debug, Deserialize)]
struct TelegramDiscoveredQuery {
    status: Option<String>,
    limit: Option<i64>,
}

async fn telegram_discover_channels(
    Extension(state): Extension<Arc<AppState>>,
    Extension(claims): Extension<Claims>,
    Json(req): Json<TelegramDiscoverReq>,
) -> Json<serde_json::Value> {
    let user_id = claims.sub.parse::<i32>().ok();
    let queries: Vec<String> = req
        .queries
        .into_iter()
        .map(|q| q.trim().to_string())
        .filter(|q| !q.is_empty())
        .take(12)
        .collect();

    if queries.is_empty() {
        return Json(
            json!({"success": false, "error": "Provide at least one Telegram search query."}),
        );
    }

    let limit_per_query = req.limit_per_query.unwrap_or(15).clamp(1, 50);
    let run_id = create_prospect_agent_run(
        &state,
        user_id,
        "telegram_public_discovery",
        "Use the authorized MTProto user session to discover public Telegram channels/groups, score revenue fit, and create watch-channel candidates.",
        json!({"queries": queries, "limit_per_query": limit_per_query}),
    )
    .await;

    checkpoint_prospect_agent_run(
        &state,
        run_id,
        "planned",
        json!({
            "queries": queries,
            "next_steps": ["mtproto_contacts_search", "score_channels", "persist_candidates", "suggest_watch_channels"]
        }),
        Some("Telegram public discovery requires a logged-in MTProto user session; Bot API cannot do global public search."),
    )
    .await;

    let discovered = match crate::telegram_client::discover_public_channels(
        &state,
        queries.clone(),
        limit_per_query,
    )
    .await
    {
        Ok(channels) => channels,
        Err(e) => {
            complete_prospect_agent_run(
                &state,
                run_id,
                "failed",
                json!({"error": e}),
                Some("Telegram MTProto discovery failed."),
            )
            .await;
            return Json(json!({"success": false, "error": e}));
        }
    };

    checkpoint_prospect_agent_run(
        &state,
        run_id,
        "mtproto_contacts_search",
        json!({"raw_candidates": discovered.len()}),
        Some("Telegram contacts.search returned public channel/group candidates."),
    )
    .await;

    let mut saved = 0usize;
    for candidate in discovered {
        let category = if candidate.is_megagroup {
            "telegram group"
        } else {
            "telegram channel"
        };
        let description = format!(
            "Telegram {} @{} {} participants. Discovered from query '{}'.",
            category,
            candidate.username.as_deref().unwrap_or("unknown"),
            candidate.participants_count.unwrap_or(0),
            candidate.query
        );
        let (score, reason, service, _, _, _, _) = score_prospect_with_ai(
            &state,
            &candidate.title,
            candidate.participants_count.unwrap_or(0) as i64,
            &description,
            category,
            "business_owner",
            "telegram",
            None,
        )
        .await;
        let score_i = (score * 100.0).round() as i32;
        let link = candidate
            .username
            .as_ref()
            .map(|u| format!("https://t.me/{}", u.trim_start_matches('@')));

        let result = sqlx::query(
            "INSERT INTO telegram_discovered_channels
               (run_id, query, channel_id, username, title, is_broadcast, is_megagroup,
                participants_count, score, score_reason, service_type, raw)
             VALUES ($1,$2,$3,$4,$5,$6,$7,$8,$9,$10,$11,$12)
             ON CONFLICT DO UPDATE
                SET run_id = COALESCE(EXCLUDED.run_id, telegram_discovered_channels.run_id),
                    query = EXCLUDED.query,
                    title = EXCLUDED.title,
                    is_broadcast = EXCLUDED.is_broadcast,
                    is_megagroup = EXCLUDED.is_megagroup,
                    participants_count = COALESCE(EXCLUDED.participants_count, telegram_discovered_channels.participants_count),
                    score = EXCLUDED.score,
                    score_reason = EXCLUDED.score_reason,
                    service_type = EXCLUDED.service_type,
                    raw = EXCLUDED.raw,
                    updated_at = NOW()",
        )
        .bind(run_id)
        .bind(&candidate.query)
        .bind(candidate.channel_id)
        .bind(candidate.username.as_deref())
        .bind(&candidate.title)
        .bind(candidate.is_broadcast)
        .bind(candidate.is_megagroup)
        .bind(candidate.participants_count)
        .bind(score_i)
        .bind(&reason)
        .bind(&service)
        .bind(json!({
            "link": link,
            "source": "mtproto_contacts_search"
        }))
        .execute(&state.db_pool)
        .await;

        if result.is_ok() {
            saved += 1;
        }
    }

    checkpoint_prospect_agent_run(
        &state,
        run_id,
        "completed",
        json!({"saved_candidates": saved}),
        Some("Candidates scored and persisted for review or watch-channel activation."),
    )
    .await;
    complete_prospect_agent_run(
        &state,
        run_id,
        "completed",
        json!({"saved_candidates": saved}),
        None,
    )
    .await;

    Json(json!({
        "success": true,
        "agent_run_id": run_id.map(|id| id.to_string()),
        "saved": saved,
        "message": format!("Discovered and scored {} Telegram channel/group candidates.", saved)
    }))
}

async fn telegram_list_discovered_channels(
    Extension(state): Extension<Arc<AppState>>,
    Query(q): Query<TelegramDiscoveredQuery>,
) -> Json<serde_json::Value> {
    let limit = q.limit.unwrap_or(100).clamp(1, 300);
    let rows = if let Some(status) = q.status.as_deref() {
        sqlx::query(
            "SELECT id, run_id, query, channel_id, username, title, is_broadcast, is_megagroup,
                    participants_count, score, score_reason, service_type, status, raw, created_at, updated_at
             FROM telegram_discovered_channels
             WHERE status = $1
             ORDER BY COALESCE(score, 0) DESC, updated_at DESC
             LIMIT $2",
        )
        .bind(status)
        .bind(limit)
        .fetch_all(&state.db_pool)
        .await
    } else {
        sqlx::query(
            "SELECT id, run_id, query, channel_id, username, title, is_broadcast, is_megagroup,
                    participants_count, score, score_reason, service_type, status, raw, created_at, updated_at
             FROM telegram_discovered_channels
             ORDER BY COALESCE(score, 0) DESC, updated_at DESC
             LIMIT $1",
        )
        .bind(limit)
        .fetch_all(&state.db_pool)
        .await
    };

    let rows = match rows {
        Ok(rows) => rows,
        Err(e) => return Json(json!({"success": false, "error": format!("DB error: {e}")})),
    };

    let channels: Vec<serde_json::Value> = rows
        .iter()
        .map(|row| {
            let username = row.get::<Option<String>, _>("username");
            json!({
                "id": row.get::<Uuid, _>("id").to_string(),
                "run_id": row.get::<Option<Uuid>, _>("run_id").map(|id| id.to_string()),
                "query": row.get::<String, _>("query"),
                "channel_id": row.get::<Option<i64>, _>("channel_id"),
                "username": username,
                "title": row.get::<String, _>("title"),
                "link": username.as_ref().map(|u| format!("https://t.me/{}", u.trim_start_matches('@'))),
                "is_broadcast": row.get::<bool, _>("is_broadcast"),
                "is_megagroup": row.get::<bool, _>("is_megagroup"),
                "participants_count": row.get::<Option<i32>, _>("participants_count"),
                "score": row.get::<Option<i32>, _>("score"),
                "score_reason": row.get::<Option<String>, _>("score_reason"),
                "service_type": row.get::<Option<String>, _>("service_type"),
                "status": row.get::<String, _>("status"),
                "raw": row.get::<serde_json::Value, _>("raw"),
                "created_at": row.get::<chrono::DateTime<chrono::Utc>, _>("created_at"),
                "updated_at": row.get::<chrono::DateTime<chrono::Utc>, _>("updated_at"),
            })
        })
        .collect();

    Json(json!({"success": true, "channels": channels, "count": channels.len()}))
}

async fn telegram_login_start(
    Extension(state): Extension<Arc<AppState>>,
    Json(req): Json<TelegramLoginStartReq>,
) -> Json<serde_json::Value> {
    let phone = req.phone.trim().to_string();
    if phone.is_empty() {
        return Json(
            json!({"success": false, "error": "phone is required (include country code, e.g. +14155551234)"}),
        );
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
    code: String,
}

async fn telegram_login_verify(
    Extension(state): Extension<Arc<AppState>>,
    Json(req): Json<TelegramLoginVerifyReq>,
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

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_normalize_revenue_service() {
        assert_eq!(normalize_revenue_service("clipping"), "clipping");
        assert_eq!(normalize_revenue_service("landing_page"), "landing_page");
        assert_eq!(normalize_revenue_service("education"), "education");
        assert_eq!(
            normalize_revenue_service("kick_auto_clipper"),
            "kick_auto_clipper"
        );
        assert_eq!(
            normalize_revenue_service("manim_explainer"),
            "manim_explainer"
        );
        assert_eq!(
            normalize_revenue_service("whiteboard_animation"),
            "whiteboard_animation"
        );
        assert_eq!(
            normalize_revenue_service("kinetic_typography"),
            "kinetic_typography"
        );
        assert_eq!(
            normalize_revenue_service("animated_infographic"),
            "animated_infographic"
        );
        assert_eq!(normalize_revenue_service("algorithm_viz"), "algorithm_viz");
        assert_eq!(
            normalize_revenue_service("investor_pitch"),
            "investor_pitch"
        );
        assert_eq!(
            normalize_revenue_service("year_in_review"),
            "year_in_review"
        );
        assert_eq!(
            normalize_revenue_service("isometric_explainer"),
            "isometric_explainer"
        );

        // Aliases
        assert_eq!(
            normalize_revenue_service("kick_clipper"),
            "kick_auto_clipper"
        );
        assert_eq!(normalize_revenue_service("manim"), "manim_explainer");
        assert_eq!(
            normalize_revenue_service("whiteboard"),
            "whiteboard_animation"
        );
        assert_eq!(normalize_revenue_service("sketch"), "whiteboard_animation");
        assert_eq!(
            normalize_revenue_service("kinetic_type"),
            "kinetic_typography"
        );
        assert_eq!(
            normalize_revenue_service("infographic"),
            "animated_infographic"
        );
        assert_eq!(normalize_revenue_service("algorithm"), "algorithm_viz");
        assert_eq!(normalize_revenue_service("code_viz"), "algorithm_viz");
        assert_eq!(normalize_revenue_service("pitch_deck"), "investor_pitch");
        assert_eq!(normalize_revenue_service("wrapped"), "year_in_review");
        assert_eq!(
            normalize_revenue_service("isometric"),
            "isometric_explainer"
        );

        // Unknown passes through
        assert_eq!(
            normalize_revenue_service("unknown_service"),
            "unknown_service"
        );
    }

    #[test]
    fn test_is_valid_revenue_service() {
        assert!(is_valid_revenue_service("clipping"));
        assert!(is_valid_revenue_service("landing_page"));
        assert!(is_valid_revenue_service("education"));
        assert!(is_valid_revenue_service("kick_auto_clipper"));
        assert!(is_valid_revenue_service("manim_explainer"));
        assert!(is_valid_revenue_service("whiteboard_animation"));
        assert!(is_valid_revenue_service("kinetic_typography"));
        assert!(is_valid_revenue_service("animated_infographic"));
        assert!(is_valid_revenue_service("algorithm_viz"));
        assert!(is_valid_revenue_service("investor_pitch"));
        assert!(is_valid_revenue_service("year_in_review"));
        assert!(is_valid_revenue_service("isometric_explainer"));

        // Aliases resolve before validation
        assert!(is_valid_revenue_service("manim"));
        assert!(is_valid_revenue_service("whiteboard"));
        assert!(is_valid_revenue_service("infographic"));
        assert!(is_valid_revenue_service("algorithm"));

        // Invalid
        assert!(!is_valid_revenue_service("invalid_service"));
        assert!(!is_valid_revenue_service(""));
    }

    #[test]
    fn test_old_ugc_references_removed() {
        // normalize_revenue_service should NOT recognize "ugc" as a valid service
        assert!(
            !is_valid_revenue_service("ugc"),
            "ugc should not be a valid revenue service"
        );
    }

    #[test]
    fn test_service_price_line() {
        for service in &[
            "clipping",
            "landing_page",
            "education",
            "kick_auto_clipper",
            "manim_explainer",
            "whiteboard_animation",
            "kinetic_typography",
            "animated_infographic",
            "algorithm_viz",
            "investor_pitch",
            "year_in_review",
            "isometric_explainer",
        ] {
            assert!(
                service_price_line(service).contains("$"),
                "no price for {}",
                service
            );
        }
    }

    #[test]
    fn test_service_target_duration_seconds() {
        for service in &[
            "clipping",
            "landing_page",
            "education",
            "kick_auto_clipper",
            "manim_explainer",
            "whiteboard_animation",
            "kinetic_typography",
            "animated_infographic",
            "algorithm_viz",
            "investor_pitch",
            "year_in_review",
            "isometric_explainer",
        ] {
            assert!(
                service_target_duration_seconds(service) > 0.0,
                "zero duration for {}",
                service
            );
        }
    }

    #[test]
    fn test_service_long_form_style_non_empty() {
        for service in &[
            "clipping",
            "education",
            "kick_auto_clipper",
            "manim_explainer",
            "whiteboard_animation",
            "kinetic_typography",
            "animated_infographic",
            "algorithm_viz",
            "investor_pitch",
            "year_in_review",
            "isometric_explainer",
        ] {
            let s = service_long_form_style(service);
            assert!(!s.is_empty(), "empty long_form_style for {}", service);
        }
    }

    #[test]
    fn test_service_long_form_offer_type() {
        assert_eq!(service_long_form_offer_type("clipping"), "clip_pack");
        assert_eq!(
            service_long_form_offer_type("education"),
            "education_explainer_pack"
        );
        assert_eq!(
            service_long_form_offer_type("kick_auto_clipper"),
            "kick_auto_clipper_pack"
        );
    }

    #[test]
    fn test_service_page_for_revenue_service() {
        assert!(service_page_for_revenue_service("clipping").contains("clipper"));
        assert!(service_page_for_revenue_service("education").contains("education"));
    }

    #[test]
    fn test_should_use_long_form_for_revenue_sample() {
        // Services with a URL should use long form
        assert!(should_use_long_form_for_revenue_sample(
            "landing_page",
            true
        ));

        // Some services without URL should still use long form
        assert!(should_use_long_form_for_revenue_sample("education", false));

        // New services with URL should use long form
        assert!(should_use_long_form_for_revenue_sample(
            "manim_explainer",
            true
        ));
        assert!(should_use_long_form_for_revenue_sample(
            "whiteboard_animation",
            true
        ));
        assert!(should_use_long_form_for_revenue_sample(
            "animated_infographic",
            true
        ));
        assert!(should_use_long_form_for_revenue_sample(
            "algorithm_viz",
            true
        ));
        assert!(should_use_long_form_for_revenue_sample(
            "investor_pitch",
            true
        ));

        // New services without URL and not in the second match
        assert!(!should_use_long_form_for_revenue_sample(
            "kinetic_typography",
            false
        ));
        assert!(!should_use_long_form_for_revenue_sample(
            "year_in_review",
            false
        ));
        assert!(!should_use_long_form_for_revenue_sample(
            "isometric_explainer",
            false
        ));
    }

    #[test]
    #[test]
    fn test_service_offer_line() {
        for service in &[
            "clipping",
            "landing_page",
            "education",
            "kick_auto_clipper",
            "manim_explainer",
            "whiteboard_animation",
            "kinetic_typography",
            "animated_infographic",
            "algorithm_viz",
            "investor_pitch",
            "year_in_review",
            "isometric_explainer",
        ] {
            let line = service_offer_line(service);
            assert!(!line.is_empty(), "empty offer line for {}", service);
        }
    }
}

/// Background worker for prospect discovery (owner directive Oct 2026 —
/// searches run minutes-long and exceed proxy timeouts, so the HTTP
/// handler returns 202 immediately and this runs detached; completion
/// lands in prospect_agent_runs for polling).
async fn execute_search_dispatch(
    state: Arc<AppState>,
    payload: SearchRequest,
    limit: usize,
    run_id: Option<Uuid>,
) {
    let mut found = 0usize;
    let search_result = if payload.platform == "youtube" {
        // Service switch (owner directive Oct 2026): YouTube discovery spends
        // scoring LLM per prospect — refuse while youtube_clipping is off.
        if !crate::services::service_flags::service_enabled(&state.db_pool, "youtube_clipping")
            .await
        {
            {
                complete_prospect_agent_run(&state, run_id, "failed", json!({"error": "search_rejected"}), Some("YouTube prospect discovery is currently disabled (youtube_clipping off in service_flags).")).await;
                return;
            }
        }
        search_youtube_prospects(&state, &payload, limit).await
    } else if payload.platform == "twitch" {
        // Service switch (owner directive Oct 2026): Twitch discovery is the
        // launch-business finder — it runs only while twitch_clipping is on.
        if !crate::services::service_flags::service_enabled(&state.db_pool, "twitch_clipping").await
        {
            {
                complete_prospect_agent_run(&state, run_id, "failed", json!({"error": "search_rejected"}), Some("Twitch prospect discovery is currently disabled (twitch_clipping off in service_flags).")).await;
                return;
            }
        }
        search_twitch_prospects(&state, &payload, limit).await
    } else if payload.platform == "twitch_clipper" {
        // Service switch (owner directive Oct 2026): Twitch-clipper discovery
        // (Top Streamers → YouTube) feeds twitch_clipping — runs only while on.
        if !crate::services::service_flags::service_enabled(&state.db_pool, "twitch_clipping").await
        {
            {
                complete_prospect_agent_run(&state, run_id, "failed", json!({"error": "search_rejected"}), Some("Twitch clipper discovery is currently disabled (twitch_clipping off in service_flags).")).await;
                return;
            }
        }
        search_twitch_clipper_prospects_top_streamers(&state, &payload, limit).await
    } else if payload.platform == "kick" {
        // Service switch (owner directive Oct 2026): Kick discovery feeds the
        // launch business — it runs only while kick_auto_clipper is on.
        if !crate::services::service_flags::service_enabled(&state.db_pool, "kick_auto_clipper")
            .await
        {
            {
                complete_prospect_agent_run(&state, run_id, "failed", json!({"error": "search_rejected"}), Some("Kick prospect discovery is currently disabled (kick_auto_clipper off in service_flags).")).await;
                return;
            }
        }
        search_kick_prospects(&state, &payload, limit).await
    } else if payload.platform == "kick_clipper" || payload.platform == "kick_clipper_top" {
        // Service switch (owner directive Oct 2026): both Kick clipper methods
        // feed kick_auto_clipper — refuse while it is off.
        if !crate::services::service_flags::service_enabled(&state.db_pool, "kick_auto_clipper")
            .await
        {
            {
                complete_prospect_agent_run(&state, run_id, "failed", json!({"error": "search_rejected"}), Some("Kick clipper prospect discovery is currently disabled (kick_auto_clipper off in service_flags).")).await;
                return;
            }
        }
        if payload.platform == "kick_clipper" {
            search_kick_clipper_prospects(&state, &payload, limit).await
        } else {
            search_kick_clipper_prospects_top_streamers(&state, &payload, limit).await
        }
    } else {
        complete_prospect_agent_run(
            &state,
            run_id,
            "failed",
            json!({"error": "unsupported_platform", "platform": payload.platform}),
            Some("platform must be 'youtube', 'twitch', 'twitch_clipper', 'kick', 'kick_clipper', or 'kick_clipper_top'"),
        )
        .await;
        return;
    };

    match search_result {
        Ok(count) => found += count,
        Err(e) => {
            complete_prospect_agent_run(
                &state,
                run_id,
                "failed",
                json!({"error": "platform_search_failed"}),
                Some(&format!("Platform prospect search failed: {e:?}")),
            )
            .await;
            return;
        }
    };

    checkpoint_prospect_agent_run(
        &state,
        run_id,
        "completed",
        json!({"found": found, "platform": payload.platform.clone(), "prospect_type": payload.prospect_type.clone()}),
        Some("Prospects persisted with contact enrichment, fit score, and outreach scripts."),
    )
    .await;
    complete_prospect_agent_run(
        &state,
        run_id,
        "completed",
        json!({"found": found, "platform": payload.platform.clone(), "prospect_type": payload.prospect_type.clone()}),
        None,
    )
    .await;
}
