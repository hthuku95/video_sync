use crate::AppState;
use axum::{
    extract::{Extension, Path},
    http::StatusCode,
    routing::{get, post},
    Json, Router,
};
use serde::Deserialize;
use serde_json::json;
use std::sync::Arc;
use uuid::Uuid; // Already used — json! macro must be in scope

// Ensure the json! macro is in scope for all handlers below.

/// Client-facing campaign routes (with auth middleware).
pub fn campaign_routes() -> Router {
    Router::new()
        .route("/api/campaigns", get(client_list_campaigns))
        .route("/api/campaigns/:id", get(client_get_campaign))
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
        .layer(axum::middleware::from_fn(crate::middleware::admin::admin_middleware))
        .layer(axum::middleware::from_fn(crate::middleware::auth::auth_middleware))
}

// ── Request/Response types ──────────────────────────────────────────────────

#[derive(Deserialize)]
pub struct CreateCampaignRequest {
    pub name: String,
    pub service_type: String,       // "clipping" | "education"
    pub brief: String,
    pub style: Option<String>,
    pub duration: Option<f64>,
    pub schedule: serde_json::Value, // [{"time":"08:00","platform":"youtube"}, ...]
    pub platforms: serde_json::Value, // [{"platform":"youtube","account_id":"..."}, ...]
    pub posts_per_day: Option<i32>,
    pub start_date: String,         // ISO 8601
    pub end_date: String,           // ISO 8601
}

// ── Admin: List all campaigns ───────────────────────────────────────────────

async fn admin_list_campaigns(
    Extension(state): Extension<Arc<AppState>>,
) -> Json<serde_json::Value> {
    let rows = sqlx::query_as::<_, (
        Uuid, i32, String, String, String, String, f64, serde_json::Value, serde_json::Value, i32,
        chrono::DateTime<chrono::Utc>, chrono::DateTime<chrono::Utc>, Option<String>, String, i32, i32,
        chrono::DateTime<chrono::Utc>,
    )>(
        "SELECT c.id, c.user_id, u.email, c.name, c.service_type, c.brief, c.style, c.duration, \
                c.schedule, c.platforms, c.posts_per_day, c.start_date, c.end_date, \
                c.zernio_profile_id, c.status, c.total_posts_planned, c.total_posts_published, c.created_at \
         FROM campaigns c JOIN users u ON u.id = c.user_id \
         ORDER BY c.created_at DESC",
    )
    .fetch_all(&state.db_pool)
    .await;

    let campaigns: Vec<serde_json::Value> = match rows {
        Ok(rows) => rows.into_iter().map(|(
            id, user_id, email, name, service_type, brief, style, duration, schedule, platforms,
            posts_per_day, start_date, end_date, zernio_profile_id, status, total_planned, total_published, created_at,
        )| {
            json!({
                "id": id.to_string(),
                "user_id": user_id,
                "user_email": email,
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
                "status": status,
                "total_posts_planned": total_planned,
                "total_posts_published": total_published,
                "created_at": created_at.to_rfc3339(),
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

    let id = sqlx::query_scalar::<_, Uuid>(
        "INSERT INTO campaigns (user_id, name, service_type, brief, style, duration, schedule, platforms, \
                                posts_per_day, start_date, end_date) \
         VALUES ($1, $2, $3, $4, $5, $6, $7, $8, $9, $10, $11) RETURNING id",
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
    .fetch_one(&state.db_pool)
    .await
    .map_err(|e| (StatusCode::INTERNAL_SERVER_ERROR, Json(json!({"error": e.to_string()}))))?;

    Ok(Json(json!({"success": true, "id": id.to_string()})))
}

// ── Admin: Get campaign details ─────────────────────────────────────────────

async fn admin_get_campaign(
    Extension(state): Extension<Arc<AppState>>,
    Path(id): Path<Uuid>,
) -> Json<serde_json::Value> {
    let campaign = sqlx::query_as::<_, (
        Uuid, String, String, String, String, f64, serde_json::Value, serde_json::Value, i32,
        chrono::DateTime<chrono::Utc>, chrono::DateTime<chrono::Utc>, Option<String>, String, i32, i32,
    )>(
        "SELECT id, name, service_type, brief, style, duration, schedule, platforms, \
                posts_per_day, start_date, end_date, zernio_profile_id, status, \
                total_posts_planned, total_posts_published \
         FROM campaigns WHERE id = $1",
    )
    .bind(id)
    .fetch_optional(&state.db_pool)
    .await;

    let (id, name, service_type, brief, style, duration, schedule, platforms, posts_per_day,
         start_date, end_date, zernio_profile_id, status, total_planned, total_published) = match campaign {
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
            "status": status,
            "total_posts_planned": total_planned,
            "total_posts_published": total_published,
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

// ── Client: List own campaigns ──────────────────────────────────────────────

async fn client_list_campaigns(
    Extension(state): Extension<Arc<AppState>>,
    Extension(claims): Extension<crate::models::auth::Claims>,
) -> Json<serde_json::Value> {
    let user_id: i32 = claims.sub.parse().unwrap_or(0);
    let rows = sqlx::query_as::<_, (
        Uuid, String, String, String, serde_json::Value, i32, i32, i32, chrono::DateTime<chrono::Utc>,
    )>(
        "SELECT id, name, service_type, status, schedule, posts_per_day, \
                total_posts_planned, total_posts_published, created_at \
         FROM campaigns WHERE user_id = $1 ORDER BY created_at DESC",
    )
    .bind(user_id)
    .fetch_all(&state.db_pool)
    .await;

    let campaigns: Vec<serde_json::Value> = match rows {
        Ok(rows) => rows.into_iter().map(|(
            id, name, service_type, status, schedule, posts_per_day,
            total_planned, total_published, created_at,
        )| {
            json!({
                "id": id.to_string(),
                "name": name,
                "service_type": service_type,
                "status": status,
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
