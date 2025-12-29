// Check conversation message timestamps in the database
use sqlx::postgres::PgPoolOptions;
use std::env;

#[tokio::main]
async fn main() -> Result<(), Box<dyn std::error::Error>> {
    dotenvy::dotenv().ok();

    let database_url = env::var("DATABASE_URL")
        .expect("DATABASE_URL must be set");

    let pool = PgPoolOptions::new()
        .max_connections(2)
        .connect(&database_url)
        .await?;

    println!("\n=== Checking Conversation Messages Timestamps ===\n");

    // Check conversation_messages table
    let messages = sqlx::query_as::<_, (i32, i32, String, String, chrono::DateTime<chrono::Utc>)>(
        "SELECT id, session_id, role, LEFT(content, 60) as content_preview, created_at
         FROM conversation_messages
         ORDER BY created_at DESC
         LIMIT 20"
    )
    .fetch_all(&pool)
    .await?;

    if messages.is_empty() {
        println!("No messages found in conversation_messages table");
    } else {
        println!("Recent messages (showing last 20):\n");
        for (id, session_id, role, content_preview, created_at) in messages {
            println!("ID: {:<6} | Session: {:<4} | Role: {:<10} | Time: {} | Content: {}...",
                id, session_id, role, created_at.format("%Y-%m-%d %H:%M:%S UTC"), content_preview);
        }
    }

    println!("\n=== Checking Chat Sessions ===\n");

    // Check chat_sessions table
    let sessions = sqlx::query_as::<_, (i32, String, String, chrono::DateTime<chrono::Utc>)>(
        "SELECT id, session_uuid, title, created_at
         FROM chat_sessions
         ORDER BY created_at DESC
         LIMIT 10"
    )
    .fetch_all(&pool)
    .await?;

    if sessions.is_empty() {
        println!("No sessions found");
    } else {
        println!("Recent sessions (showing last 10):\n");
        for (id, uuid, title, created_at) in sessions {
            let message_count = sqlx::query_scalar::<_, i64>(
                "SELECT COUNT(*) FROM conversation_messages WHERE session_id = $1"
            )
            .bind(id)
            .fetch_one(&pool)
            .await?;

            println!("ID: {:<4} | UUID: {} | Created: {} | Messages: {} | Title: {}",
                id, uuid, created_at.format("%Y-%m-%d %H:%M:%S UTC"), message_count, title);
        }
    }

    Ok(())
}
