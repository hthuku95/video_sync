use axum::{Extension, Router};
use std::sync::Arc;
use tower_http::cors::CorsLayer;

mod agent;
mod db;
mod gemini_client;
mod claude_client;
mod voyage_embeddings;
mod elevenlabs_client; // 🎙️ Eleven Labs TTS, Sound Effects, Music
mod youtube_client; // 📺 YouTube Data API v3 for video uploads
mod youtube_analytics_client; // 📊 YouTube Analytics API for metrics and insights
mod handlers;
mod jobs; // 🆕 Background job system for video editing
mod workflow; // 🆕 LangGraph-style workflow orchestration
mod middleware;
mod models;
mod pexels_client;
mod qdrant_client;
mod services;
mod vector_db;
mod clipping; // 📹 YouTube clipping feature

// Video processing modules (from lib.rs)
mod types;
mod core;
mod audio;
mod visual;
mod transform;
mod advanced;
mod export;
mod utils;

// AppState now holds the database connection pool, vector database clients, Claude/Gemini client, Pexels client, job manager, and workflow checkpointer
pub struct AppState {
    pub db_pool: sqlx::PgPool,
    pub vector_db: Option<vector_db::AstraDBClient>, // Keep for backward compatibility
    pub qdrant_client: Option<qdrant_client::QdrantClient>,
    pub gemini_client: Option<gemini_client::GeminiClient>, // Keep for fallback
    pub claude_client: Option<claude_client::ClaudeClient>,
    pub voyage_embeddings: Option<voyage_embeddings::VoyageEmbeddings>,
    pub pexels_client: Option<pexels_client::PexelsClient>,
    pub elevenlabs_client: Option<elevenlabs_client::ElevenLabsClient>, // 🎙️ Audio generation
    pub youtube_client: Option<youtube_client::YouTubeClient>, // 📺 YouTube integration
    pub youtube_analytics_client: Option<youtube_analytics_client::YouTubeAnalyticsClient>, // 📊 YouTube Analytics
    pub google_oauth_client_id: Option<String>, // Google OAuth client ID
    pub google_oauth_client_secret: Option<String>, // Google OAuth client secret
    pub job_manager: jobs::SharedJobManager, // 🆕 Background job management
    pub workflow_checkpointer: Option<workflow::checkpoint::WorkflowCheckpointer>, // 🆕 Workflow state persistence
}

#[tokio::main]
async fn main() {
    // Load environment variables from .env file
    dotenvy::dotenv().ok();

    // Initialize production-grade logging
    init_logging().expect("Failed to initialize logging");

    // Ensure outputs, uploads, and downloads directories exist
    if let Err(e) = std::fs::create_dir_all("outputs") {
        tracing::warn!("Failed to create outputs directory: {}", e);
    } else {
        tracing::info!("Outputs directory ready");
    }

    if let Err(e) = std::fs::create_dir_all("uploads") {
        tracing::warn!("Failed to create uploads directory: {}", e);
    } else {
        tracing::info!("Uploads directory ready");
    }

    if let Err(e) = std::fs::create_dir_all("downloads") {
        tracing::warn!("Failed to create downloads directory: {}", e);
    } else {
        tracing::info!("Downloads directory ready (for yt-dlp)");
    }

    // Create the database connection pool
    let db_pool = db::create_pool()
        .await
        .expect("Failed to create database pool.");

    // Initialize Astra DB client if credentials are provided
    let vector_db = match (
        std::env::var("ASTRA_DB_API_ENDPOINT").ok(),
        std::env::var("ASTRA_DB_APPLICATION_TOKEN").ok(),
        std::env::var("ASTRA_DB_KEYSPACE").ok(),
    ) {
        (Some(endpoint), Some(token), Some(keyspace)) => {
            tracing::info!("Initializing Astra DB connection...");
            let client = vector_db::AstraDBClient::new(endpoint, token, keyspace);
            
            // Try to create the collection (will succeed if it doesn't exist)
            match client.create_collection().await {
                Ok(_) => {
                    tracing::info!("Astra DB initialized successfully");
                    Some(client)
                }
                Err(e) => {
                    tracing::error!("Failed to initialize Astra DB: {}", e);
                    None
                }
            }
        }
        _ => {
            tracing::warn!("Astra DB credentials not found. Vector memory features will be disabled.");
            tracing::info!("To enable vector memory, set: ASTRA_DB_API_ENDPOINT, ASTRA_DB_APPLICATION_TOKEN, ASTRA_DB_KEYSPACE");
            None
        }
    };

    // Initialize Claude client if API key is provided
    let claude_client = match std::env::var("ANTHROPIC_API_KEY").ok() {
        Some(api_key) => {
            tracing::info!("Initializing Claude AI client (Sonnet 4.5)...");
            Some(claude_client::ClaudeClient::new(api_key))
        }
        None => {
            tracing::warn!("ANTHROPIC_API_KEY not found. Claude AI features will be disabled.");
            None
        }
    };

    // Initialize Voyage embeddings for Claude-compatible embeddings
    let voyage_embeddings = match std::env::var("VOYAGEAI_API_KEY").ok() {
        Some(api_key) => {
            tracing::info!("Initializing Voyage AI embeddings...");
            Some(voyage_embeddings::VoyageEmbeddings::new(api_key))
        }
        None => {
            tracing::warn!("VOYAGEAI_API_KEY not found. Using simple text embeddings fallback.");
            tracing::info!("To enable Voyage AI embeddings, set: VOYAGEAI_API_KEY");
            None
        }
    };

    // Initialize Gemini client if API key is provided
    let gemini_client = match std::env::var("GEMINI_API_KEY").ok() {
        Some(api_key) => {
            tracing::info!("Initializing Gemini AI client (2.5 Flash)...");
            Some(gemini_client::GeminiClient::new(api_key))
        }
        None => {
            tracing::warn!("GEMINI_API_KEY not found. Gemini AI features will be disabled.");
            None
        }
    };

    // Initialize Qdrant client if API key is provided  
    let qdrant_client = match std::env::var("QDRANT_API_KEY").ok() {
        Some(api_key) => {
            tracing::info!("Initializing Qdrant vector database...");
            let qdrant_url = std::env::var("QDRANT_URL")
                .unwrap_or_else(|_| "https://18635ac0-f6b3-43b3-9255-54a553f6c2fb.us-west-1-0.aws.cloud.qdrant.io:6334".to_string());
            
            match qdrant_client::QdrantClient::new(qdrant_url, Some(api_key)).await {
                Ok(client) => {
                    // Try to create the collection
                    match client.create_collection().await {
                        Ok(_) => {
                            tracing::info!("Qdrant initialized successfully");
                            Some(client)
                        }
                        Err(e) => {
                            tracing::error!("Failed to initialize Qdrant collection: {}", e);
                            None
                        }
                    }
                }
                Err(e) => {
                    tracing::error!("Failed to connect to Qdrant: {}", e);
                    None
                }
            }
        }
        None => {
            tracing::warn!("QDRANT_API_KEY not found. Using AstraDB fallback.");
            tracing::info!("To enable Qdrant, set: QDRANT_API_KEY and optionally QDRANT_URL");
            None
        }
    };

    // Initialize Pexels client if API key is provided
    let pexels_client = match std::env::var("PEXELS_API_KEY").ok() {
        Some(api_key) => {
            tracing::info!("Initializing Pexels stock media client...");
            Some(pexels_client::PexelsClient::new(api_key))
        }
        None => {
            tracing::warn!("PEXELS_API_KEY not found. Video generation features will be limited.");
            tracing::info!("To enable Pexels integration, set: PEXELS_API_KEY");
            None
        }
    };

    // Initialize Eleven Labs client if API key is provided
    let elevenlabs_client = match std::env::var("ELEVEN_LABS_API_KEY").ok() {
        Some(api_key) if !api_key.is_empty() => {
            tracing::info!("Initializing Eleven Labs audio client (TTS, Sound Effects, Music)...");
            Some(elevenlabs_client::ElevenLabsClient::new(api_key))
        }
        _ => {
            tracing::warn!("ELEVEN_LABS_API_KEY not found. Audio generation features will be limited.");
            tracing::info!("To enable Eleven Labs integration, set: ELEVEN_LABS_API_KEY");
            None
        }
    };

    // Initialize YouTube client if API key is provided
    let youtube_client = match std::env::var("YOUTUBE_API_KEY").ok() {
        Some(api_key) if !api_key.is_empty() => {
            tracing::info!("Initializing YouTube Data API client...");
            Some(youtube_client::YouTubeClient::new(api_key))
        }
        _ => {
            tracing::warn!("YOUTUBE_API_KEY not found. YouTube integration disabled.");
            tracing::info!("To enable YouTube, set: YOUTUBE_API_KEY, GOOGLE_OAUTH_CLIENT_ID, GOOGLE_OAUTH_CLIENT_SECRET");
            None
        }
    };

    // Initialize YouTube Analytics client (always available - no API key needed, uses OAuth)
    let youtube_analytics_client = if youtube_client.is_some() {
        tracing::info!("Initializing YouTube Analytics API client...");
        Some(youtube_analytics_client::YouTubeAnalyticsClient::new())
    } else {
        tracing::info!("YouTube Analytics disabled (YouTube Data API not configured)");
        None
    };

    // Load Google OAuth credentials
    let google_oauth_client_id = std::env::var("GOOGLE_OAUTH_CLIENT_ID").ok();
    let google_oauth_client_secret = std::env::var("GOOGLE_OAUTH_CLIENT_SECRET").ok();

    if google_oauth_client_id.is_some() && google_oauth_client_secret.is_some() {
        tracing::info!("✅ Google OAuth credentials loaded");
    } else {
        tracing::warn!("Google OAuth credentials not complete. Sign in with Google disabled.");
    }

    // Initialize JobManager for background video editing tasks
    let job_manager = Arc::new(jobs::JobManager::new());
    tracing::info!("🎬 Job manager initialized for background video processing");

    // Initialize workflow checkpointer
    let workflow_checkpointer = Some(workflow::checkpoint::WorkflowCheckpointer::new(db_pool.clone()));
    if let Some(ref checkpointer) = workflow_checkpointer {
        match checkpointer.setup().await {
            Ok(_) => tracing::info!("✅ Workflow checkpointing enabled (PostgreSQL)"),
            Err(e) => tracing::error!("❌ Failed to setup workflow checkpointing: {}", e),
        }
    }

    // Create the shared state
    let shared_state = Arc::new(AppState {
        db_pool,
        vector_db,
        qdrant_client,
        gemini_client,
        claude_client,
        voyage_embeddings,
        pexels_client,
        elevenlabs_client,
        youtube_client,
        youtube_analytics_client,
        google_oauth_client_id,
        google_oauth_client_secret,
        job_manager,
        workflow_checkpointer,
    });

    // Build our application with all routes and shared state
    let app = Router::new()
        .merge(handlers::ui::ui_routes())
        .merge(handlers::auth::auth_routes())
        .merge(handlers::chat::chat_routes())
        .merge(handlers::upload::upload_routes())
        .merge(handlers::output::output_routes())
        .merge(handlers::admin::admin_routes())
        .merge(handlers::background_routes::background_routes())
        .merge(handlers::jobs::job_routes()) // 🆕 Job control endpoints
        .merge(handlers::youtube::youtube_routes()) // 📺 YouTube integration
        .merge(handlers::clipping::clipping_routes()) // 📹 YouTube clipping feature
        .route("/api/docs", axum::routing::get(api_documentation))
        .route("/api/status", axum::routing::get(api_status))
        // .layer(axum::middleware::from_fn(middleware::frontend_rate_limit::frontend_rate_limit_middleware))
        // .layer(axum::middleware::from_fn(middleware::rate_limit::rate_limit_middleware))
        .layer(axum::middleware::from_fn(middleware::logging::request_logging_middleware))
        .layer(CorsLayer::permissive())
        .layer(Extension(shared_state.clone()));

    // Start background polling task for YouTube clipping
    if shared_state.youtube_client.is_some() {
        let polling_state = shared_state.clone();
        tokio::spawn(async move {
            tracing::info!("📹 Starting YouTube channel polling for clipping...");

            // Create channel monitor
            let youtube_client = polling_state.youtube_client.as_ref().unwrap().clone();
            let monitor = clipping::monitor::ChannelMonitor::new(
                Arc::new(youtube_client),
                polling_state.db_pool.clone(),
            );

            loop {
                match monitor.poll_all_channels().await {
                    Ok(_) => tracing::debug!("✅ Channel polling cycle completed"),
                    Err(e) => tracing::error!("❌ Channel polling failed: {}", e),
                }

                // Wait 5 minutes before next poll
                tokio::time::sleep(tokio::time::Duration::from_secs(300)).await;
            }
        });
    } else {
        tracing::warn!("YouTube client not available - clipping polling disabled");
    }

    // Run the server with ConnectInfo to provide socket addresses for rate limiting
    let listener = tokio::net::TcpListener::bind("0.0.0.0:3000")
        .await
        .unwrap();
    tracing::info!("listening on {}", listener.local_addr().unwrap());
    axum::serve(listener, app.into_make_service_with_connect_info::<std::net::SocketAddr>())
        .await
        .unwrap();
}

// Production-grade logging configuration
fn init_logging() -> Result<(), Box<dyn std::error::Error>> {
    use tracing_subscriber::{layer::SubscriberExt, util::SubscriberInitExt, EnvFilter, fmt, Layer};
    
    // Get log level from environment or default to INFO for production
    let log_level = std::env::var("RUST_LOG")
        .unwrap_or_else(|_| {
            if cfg!(debug_assertions) {
                "debug,video_editor=trace,sqlx=info,reqwest=info,hyper=info,tower=info".to_string()
            } else {
                "info,video_editor=info,sqlx=warn,reqwest=warn,hyper=warn,tower=warn".to_string()
            }
        });
    
    let env_filter = EnvFilter::try_from_default_env()
        .or_else(|_| EnvFilter::try_new(&log_level))?;
    
    // Configure structured logging for production
    let fmt_layer = if std::env::var("LOG_FORMAT").as_deref() == Ok("json") {
        // JSON logging for production (easier for log aggregation)
        fmt::layer()
            .json()
            .with_current_span(true)
            .with_span_list(false)
            .with_target(true)
            .with_thread_ids(true)
            .with_thread_names(true)
.boxed()
    } else {
        // Human-readable logging for development
        fmt::layer()
            .with_target(true)
            .with_thread_ids(false)
            .with_thread_names(false)
            .with_file(true)
            .with_line_number(true)
.boxed()
    };
    
    tracing_subscriber::registry()
        .with(env_filter)
        .with(fmt_layer)
        .init();
    
    // Log startup information
    tracing::info!("🎬 VideoSync starting up...");
    tracing::info!("Version: {}", env!("CARGO_PKG_VERSION"));
    tracing::info!("Build mode: {}", if cfg!(debug_assertions) { "development" } else { "production" });
    tracing::info!("Log level: {}", log_level);
    
    // Log environment configuration
    let gemini_configured = std::env::var("GEMINI_API_KEY").is_ok();
    let qdrant_configured = std::env::var("QDRANT_API_KEY").is_ok();
    let astra_configured = std::env::var("ASTRA_DB_API_ENDPOINT").is_ok() && std::env::var("ASTRA_DB_APPLICATION_TOKEN").is_ok();
    let db_configured = std::env::var("DATABASE_URL").is_ok();
    
    tracing::info!("Configuration - Database: {}, Gemini AI: {}, Qdrant: {}, AstraDB: {}", 
        if db_configured { "✅" } else { "❌" },
        if gemini_configured { "✅" } else { "❌" },
        if qdrant_configured { "✅" } else { "❌" },
        if astra_configured { "✅" } else { "❌" }
    );
    
    Ok(())
}

// API Documentation endpoint
async fn api_documentation() -> axum::response::Html<String> {
    let html = r###"
<!DOCTYPE html>
<html lang="en">
<head>
    <meta charset="UTF-8">
    <meta name="viewport" content="width=device-width, initial-scale=1.0">
    <title>VideoSync - API Documentation</title>
    <style>
        body { font-family: -apple-system, BlinkMacSystemFont, 'Segoe UI', Roboto, sans-serif; max-width: 1200px; margin: 0 auto; padding: 20px; line-height: 1.6; }
        .header { background: linear-gradient(135deg, #667eea 0%, #764ba2 100%); color: white; padding: 2rem; border-radius: 10px; margin-bottom: 2rem; }
        .endpoint { background: #f8f9fa; border-left: 4px solid #007bff; padding: 1rem; margin: 1rem 0; border-radius: 5px; }
        .method { display: inline-block; padding: 0.25rem 0.5rem; border-radius: 3px; color: white; font-weight: bold; margin-right: 0.5rem; }
        .get { background: #28a745; }
        .post { background: #007bff; }
        .delete { background: #dc3545; }
        .websocket { background: #6f42c1; }
        code { background: #e9ecef; padding: 0.2rem 0.4rem; border-radius: 3px; }
        .section { margin: 2rem 0; }
        .auth-note { background: #fff3cd; border: 1px solid #ffeaa7; padding: 1rem; border-radius: 5px; margin: 1rem 0; }
    </style>
</head>
<body>
    <div class="header">
        <h1>🎬 VideoSync API</h1>
        <p>Complete REST API and WebSocket interface for AI-powered video editing</p>
    </div>

    <div class="section">
        <h2>🔐 Authentication</h2>
        <div class="auth-note">
            <strong>Protected endpoints require JWT authentication.</strong><br>
            Include: <code>Authorization: Bearer &lt;your_jwt_token&gt;</code> in request headers.
        </div>
        
        <div class="endpoint">
            <span class="method post">POST</span>
            <strong>/api/auth/register</strong><br>
            Register a new user account<br>
            <strong>Body:</strong> <code>{"email": "user@example.com", "username": "user", "password": "password123"}</code>
        </div>
        
        <div class="endpoint">
            <span class="method post">POST</span>
            <strong>/api/auth/login</strong><br>
            Login and receive JWT token<br>
            <strong>Body:</strong> <code>{"email": "user@example.com", "password": "password123"}</code>
        </div>
        
        <div class="endpoint">
            <span class="method get">GET</span>
            <strong>/api/auth/verify</strong> 🔒<br>
            Verify JWT token validity<br>
            <strong>Headers:</strong> <code>Authorization: Bearer &lt;token&gt;</code>
        </div>
    </div>

    <div class="section">
        <h2>🤖 AI Chat Interface</h2>
        
        <div class="endpoint">
            <span class="method websocket">WS</span>
            <strong>/ws</strong><br>
            Real-time chat with AI video editing agent<br>
            <strong>Usage:</strong> Connect via WebSocket, send text messages, receive AI responses<br>
            <strong>Features:</strong> Access to 25+ video editing tools, context memory, file references
        </div>
        
        <div class="endpoint">
            <span class="method get">GET</span>
            <strong>/api/chat/history/:session_id</strong> 🔒<br>
            Get chat conversation history<br>
            <strong>Returns:</strong> Array of chat messages for the session
        </div>
    </div>

    <div class="section">
        <h2>📁 File Upload & Management</h2>
        
        <div class="endpoint">
            <span class="method post">POST</span>
            <strong>/upload</strong><br>
            Upload files (public endpoint)<br>
            <strong>Body:</strong> multipart/form-data with file(s)<br>
            <strong>Limit:</strong> Up to 5 files per request
        </div>
        
        <div class="endpoint">
            <span class="method post">POST</span>
            <strong>/upload/session/:session_uuid</strong> 🔒<br>
            Upload files to specific chat session<br>
            <strong>Body:</strong> multipart/form-data with file(s)
        </div>
        
        <div class="endpoint">
            <span class="method get">GET</span>
            <strong>/files/session/:session_uuid</strong> 🔒<br>
            Get all files for a chat session<br>
            <strong>Returns:</strong> Array of file metadata
        </div>
        
        <div class="endpoint">
            <span class="method get">GET</span>
            <strong>/upload/status/:file_id</strong><br>
            Check upload status and file details<br>
            <strong>Returns:</strong> File status and metadata
        </div>
        
        <div class="endpoint">
            <span class="method get">GET</span>
            <strong>/upload/form</strong><br>
            HTML upload form for testing<br>
            <strong>Returns:</strong> Interactive file upload interface
        </div>
    </div>

    <div class="section">
        <h2>🎬 Video Editing Tools (via AI Agent)</h2>
        <p>The following tools are available through the WebSocket chat interface. Send natural language requests to the AI agent:</p>

        <h3>🎙️ Audio Generation (ElevenLabs)</h3>
        <ul>
            <li><strong>generate_text_to_speech</strong> - Generate professional voiceovers with 17+ voices (Rachel, Drew, Adam, Bella, etc.)</li>
            <li><strong>generate_sound_effect</strong> - Create custom sound effects from text descriptions (0.5-30 seconds)</li>
            <li><strong>generate_music</strong> - Generate studio-grade background music (10-300 seconds, any genre)</li>
            <li><strong>add_voiceover_to_video</strong> - One-step tool: generates voiceover + adds to video automatically</li>
        </ul>

        <h3>Core Operations</h3>
        <ul>
            <li><strong>trim_video</strong> - Trim video to specific time range</li>
            <li><strong>merge_videos</strong> - Combine multiple videos</li>
            <li><strong>split_video</strong> - Split video into segments</li>
            <li><strong>analyze_video</strong> - Get video metadata and properties</li>
        </ul>

        <h3>Transform</h3>
        <ul>
            <li><strong>resize_video</strong> - Change video dimensions</li>
            <li><strong>crop_video</strong> - Crop video to specific area</li>
            <li><strong>rotate_video</strong> - Rotate video by degrees</li>
            <li><strong>adjust_speed</strong> - Change playback speed</li>
            <li><strong>flip_video</strong> - Flip horizontal/vertical</li>
            <li><strong>scale_video</strong> - Scale by factor</li>
            <li><strong>stabilize_video</strong> - Video stabilization</li>
        </ul>

        <h3>Visual Effects</h3>
        <ul>
            <li><strong>add_text_overlay</strong> - Add text to video</li>
            <li><strong>add_overlay</strong> - Add image/video overlay</li>
            <li><strong>apply_filter</strong> - Apply visual filters</li>
            <li><strong>adjust_color</strong> - Color correction</li>
            <li><strong>add_subtitles</strong> - Add subtitle files</li>
        </ul>

        <h3>Audio Processing</h3>
        <ul>
            <li><strong>extract_audio</strong> - Extract audio track</li>
            <li><strong>add_audio</strong> - Add background music</li>
            <li><strong>adjust_volume</strong> - Volume control</li>
            <li><strong>fade_audio</strong> - Fade in/out effects</li>
        </ul>

        <h3>Export & Compression</h3>
        <ul>
            <li><strong>convert_format</strong> - Change video format</li>
            <li><strong>compress_video</strong> - Reduce file size</li>
            <li><strong>export_for_platform</strong> - Optimize for social media</li>
            <li><strong>create_thumbnail</strong> - Generate thumbnails</li>
            <li><strong>extract_frames</strong> - Export individual frames</li>
        </ul>

        <h3>Advanced</h3>
        <ul>
            <li><strong>picture_in_picture</strong> - PiP effects</li>
            <li><strong>chroma_key</strong> - Green screen effects</li>
            <li><strong>split_screen</strong> - Multi-video layouts</li>
        </ul>
    </div>

    <div class="section">
        <h2>🌐 Web Interface</h2>
        
        <div class="endpoint">
            <span class="method get">GET</span>
            <strong>/</strong><br>
            Landing page with application overview
        </div>
        
        <div class="endpoint">
            <span class="method get">GET</span>
            <strong>/login</strong><br>
            User login page
        </div>
        
        <div class="endpoint">
            <span class="method get">GET</span>
            <strong>/signup</strong><br>
            User registration page
        </div>
        
        <div class="endpoint">
            <span class="method get">GET</span>
            <strong>/dashboard</strong><br>
            User dashboard (requires login)
        </div>
        
        <div class="endpoint">
            <span class="method get">GET</span>
            <strong>/chat</strong><br>
            Chat interface with AI agent
        </div>
    </div>

    <div class="section">
        <h2>🛡️ Admin Panel (Staff/Superuser Only)</h2>
        
        <div class="endpoint">
            <span class="method get">GET</span>
            <strong>/admin/login</strong><br>
            Admin login page
        </div>
        
        <div class="endpoint">
            <span class="method get">GET</span>
            <strong>/admin/dashboard</strong><br>
            Admin dashboard with system statistics
        </div>
        
        <h3>User Management</h3>
        <div class="endpoint">
            <span class="method get">GET</span>
            <strong>/api/admin/stats</strong> 🔒<br>
            Get system statistics (users, files, sessions)<br>
            <strong>Requires:</strong> Admin privileges
        </div>
        
        <div class="endpoint">
            <span class="method get">GET</span>
            <strong>/api/admin/users</strong> 🔒<br>
            List all users with pagination and search<br>
            <strong>Query params:</strong> page, limit, search<br>
            <strong>Requires:</strong> Admin privileges
        </div>
        
        <h3>Email Whitelist Management</h3>
        <div class="endpoint">
            <span class="method get">GET</span>
            <strong>/api/admin/whitelist/status</strong> 🔒<br>
            Get whitelist status and email count<br>
            <strong>Returns:</strong> <code>{"enabled": boolean, "total_emails": number}</code>
        </div>
        
        <div class="endpoint">
            <span class="method post">POST</span>
            <strong>/api/admin/whitelist/toggle</strong> 🔒<br>
            Enable/disable email whitelist restriction<br>
            <strong>Body:</strong> <code>{"enabled": boolean}</code><br>
            <strong>Note:</strong> When enabled, only whitelisted emails can register/login
        </div>
        
        <div class="endpoint">
            <span class="method get">GET</span>
            <strong>/api/admin/whitelist/emails</strong> 🔒<br>
            List all whitelisted email addresses<br>
            <strong>Returns:</strong> Array of whitelisted email objects
        </div>
        
        <div class="endpoint">
            <span class="method post">POST</span>
            <strong>/api/admin/whitelist/emails</strong> 🔒<br>
            Add email to whitelist<br>
            <strong>Body:</strong> <code>{"email": "user@example.com"}</code>
        </div>
        
        <div class="endpoint">
            <span class="method delete">DELETE</span>
            <strong>/api/admin/whitelist/emails/:id</strong> 🔒<br>
            Remove email from whitelist<br>
            <strong>Params:</strong> id (whitelist entry ID)
        </div>
    </div>

    <div class="section">
        <h2>⚙️ System</h2>
        
        <div class="endpoint">
            <span class="method get">GET</span>
            <strong>/api/status</strong><br>
            API health check and system status
        </div>
        
        <div class="endpoint">
            <span class="method get">GET</span>
            <strong>/api/docs</strong><br>
            This documentation page
        </div>
    </div>

    <div class="section">
        <h2>🔧 Rate Limits</h2>
        <ul>
            <li><strong>General API:</strong> 100 requests per minute per IP</li>
            <li><strong>Authentication:</strong> 10 requests per minute per IP</li>
            <li><strong>File Upload:</strong> Limited by file size and count</li>
        </ul>
    </div>

    <div class="section">
        <h2>📝 Example Usage</h2>
        <h3>JavaScript WebSocket Chat</h3>
        <pre><code>const ws = new WebSocket('ws://localhost:3000/ws');
ws.onmessage = (event) => console.log('AI Response:', event.data);
ws.send('Trim my video from 10 seconds to 30 seconds');</code></pre>
        
        <h3>File Upload with Fetch</h3>
        <pre><code>const formData = new FormData();
formData.append('files', fileInput.files[0]);
fetch('/upload/session/my-session-123', {
    method: 'POST',
    headers: { 'Authorization': 'Bearer ' + token },
    body: formData
});</code></pre>
    </div>

    <footer style="text-align: center; margin-top: 3rem; padding: 2rem; color: #6c757d;">
        <p>🎬 VideoSync API - Built with Rust & Axum</p>
        <p>For support, visit the web interface at <a href="/">/</a></p>
    </footer>
</body>
</html>
    "###;
    
    axum::response::Html(html.to_string())
}

// API Status endpoint
async fn api_status(Extension(state): Extension<Arc<AppState>>) -> axum::response::Json<serde_json::Value> {
    use serde_json::json;
    
    let db_status = match sqlx::query("SELECT 1").fetch_one(&state.db_pool).await {
        Ok(_) => "healthy",
        Err(_) => "unhealthy"
    };
    
    let gemini_status = if state.gemini_client.is_some() { "configured" } else { "not_configured" };
    let claude_status = if state.claude_client.is_some() { "configured" } else { "not_configured" };
    let qdrant_status = if state.qdrant_client.is_some() { "configured" } else { "not_configured" };
    let astra_status = if state.vector_db.is_some() { "configured" } else { "not_configured" };
    let elevenlabs_status = if state.elevenlabs_client.is_some() { "configured" } else { "not_configured" };
    
    axum::response::Json(json!({
        "status": "operational",
        "version": env!("CARGO_PKG_VERSION"),
        "services": {
            "database": db_status,
            "claude_ai": claude_status,
            "gemini_ai": gemini_status,
            "elevenlabs_audio": elevenlabs_status,
            "qdrant_vector_db": qdrant_status,
            "astra_vector_db": astra_status
        },
        "features": {
            "video_editing_tools": 45,
            "audio_generation_tools": 4,
            "elevenlabs_integration": true,
            "authentication": true,
            "file_upload": true,
            "websocket_chat": true,
            "rate_limiting": true,
            "vector_memory": qdrant_status == "configured" || astra_status == "configured"
        },
        "endpoints": {
            "documentation": "/api/docs",
            "status": "/api/status",
            "websocket": "/ws",
            "auth": "/api/auth/*",
            "upload": "/upload/*",
            "chat": "/api/chat/*"
        }
    }))
}
