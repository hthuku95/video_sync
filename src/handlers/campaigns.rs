use crate::AppState;
use axum::{
    extract::{multipart::Multipart, DefaultBodyLimit, Extension, Path},
    http::StatusCode,
    routing::{delete, get, post},
    Json, Router,
};
use serde::Deserialize;
use serde_json::json;
use sqlx::Row;
use std::sync::Arc;
use tokio::fs;
use tracing::info;
use uuid::Uuid;

fn campaign_price_cents(service_type: &str) -> u64 {
    match service_type {
        "clipping" | "kick_auto_clipper" => 29700,
        "education" => 19900,
        "landing_page" => 14900,
        "manim_explainer" | "whiteboard_animation" | "kinetic_typography"
        | "animated_infographic" | "algorithm_viz" | "investor_pitch"
        | "year_in_review" | "isometric_explainer" => 14900,
        _ => 14900,
    }
}

fn base_url() -> String {
    std::env::var("PUBLIC_BASE_URL").unwrap_or_else(|_| "https://videosync.video".to_string())
}

/// Client-facing campaign routes (with auth middleware).
pub fn campaign_routes() -> Router {
    Router::new()
        .route("/api/campaigns", get(client_list_campaigns).post(client_create_campaign))
        .route("/api/campaigns/:id", get(client_get_campaign))
        .route("/api/campaigns/:id/pause", post(client_pause_campaign))
        .route("/api/campaigns/:id/resume", post(client_resume_campaign))
        .route("/api/campaigns/:id/cancel", post(client_cancel_campaign))
        .route("/api/campaigns/:id/pay-spec", get(campaign_pay_spec))
        .route("/api/campaigns/:id/settle", post(campaign_settle))
        .route("/api/campaigns/:id/chat", post(campaign_chat))
        .route("/api/campaigns/assistant", post(campaign_assistant_chat))
        .layer(axum::middleware::from_fn(crate::middleware::auth::auth_middleware))
}

/// Admin-only campaign routes (merged into admin_routes).
pub fn admin_campaign_routes() -> Router {
    Router::new()
        .route("/api/admin/campaigns", get(admin_list_campaigns).post(admin_create_campaign))
        .route("/api/admin/campaigns/:id", get(admin_get_campaign))
        .route("/api/admin/campaigns/:id/pause", post(admin_pause_campaign))
        .route("/api/admin/campaigns/:id/resume", post(admin_resume_campaign))
        .route("/api/admin/campaigns/:id/cancel", post(admin_cancel_campaign))
        .route("/api/admin/campaigns/:id/pay-spec", get(campaign_pay_spec))
        .route("/api/admin/campaigns/:id/settle", post(campaign_settle))
        .route("/api/admin/campaigns/:id/files", get(list_campaign_files).post(upload_campaign_file))
        .route("/api/admin/campaigns/:id/files/:file_id", delete(delete_campaign_file))
        .layer(DefaultBodyLimit::max(100 * 1024 * 1024))
        .layer(axum::middleware::from_fn(crate::middleware::admin::admin_middleware))
        .layer(axum::middleware::from_fn(crate::middleware::auth::auth_middleware))
}

// ── Request/Response types ──────────────────────────────────────────────────

#[derive(Deserialize)]
pub struct CreateCampaignRequest {
    pub name: String,
    pub service_type: String,       // "clipping" | "education" | "landing_page" | "kick_auto_clipper" | "manim_explainer" | "whiteboard_animation" | "kinetic_typography" | "animated_infographic" | "algorithm_viz" | "investor_pitch" | "year_in_review" | "isometric_explainer"
    pub brief: String,
    pub style: Option<String>,
    pub duration: Option<f64>,
    pub schedule: serde_json::Value, // [{"time":"08:00","platform":"youtube"}, ...]
    pub platforms: serde_json::Value, // [{"platform":"youtube","account_id":"..."}, ...]
    pub posts_per_day: Option<i32>,
    pub start_date: String,         // ISO 8601
    pub end_date: String,           // ISO 8601
    pub zernio_profile_id: Option<String>,
    pub source_url: Option<String>, // Source video URL (YouTube/Twitch/Kick) for clipping services
}

// ── Admin: List all campaigns ───────────────────────────────────────────────

async fn admin_list_campaigns(
    Extension(state): Extension<Arc<AppState>>,
) -> Json<serde_json::Value> {
    let rows = sqlx::query(
        "SELECT c.id, c.user_id, u.email, c.name, c.service_type, c.brief, c.style, c.duration, \
                c.schedule, c.platforms, c.posts_per_day, c.start_date, c.end_date, \
                c.zernio_profile_id, c.source_url, c.status, c.total_posts_planned, c.total_posts_published, \
                c.paid_until, c.created_at \
         FROM campaigns c LEFT JOIN users u ON u.id = c.user_id \
         ORDER BY c.created_at DESC",
    )
    .fetch_all(&state.db_pool)
    .await;

    let campaigns: Vec<serde_json::Value> = match rows {
        Ok(rows) => rows.iter().map(|row| {
            json!({
                "id": row.get::<Uuid, _>("id").to_string(),
                "user_id": row.get::<i32, _>("user_id"),
                "user_email": row.get::<String, _>("email"),
                "name": row.get::<String, _>("name"),
                "service_type": row.get::<String, _>("service_type"),
                "brief": row.get::<String, _>("brief"),
                "style": row.get::<String, _>("style"),
                "duration": row.get::<f64, _>("duration"),
                "schedule": row.get::<serde_json::Value, _>("schedule"),
                "platforms": row.get::<serde_json::Value, _>("platforms"),
                "posts_per_day": row.get::<i32, _>("posts_per_day"),
                "start_date": row.get::<chrono::DateTime<chrono::Utc>, _>("start_date").to_rfc3339(),
                "end_date": row.get::<chrono::DateTime<chrono::Utc>, _>("end_date").to_rfc3339(),
                "zernio_profile_id": row.get::<Option<String>, _>("zernio_profile_id"),
                "source_url": row.get::<Option<String>, _>("source_url"),
                "status": row.get::<String, _>("status"),
                "total_posts_planned": row.get::<i32, _>("total_posts_planned"),
                "total_posts_published": row.get::<i32, _>("total_posts_published"),
                "paid_until": row.get::<Option<chrono::DateTime<chrono::Utc>>, _>("paid_until").map(|d| d.to_rfc3339()),
                "created_at": row.get::<chrono::DateTime<chrono::Utc>, _>("created_at").to_rfc3339(),
            })
        }).collect(),
        Err(e) => return Json(json!({"success": false, "error": e.to_string()})),
    };

    Json(json!({"success": true, "campaigns": campaigns}))
}

// ── Admin: Create campaign ─────────────────────────────────────────────────

async fn admin_create_campaign(
    Extension(state): Extension<Arc<AppState>>,
    Json(req): Json<CreateCampaignRequest>,
) -> Result<Json<serde_json::Value>, (StatusCode, Json<serde_json::Value>)> {
    let start_date = chrono::DateTime::parse_from_rfc3339(&req.start_date)
        .map_err(|e| (StatusCode::BAD_REQUEST, Json(json!({"error": format!("Invalid start_date: {e}")}))))?
        .with_timezone(&chrono::Utc);
    let end_date = chrono::DateTime::parse_from_rfc3339(&req.end_date)
        .map_err(|e| (StatusCode::BAD_REQUEST, Json(json!({"error": format!("Invalid end_date: {e}")}))))?
        .with_timezone(&chrono::Utc);

    if end_date <= start_date {
        return Err((StatusCode::BAD_REQUEST, Json(json!({"error": "end_date must be after start_date"}))));
    }

    let style = req.style.unwrap_or_else(|| "cinematic".to_string());
    let duration = req.duration.unwrap_or(30.0);
    let posts_per_day = req.posts_per_day.unwrap_or(3);

    let source_url = req.source_url.as_deref().filter(|s| !s.is_empty());

    let id = sqlx::query_scalar::<_, Uuid>(
        "INSERT INTO campaigns (user_id, name, service_type, brief, style, duration, schedule, platforms, \
                                posts_per_day, start_date, end_date, source_url, status, paid_until) \
         VALUES ($1, $2, $3, $4, $5, $6, $7, $8, $9, $10, $11, $12, 'active', NOW() + INTERVAL '365 days') RETURNING id",
    )
    .bind(0i32) // user_id — admin-created, no specific user
    .bind(&req.name)
    .bind(&req.service_type)
    .bind(&req.brief)
    .bind(&style)
    .bind(duration)
    .bind(&req.schedule)
    .bind(&req.platforms)
    .bind(posts_per_day)
    .bind(start_date)
    .bind(end_date)
    .bind(source_url)
    .fetch_one(&state.db_pool)
    .await
    .map_err(|e| (StatusCode::INTERNAL_SERVER_ERROR, Json(json!({"error": e.to_string()}))))?;

    Ok(Json(json!({"success": true, "id": id.to_string(), "status": "active"})))
}

// ── Admin: Get campaign details ─────────────────────────────────────────────

async fn admin_get_campaign(
    Extension(state): Extension<Arc<AppState>>,
    Path(id): Path<Uuid>,
) -> Json<serde_json::Value> {
    let campaign = sqlx::query_as::<_, (
        Uuid, String, String, String, String, f64, serde_json::Value, serde_json::Value, i32,
        chrono::DateTime<chrono::Utc>, chrono::DateTime<chrono::Utc>, Option<String>, Option<String>, String, i32, i32,
    )>(
        "SELECT id, name, service_type, brief, style, duration, schedule, platforms, \
                posts_per_day, start_date, end_date, zernio_profile_id, source_url, status, \
                total_posts_planned, total_posts_published \
         FROM campaigns WHERE id = $1",
    )
    .bind(id)
    .fetch_optional(&state.db_pool)
    .await;

    let (id, name, service_type, brief, style, duration, schedule, platforms, posts_per_day,
         start_date, end_date, zernio_profile_id, source_url, status, total_planned, total_published) = match campaign {
        Ok(Some(r)) => r,
        Ok(None) => return Json(json!({"success": false, "error": "Campaign not found"})),
        Err(e) => return Json(json!({"success": false, "error": e.to_string()})),
    };

    // Fetch posts
    let posts = sqlx::query_as::<_, (Uuid, i32, i32, chrono::DateTime<chrono::Utc>, Option<String>, Option<String>, Option<String>, String, Option<String>)>(
        "SELECT id, day_number, slot_index, scheduled_at, variation_prompt, caption, media_r2_url, status, \
                zernio_post_id \
         FROM campaign_posts WHERE campaign_id = $1 \
         ORDER BY day_number, slot_index",
    )
    .bind(id)
    .fetch_all(&state.db_pool)
    .await
    .unwrap_or_default();

    let posts_json: Vec<serde_json::Value> = posts.into_iter().map(|(
        post_id, day, slot, scheduled_at, variation, caption, media_url, post_status, zernio_id,
    )| {
        json!({
            "id": post_id.to_string(),
            "day_number": day,
            "slot_index": slot,
            "scheduled_at": scheduled_at.to_rfc3339(),
            "variation_prompt": variation,
            "caption": caption,
            "media_r2_url": media_url,
            "status": post_status,
            "zernio_post_id": zernio_id,
        })
    }).collect();

    // Fetch reference files
    let files = sqlx::query_as::<_, (Uuid, String, String, String, chrono::DateTime<chrono::Utc>)>(
        "SELECT id, file_name, r2_url, file_type, uploaded_at FROM campaign_files WHERE campaign_id = $1 ORDER BY uploaded_at",
    )
    .bind(id)
    .fetch_all(&state.db_pool)
    .await
    .unwrap_or_default();

    let files_json: Vec<serde_json::Value> = files.into_iter().map(|(fid, fname, url, ftype, at)| {
        json!({"id": fid.to_string(), "file_name": fname, "r2_url": url, "file_type": ftype, "uploaded_at": at.to_rfc3339()})
    }).collect();

    Json(json!({
        "success": true,
        "campaign": {
            "id": id.to_string(),
            "name": name,
            "service_type": service_type,
            "brief": brief,
            "style": style,
            "duration": duration,
            "schedule": schedule,
            "platforms": platforms,
            "posts_per_day": posts_per_day,
            "start_date": start_date.to_rfc3339(),
            "end_date": end_date.to_rfc3339(),
            "zernio_profile_id": zernio_profile_id,
            "source_url": source_url,
            "status": status,
            "total_posts_planned": total_planned,
            "total_posts_published": total_published,
            "files": files_json,
        },
        "posts": posts_json,
    }))
}

// ── Admin: Pause / Resume / Cancel ─────────────────────────────────────────

async fn admin_pause_campaign(
    Extension(state): Extension<Arc<AppState>>,
    Path(id): Path<Uuid>,
) -> Json<serde_json::Value> {
    set_campaign_status(state, id, "paused").await
}

async fn admin_resume_campaign(
    Extension(state): Extension<Arc<AppState>>,
    Path(id): Path<Uuid>,
) -> Json<serde_json::Value> {
    set_campaign_status(state, id, "active").await
}

async fn admin_cancel_campaign(
    Extension(state): Extension<Arc<AppState>>,
    Path(id): Path<Uuid>,
) -> Json<serde_json::Value> {
    set_campaign_status(state, id, "cancelled").await
}

async fn set_campaign_status(
    state: Arc<AppState>,
    id: Uuid,
    status: &str,
) -> Json<serde_json::Value> {
    let result = sqlx::query(
        "UPDATE campaigns SET status = $1, updated_at = NOW() WHERE id = $2",
    )
    .bind(status)
    .bind(id)
    .execute(&state.db_pool)
    .await;

    match result {
        Ok(r) if r.rows_affected() > 0 => Json(json!({"success": true, "status": status})),
        Ok(_) => Json(json!({"success": false, "error": "Campaign not found"})),
        Err(e) => Json(json!({"success": false, "error": e.to_string()})),
    }
}

// ── Client: Pause / Resume / Cancel (scoped to user) ─────────────────────────

async fn client_pause_campaign(
    Extension(state): Extension<Arc<AppState>>,
    Extension(claims): Extension<crate::models::auth::Claims>,
    Path(id): Path<Uuid>,
) -> Json<serde_json::Value> {
    client_set_campaign_status(state, claims, id, "paused").await
}

async fn client_resume_campaign(
    Extension(state): Extension<Arc<AppState>>,
    Extension(claims): Extension<crate::models::auth::Claims>,
    Path(id): Path<Uuid>,
) -> Json<serde_json::Value> {
    client_set_campaign_status(state, claims, id, "active").await
}

async fn client_cancel_campaign(
    Extension(state): Extension<Arc<AppState>>,
    Extension(claims): Extension<crate::models::auth::Claims>,
    Path(id): Path<Uuid>,
) -> Json<serde_json::Value> {
    client_set_campaign_status(state, claims, id, "cancelled").await
}

async fn client_set_campaign_status(
    state: Arc<AppState>,
    claims: crate::models::auth::Claims,
    id: Uuid,
    status: &str,
) -> Json<serde_json::Value> {
    let user_id: i32 = claims.sub.parse().unwrap_or(0);
    let result = sqlx::query(
        "UPDATE campaigns SET status = $1, updated_at = NOW() WHERE id = $2 AND user_id = $3",
    )
    .bind(status)
    .bind(id)
    .bind(user_id)
    .execute(&state.db_pool)
    .await;

    match result {
        Ok(r) if r.rows_affected() > 0 => Json(json!({"success": true, "status": status})),
        Ok(_) => Json(json!({"success": false, "error": "Campaign not found"})),
        Err(e) => Json(json!({"success": false, "error": e.to_string()})),
    }
}

// ── Client: Create campaign (DIY self-service) ───────────────────────────────

async fn client_create_campaign(
    Extension(state): Extension<Arc<AppState>>,
    Extension(claims): Extension<crate::models::auth::Claims>,
    Json(req): Json<CreateCampaignRequest>,
) -> Result<Json<serde_json::Value>, (StatusCode, Json<serde_json::Value>)> {
    let user_id: i32 = claims.sub.parse().unwrap_or(0);
    let start_date = chrono::DateTime::parse_from_rfc3339(&req.start_date)
        .map_err(|e| (StatusCode::BAD_REQUEST, Json(json!({"error": format!("Invalid start_date: {e}")}))))?
        .with_timezone(&chrono::Utc);
    let end_date = chrono::DateTime::parse_from_rfc3339(&req.end_date)
        .map_err(|e| (StatusCode::BAD_REQUEST, Json(json!({"error": format!("Invalid end_date: {e}")}))))?
        .with_timezone(&chrono::Utc);

    if end_date <= start_date {
        return Err((StatusCode::BAD_REQUEST, Json(json!({"error": "end_date must be after start_date"}))));
    }

    if !matches!(req.service_type.as_str(), "clipping" | "education" | "landing_page" | "kick_auto_clipper" | "manim_explainer" | "whiteboard_animation" | "kinetic_typography" | "animated_infographic" | "algorithm_viz" | "investor_pitch" | "year_in_review" | "isometric_explainer") {
        return Err((StatusCode::BAD_REQUEST, Json(json!({"error": "service_type must be one of: clipping, education, landing_page, kick_auto_clipper, manim_explainer, whiteboard_animation, kinetic_typography, animated_infographic, algorithm_viz, investor_pitch, year_in_review, isometric_explainer"}))));
    }

    // Staff, superusers, and whitelisted users bypass payment — campaign is active immediately.
    let is_privileged = claims.is_superuser || claims.is_staff || {
        sqlx::query_scalar::<_, bool>(
            "SELECT EXISTS(SELECT 1 FROM whitelist_emails WHERE email = $1)",
        )
        .bind(&claims.email)
        .fetch_one(&state.db_pool)
        .await
        .unwrap_or(false)
    };

    let (status, paid_until) = if is_privileged {
        ("active", Some(chrono::Utc::now() + chrono::Duration::days(365)))
    } else {
        ("pending_payment", None)
    };

    let style = req.style.unwrap_or_else(|| "cinematic".to_string());
    let duration = req.duration.unwrap_or(30.0);
    let posts_per_day = req.posts_per_day.unwrap_or(3);
    let source_url = req.source_url.as_deref().filter(|s| !s.is_empty());

    let id = sqlx::query_scalar::<_, Uuid>(
        "INSERT INTO campaigns (user_id, name, service_type, brief, style, duration, schedule, platforms, \
                                posts_per_day, start_date, end_date, zernio_profile_id, source_url, status, paid_until) \
         VALUES ($1, $2, $3, $4, $5, $6, $7, $8, $9, $10, $11, $12, $13, $14, $15) RETURNING id",
    )
    .bind(user_id)
    .bind(&req.name)
    .bind(&req.service_type)
    .bind(&req.brief)
    .bind(&style)
    .bind(duration)
    .bind(&req.schedule)
    .bind(&req.platforms)
    .bind(posts_per_day)
    .bind(start_date)
    .bind(end_date)
    .bind(&req.zernio_profile_id)
    .bind(source_url)
    .bind(status)
    .bind(paid_until)
    .fetch_one(&state.db_pool)
    .await
    .map_err(|e| (StatusCode::INTERNAL_SERVER_ERROR, Json(json!({"error": e.to_string()}))))?;

    if is_privileged {
        Ok(Json(json!({"success": true, "id": id.to_string(), "status": "active"})))
    } else {
        let payment_url = format!("/api/campaigns/{}/pay-spec", id);
        Ok(Json(json!({"success": true, "id": id.to_string(), "payment_url": payment_url, "status": "pending_payment"})))
    }
}

// ── Client: List own campaigns ──────────────────────────────────────────────

async fn client_list_campaigns(
    Extension(state): Extension<Arc<AppState>>,
    Extension(claims): Extension<crate::models::auth::Claims>,
) -> Json<serde_json::Value> {
    let user_id: i32 = claims.sub.parse().unwrap_or(0);
    let rows = sqlx::query_as::<_, (
        Uuid, String, String, String, Option<String>, serde_json::Value, i32, i32, i32, chrono::DateTime<chrono::Utc>,
    )>(
        "SELECT id, name, service_type, status, source_url, schedule, posts_per_day, \
                total_posts_planned, total_posts_published, created_at \
         FROM campaigns WHERE user_id = $1 ORDER BY created_at DESC",
    )
    .bind(user_id)
    .fetch_all(&state.db_pool)
    .await;

    let campaigns: Vec<serde_json::Value> = match rows {
        Ok(rows) => rows.into_iter().map(|(
            id, name, service_type, status, source_url, schedule, posts_per_day,
            total_planned, total_published, created_at,
        )| {
            json!({
                "id": id.to_string(),
                "name": name,
                "service_type": service_type,
                "status": status,
                "source_url": source_url,
                "posts_per_day": posts_per_day,
                "total_posts_planned": total_planned,
                "total_posts_published": total_published,
                "created_at": created_at.to_rfc3339(),
            })
        }).collect(),
        Err(e) => return Json(json!({"success": false, "error": e.to_string()})),
    };

    Json(json!({"success": true, "campaigns": campaigns}))
}

// ── Client: Get campaign with post calendar ─────────────────────────────────

async fn client_get_campaign(
    Extension(state): Extension<Arc<AppState>>,
    Extension(claims): Extension<crate::models::auth::Claims>,
    Path(id): Path<Uuid>,
) -> Json<serde_json::Value> {
    let user_id: i32 = claims.sub.parse().unwrap_or(0);
    let campaign = sqlx::query_as::<_, (
        Uuid, String, String, String, String, f64, serde_json::Value, serde_json::Value,
        i32, chrono::DateTime<chrono::Utc>, chrono::DateTime<chrono::Utc>, String, i32, i32,
    )>(
        "SELECT id, name, service_type, brief, style, duration, schedule, platforms, \
                posts_per_day, start_date, end_date, status, \
                total_posts_planned, total_posts_published \
         FROM campaigns WHERE id = $1 AND user_id = $2",
    )
    .bind(id)
    .bind(user_id)
    .fetch_optional(&state.db_pool)
    .await;

    let (id, name, service_type, brief, style, duration, schedule, platforms, posts_per_day,
         start_date, end_date, status, total_planned, total_published) = match campaign {
        Ok(Some(r)) => r,
        Ok(None) => return Json(json!({"success": false, "error": "Campaign not found"})),
        Err(e) => return Json(json!({"success": false, "error": e.to_string()})),
    };

    let posts = sqlx::query_as::<_, (Uuid, i32, i32, chrono::DateTime<chrono::Utc>, Option<String>, String, Option<String>)>(
        "SELECT id, day_number, slot_index, scheduled_at, media_r2_url, status, zernio_post_id \
         FROM campaign_posts WHERE campaign_id = $1 \
         ORDER BY day_number, slot_index",
    )
    .bind(id)
    .fetch_all(&state.db_pool)
    .await
    .unwrap_or_default();

    let posts_json: Vec<serde_json::Value> = posts.into_iter().map(|(
        post_id, day, slot, scheduled_at, media_url, post_status, zernio_id,
    )| {
        json!({
            "id": post_id.to_string(),
            "day_number": day,
            "slot_index": slot,
            "scheduled_at": scheduled_at.to_rfc3339(),
            "media_r2_url": media_url,
            "status": post_status,
            "zernio_post_id": zernio_id,
        })
    }).collect();

    Json(json!({
        "success": true,
        "campaign": {
            "id": id.to_string(),
            "name": name,
            "service_type": service_type,
            "brief": brief,
            "style": style,
            "duration": duration,
            "schedule": schedule,
            "platforms": platforms,
            "posts_per_day": posts_per_day,
            "start_date": start_date.to_rfc3339(),
            "end_date": end_date.to_rfc3339(),
            "status": status,
            "total_posts_planned": total_planned,
            "total_posts_published": total_published,
        },
        "posts": posts_json,
    }))
}

// ── Campaign Payment (x402) ──────────────────────────────────────────────────

/// GET /api/campaigns/:id/pay-spec — returns x402 payment requirements to activate the campaign
async fn campaign_pay_spec(
    Path(id): Path<Uuid>,
    Extension(state): Extension<Arc<AppState>>,
) -> Result<Json<serde_json::Value>, (StatusCode, Json<serde_json::Value>)> {
    let row = sqlx::query(
        "SELECT service_type, status, name FROM campaigns WHERE id = $1",
    )
    .bind(id)
    .fetch_optional(&state.db_pool)
    .await
    .map_err(|e| (StatusCode::INTERNAL_SERVER_ERROR, Json(json!({"error": e.to_string()}))))?
    .ok_or_else(|| (StatusCode::NOT_FOUND, Json(json!({"error": "Campaign not found"}))))?;

    let status: String = row.get("status");
    if status == "active" {
        return Ok(Json(json!({"success": true, "already_active": true})));
    }

    let service_type: String = row.get("service_type");
    let name: String = row.get("name");
    let price_cents = campaign_price_cents(&service_type);

    let recipient = std::env::var("X402_RECIPIENT_ADDRESS")
        .map_err(|_| (StatusCode::INTERNAL_SERVER_ERROR, Json(json!({"error": "X402_RECIPIENT_ADDRESS not configured"}))))?;

    let resource_url = format!("{}/api/campaigns/{}/settle", base_url(), id);
    let description = format!("Activate campaign '{}' — ${:.2} month", name, price_cents as f64 / 100.0);

    let spec = crate::x402::build_payment_required(price_cents, &recipient, &resource_url, &description);
    Ok(Json(serde_json::to_value(spec).unwrap_or(json!({"error": "spec serialise failed"}))))
}

/// POST /api/campaigns/:id/settle — accepts X-Payment header, activates campaign for 30 days
async fn campaign_settle(
    Path(id): Path<Uuid>,
    headers: axum::http::HeaderMap,
    Extension(state): Extension<Arc<AppState>>,
) -> Result<Json<serde_json::Value>, (StatusCode, Json<serde_json::Value>)> {
    let row = sqlx::query(
        "SELECT user_id, service_type, status FROM campaigns WHERE id = $1",
    )
    .bind(id)
    .fetch_optional(&state.db_pool)
    .await
    .map_err(|e| (StatusCode::INTERNAL_SERVER_ERROR, Json(json!({"error": e.to_string()}))))?
    .ok_or_else(|| (StatusCode::NOT_FOUND, Json(json!({"error": "Campaign not found"}))))?;

    let status: String = row.get("status");
    if status == "active" {
        return Ok(Json(json!({"success": true, "already_active": true})));
    }

    let service_type: String = row.get("service_type");
    let user_id: i32 = row.get("user_id");
    let price_cents = campaign_price_cents(&service_type);

    let x_payment = headers.get("X-Payment").and_then(|h| h.to_str().ok())
        .ok_or_else(|| (StatusCode::PAYMENT_REQUIRED, Json(json!({"error": "Missing X-Payment header"}))))?;

    let recipient = std::env::var("X402_RECIPIENT_ADDRESS")
        .map_err(|_| (StatusCode::INTERNAL_SERVER_ERROR, Json(json!({"error": "X402_RECIPIENT_ADDRESS not configured"}))))?;

    let resource_url = format!("{}/api/campaigns/{}/settle", base_url(), id);
    let description = format!("Activate campaign '{}'", "campaign");

    let spec = crate::x402::build_payment_required(price_cents, &recipient, &resource_url, &description);
    let req = spec.accepts.into_iter().next()
        .ok_or_else(|| (StatusCode::INTERNAL_SERVER_ERROR, Json(json!({"error": "No payment requirements"}))))?;

    let tx_hash = crate::x402::settle_or_reject(x_payment, &req).await
        .map_err(|e| (StatusCode::PAYMENT_REQUIRED, Json(json!({"error": e}))))?;

    // Activate campaign for 30 days
    sqlx::query(
        "UPDATE campaigns SET status = 'active', paid_until = NOW() + INTERVAL '30 days' WHERE id = $1",
    )
    .bind(id)
    .execute(&state.db_pool)
    .await
    .map_err(|e| (StatusCode::INTERNAL_SERVER_ERROR, Json(json!({"error": e.to_string()}))))?;

    // Auto-create referral commission if this user was referred
    let referrer_id: Option<i32> = sqlx::query_scalar(
        "SELECT referrer_user_id FROM users WHERE id = $1 AND referrer_user_id IS NOT NULL",
    )
    .bind(user_id)
    .fetch_optional(&state.db_pool)
    .await
    .unwrap_or(None)
    .flatten();

    if let Some(referrer_uid) = referrer_id {
        let commission_id: Result<Uuid, _> = sqlx::query_scalar(
            "INSERT INTO referral_commission (referrer_user_id, deal_amount_cents, commission_rate) \
             VALUES ($1, $2, 0.40) RETURNING id",
        )
        .bind(referrer_uid)
        .bind(price_cents as i32)
        .fetch_one(&state.db_pool)
        .await;

        if let Ok(cid) = commission_id {
            info!("Created referral commission {cid} for referrer {referrer_uid} on campaign {id}");
        }
    }

    Ok(Json(json!({
        "success": true,
        "tx_hash": tx_hash,
        "active_until": (chrono::Utc::now() + chrono::Duration::days(30)).to_rfc3339(),
    })))
}

// ── Campaign Files: Upload, List, Delete ─────────────────────────────────────

async fn upload_campaign_file(
    Extension(state): Extension<Arc<AppState>>,
    Path(id): Path<Uuid>,
    mut multipart: Multipart,
) -> Json<serde_json::Value> {
    let mut uploaded = Vec::new();

    loop {
        let field = match multipart.next_field().await {
            Ok(Some(f)) => f,
            Ok(None) => break,
            Err(e) => {
                tracing::warn!("Multipart error: {e}");
                break;
            }
        };
        let filename = field.file_name().unwrap_or("file").to_string();
        let data = match field.bytes().await {
            Ok(d) => d,
            Err(_) => continue,
        };
        if data.is_empty() { continue; }

        let ext = std::path::Path::new(&filename)
            .extension()
            .and_then(|e| e.to_str())
            .unwrap_or("bin");
        let file_type = match ext {
            "png"|"jpg"|"jpeg"|"gif"|"webp" => "image",
            "mp4"|"mov"|"avi"|"webm" => "video",
            "pdf"|"doc"|"docx"|"txt" => "document",
            _ => "document",
        };

        let tmp = format!("/tmp/campaign_{id}_{}", Uuid::new_v4());
        if fs::write(&tmp, &data).await.is_err() { continue; }

        let r2_key = format!("campaigns/{id}/{}", Uuid::new_v4());
        let r2_url = match &state.r2_client {
            Some(r2) => {
                match r2.upload_file(&tmp, &r2_key).await {
                    Ok(url) => url,
                    Err(e) => {
                        tracing::error!("R2 upload failed for campaign file: {e}");
                        let _ = fs::remove_file(&tmp).await;
                        continue;
                    }
                }
            }
            None => {
                let _ = fs::remove_file(&tmp).await;
                return Json(json!({"success": false, "error": "R2 not configured"}));
            }
        };

        let _ = fs::remove_file(&tmp).await;

        let file_id = Uuid::new_v4();
        if let Err(e) = sqlx::query(
            "INSERT INTO campaign_files (id, campaign_id, file_name, r2_url, file_type) VALUES ($1, $2, $3, $4, $5)"
        )
        .bind(file_id)
        .bind(id)
        .bind(&filename)
        .bind(&r2_url)
        .bind(file_type)
        .execute(&state.db_pool)
        .await
        {
            tracing::error!("Failed to insert campaign_file: {e}");
            continue;
        }

        uploaded.push(json!({
            "id": file_id.to_string(),
            "file_name": filename,
            "r2_url": r2_url,
            "file_type": file_type,
        }));
    }

    Json(json!({"success": true, "files": uploaded}))
}

async fn list_campaign_files(
    Extension(state): Extension<Arc<AppState>>,
    Path(id): Path<Uuid>,
) -> Json<serde_json::Value> {
    let rows = sqlx::query_as::<_, (Uuid, String, String, String, chrono::DateTime<chrono::Utc>)>(
        "SELECT id, file_name, r2_url, file_type, uploaded_at FROM campaign_files WHERE campaign_id = $1 ORDER BY uploaded_at",
    )
    .bind(id)
    .fetch_all(&state.db_pool)
    .await;

    let files: Vec<serde_json::Value> = match rows {
        Ok(rows) => rows.into_iter().map(|(fid, name, url, ftype, at)| {
            json!({"id": fid.to_string(), "file_name": name, "r2_url": url, "file_type": ftype, "uploaded_at": at.to_rfc3339()})
        }).collect(),
        Err(e) => return Json(json!({"success": false, "error": e.to_string()})),
    };

    Json(json!({"success": true, "files": files}))
}

async fn delete_campaign_file(
    Extension(state): Extension<Arc<AppState>>,
    Path((id, file_id)): Path<(Uuid, Uuid)>,
) -> Json<serde_json::Value> {
    // Get the R2 key from the URL
    let row = sqlx::query("SELECT r2_url FROM campaign_files WHERE id = $1 AND campaign_id = $2")
        .bind(file_id)
        .bind(id)
        .fetch_optional(&state.db_pool)
        .await;

    match row {
        Ok(Some(r)) => {
            let r2_url: String = r.get("r2_url");
            // Delete from R2
            if let Some(r2) = &state.r2_client {
                // Extract key from presigned URL — it's the part after the bucket endpoint
                if let Some(key) = extract_r2_key_from_url(&r2_url) {
                    let _ = r2.delete(&key).await;
                }
            }
            // Delete from DB
            if let Err(e) = sqlx::query("DELETE FROM campaign_files WHERE id = $1")
                .bind(file_id)
                .execute(&state.db_pool)
                .await
            {
                return Json(json!({"success": false, "error": e.to_string()}));
            }
            Json(json!({"success": true}))
        }
        Ok(None) => Json(json!({"success": false, "error": "File not found"})),
        Err(e) => Json(json!({"success": false, "error": e.to_string()})),
    }
}

// ── Campaign Chat ──────────────────────────────────────────────────

#[derive(Deserialize)]
pub struct CampaignChatRequest {
    pub message: String,
}

#[derive(Deserialize)]
pub struct CampaignAssistantRequest {
    pub message: String,
    pub service: Option<String>,
}

/// Service catalog for the pre-sales assistant (mirrors the 12 Managed Campaign services).
fn service_catalog_context(service: &str) -> String {
    match service {
        "clipping" => "Social Clipping ($297/mo): AI extracts daily highlights from your streams/VODs (Twitch, YouTube, Kick), adds captions/hooks/thumbnails, and auto-posts to TikTok, Shorts, Reels, X.".to_string(),
        "kick_auto_clipper" => "Kick.com Clipping ($297/mo): automated Kick clip generation with Kick-compliant branding (logo, karaoke captions, 9:16 vertical, outro) posted daily to your social accounts.".to_string(),
        "landing_page" => "SaaS Demo Video / Landing Page Hero ($149/mo): animated homepage hero loops and narrated product demos generated from your website URL.".to_string(),
        "education" => "Education Explainer ($199/mo): Manim/LaTeX animated lessons of any duration for courses and edu channels.".to_string(),
        "manim_explainer" => "Manim Explainer ($149/mo): general Manim-animated explainer campaign.".to_string(),
        "whiteboard_animation" => "Whiteboard Animation ($149/mo): whiteboard-style explainer campaign.".to_string(),
        "kinetic_typography" => "Kinetic Typography ($149/mo): animated text/lyric-style campaign.".to_string(),
        "animated_infographic" => "Animated Infographic ($149/mo): data-driven animated infographic campaign.".to_string(),
        "algorithm_viz" => "Algorithm Visualization ($149/mo): step-by-step algorithm/code visualizations for ed-tech audiences.".to_string(),
        "investor_pitch" => "Investor Pitch ($149/mo): animated pitch-deck video campaign for fundraising.".to_string(),
        "year_in_review" => "Year in Review ($149/mo): seasonal recap/recap-stats animated campaign.".to_string(),
        "isometric_explainer" => "Isometric Explainer ($149/mo): isometric 3D-style process explainer campaign.".to_string(),
        "" => "No service selected yet — help the visitor choose from: clipping, kick_auto_clipper, landing_page, education, manim_explainer, whiteboard_animation, kinetic_typography, animated_infographic, algorithm_viz, investor_pitch, year_in_review, isometric_explainer.".to_string(),
        other => format!("Service: {other}."),
    }
}

/// POST /api/campaigns/assistant — PRE-SALES campaign assistant chat.
/// Same agent machinery as campaign_chat but scoped to a service type instead of an
/// existing campaign. Lets a logged-in user explore/design a campaign before creating one;
/// the assistant funnels them to /campaigns/new?service=X when ready.
async fn campaign_assistant_chat(
    Extension(state): Extension<Arc<AppState>>,
    Extension(claims): Extension<crate::models::auth::Claims>,
    Json(req): Json<CampaignAssistantRequest>,
) -> Result<Json<serde_json::Value>, (StatusCode, Json<serde_json::Value>)> {
    let user_id: i32 = claims.sub.parse().unwrap_or(0);
    let service = req.service.clone().unwrap_or_default();

    // Skills are already stored per-service scope — reuse them here (campaign_id = None).
    let skills = crate::services::skills::get_relevant_skills(
        &state.db_pool,
        if service.is_empty() { None } else { Some(service.as_str()) },
        None,
        Some(user_id),
        5,
    )
    .await
    .unwrap_or_default();
    let skills_context = crate::services::skills::format_skills_context(&skills);

    let context = format!(
        "## ROLE\nYou are the VideoSync Campaign Assistant helping a prospective client BEFORE \
         they create a campaign. Explain how the service works, recommend brief/schedule/platform \
         setups, and answer questions. When they are ready, tell them to click \
         'Start your campaign' which goes to /campaigns/new?service={service_slug}.\n\n\
         ## SELECTED SERVICE\n{service_ctx}\n\n\
         {skills_context}\
         ## USER QUESTION\n{user_message}",
        service_slug = service.as_str(),
        service_ctx = service_catalog_context(&service),
        skills_context = skills_context,
        user_message = req.message,
    );

    let session_uuid = if service.is_empty() {
        format!("campaign-assistant-{}", user_id)
    } else {
        format!("campaign-assistant-{}-{}", user_id, service)
    };
    let _ = crate::handlers::upload::get_or_create_session(&state, &session_uuid, Some(user_id)).await;

    let gemini_client = match state.video_gemini_client.as_ref().or(state.gemini_client.as_ref()) {
        Some(c) => Arc::new(c.clone()),
        None => return Err((StatusCode::SERVICE_UNAVAILABLE, Json(json!({"error": "AI service unavailable"})))),
    };

    let ollama_client = state.ollama_client.clone().map(Arc::new);

    let agent = crate::agent::stateful_agent::StatefulGeminiAgent::new_with_nvidia(
        gemini_client,
        state.bedrock_client.clone(),
        state.nvidia_nim_client.clone().map(Arc::new),
        ollama_client,
    );

    let agent_result = agent.chat(
        &req.message,
        &session_uuid,
        context,
        state.clone(),
        state.job_manager.clone(),
        None,
        None,
        None,
        Some(user_id),
    ).await.map_err(|e| (StatusCode::INTERNAL_SERVER_ERROR, Json(json!({"error": e}))))?;

    Ok(Json(json!({
        "success": true,
        "session_id": session_uuid,
        "response": agent_result,
        "start_url": if service.is_empty() { "/campaigns/new".to_string() } else { format!("/campaigns/new?service={}", service) },
    })))
}

/// POST /api/campaigns/:id/chat — Chat with the AI about a specific campaign.
/// Injects campaign context, past posts, and reference files into the agent prompt.
async fn campaign_chat(
    Extension(state): Extension<Arc<AppState>>,
    Extension(claims): Extension<crate::models::auth::Claims>,
    Path(id): Path<Uuid>,
    Json(req): Json<CampaignChatRequest>,
) -> Result<Json<serde_json::Value>, (StatusCode, Json<serde_json::Value>)> {
    let user_id: i32 = claims.sub.parse().unwrap_or(0);

    let campaign_row = sqlx::query_as::<_, (
        Uuid, String, String, String, String, f64, serde_json::Value, serde_json::Value,
        i32, chrono::DateTime<chrono::Utc>, chrono::DateTime<chrono::Utc>, Option<String>, String, i32, i32,
    )>(
        "SELECT id, name, service_type, brief, style, duration, schedule, platforms, \
                posts_per_day, start_date, end_date, source_url, status, \
                total_posts_planned, total_posts_published \
         FROM campaigns WHERE id = $1 AND user_id = $2",
    )
    .bind(id)
    .bind(user_id)
    .fetch_optional(&state.db_pool)
    .await
    .map_err(|e| (StatusCode::INTERNAL_SERVER_ERROR, Json(json!({"error": e.to_string()}))))?
    .ok_or_else(|| (StatusCode::NOT_FOUND, Json(json!({"error": "Campaign not found"}))))?;

    let (campaign_id, name, service_type, brief, style, duration, _schedule, _platforms, posts_per_day,
         start_date, end_date, source_url, status, total_planned, total_published) = campaign_row;

    let posts = sqlx::query_as::<_, (Uuid, i32, i32, chrono::DateTime<chrono::Utc>, Option<String>, Option<String>, Option<String>, String)>(
        "SELECT id, day_number, slot_index, scheduled_at, variation_prompt, caption, media_r2_url, status \
         FROM campaign_posts WHERE campaign_id = $1 \
         ORDER BY scheduled_at DESC LIMIT 10",
    )
    .bind(campaign_id)
    .fetch_all(&state.db_pool)
    .await
    .unwrap_or_default();

    let files = sqlx::query_as::<_, (String, String, String)>(
        "SELECT file_name, r2_url, file_type FROM campaign_files WHERE campaign_id = $1 ORDER BY uploaded_at",
    )
    .bind(campaign_id)
    .fetch_all(&state.db_pool)
    .await
    .unwrap_or_default();

    let posts_context = if posts.is_empty() {
        "No posts yet.".to_string()
    } else {
        posts.iter().map(|(_, day, slot, _, variation, caption, media_url, post_status)| {
            let var_str = variation.as_deref().unwrap_or("no prompt");
            let cap_str = caption.as_deref().unwrap_or("no caption");
            let url_str = media_url.as_deref().unwrap_or("no URL");
            format!("- Day {} Slot {} [{}]: var=\"{}\" cap=\"{}\" url={}", day, slot, post_status, var_str, cap_str, url_str)
        }).collect::<Vec<_>>().join("\n")
    };

    let files_context = if files.is_empty() {
        "No reference files.".to_string()
    } else {
        files.iter().map(|(fname, url, ftype)| {
            format!("- {} ({}) url={}", fname, ftype, url)
        }).collect::<Vec<_>>().join("\n")
    };

    // Load relevant skills for context injection
    let skills = crate::services::skills::get_relevant_skills(
        &state.db_pool,
        Some(&service_type),
        Some(campaign_id),
        Some(user_id),
        5,
    )
    .await
    .unwrap_or_default();
    let skills_context = crate::services::skills::format_skills_context(&skills);

    let context = format!(
        "## ACTIVE CAMPAIGN CONTEXT\n\
         Name: {name}\n\
         Service: {service_type}\n\
         Brief: {brief}\n\
         Style: {style}\n\
         Duration: {duration}s\n\
         Status: {status}\n\
         Posts Per Day: {posts_per_day}\n\
         Period: {start} to {end}\n\
         Source URL: {source_url}\n\
         Published: {published}/{planned}\n\n\
         ## RECENT POSTS\n{posts_context}\n\n\
         ## REFERENCE FILES\n{files_context}\n\n\
         {skills_context}\
         ## USER QUESTION\n{user_message}",
        name = name,
        service_type = service_type,
        brief = brief,
        style = style,
        duration = duration,
        status = status,
        posts_per_day = posts_per_day,
        start = start_date.format("%Y-%m-%d"),
        end = end_date.format("%Y-%m-%d"),
        source_url = source_url.as_deref().unwrap_or("none"),
        published = total_published,
        planned = total_planned,
        posts_context = posts_context,
        files_context = files_context,
        skills_context = skills_context,
        user_message = req.message,
    );

    let session_uuid = format!("campaign-chat-{}", campaign_id);
    let _ = crate::handlers::upload::get_or_create_session(&state, &session_uuid, Some(user_id)).await;

    let gemini_client = match state.video_gemini_client.as_ref().or(state.gemini_client.as_ref()) {
        Some(c) => Arc::new(c.clone()),
        None => return Err((StatusCode::SERVICE_UNAVAILABLE, Json(json!({"error": "AI service unavailable"})))),
    };

    let gemini_client_for_corrections = gemini_client.clone();

    let ollama_client = state.ollama_client.clone().map(Arc::new);

    let agent = crate::agent::stateful_agent::StatefulGeminiAgent::new_with_nvidia(
        gemini_client,
        state.bedrock_client.clone(),
        state.nvidia_nim_client.clone().map(Arc::new),
        ollama_client,
    );

    let agent_result = agent.chat(
        &req.message,
        &session_uuid,
        context,
        state.clone(),
        state.job_manager.clone(),
        None,
        None,
        None,
        Some(user_id),
    ).await.map_err(|e| (StatusCode::INTERNAL_SERVER_ERROR, Json(json!({"error": e}))))?;

    // Background skill detection from corrections
    let user_message = req.message.clone();
    let agent_result_for_corrections = agent_result.clone();
    let state_for_corrections = state.clone();
    tokio::spawn(async move {
        crate::services::skills::detect_and_store_correction(
            state_for_corrections.db_pool.clone(),
            state_for_corrections.qdrant_client.clone(),
            Some(gemini_client_for_corrections),
            state_for_corrections.ollama_client.as_ref(),
            state_for_corrections.deepseek_client.as_ref(),
            state_for_corrections.gemini_client.as_ref(),
            Some(user_id),
            Some(service_type),
            Some(campaign_id),
            user_message,
            agent_result_for_corrections,
        )
        .await;
    });

    Ok(Json(json!({
        "success": true,
        "session_id": session_uuid,
        "response": agent_result,
    })))
}

fn extract_r2_key_from_url(url: &str) -> Option<String> {
    let after_bucket = url.split(".r2.cloudflarestorage.com/").nth(1)?;
    let key = after_bucket.split('?').next()?;
    Some(urlencoding::decode(key).ok()?.into_owned())
}
