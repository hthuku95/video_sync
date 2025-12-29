use std::env;
use sqlx::postgres::PgPool;

#[tokio::main]
async fn main() -> Result<(), Box<dyn std::error::Error>> {
    // Load environment variables
    dotenv::dotenv().ok();

    let database_url = env::var("DATABASE_URL")
        .expect("DATABASE_URL must be set");

    let pool = PgPool::connect(&database_url).await?;

    // Check for the previous session
    let previous_session_uuid = "d8f2370a-7954-43cc-8b84-6bd4956d8159";
    let current_session_uuid = "1242f266-d11f-4874-85ca-f682428747ff";

    println!("Checking database for session persistence...\n");

    // Check if previous session exists in chat_sessions
    let previous_session = sqlx::query!(
        "SELECT id, user_id, title, created_at FROM chat_sessions WHERE session_uuid = $1",
        previous_session_uuid
    )
    .fetch_optional(&pool)
    .await?;

    match previous_session {
        Some(session) => {
            println!("✅ Previous session FOUND in database:");
            println!("  - Session ID: {}", session.id);
            println!("  - User ID: {}", session.user_id);
            println!("  - Title: {}", session.title);
            println!("  - Created: {}", session.created_at);

            // Check for messages in the old chat_messages table
            let old_messages = sqlx::query!(
                "SELECT COUNT(*) as count FROM chat_messages WHERE session_id = $1",
                session.id
            )
            .fetch_one(&pool)
            .await?;

            println!("  - Old messages count: {}", old_messages.count.unwrap_or(0));

            // Check for messages in the new conversation_messages table
            let new_messages = sqlx::query!(
                "SELECT COUNT(*) as count FROM conversation_messages WHERE session_id = $1",
                session.id
            )
            .fetch_one(&pool)
            .await?;

            println!("  - New messages count: {}", new_messages.count.unwrap_or(0));

            // Check for uploaded files
            let files = sqlx::query!(
                "SELECT COUNT(*) as count FROM uploaded_files WHERE session_id = $1",
                session.id
            )
            .fetch_one(&pool)
            .await?;

            println!("  - Uploaded files count: {}", files.count.unwrap_or(0));
        }
        None => {
            println!("❌ Previous session NOT FOUND in database");
        }
    }

    // Check current session
    let current_session = sqlx::query!(
        "SELECT id, user_id, title, created_at FROM chat_sessions WHERE session_uuid = $1",
        current_session_uuid
    )
    .fetch_optional(&pool)
    .await?;

    match current_session {
        Some(session) => {
            println!("\n✅ Current session FOUND in database:");
            println!("  - Session ID: {}", session.id);
            println!("  - User ID: {}", session.user_id);
            println!("  - Title: {}", session.title);
            println!("  - Created: {}", session.created_at);
        }
        None => {
            println!("\n❌ Current session NOT FOUND in database");
        }
    }

    pool.close().await;
    Ok(())
}