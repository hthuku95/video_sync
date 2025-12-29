use std::io::{self, Write};
use dotenvy::dotenv;
use bcrypt::{hash, DEFAULT_COST};
use sqlx::{postgres::PgPoolOptions, Row};

#[tokio::main]
async fn main() -> Result<(), Box<dyn std::error::Error>> {
    println!("🛡️  VideoSync - Create Superuser");
    println!("==========================================");
    
    // Load environment variables
    dotenv().ok();
    
    // Connect to database
    let database_url = std::env::var("DATABASE_URL")
        .expect("DATABASE_URL must be set in .env file");
    
    let pool = PgPoolOptions::new()
        .max_connections(5)
        .connect(&database_url)
        .await?;
    
    // Get superuser details from input
    print!("Email address: ");
    io::stdout().flush()?;
    let mut email = String::new();
    io::stdin().read_line(&mut email)?;
    let email = email.trim().to_string();
    
    if email.is_empty() || !email.contains('@') {
        eprintln!("❌ Invalid email address");
        return Ok(());
    }
    
    print!("Username: ");
    io::stdout().flush()?;
    let mut username = String::new();
    io::stdin().read_line(&mut username)?;
    let username = username.trim().to_string();
    
    if username.is_empty() {
        eprintln!("❌ Username cannot be empty");
        return Ok(());
    }
    
    // Check if user already exists
    let existing_user = sqlx::query("SELECT id FROM users WHERE email = $1 OR username = $2")
        .bind(&email)
        .bind(&username)
        .fetch_optional(&pool)
        .await?;
    
    if existing_user.is_some() {
        eprintln!("❌ User with this email or username already exists");
        return Ok(());
    }
    
    print!("Password: ");
    io::stdout().flush()?;
    let password = rpassword::read_password()?;
    
    if password.len() < 6 {
        eprintln!("❌ Password must be at least 6 characters long");
        return Ok(());
    }
    
    print!("Password (again): ");
    io::stdout().flush()?;
    let password_confirm = rpassword::read_password()?;
    
    if password != password_confirm {
        eprintln!("❌ Passwords don't match");
        return Ok(());
    }
    
    // Hash password
    let password_hash = hash(&password, DEFAULT_COST)?;
    
    // Create superuser
    let result = sqlx::query(
        "INSERT INTO users (email, username, password_hash, is_active, is_superuser, is_staff, created_at, updated_at)
         VALUES ($1, $2, $3, true, true, true, NOW(), NOW())
         RETURNING id, username, email"
    )
    .bind(&email)
    .bind(&username)
    .bind(&password_hash)
    .fetch_one(&pool)
    .await;
    
    match result {
        Ok(row) => {
            let id: i32 = row.get("id");
            let username: String = row.get("username");
            let email: String = row.get("email");
            
            println!();
            println!("✅ Superuser created successfully!");
            println!("   ID: {}", id);
            println!("   Username: {}", username);
            println!("   Email: {}", email);
            println!("   Admin Access: YES");
            println!("   Superuser: YES");
            println!();
            println!("🌐 You can now access the admin panel at: http://localhost:3000/admin");
            println!("🔐 Login with the credentials you just created");
        }
        Err(e) => {
            eprintln!("❌ Failed to create superuser: {}", e);
        }
    }
    
    pool.close().await;
    Ok(())
}