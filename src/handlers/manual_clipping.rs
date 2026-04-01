// Manual clipping API — lets any authenticated user (including clippers) paste a
// YouTube or Twitch URL and get download links for the extracted clips.
// No destination YouTube channel required.

use crate::jobs::manual_clipping_job::execute_manual_clipping_job;
use crate::middleware::auth::auth_middleware;
use crate::models::auth::{Claims, ErrorResponse};
use crate::AppState;
use axum::{
    extract::{Extension, Path, Query},
    http::StatusCode,
    response::Json,
    routing::{delete, get, post},
    Router,
};
use serde::{Deserialize, Serialize};
use serde_json::json;
use sqlx::Row;
use std::sync::Arc;
use uuid::Uuid;

pub fn manual_clipping_routes() -> Router {
    Router::new()
        .route("/api/manual-clipping/jobs", post(create_job))
        .route("/api/manual-clipping/jobs", get(list_jobs))
        .route("/api/manual-clipping/jobs/:id", get(get_job))
        .route("/api/manual-clipping/jobs/:id", delete(delete_job))
        .layer(axum::middleware::from_fn(auth_middleware))
}

#[derive(Debug, Deserialize)]
struct CreateJobRequest {
    video_url: String,
    clips_count: Option<i32>,
    min_duration: Option<i32>,
    max_duration: Option<i32>,
}

#[derive(Debug, Deserialize)]
struct ListJobsQuery {
    page: Option<i64>,
}

fn detect_platform(url: &str) -> &'static str {
    if url.contains("twitch.tv") || url.contains("twitch.com") {
        "twitch"
    } else {
        "youtube"
    }
}

async fn create_job(
    Extension(state): Extension<Arc<AppState>>,
    Extension(claims): Extension<Claims>,
    Json(payload): Json<CreateJobRequest>,
) -> Result<Json<serde_json::Value>, (StatusCode, Json<ErrorResponse>)> {
    if payload.video_url.trim().is_empty() {
        return Err((
            StatusCode::BAD_REQUEST,
            Json(ErrorResponse { success: false, message: "video_url is required".to_string() }),
        ));
    }

    let user_id: i32 = claims.sub.parse().unwrap_or(0);
    let platform = detect_platform(&payload.video_url);
    let clips_count = payload.clips_count.unwrap_or(3).clamp(1, 5);
    let min_duration = payload.min_duration.unwrap_or(30).clamp(10, 300);
    let max_duration = payload.max_duration.unwrap_or(120).clamp(30, 600);

    let job_id: Uuid = sqlx::query_scalar(
        "INSERT INTO manual_clipping_jobs
         (user_id, video_url, video_platform, clips_requested,
          min_clip_duration_seconds, max_clip_duration_seconds)
         VALUES ($1, $2, $3, $4, $5, $6)
         RETURNING id",
    )
    .bind(user_id)
    .bind(&payload.video_url)
    .bind(platform)
    .bind(clips_count)
    .bind(min_duration)
    .bind(max_duration)
    .fetch_one(&state.db_pool)
    .await
    .map_err(|e| {
        tracing::error!("Failed to create manual clipping job: {}", e);
        (
            StatusCode::INTERNAL_SERVER_ERROR,
            Json(ErrorResponse { success: false, message: "Failed to create job".to_string() }),
        )
    })?;

    // Spawn background execution
    let state_clone = Arc::clone(&state);
    tokio::spawn(async move {
        if let Err(e) = execute_manual_clipping_job(job_id, state_clone.clone()).await {
            tracing::error!("Manual clipping job {} failed: {}", job_id, e);
            let _ = sqlx::query(
                "UPDATE manual_clipping_jobs SET status='failed', error_message=$1, updated_at=NOW() WHERE id=$2"
            )
            .bind(&e)
            .bind(job_id)
            .execute(&state_clone.db_pool)
            .await;
        }
    });

    Ok(Json(json!({
        "success": true,
        "job_id": job_id.to_string(),
        "status": "pending",
        "platform": platform
    })))
}

async fn list_jobs(
    Extension(state): Extension<Arc<AppState>>,
    Extension(claims): Extension<Claims>,
    Query(query): Query<ListJobsQuery>,
) -> Result<Json<serde_json::Value>, (StatusCode, Json<ErrorResponse>)> {
    let user_id: i32 = claims.sub.parse().unwrap_or(0);
    let page = query.page.unwrap_or(1).max(1);
    let limit: i64 = 20;
    let offset = (page - 1) * limit;

    let rows = sqlx::query(
        "SELECT id, video_url, video_platform, video_title, status, progress_percent,
                clips_count, error_message, created_at, completed_at
         FROM manual_clipping_jobs
         WHERE user_id = $1
         ORDER BY created_at DESC
         LIMIT $2 OFFSET $3",
    )
    .bind(user_id)
    .bind(limit)
    .bind(offset)
    .fetch_all(&state.db_pool)
    .await
    .map_err(|e| {
        tracing::error!("Failed to list manual jobs: {}", e);
        (
            StatusCode::INTERNAL_SERVER_ERROR,
            Json(ErrorResponse { success: false, message: "Failed to fetch jobs".to_string() }),
        )
    })?;

    let jobs: Vec<serde_json::Value> = rows
        .iter()
        .map(|r| {
            json!({
                "id": r.get::<Uuid, _>("id").to_string(),
                "video_url": r.get::<String, _>("video_url"),
                "video_platform": r.get::<String, _>("video_platform"),
                "video_title": r.get::<Option<String>, _>("video_title"),
                "status": r.get::<String, _>("status"),
                "progress_percent": r.get::<i32, _>("progress_percent"),
                "clips_count": r.get::<i32, _>("clips_count"),
                "error_message": r.get::<Option<String>, _>("error_message"),
                "created_at": r.get::<chrono::DateTime<chrono::Utc>, _>("created_at"),
                "completed_at": r.get::<Option<chrono::DateTime<chrono::Utc>>, _>("completed_at"),
            })
        })
        .collect();

    Ok(Json(json!({ "success": true, "jobs": jobs, "page": page })))
}

async fn get_job(
    Extension(state): Extension<Arc<AppState>>,
    Extension(claims): Extension<Claims>,
    Path(id): Path<Uuid>,
) -> Result<Json<serde_json::Value>, (StatusCode, Json<ErrorResponse>)> {
    let user_id: i32 = claims.sub.parse().unwrap_or(0);

    // Admins/staff see any job; clippers only see their own
    let row = if claims.is_superuser || claims.is_staff {
        sqlx::query(
            "SELECT id, user_id, video_url, video_platform, video_title, clips_requested,
                    min_clip_duration_seconds, max_clip_duration_seconds,
                    status, progress_percent, clips_count, error_message,
                    created_at, completed_at
             FROM manual_clipping_jobs WHERE id = $1",
        )
        .bind(id)
        .fetch_optional(&state.db_pool)
        .await
    } else {
        sqlx::query(
            "SELECT id, user_id, video_url, video_platform, video_title, clips_requested,
                    min_clip_duration_seconds, max_clip_duration_seconds,
                    status, progress_percent, clips_count, error_message,
                    created_at, completed_at
             FROM manual_clipping_jobs WHERE id = $1 AND user_id = $2",
        )
        .bind(id)
        .bind(user_id)
        .fetch_optional(&state.db_pool)
        .await
    }
    .map_err(|e| {
        tracing::error!("DB error fetching manual job: {}", e);
        (
            StatusCode::INTERNAL_SERVER_ERROR,
            Json(ErrorResponse { success: false, message: "Failed to fetch job".to_string() }),
        )
    })?;

    let row = row.ok_or_else(|| {
        (
            StatusCode::NOT_FOUND,
            Json(ErrorResponse { success: false, message: "Job not found".to_string() }),
        )
    })?;

    // Fetch clips for this job
    let clip_rows = sqlx::query(
        "SELECT id, clip_number, title, description, start_time_seconds, end_time_seconds,
                duration_seconds, quality_score, viral_factors,
                r2_clip_url, r2_clip_url_expires_at, thumbnail_r2_url
         FROM manual_clipping_clips
         WHERE job_id = $1
         ORDER BY clip_number ASC",
    )
    .bind(id)
    .fetch_all(&state.db_pool)
    .await
    .unwrap_or_default();

    let clips: Vec<serde_json::Value> = clip_rows
        .iter()
        .map(|c| {
            json!({
                "id": c.get::<Uuid, _>("id").to_string(),
                "clip_number": c.get::<i32, _>("clip_number"),
                "title": c.get::<Option<String>, _>("title"),
                "description": c.get::<Option<String>, _>("description"),
                "start_time_seconds": c.get::<Option<f64>, _>("start_time_seconds"),
                "end_time_seconds": c.get::<Option<f64>, _>("end_time_seconds"),
                "duration_seconds": c.get::<Option<f64>, _>("duration_seconds"),
                "quality_score": c.get::<Option<f64>, _>("quality_score"),
                "download_url": c.get::<Option<String>, _>("r2_clip_url"),
                "thumbnail_url": c.get::<Option<String>, _>("thumbnail_r2_url"),
                "url_expires_at": c.get::<Option<chrono::DateTime<chrono::Utc>>, _>("r2_clip_url_expires_at"),
            })
        })
        .collect();

    Ok(Json(json!({
        "success": true,
        "job": {
            "id": row.get::<Uuid, _>("id").to_string(),
            "video_url": row.get::<String, _>("video_url"),
            "video_platform": row.get::<String, _>("video_platform"),
            "video_title": row.get::<Option<String>, _>("video_title"),
            "clips_requested": row.get::<i32, _>("clips_requested"),
            "status": row.get::<String, _>("status"),
            "progress_percent": row.get::<i32, _>("progress_percent"),
            "clips_count": row.get::<i32, _>("clips_count"),
            "error_message": row.get::<Option<String>, _>("error_message"),
            "created_at": row.get::<chrono::DateTime<chrono::Utc>, _>("created_at"),
            "completed_at": row.get::<Option<chrono::DateTime<chrono::Utc>>, _>("completed_at"),
        },
        "clips": clips
    })))
}

async fn delete_job(
    Extension(state): Extension<Arc<AppState>>,
    Extension(claims): Extension<Claims>,
    Path(id): Path<Uuid>,
) -> Result<Json<serde_json::Value>, (StatusCode, Json<ErrorResponse>)> {
    let user_id: i32 = claims.sub.parse().unwrap_or(0);

    // Mark as cancelled (don't hard-delete — worker may be mid-run)
    let affected = if claims.is_superuser || claims.is_staff {
        sqlx::query(
            "UPDATE manual_clipping_jobs SET status='cancelled', updated_at=NOW() WHERE id=$1 AND status IN ('pending','analyzing','downloading','extracting')"
        )
        .bind(id)
        .execute(&state.db_pool)
        .await
    } else {
        sqlx::query(
            "UPDATE manual_clipping_jobs SET status='cancelled', updated_at=NOW() WHERE id=$1 AND user_id=$2 AND status IN ('pending','analyzing','downloading','extracting')"
        )
        .bind(id)
        .bind(user_id)
        .execute(&state.db_pool)
        .await
    }
    .map_err(|e| {
        tracing::error!("Failed to cancel job: {}", e);
        (
            StatusCode::INTERNAL_SERVER_ERROR,
            Json(ErrorResponse { success: false, message: "Failed to cancel job".to_string() }),
        )
    })?;

    if affected.rows_affected() == 0 {
        return Err((
            StatusCode::NOT_FOUND,
            Json(ErrorResponse { success: false, message: "Job not found or not cancellable".to_string() }),
        ));
    }

    Ok(Json(json!({ "success": true, "message": "Job cancelled" })))
}
