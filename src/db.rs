// src/db.rs
use sqlx::postgres::PgPoolOptions;
use sqlx::PgPool;
use std::env;
use std::time::Duration;

pub async fn create_pool() -> Result<PgPool, sqlx::Error> {
    let db_url = env::var("DATABASE_URL").expect("DATABASE_URL must be set");
    let pool = PgPoolOptions::new()
        .max_connections(5)
        .acquire_timeout(Duration::from_secs(30))
        .connect(&db_url)
        .await?;
    
    // Run migrations on startup
    run_migrations(&pool).await?;
    
    Ok(pool)
}

pub async fn run_migrations(pool: &PgPool) -> Result<(), sqlx::Error> {
    tracing::info!("Running database migrations...");
    
    // Use SQLx's built-in migrator for better reliability
    // This handles migration tracking and ensures proper execution order
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
