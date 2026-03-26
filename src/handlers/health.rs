// Health check and monitoring endpoints
use axum::{
    extract::Extension,
    http::StatusCode,
    response::Json,
    routing::get,
    Router,
};
use serde_json::{json, Value};
use std::sync::Arc;
use sqlx::Row;
use crate::AppState;

pub fn health_routes() -> Router {
    Router::new()
        .route("/health", get(health_check))
        .route("/health/detailed", get(detailed_health_check))
        .route("/health/circuit-breaker", get(circuit_breaker_status))
}

/// Health check endpoint — IETF Health Check draft format.
/// Used by Render.com to determine if the service is healthy.
/// HTTP 200 = pass, 207 = warn (degraded but alive), 503 = fail.
async fn health_check(
    Extension(state): Extension<Arc<AppState>>,
) -> impl axum::response::IntoResponse {
    let start = std::time::Instant::now();

    // Database check
    let db_start = std::time::Instant::now();
    let db_ok = sqlx::query("SELECT 1")
        .fetch_one(&state.db_pool)
        .await
        .is_ok();
    let db_ms = db_start.elapsed().as_millis();

    // Worker liveness check (last heartbeat must be within 3 minutes)
    let worker_row: Option<(String, i32, i32, Option<i32>)> = sqlx::query_as(
        "SELECT worker_id, jobs_processed, jobs_failed, current_job_id \
         FROM worker_heartbeats \
         ORDER BY last_seen_at DESC \
         LIMIT 1"
    )
    .fetch_optional(&state.db_pool)
    .await
    .ok()
    .flatten();

    let worker_alive: bool = sqlx::query_scalar(
        "SELECT COUNT(*) > 0 FROM worker_heartbeats \
         WHERE last_seen_at > NOW() - INTERVAL '3 minutes'"
    )
    .fetch_one(&state.db_pool)
    .await
    .unwrap_or(false);

    let worker_last_heartbeat: Option<String> = sqlx::query_scalar(
        "SELECT last_seen_at::text FROM worker_heartbeats \
         ORDER BY last_seen_at DESC LIMIT 1"
    )
    .fetch_optional(&state.db_pool)
    .await
    .ok()
    .flatten()
    .flatten();

    // Queue depth
    let pending_total: i64 = sqlx::query_scalar(
        "SELECT COUNT(*) FROM clipping_jobs WHERE status = 'pending'"
    )
    .fetch_one(&state.db_pool)
    .await
    .unwrap_or(0);

    let pending_old: i64 = sqlx::query_scalar(
        "SELECT COUNT(*) FROM clipping_jobs \
         WHERE status = 'pending' AND claimed_by IS NULL \
         AND created_at < NOW() - INTERVAL '15 minutes'"
    )
    .fetch_one(&state.db_pool)
    .await
    .unwrap_or(0);

    let failed_jobs: i64 = sqlx::query_scalar(
        "SELECT COUNT(*) FROM clipping_jobs WHERE status = 'failed'"
    )
    .fetch_one(&state.db_pool)
    .await
    .unwrap_or(0);

    let discarded_jobs: i64 = sqlx::query_scalar(
        "SELECT COUNT(*) FROM clipping_jobs WHERE status = 'discarded'"
    )
    .fetch_one(&state.db_pool)
    .await
    .unwrap_or(0);

    // Determine overall status
    let db_status = if db_ok { "pass" } else { "fail" };
    let worker_status = if worker_alive { "pass" } else { "warn" };
    let queue_status = if pending_old > 0 { "warn" } else { "pass" };

    let overall = if !db_ok {
        "fail"
    } else if !worker_alive || pending_old > 0 {
        "warn"
    } else {
        "pass"
    };

    let (jobs_processed, jobs_failed, current_job_id) = worker_row
        .map(|(_, p, f, c)| (p, f, c))
        .unwrap_or((0, 0, None));

    let body = json!({
        "status": overall,
        "version": "1",
        "description": "VideoSync clipping system health",
        "responseTimeMs": start.elapsed().as_millis(),
        "checks": {
            "database": [{
                "status": db_status,
                "responseTimeMs": db_ms
            }],
            "worker": [{
                "status": worker_status,
                "lastHeartbeat": worker_last_heartbeat,
                "jobsProcessed": jobs_processed,
                "jobsFailed": jobs_failed,
                "currentJobId": current_job_id
            }],
            "queue": [{
                "status": queue_status,
                "pendingJobs": pending_total,
                "pendingOlderThan15Min": pending_old,
                "failedJobs": failed_jobs,
                "discardedJobs": discarded_jobs
            }]
        }
    });

    let http_status = match overall {
        "fail" => StatusCode::SERVICE_UNAVAILABLE,
        "warn" => StatusCode::MULTI_STATUS,
        _ => StatusCode::OK,
    };

    (http_status, Json(body))
}

/// Detailed health check with all system components
/// Checks: Database, Apify, yt-dlp, Circuit Breaker, Job Status
async fn detailed_health_check(
    Extension(state): Extension<Arc<AppState>>,
) -> Result<Json<Value>, StatusCode> {
    let mut health_status = json!({
        "status": "healthy",
        "timestamp": chrono::Utc::now().to_rfc3339(),
        "service": "video-editor-backend",
        "version": env!("CARGO_PKG_VERSION"),
        "components": {}
    });

    let components = health_status["components"].as_object_mut().unwrap();

    // 1. Check Database Connection
    let db_status = check_database(&state).await;
    components.insert("database".to_string(), db_status.clone());

    // 2. Check Apify Configuration
    let apify_status = check_apify_config().await;
    components.insert("apify".to_string(), apify_status.clone());

    // 3. Check yt-dlp Availability
    let ytdlp_status = check_ytdlp().await;
    components.insert("ytdlp".to_string(), ytdlp_status.clone());

    // 4. Check AI Services
    let ai_status = check_ai_services(&state).await;
    components.insert("ai_services".to_string(), ai_status);

    // 5. Check Vector Database
    let vector_db_status = check_vector_db(&state).await;
    components.insert("vector_db".to_string(), vector_db_status);

    // 6. Job Statistics
    let job_stats = get_job_statistics(&state).await;
    components.insert("job_statistics".to_string(), job_stats);

    // 7. Stuck Jobs Count
    let stuck_jobs = get_stuck_jobs_count(&state).await;
    components.insert("stuck_jobs".to_string(), stuck_jobs);

    // Determine overall health status
    let mut overall_healthy = true;
    for (_, component) in components.iter() {
        if component["status"] == "unhealthy" || component["status"] == "degraded" {
            overall_healthy = false;
            break;
        }
    }

    health_status["status"] = if overall_healthy {
        json!("healthy")
    } else {
        json!("degraded")
    };

    Ok(Json(health_status))
}

/// Circuit breaker status endpoint
async fn circuit_breaker_status() -> Json<Value> {
    // Note: In a real implementation, you'd need to expose circuit breaker state
    // For now, we'll return a placeholder
    Json(json!({
        "circuit_breaker": {
            "apify": {
                "state": "closed",
                "note": "Circuit breaker state tracking requires shared state implementation"
            }
        },
        "timestamp": chrono::Utc::now().to_rfc3339()
    }))
}

// Helper functions for component checks

async fn check_database(state: &Arc<AppState>) -> Value {
    match sqlx::query("SELECT 1 as health_check")
        .fetch_one(&state.db_pool)
        .await
    {
        Ok(_) => json!({
            "status": "healthy",
            "message": "Database connection successful"
        }),
        Err(e) => json!({
            "status": "unhealthy",
            "message": format!("Database connection failed: {}", e)
        }),
    }
}

async fn check_apify_config() -> Value {
    let apify_token = std::env::var("APIFY_TOKEN").ok();
    let apify_actor = std::env::var("APIFY_YOUTUBE_CLIENT_ACTOR").ok();

    match (apify_token, apify_actor) {
        (Some(token), Some(actor)) if !token.is_empty() && !actor.is_empty() => {
            json!({
                "status": "healthy",
                "configured": true,
                "message": "Apify credentials configured"
            })
        }
        _ => json!({
            "status": "degraded",
            "configured": false,
            "message": "Apify not configured, using yt-dlp only"
        }),
    }
}

async fn check_ytdlp() -> Value {
    match tokio::process::Command::new("yt-dlp")
        .arg("--version")
        .output()
        .await
    {
        Ok(output) if output.status.success() => {
            let version = String::from_utf8_lossy(&output.stdout).trim().to_string();
            json!({
                "status": "healthy",
                "available": true,
                "version": version,
                "message": "yt-dlp available"
            })
        }
        Ok(_) => json!({
            "status": "unhealthy",
            "available": false,
            "message": "yt-dlp found but version check failed"
        }),
        Err(_) => json!({
            "status": "unhealthy",
            "available": false,
            "message": "yt-dlp not found in PATH"
        }),
    }
}

async fn check_ai_services(state: &Arc<AppState>) -> Value {
    let claude_available = state.claude_client.is_some();
    let gemini_available = state.gemini_client.is_some();
    let voyage_available = state.voyage_embeddings.is_some();

    let status = if claude_available || gemini_available {
        "healthy"
    } else {
        "unhealthy"
    };

    json!({
        "status": status,
        "claude": claude_available,
        "gemini": gemini_available,
        "voyage_embeddings": voyage_available,
        "message": format!("Vision AI: {}, Embeddings: {}",
            if claude_available || gemini_available { "available" } else { "unavailable" },
            if voyage_available || gemini_available { "available" } else { "unavailable" }
        )
    })
}

async fn check_vector_db(state: &Arc<AppState>) -> Value {
    let qdrant_available = state.qdrant_client.is_some();
    let astra_available = state.vector_db.is_some();

    let status = if qdrant_available || astra_available {
        "healthy"
    } else {
        "degraded"
    };

    json!({
        "status": status,
        "qdrant": qdrant_available,
        "astra_db": astra_available,
        "message": if qdrant_available || astra_available {
            "Vector database available"
        } else {
            "No vector database configured"
        }
    })
}

async fn get_job_statistics(state: &Arc<AppState>) -> Value {
    // Get job statistics for the last 24 hours
    let stats_result = sqlx::query(
        "SELECT
            COUNT(*) as total_jobs,
            SUM(CASE WHEN status = 'completed' THEN 1 ELSE 0 END) as completed,
            SUM(CASE WHEN status = 'failed' THEN 1 ELSE 0 END) as failed,
            SUM(CASE WHEN status = 'pending' THEN 1 ELSE 0 END) as pending,
            SUM(CASE WHEN status IN ('downloading', 'analyzing', 'extracting_clips', 'posting') THEN 1 ELSE 0 END) as in_progress,
            AVG(CASE WHEN retry_count IS NOT NULL THEN retry_count ELSE 0 END) as avg_retries,
            MAX(retry_count) as max_retries
         FROM clipping_jobs
         WHERE created_at > NOW() - INTERVAL '24 hours'"
    )
    .fetch_one(&state.db_pool)
    .await;

    match stats_result {
        Ok(row) => {
            let total: i64 = row.try_get("total_jobs").unwrap_or(0);
            let completed: i64 = row.try_get("completed").unwrap_or(0);
            let failed: i64 = row.try_get("failed").unwrap_or(0);
            let pending: i64 = row.try_get("pending").unwrap_or(0);
            let in_progress: i64 = row.try_get("in_progress").unwrap_or(0);
            let avg_retries: f64 = row.try_get::<Option<f64>, _>("avg_retries").unwrap_or(None).unwrap_or(0.0);
            let max_retries: i32 = row.try_get::<Option<i32>, _>("max_retries").unwrap_or(None).unwrap_or(0);

            let success_rate = if total > 0 {
                (completed as f64 / total as f64 * 100.0).round()
            } else {
                0.0
            };

            json!({
                "status": "healthy",
                "period": "last_24_hours",
                "total_jobs": total,
                "completed": completed,
                "failed": failed,
                "pending": pending,
                "in_progress": in_progress,
                "success_rate_percent": success_rate,
                "avg_retries": format!("{:.1}", avg_retries),
                "max_retries": max_retries
            })
        }
        Err(e) => json!({
            "status": "error",
            "message": format!("Failed to fetch job statistics: {}", e)
        }),
    }
}

async fn get_stuck_jobs_count(state: &Arc<AppState>) -> Value {
    // Count jobs stuck in intermediate states
    let stuck_result = sqlx::query(
        "SELECT
            COUNT(*) as total_stuck,
            SUM(CASE WHEN status = 'downloading' AND updated_at < NOW() - INTERVAL '10 minutes' THEN 1 ELSE 0 END) as stuck_downloading,
            SUM(CASE WHEN status = 'analyzing' AND updated_at < NOW() - INTERVAL '60 minutes' THEN 1 ELSE 0 END) as stuck_analyzing,
            SUM(CASE WHEN status = 'extracting_clips' AND updated_at < NOW() - INTERVAL '15 minutes' THEN 1 ELSE 0 END) as stuck_extracting,
            SUM(CASE WHEN status = 'posting' AND updated_at < NOW() - INTERVAL '20 minutes' THEN 1 ELSE 0 END) as stuck_posting
         FROM clipping_jobs
         WHERE (
             (status = 'downloading' AND updated_at < NOW() - INTERVAL '10 minutes') OR
             (status = 'analyzing' AND updated_at < NOW() - INTERVAL '60 minutes') OR
             (status = 'extracting_clips' AND updated_at < NOW() - INTERVAL '15 minutes') OR
             (status = 'posting' AND updated_at < NOW() - INTERVAL '20 minutes')
         )"
    )
    .fetch_one(&state.db_pool)
    .await;

    match stuck_result {
        Ok(row) => {
            let total: i64 = row.try_get("total_stuck").unwrap_or(0);
            let stuck_downloading: i64 = row.try_get("stuck_downloading").unwrap_or(0);
            let stuck_analyzing: i64 = row.try_get("stuck_analyzing").unwrap_or(0);
            let stuck_extracting: i64 = row.try_get("stuck_extracting").unwrap_or(0);
            let stuck_posting: i64 = row.try_get("stuck_posting").unwrap_or(0);

            let status = if total == 0 { "healthy" } else { "warning" };

            json!({
                "status": status,
                "total_stuck": total,
                "by_state": {
                    "downloading": stuck_downloading,
                    "analyzing": stuck_analyzing,
                    "extracting_clips": stuck_extracting,
                    "posting": stuck_posting
                },
                "message": if total == 0 {
                    "No stuck jobs detected".to_string()
                } else {
                    format!("{} job(s) appear stuck and will be auto-recovered", total)
                }
            })
        }
        Err(e) => json!({
            "status": "error",
            "message": format!("Failed to check stuck jobs: {}", e)
        }),
    }
}
