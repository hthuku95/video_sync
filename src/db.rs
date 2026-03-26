// src/db.rs
use sqlx::postgres::PgPoolOptions;
use sqlx::PgPool;
use std::env;
use std::time::Duration;

pub async fn create_pool() -> Result<PgPool, sqlx::Error> {
    let db_url = env::var("DATABASE_URL").expect("DATABASE_URL must be set");

    // Dynamic connection pool sizing based on worker concurrency
    // Formula: 5 (API endpoints) + worker_concurrency + 2 (buffer)
    // Allow manual override via DATABASE_MAX_CONNECTIONS env var
    let max_connections = if let Ok(max_conn_str) = env::var("DATABASE_MAX_CONNECTIONS") {
        max_conn_str
            .parse::<u32>()
            .unwrap_or_else(|_| {
                tracing::warn!("Invalid DATABASE_MAX_CONNECTIONS value, using default");
                calculate_recommended_pool_size()
            })
    } else {
        calculate_recommended_pool_size()
    };

    tracing::info!("📊 Database connection pool size: {} connections", max_connections);

    let pool = PgPoolOptions::new()
        .max_connections(max_connections)
        .acquire_timeout(Duration::from_secs(60))
        .connect(&db_url)
        .await?;

    // Run migrations on startup
    run_migrations(&pool).await?;

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
    // embed all current migration files (including 20260319000000_add_r2_storage).
    sqlx::migrate!("./migrations").run(pool).await?;
    
    tracing::info!("Database migrations completed successfully");
    Ok(())
}

fn split_sql_statements(sql: &str) -> Vec<String> {
    let mut statements = Vec::new();
    let mut current_statement = String::new();
    let mut in_single_quote = false;
    let mut in_double_quote = false;
    let mut chars = sql.chars().peekable();
    
    while let Some(ch) = chars.next() {
        match ch {
            '\'' if !in_double_quote => {
                in_single_quote = !in_single_quote;
                current_statement.push(ch);
            }
            '"' if !in_single_quote => {
                in_double_quote = !in_double_quote;
                current_statement.push(ch);
            }
            ';' if !in_single_quote && !in_double_quote => {
                current_statement.push(ch);
                let trimmed = current_statement.trim().to_string();
                if !trimmed.is_empty() && !trimmed.starts_with("--") {
                    statements.push(trimmed);
                }
                current_statement.clear();
            }
            _ => {
                current_statement.push(ch);
            }
        }
    }
    
    // Add the last statement if it doesn't end with semicolon
    let trimmed = current_statement.trim().to_string();
    if !trimmed.is_empty() && !trimmed.starts_with("--") {
        statements.push(trimmed);
    }
    
    statements
}
