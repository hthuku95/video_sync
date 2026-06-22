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
use uuid::Uuid;

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
    let rows = sqlx::query(
        "SELECT c.id, c.user_id, u.email, c.name, c.service_type, c.brief, c.style, c.duration, \
                c.schedule, c.platforms, c.posts_per_day, c.start_date, c.end_date, \
                c.zernio_profile_id, c.status, c.total_posts_planned, c.total_posts_published, c.created_at \
         FROM campaigns c JOIN users u ON u.id = c.user_id \
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
                "status": row.get::<String, _>("status"),
                "total_posts_planned": row.get::<i32, _>("total_posts_planned"),
                "total_posts_published": row.get::<i32, _>("total_posts_published"),
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

    let files = match rows {
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

fn extract_r2_key_from_url(url: &str) -> Option<String> {
    // Presigned URL format: https://<bucket>.<account_id>.r2.cloudflarestorage.com/<key>?...
    // Try to extract the key portion
    let after_bucket = url.split(".r2.cloudflarestorage.com/").nth(1)?;
    let key = after_bucket.split('?').next()?;
    Some(urlencoding::decode(key).ok()?.into_owned())
}
