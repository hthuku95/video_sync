use sqlx::postgres::PgPool;
use std::env;

use dotenvy;

#[tokio::main]
async fn main() -> Result<(), sqlx::Error> {
    // Load environment variables
    dotenvy::dotenv().ok();

    let database_url = env::var("DATABASE_URL")
        .expect("DATABASE_URL must be set");

    println!("Connecting to database...");
    let pool = PgPool::connect(&database_url).await?;

    // Test 1: Check if conversation_messages table exists
    println!("Testing if conversation_messages table exists...");
    let result = sqlx::query("SELECT COUNT(*) FROM conversation_messages")
        .fetch_one(&pool)
        .await;

    match result {
        Ok(_) => println!("✅ conversation_messages table EXISTS"),
        Err(e) => {
            println!("❌ conversation_messages table NOT FOUND: {}", e);
            // Check what tables DO exist
            println!("Checking what tables exist...");
            let tables = sqlx::query_as::<_, (String,)>(
                "SELECT tablename FROM pg_tables WHERE schemaname = 'public'"
            )
            .fetch_all(&pool)
            .await?;

            println!("Available tables:");
            for (table,) in tables {
                println!("  - {}", table);
            }
        }
    }

    // Test 2: Check _sqlx_migrations table
    println!("\nTesting _sqlx_migrations table...");
    let migrations = sqlx::query_as::<_, (i64, String, bool)>(
        "SELECT version, description, installed_on IS NOT NULL as installed FROM _sqlx_migrations ORDER BY version DESC"
    )
    .fetch_all(&pool)
    .await;

    match migrations {
        Ok(migrations) => {
            println!("✅ Migrations:");
            for (version, description, installed) in migrations {
                println!("  - {} {} (Installed: {})", version, description, installed);
            }
        }
        Err(e) => println!("❌ Error querying _sqlx_migrations: {}", e),
    }

    Ok(())
}