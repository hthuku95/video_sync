// src/db.rs
use sqlx::postgres::{PgConnectOptions, PgPoolOptions};
use sqlx::PgPool;
use std::env;
use std::str::FromStr;
use std::time::Duration;
use tokio::time::timeout;

pub async fn create_pool() -> Result<PgPool, sqlx::Error> {
    let db_url = env::var("DATABASE_URL").expect("DATABASE_URL must be set");

    // Dynamic connection pool sizing based on worker concurrency
    // Formula: 5 (API endpoints) + worker_concurrency + 2 (buffer)
    // Allow manual override via DATABASE_MAX_CONNECTIONS env var
    let max_connections = if let Ok(max_conn_str) = env::var("DATABASE_MAX_CONNECTIONS") {
        max_conn_str.parse::<u32>().unwrap_or_else(|_| {
            tracing::warn!("Invalid DATABASE_MAX_CONNECTIONS value, using default");
            calculate_recommended_pool_size()
        })
    } else {
        calculate_recommended_pool_size()
    };

    tracing::info!(
        "📊 Database connection pool size: {} connections",
        max_connections
    );

    let connect_options = PgConnectOptions::from_str(&db_url)?
        // Our production DB uses a pooled Neon endpoint. Disabling SQLx's
        // prepared-statement cache avoids "cached plan must not change result
        // type" failures after schema changes when the pooler reuses backend
        // sessions with stale prepared plans.
        .statement_cache_capacity(0);

    let pool = PgPoolOptions::new()
        .max_connections(max_connections)
        .acquire_timeout(Duration::from_secs(60))
        .connect_with(connect_options)
        .await?;

    // Run migrations on startup, but do not let a migration lock wait prevent
    // Cloud Run from ever seeing the HTTP port. Real migration SQL errors still
    // fail startup; only a long wait is treated as a deploy-health risk.
    run_startup_migrations_with_timeout(&pool).await?;

    Ok(pool)
}

/// Calculate recommended pool size based on worker concurrency
fn calculate_recommended_pool_size() -> u32 {
    let worker_concurrency = env::var("CLIPPING_WORKER_CONCURRENCY")
        .ok()
        .and_then(|v| v.parse::<u32>().ok())
        .unwrap_or(3);

    // Formula: 5 (API) + concurrency + 10 (background tasks: monitor, token refresh, stuck detection)
    // Note: background tasks (channel monitor, token refresh, stuck detection, vectorization heartbeats)
    // can hold connections concurrently with the worker. The old 2-connection buffer was too small.
    let recommended = 5 + worker_concurrency + 10;

    tracing::debug!(
        "Calculated pool size: 5 (API) + {} (workers) + 10 (background) = {}",
        worker_concurrency,
        recommended
    );

    recommended
}

pub async fn run_migrations(pool: &PgPool) -> Result<(), sqlx::Error> {
    tracing::info!("Running database migrations...");

    // sqlx::migrate!() is a proc macro — Cargo only re-runs it when THIS file
    // changes. Touching this file forces the macro to re-scan ./migrations and
    // embed all current migration files.
    // Last touched: 2026-07-19 to force sqlx::migrate!() to re-embed the
    // current migration set, including:
    //   20260719000000 — instagram_leads.contact_enrichment JSONB column
    //   20260706000000 — user_zernio_profiles + user_zernio_accounts tables
    //   20260623000002 - campaigns table
    //   20260623000003 - campaign_posts table
    //   20260623000004 - campaign_files table
    //   20260623000001 - zernio columns on deliveries
    //   20260623000000 - is_dfy_customer column on users
    //   (many more above)
    // Previous touch: 2026-05-19 for:
    //   20260413000000 — scope_instagram_leads_per_user
    //   20260510000000 — app_workflows
    //   20260510010000 — workflow links on clipping_jobs/manual_clipping_jobs
    //   20260510020000 — workflow link on deliveries
    //   20260510030000 — workflow link on gig_sample_videos
    // Earlier touch notes:
    // Last touched: 2026-04-17 to pick up:
    //   20260417000000 — preview_r2_url + qa_retry_count + source_url
    //                    on deliveries (iterative QA + free/paid split)
    // 2026-04-16 to pick up:
    //   20260416000002 — telegram_sessions (MTProto watcher auth state)
    //   20260416000001 — blender_render_reviews (LLM QA audit log)
    //   20260416000000 — lead attribution (first_contacted_at /
    //                    converted_at + deliveries.sourced_from_lead_id)
    // Earlier touches:
    // 2026-04-15 — 20260415000002 (telegram opportunities + watch channels);
    // 20260415000001 (user subscriptions paywall), 20260415000000
    // (api_subscriptions table for agency licensing).
    // Older notes:
    // 2026-04-14 — 20260413000001/2/3 (x402 paywall + service_type on
    // prospects + IG leads service/sample) after the 20260413000000
    // checksum-mismatch fix.
    sqlx::migrate!("./migrations").run(pool).await?;

    tracing::info!("Database migrations completed successfully");
    Ok(())
}

async fn run_startup_migrations_with_timeout(pool: &PgPool) -> Result<(), sqlx::Error> {
    let timeout_secs = env::var("VIDEO_SYNC_MIGRATION_TIMEOUT_SECONDS")
        .ok()
        .and_then(|value| value.parse::<u64>().ok())
        .unwrap_or(45);

    match timeout(Duration::from_secs(timeout_secs), run_migrations(pool)).await {
        Ok(result) => result,
        Err(_) => {
            tracing::warn!(
                "Database migrations did not finish within {}s; continuing startup so Cloud Run can bind. Run migrations out-of-band if a new schema change is pending.",
                timeout_secs
            );
            Ok(())
        }
    }
}
