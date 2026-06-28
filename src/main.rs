// Build: 2026-03-10 — deploy with YOUTUBE_API_KEY + Gemini retry fixes
use axum::{extract::DefaultBodyLimit, Extension, Router};
mod tool_registry;
use std::collections::HashMap;
use std::sync::Arc;
use tokio::sync::mpsc::UnboundedSender;
use tokio::sync::Semaphore;
use tower_http::cors::CorsLayer;

mod agent;
mod ai_tool_selector; // 🧠 AI-driven tool selection — Gemma 4 picks relevant tools per request
mod blender_mcp_client; // 🎨 BlenderMCPServer — 3D rendering + Manim
mod browserbase_client; // 🌐 BrowserBase — cloud browser fetch + search
mod bedrock_client;
mod blender_quality;
mod claude_client;
mod clipping; // 📹 YouTube clipping feature
mod cloud_storage;
mod db;
mod deepseek_client;
mod email;
mod ffmpeg_mcp_client;
mod elevenlabs_client; // 🎙️ Eleven Labs TTS, Sound Effects, Music
mod gemini_client;
mod gcs_client;
mod handlers;
mod jobs; // 🆕 Background job system for video editing
mod kick_client; // 📺 Kick.com API client
mod llm_utils;
mod middleware;
mod models;
mod nvidia_nim_client;
mod ollama_client;
mod pexels_client;
mod phantombuster_client; // 🎯 PhantomBuster — LinkedIn Sales Navigator scraping
mod portfolio_samples;
mod qdrant_client;
mod r2_client; // ☁️ Cloudflare R2 object storage
mod render_review; // 🔍 LLM QA review of every render before handoff
mod services;
mod sketchfab_client;
mod telegram_bot; // ✈️ Telegram Bot API — admin pings + AI sales DM replies
mod telegram_client; // 🛰️ Telegram MTProto userbot — channel watcher for paid-gig leads
mod token_manager; // 🔧 Centralized YouTube OAuth token refresh
mod twitch_client; // 📺 Twitch Helix API client
mod utils; // 🔧 Utility modules (FFmpeg utilities, etc.)
mod vector_db;
mod vertex_multimodal_embeddings;
mod vibevoice_client; // 🎤 VibeVoice microservice — TTS + transcription
mod voyage_embeddings;
mod x402; // 💰 x402 HTTP-402 payment protocol (USDC on Base)
mod youtube_analytics_client; // 📊 YouTube Analytics API for metrics and insights
mod youtube_client; // 📺 YouTube Data API v3 for video uploads // 💼 Monetizable portfolio sample targets + prompts
mod zernio_client; // 📱 Zernio — multi-platform social media publishing API

// Video processing modules (from lib.rs)
mod advanced;
mod audio;
mod core;
mod export;
mod manual_clipping_tests;
mod portfolio_tests;
mod transform;
mod types;
mod visual;
mod workflows; // Named multi-step FFmpeg workflow chains

// AppState now holds the database connection pool, vector database clients, Claude/Gemini
// clients, production integrations, and shared runtime services.
pub struct AppState {
    pub db_pool: sqlx::PgPool,
    pub vector_db: Option<vector_db::AstraDBClient>, // Keep for backward compatibility
    pub qdrant_client: Option<qdrant_client::QdrantClient>,
    pub gemini_client: Option<gemini_client::GeminiClient>, // Auto clipping pipeline only
    pub manual_clipping_gemini_client: Option<gemini_client::GeminiClient>, // Manual clipping only
    pub video_gemini_client: Option<gemini_client::GeminiClient>, // Video editing, generation, agents, Blender MCP
    pub gemma_client: Option<gemini_client::GeminiClient>, // Gemma 4 via Google AI Studio (text tasks, own quota)
    pub bedrock_client: Option<std::sync::Arc<crate::bedrock_client::BedrockClient>>, // AWS Bedrock (Meta Llama 3.2 90B, open-source)
    pub nvidia_nim_client: Option<nvidia_nim_client::NvidiaNimClient>, // NVIDIA NIM (text + tools, 40 RPM)
    pub nvidia_nim_vision_client: Option<nvidia_nim_client::NvidiaNimClient>, // NVIDIA NIM (vision + tools, Gemini fallback)
    pub deepseek_client: Option<deepseek_client::DeepSeekClient>, // DeepSeek V4 (OpenAI-compatible, tool calling)
    pub ollama_client: Option<ollama_client::OllamaClient>, // Self-hosted Gemma 4B via Ollama (free, multimodal)
    pub ollama_fast_client: Option<ollama_client::OllamaClient>, // Self-hosted Qwen 3 4B (fast, lightweight)
    pub claude_client: Option<claude_client::ClaudeClient>,
    pub vertex_multimodal_embeddings: Option<vertex_multimodal_embeddings::VertexMultimodalEmbeddingsClient>,
    pub voyage_embeddings: Option<voyage_embeddings::VoyageEmbeddings>,
    pub pexels_client: Option<pexels_client::PexelsClient>,
    pub elevenlabs_client: Option<elevenlabs_client::ElevenLabsClient>, // 🎙️ Audio generation
    pub vibevoice_client: Option<vibevoice_client::VibeVoiceClient>, // 🎤 Shared TTS + transcription microservice
    pub blender_mcp_client: Option<blender_mcp_client::BlenderMCPClient>, // 🎨 3D rendering + Manim
    pub r2_client: Option<Arc<r2_client::R2Client>>,                 // ☁️ Cloudflare R2 storage
    pub gcs_client: Option<gcs_client::GcsClient>,                   // 🌐 Google Cloud Storage
    pub youtube_client: Option<youtube_client::YouTubeClient>,       // 📺 YouTube integration
    pub youtube_analytics_client: Option<youtube_analytics_client::YouTubeAnalyticsClient>, // 📊 YouTube Analytics
    pub google_oauth_client_id: Option<String>, // Google OAuth client ID
    pub google_oauth_client_secret: Option<String>, // Google OAuth client secret
    pub job_manager: jobs::SharedJobManager,    // 🆕 Background job management
    pub token_manager: Option<Arc<token_manager::TokenManager>>, // 🔧 Centralized token refresh
    pub twitch_client: Option<Arc<twitch_client::TwitchClient>>, // 📺 Twitch Helix API
    pub download_semaphore: Arc<Semaphore>, // 🔒 Limits concurrent downloads to 2
    pub delivery_render_semaphore: Arc<Semaphore>, // 🎬 Limits concurrent delivery renders to avoid OOM
    pub phantombuster_client: Option<phantombuster_client::PhantomBusterClient>, // 🎯 LinkedIn scraping
    pub active_agent_channels: Arc<tokio::sync::RwLock<HashMap<String, UnboundedSender<String>>>>, // Interactive agent channels
    pub kick_client: Option<kick_client::KickClient>, // 📺 Kick.com API client
    pub zernio_client: Option<zernio_client::ZernioClient>, // 📱 Multi-platform social publishing
}

/// Validate Apify API token on startup
///
/// Tests the Apify API token to ensure it's valid before starting the clipping service.
/// Logs a warning if the token is invalid but doesn't fail startup (yt-dlp can still work).
async fn validate_apify_token() {
    match (
        std::env::var("APIFY_TOKEN").ok(),
        std::env::var("APIFY_YOUTUBE_CLIENT_ACTOR").ok(),
    ) {
        (Some(token), Some(actor)) if !token.is_empty() && !actor.is_empty() => {
            tracing::info!("🔍 Validating Apify API token...");

            let client = clipping::apify_client::ApifyClient::new(token.clone(), actor.clone());

            match client.validate_token().await {
                Ok(_) => {
                    tracing::info!("✅ Apify API token validated successfully");
                }
                Err(e) => {
                    tracing::error!(
                        "⚠️ INVALID APIFY TOKEN: {}. Check your .env file configuration.",
                        e
                    );
                    tracing::warn!(
                        "⚠️ Clipping will fall back to yt-dlp (may be slower and less reliable)"
                    );
                }
            }
        }
        _ => {
            tracing::warn!("⚠️ Apify credentials not configured. Clipping will use yt-dlp only.");
            tracing::info!("To enable Apify, set: APIFY_TOKEN and APIFY_YOUTUBE_CLIENT_ACTOR");
        }
    }
}

/// Reset orphaned jobs on startup
///
/// Jobs that were in intermediate states when the server crashed or was stopped
/// will be stuck forever unless we reset them. This function runs on startup to
/// catch any orphaned jobs and reset them to 'failed' so they can be retried.
///
/// Threshold: Jobs stuck for more than 1 hour are considered orphaned.
async fn reset_orphaned_jobs(db_pool: &sqlx::PgPool) {
    tracing::info!("🔄 Checking for orphaned jobs from previous server session...");

    match sqlx::query(
        "UPDATE clipping_jobs
         SET status = 'failed',
             error_message = 'Job was orphaned (server restarted while job was in progress). Automatically reset on startup.',
             completed_at = NOW(),
             updated_at = NOW()
         WHERE status IN ('downloading', 'analyzing', 'extracting_clips', 'posting')
         AND updated_at < NOW() - INTERVAL '1 hour'"
    )
    .execute(db_pool)
    .await
    {
        Ok(result) => {
            let count = result.rows_affected();
            if count > 0 {
                tracing::warn!("⚠️ Reset {} orphaned job(s) on startup", count);
            } else {
                tracing::info!("✅ No orphaned jobs found");
            }
        }
        Err(e) => {
            tracing::error!("❌ Failed to reset orphaned jobs: {}", e);
        }
    }
}

/// Reset app_workflows stuck in 'queued' or 'running' for more than 1 hour.
/// These get orphaned when the server restarts while a tokio::spawn task
/// was in flight or when the background executor crashes before calling heartbeat.
async fn reset_orphaned_workflows(db_pool: &sqlx::PgPool) {
    tracing::info!("🔄 Checking for orphaned workflows from previous server session...");

    match sqlx::query(
        "UPDATE app_workflows
         SET status = 'failed',
             error_message = 'Workflow was orphaned (server restarted while in progress). Automatically reset on startup.',
             completed_at = NOW(),
             updated_at = NOW()
         WHERE status IN ('queued', 'running')
         AND updated_at < NOW() - INTERVAL '1 hour'"
    )
    .execute(db_pool)
    .await
    {
        Ok(result) => {
            let count = result.rows_affected();
            if count > 0 {
                tracing::warn!("⚠️ Reset {} orphaned workflow(s) on startup", count);
            } else {
                tracing::info!("✅ No orphaned workflows found");
            }
        }
        Err(e) => {
            tracing::error!("❌ Failed to reset orphaned workflows: {}", e);
        }
    }
}

#[tokio::main]
async fn main() {
    // Install rustls CryptoProvider before any TLS connection is made.
    // qdrant-client 1.15+ uses rustls 0.23 which panics at runtime if both
    // ring and aws-lc-rs features are present and no provider is installed.
    let _ = rustls::crypto::ring::default_provider().install_default();

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

    // Verify yt-dlp is available (fail early on startup)
    match tokio::process::Command::new("yt-dlp")
        .arg("--version")
        .output()
        .await
    {
        Ok(output) if output.status.success() => {
            let version = String::from_utf8_lossy(&output.stdout);
            tracing::info!("✅ yt-dlp available: {}", version.trim());
        }
        Ok(_) => {
            tracing::error!("⚠️ yt-dlp found but version check failed");
        }
        Err(e) => {
            tracing::error!("❌ CRITICAL: yt-dlp is not installed or not in PATH: {}", e);
            tracing::error!("Clipping features will not work. Install with: pip install yt-dlp");
        }
    }

    // Create the database connection pool
    let db_pool = db::create_pool()
        .await
        .expect("Failed to create database pool.");

    // Do not block server startup on maintenance/validation tasks.
    // Cloud Run needs the process to bind quickly, and these can take tens of
    // seconds on pooled DB / third-party network hiccups.
    {
        let db_pool_for_reset = db_pool.clone();
        tokio::spawn(async move {
            reset_orphaned_jobs(&db_pool_for_reset).await;
            reset_orphaned_workflows(&db_pool_for_reset).await;
            handlers::prospects::backfill_null_service_types(&db_pool_for_reset).await;
        });
    }
    tokio::spawn(async {
        validate_apify_token().await;
    });

    // Initialize Astra DB client if credentials are provided
    let vector_db = match (
        std::env::var("ASTRA_DB_API_ENDPOINT").ok(),
        std::env::var("ASTRA_DB_APPLICATION_TOKEN").ok(),
        std::env::var("ASTRA_DB_KEYSPACE").ok(),
    ) {
        (Some(endpoint), Some(token), Some(keyspace)) => {
            tracing::info!("Initializing Astra DB connection...");
            let client = vector_db::AstraDBClient::new(endpoint, token, keyspace);
            let background_client = client.clone();
            tokio::spawn(async move {
                match background_client.create_collection().await {
                    Ok(_) => {
                        tracing::info!("Astra DB initialized successfully");
                    }
                    Err(e) => {
                        tracing::error!("Failed to initialize Astra DB collection: {}", e);
                    }
                }
            });
            Some(client)
        }
        _ => {
            tracing::warn!(
                "Astra DB credentials not found. Vector memory features will be disabled."
            );
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

    // Dedicated Gemini client for manual clipping — uses a separate API key so
    // manual clipping jobs don't exhaust the quota shared with the auto-clipping pipeline.
    // Falls back to the primary key if MANUAL_CLIPPING_GEMINI_API_KEY is not set.
    let manual_clipping_gemini_client = match std::env::var("MANUAL_CLIPPING_GEMINI_API_KEY").ok() {
        Some(api_key) => {
            tracing::info!("Initializing dedicated Gemini client for manual clipping...");
            Some(gemini_client::GeminiClient::new(api_key))
        }
        None => {
            tracing::warn!("MANUAL_CLIPPING_GEMINI_API_KEY not set — manual clipping will share the primary Gemini quota.");
            None
        }
    };

    // Dedicated Gemini client for video editing, generation, agents, and Blender MCP.
    // Isolated so video workloads don't exhaust the auto-clipping quota.
    // Falls back to the primary key if VIDEO_GEMINI_API_KEY is not set.
    let video_gemini_client = match std::env::var("VIDEO_GEMINI_API_KEY").ok() {
        Some(api_key) => {
            tracing::info!("Initializing dedicated Gemini client for video editing/generation...");
            Some(gemini_client::GeminiClient::new(api_key))
        }
        None => {
            tracing::warn!("VIDEO_GEMINI_API_KEY not set — video editing/generation will share the primary Gemini quota.");
            None
        }
    };

    // Gemma 4 via Google AI Studio — text-only tasks (DMs, scoring, outreach, code gen).
    // Uses own quota pool separate from Gemini Flash. Can reuse GEMINI_API_KEY.
    let gemma_client = std::env::var("GEMMA_API_KEY")
        .or_else(|_| std::env::var("GEMINI_API_KEY"))
        .ok()
        .map(|k| {
            tracing::info!("Initializing Gemma 4 client (text tasks, own quota)...");
            gemini_client::GeminiClient::new_with_model(k, std::env::var("GEMMA_MODEL").unwrap_or_else(|_| "gemma-4-26b-a4b-it".to_string()))
        });

    // NVIDIA NIM — text + tool-calling model (default: Gemma 4 31B, 40 RPM).
    let nvidia_nim_client = std::env::var("NVIDIA_API_KEY").ok().map(|k| {
        let model = std::env::var("NVIDIA_NIM_MODEL")
            .unwrap_or_else(|_| "google/gemma-4-31b-it".to_string());
        tracing::info!("Initializing NVIDIA NIM text client ({})...", model);
        nvidia_nim_client::NvidiaNimClient::with_model(k, model)
    });

    // NVIDIA NIM — vision + tool-calling model (Gemini multimodal fallback).
    let nvidia_nim_vision_client = std::env::var("NVIDIA_API_KEY").ok().map(|k| {
        let model = std::env::var("NVIDIA_NIM_VISION_MODEL")
            .unwrap_or_else(|_| "nvidia/nemotron-3-nano-omni-30b-a3b-reasoning".to_string());
        tracing::info!("Initializing NVIDIA NIM vision client ({})...", model);
        nvidia_nim_client::NvidiaNimClient::with_model(k, model)
    });

    // AWS Bedrock — Meta Llama 3.2 90B via Converse API (pay-per-token, no GPU needed).
    let bedrock_client = {
        let region = std::env::var("AWS_REGION").unwrap_or_else(|_| "us-east-1".to_string());
        let model = std::env::var("BEDROCK_MODEL_ID").ok();
        let has_creds = std::env::var("AWS_ACCESS_KEY_ID").is_ok()
            && std::env::var("AWS_SECRET_ACCESS_KEY").is_ok();
        if has_creds {
            tracing::info!(
                "Initializing AWS Bedrock client (region: {}, model: {})...",
                region,
                model.as_deref().unwrap_or("meta.llama3-2-90b-vision-instruct-maas")
            );
            Some(std::sync::Arc::new(
                crate::bedrock_client::BedrockClient::new_async(&region, model).await,
            ))
        } else {
            tracing::warn!("AWS Bedrock not configured (missing AWS_ACCESS_KEY_ID/AWS_SECRET_ACCESS_KEY)");
            None
        }
    };

    // DeepSeek V4 — OpenAI-compatible, fallback when Gemini/NIM are unavailable.
    let deepseek_client = std::env::var("DEEPSEEK_API_KEY").ok().map(|k| {
        tracing::info!("Initializing DeepSeek V4 client (model: {})...",
            std::env::var("DEEPSEEK_MODEL").unwrap_or_else(|_| "deepseek-v4-flash".to_string()));
        deepseek_client::DeepSeekClient::new(k)
    });

    // Ollama — self-hosted Gemma 4B (multimodal, free, on separate t3.xlarge).
    // Used as the primary text generation provider before NVIDIA NIM.
    let ollama_client = {
        tracing::info!("Initializing Ollama client (gemma4:12b on {})...", ollama_client::OLLAMA_DEFAULT_URL);
        let client = ollama_client::OllamaClient::new();
        // Warm up the model in background — non-blocking, non-fatal.
        let warmup_client = client.clone();
        tokio::spawn(async move {
            warmup_client.warmup().await;
        });
        Some(client)
    };

    // Ollama fast — lightweight Qwen 3 4B (2.5GB) for fast text-only tasks.
    // Used as the first attempt in generate_text_fast / generate_text_best_effort
    // before falling back to the heavier gemma4:12b.
    let ollama_fast_client = {
        tracing::info!("Initializing Ollama fast client (qwen3:4b on {})...", ollama_client::OLLAMA_DEFAULT_URL);
        let client = ollama_client::OllamaClient::new_with_model("qwen3:4b");
        let warmup_fast = client.clone();
        tokio::spawn(async move {
            warmup_fast.warmup().await;
        });
        Some(client)
    };

    let vertex_multimodal_embeddings =
        vertex_multimodal_embeddings::VertexMultimodalEmbeddingsClient::from_env().map(|client| {
            tracing::info!("Initializing Vertex multimodal embeddings client for gemini_mm lane...");
            client
        });

    // Initialize Qdrant client if both API key and URL are provided
    let qdrant_client = match (
        std::env::var("QDRANT_API_KEY").ok(),
        std::env::var("QDRANT_URL").ok(),
    ) {
        (Some(api_key), Some(qdrant_url)) => {
            tracing::info!("Initializing Qdrant vector database...");
            match qdrant_client::QdrantClient::new(qdrant_url, Some(api_key)).await {
                Ok(client) => {
                    tracing::info!("Qdrant client initialized; deferring collection/index setup to background");
                    let background_client = client.clone();
                    tokio::spawn(async move {
                        match background_client.create_collection().await {
                            Ok(_) => tracing::info!("Qdrant initialized successfully"),
                            Err(e) => tracing::error!("Failed to initialize Qdrant collection: {}", e),
                        }
                    });
                    Some(client)
                }
                Err(e) => {
                    tracing::error!("Failed to connect to Qdrant: {}", e);
                    None
                }
            }
        }
        _ => {
            tracing::info!("Qdrant disabled — set QDRANT_API_KEY and QDRANT_URL to enable it.");
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

    // Initialize BlenderMCPClient if URL is configured
    let blender_mcp_client = match (
        std::env::var("BLENDER_MCP_URL").ok(),
        std::env::var("BLENDER_MCP_API_KEY").ok(),
    ) {
        (Some(url), _) if !url.is_empty() => {
            tracing::info!("🎨 Initializing BlenderMCPClient (3D rendering + Manim)...");
            let api_key = std::env::var("BLENDER_MCP_API_KEY").unwrap_or_default();
            Some(blender_mcp_client::BlenderMCPClient::new(url, api_key))
        }
        _ => {
            tracing::info!(
                "BlenderMCPServer not configured (set BLENDER_MCP_URL to enable 3D rendering)"
            );
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
            tracing::warn!(
                "ELEVEN_LABS_API_KEY not found. Audio generation features will be limited."
            );
            tracing::info!("To enable Eleven Labs integration, set: ELEVEN_LABS_API_KEY");
            None
        }
    };

    let vibevoice_client = match std::env::var("VIBEVOICE_SERVICE_URL").ok() {
        Some(url) if !url.is_empty() => {
            tracing::info!("🎤 Initializing VibeVoice microservice client...");
            let api_key = std::env::var("VIBEVOICE_SERVICE_API_KEY").ok();
            Some(vibevoice_client::VibeVoiceClient::new(url, api_key))
        }
        _ => {
            tracing::info!("VibeVoice service not configured (set VIBEVOICE_SERVICE_URL to enable shared TTS/transcription)");
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

    // Initialize Twitch client if credentials are provided
    let twitch_client_opt: Option<Arc<twitch_client::TwitchClient>> = match (
        std::env::var("TWITCH_TV_CLIENT_ID").ok(),
        std::env::var("TWITCH_TV_CLIENT_SECRET").ok(),
    ) {
        (Some(id), Some(secret)) if !id.is_empty() && !secret.is_empty() => {
            tracing::info!("📺 Initializing Twitch Helix API client...");
            Some(Arc::new(twitch_client::TwitchClient::new(
                id,
                secret,
                db_pool.clone(),
            )))
        }
        _ => {
            tracing::warn!(
                "Twitch client not configured (set TWITCH_TV_CLIENT_ID and TWITCH_TV_CLIENT_SECRET)"
            );
            None
        }
    };

    // Initialize centralized token manager
    let token_manager = if let Some(ref yt_client) = youtube_client {
        if google_oauth_client_id.is_some() && google_oauth_client_secret.is_some() {
            let tm = token_manager::TokenManager::new(
                yt_client.clone(),
                google_oauth_client_id.clone().unwrap(),
                google_oauth_client_secret.clone().unwrap(),
                db_pool.clone(),
            );
            tracing::info!(
                "🔧 Token manager initialized for centralized YouTube OAuth token refresh"
            );
            Some(Arc::new(tm))
        } else {
            tracing::warn!("Token manager disabled (OAuth credentials not configured)");
            None
        }
    } else {
        None
    };

    // Initialize Cloudflare R2 client
    let r2_client = match (
        std::env::var("R2_ACCOUNT_ID").ok(),
        std::env::var("R2_ACCESS_KEY_ID").ok(),
        std::env::var("R2_SECRET_ACCESS_KEY").ok(),
        std::env::var("R2_BUCKET").ok(),
    ) {
        (Some(account_id), Some(access_key), Some(secret_key), Some(bucket))
            if !account_id.is_empty() && !access_key.is_empty() && !secret_key.is_empty() =>
        {
            tracing::info!("☁️ Initializing Cloudflare R2 storage client (bucket: {bucket})...");
            match r2_client::R2Client::new(&account_id, &access_key, &secret_key, &bucket).await {
                Ok(client) => {
                    tracing::info!("☁️ R2 client initialized; deferring health check to background");
                    let background_client = client.clone();
                    let bucket_name = bucket.clone();
                    tokio::spawn(async move {
                        if background_client.health_check().await {
                            tracing::info!("✅ R2 storage connected — bucket: {bucket_name}");
                        } else {
                            tracing::warn!("R2 health check failed — uploads may fall back to local disk semantics");
                        }
                    });
                    Some(Arc::new(client))
                }
                Err(e) => {
                    tracing::warn!(
                        "Failed to init R2 client: {e} — file storage will use local disk"
                    );
                    None
                }
            }
        }
        _ => {
            tracing::info!("R2 not configured (set R2_ACCOUNT_ID, R2_ACCESS_KEY_ID, R2_SECRET_ACCESS_KEY, R2_BUCKET)");
            None
        }
    };

    // Initialize GCS client (Google Cloud Storage)
    let gcs_client = gcs_client::GcsClient::from_env().await;
    if gcs_client.is_some() {
        tracing::info!("☁️ Initialized Google Cloud Storage client");
    }

    // Create the shared state
    let shared_state = Arc::new(AppState {
        db_pool,
        vector_db,
        qdrant_client,
        gemini_client,
        manual_clipping_gemini_client,
        video_gemini_client,
        gemma_client,
        bedrock_client,
        nvidia_nim_client,
        nvidia_nim_vision_client,
        deepseek_client,
        ollama_client,
        ollama_fast_client,
        claude_client,
        vertex_multimodal_embeddings,
        voyage_embeddings,
        pexels_client,
        elevenlabs_client,
        vibevoice_client,
        blender_mcp_client,
        r2_client,
        gcs_client,
        youtube_client,
        youtube_analytics_client,
        google_oauth_client_id,
        google_oauth_client_secret,
        job_manager,
        token_manager,
        twitch_client: twitch_client_opt,
        phantombuster_client: std::env::var("PHANTOMBUSTER_API_KEY")
            .ok()
            .filter(|k| !k.is_empty())
            .map(|k| {
                tracing::info!("🎯 PhantomBuster client initialized");
                phantombuster_client::PhantomBusterClient::new(k)
            }),
        download_semaphore: Arc::new(Semaphore::new(
            std::env::var("DOWNLOAD_SEMAPHORE_PERMITS")
                .ok()
                .and_then(|v| v.parse::<usize>().ok())
                .unwrap_or(4), // 4 concurrent downloads (was 2)
        )),
        delivery_render_semaphore: Arc::new(Semaphore::new(
            std::env::var("DELIVERY_RENDER_PERMITS")
                .ok()
                .and_then(|v| v.parse::<usize>().ok())
                .unwrap_or(1), // default to 1 active delivery render to reduce memory spikes
        )),
        active_agent_channels: Arc::new(tokio::sync::RwLock::new(HashMap::new())),
        kick_client: {
            let id = std::env::var("KICK_CLIENT_ID").unwrap_or_default();
            let secret = std::env::var("KICK_CLIENT_SECRET").unwrap_or_default();
            if !id.is_empty() && !secret.is_empty() {
                tracing::info!("📺 Kick.com client initialized");
                Some(kick_client::KickClient::new(id, secret))
            } else {
                tracing::warn!("📺 Kick.com client not configured (missing KICK_CLIENT_ID/SECRET)");
                None
            }
        },
        zernio_client: {
            let api_key = std::env::var("ZERNIO_API_KEY").unwrap_or_default();
            if !api_key.is_empty() {
                tracing::info!("📱 Zernio social publishing client initialized");
                Some(zernio_client::ZernioClient::new(api_key))
            } else {
                tracing::warn!("Zernio client not configured (missing ZERNIO_API_KEY)");
                None
            }
        },
    });

    // Admin-only routes
    let admin_only_routes = Router::new()
        .route("/api/docs", axum::routing::get(api_documentation))
        .layer(axum::middleware::from_fn(
            middleware::admin::admin_middleware,
        ))
        .layer(axum::middleware::from_fn(middleware::auth::auth_middleware));

    // Build our application with all routes and shared state
    let app = Router::new()
        .merge(handlers::ui::ui_routes())
        .merge(handlers::ui::ui_private_routes())
        .merge(handlers::auth::auth_routes())
        .merge(handlers::chat::chat_routes())
        .merge(handlers::upload::upload_routes())
        .merge(handlers::output::output_routes())
        .merge(handlers::admin::admin_routes())
        .merge(handlers::background_routes::background_routes())
        .merge(handlers::jobs::job_routes()) // 🆕 Job control endpoints
        .merge(handlers::youtube::youtube_routes()) // 📺 YouTube integration
        .merge(handlers::clipping::clipping_routes()) // 📹 YouTube clipping feature
        .merge(handlers::health::health_routes()) // 🏥 Health check and monitoring
        .merge(handlers::tools::tools_routes()) // 🎬 On-demand FFmpeg tools
        .merge(handlers::gig_templates::gig_template_routes()) // 💼 Gig templates
        .merge(handlers::manual_clipping::manual_clipping_routes()) // ✂️ Manual clipping
        .merge(handlers::prospects::prospect_routes()) // 🎯 Prospect finder (admin)
        .merge(handlers::prospects::instagram_routes()) // 📸 Instagram leads (all users)
        .merge(handlers::api_access::api_access_routes()) // 💳 Agency USDC license
        .merge(handlers::subscribe::subscribe_routes()) // 💳 Regular-user $15/mo paywall
        .merge(handlers::paypal::paypal_routes()) // 💳 PayPal/card checkout for service packs
        .merge(handlers::crypto_payments::crypto_routes()) // 💳 USDC on Base checkout for service packs
        .merge(handlers::social_publish::social_routes()) // 📱 Multi-platform social publishing via Zernio
        .merge(handlers::campaigns::campaign_routes()) // 📅 Content campaign engine
        .merge(handlers::auth::clipper_invite_routes()) // 🎫 Clipper invites
        .merge(admin_only_routes) // Admin-only routes like API docs
        .route("/api/status", axum::routing::get(api_status))
        // .layer(axum::middleware::from_fn(middleware::frontend_rate_limit::frontend_rate_limit_middleware))
        // .layer(axum::middleware::from_fn(middleware::rate_limit::rate_limit_middleware))
        .layer(axum::middleware::from_fn(
            middleware::logging::request_logging_middleware,
        ))
        .layer(CorsLayer::permissive())
        .layer(DefaultBodyLimit::max(100 * 1024 * 1024)) // 100MB limit for video uploads
        .layer(Extension(shared_state.clone()));

    // Bind the port as early as possible so Cloud Run startup probes can pass
    // while the rest of the background platform comes online.
    let port = std::env::var("PORT").unwrap_or_else(|_| "3000".to_string());
    let bind_addr = format!("0.0.0.0:{}", port);
    let listener = tokio::net::TcpListener::bind(&bind_addr)
        .await
        .expect(&format!("Failed to bind to {bind_addr}"));
    tracing::info!("listening on {}", listener.local_addr().unwrap());

    // Start background polling task for YouTube clipping (requires youtube_client)
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
                    Err(e) => {
                        tracing::error!("❌ Channel polling failed: {}", e);

                        // If quota exceeded, pause for 1 hour before retrying
                        if e.to_string().contains("Quota exceeded") {
                            tracing::warn!("⏸️ YouTube API quota exhausted. Pausing polling for 3600 seconds (1 hour)");
                            tokio::time::sleep(tokio::time::Duration::from_secs(3600)).await;
                            continue;
                        }
                    }
                }

                // IMPROVED: Poll every 5 minutes now that we use quota-efficient playlistItems API (1 unit vs 100)
                // With 10 channels × 288 polls/day × 1 unit = 2,880 quota (only 28.8% of daily 10,000 limit)
                // Old search API would use: 10 × 288 × 100 = 288,000 quota (28.8x over limit!)
                tokio::time::sleep(tokio::time::Duration::from_secs(300)).await;
            }
        });

        // Start token refresh worker for YouTube channels (runs every 15 minutes)
        if let Some(token_manager) = shared_state.token_manager.clone() {
            let db_pool = shared_state.db_pool.clone();
            tokio::spawn(async move {
                let mut interval = tokio::time::interval(tokio::time::Duration::from_secs(900)); // 15 minutes
                loop {
                    interval.tick().await;
                    tracing::info!(
                        "🔄 Running scheduled token refresh for all YouTube channels..."
                    );

                    match jobs::token_refresh::refresh_all_expiring_tokens(&token_manager, &db_pool)
                        .await
                    {
                        Ok(refreshed_count) => {
                            if refreshed_count > 0 {
                                tracing::info!("✅ Refreshed {} channel tokens", refreshed_count);
                            }
                        }
                        Err(e) => {
                            tracing::error!("❌ Token refresh worker error: {}", e);
                        }
                    }
                }
            });
            tracing::info!("✅ Token refresh worker started (runs every 15 minutes)");
        } else {
            tracing::warn!("TokenManager not available - token refresh worker disabled");
        }
    } else {
        tracing::warn!("YouTube client not available - YouTube channel polling disabled");
    }

    // ── Instagram lead auto-importer — polls PB every 5 min ──────────────────────────
    {
        let ig_state = shared_state.clone();
        tokio::spawn(async move {
            tracing::info!("📸 Instagram job poller started (5-min interval)");
            let mut interval = tokio::time::interval(tokio::time::Duration::from_secs(300));
            loop {
                interval.tick().await;
                handlers::prospects::poll_instagram_jobs(&ig_state).await;
            }
        });
    }

    // ── PhantomBuster launch dispatcher — every 30s promotes one queued job
    //    per agent to running if there's no running job. Exists because PB
    //    free/team plans cap parallel phantom runs, and the users' search-bar
    //    clicks + auto-discover fan-out produce "Maximum parallel executions
    //    reached" errors when we launch without queueing. See prospects.rs.
    {
        let disp_state = shared_state.clone();
        tokio::spawn(async move {
            tracing::info!("🚦 PhantomBuster dispatcher started (30s interval)");
            let mut interval = tokio::time::interval(tokio::time::Duration::from_secs(30));
            // First tick fires immediately; skip it so startup isn't noisy.
            interval.tick().await;
            loop {
                interval.tick().await;
                handlers::prospects::dispatch_queued_pb_jobs(&disp_state).await;
            }
        });
    }

    // ── Campaign engine — generates + schedules daily content ────────────────
    {
        let camp_state = shared_state.clone();
        tokio::spawn(async move {
            tracing::info!("📅 Campaign engine started (10-min interval)");
            let mut interval = tokio::time::interval(tokio::time::Duration::from_secs(600));
            interval.tick().await; // skip first immediate tick
            loop {
                interval.tick().await;
                crate::services::campaign_engine::process_campaigns(&camp_state).await;
            }
        });
    }

    // ── Telegram sales bot — long-polls @videosync_sales_bot for DMs,
    //    replies via AI (NVIDIA NIM → Gemma → Gemini). No-ops when
    //    TELEGRAM_BOT_TOKEN isn't set. Safe, stateless, no ban risk.
    {
        let tg_state = shared_state.clone();
        tokio::spawn(async move {
            telegram_bot::start_worker(tg_state).await;
        });
    }

    // ── Telegram MTProto watcher — userbot that monitors configured
    //    channels for paid-gig opportunities and pings the admin via
    //    the sales bot with a pre-written custom DM. Waits in a polling
    //    loop until the admin completes phone-code login from the admin
    //    UI, then runs `next_update()` indefinitely. No-ops when
    //    TELEGRAM_API_ID / TELEGRAM_API_HASH aren't set.
    {
        let tg_state = shared_state.clone();
        tokio::spawn(async move {
            telegram_client::start_watcher(tg_state).await;
        });
    }

    // ── Clipping worker and health tasks — always start regardless of youtube_client ──
    // V1 fix: worker must run even when YOUTUBE_API_KEY is not set. Only the channel
    // polling monitor (above) needs youtube_client. The worker itself only needs gemini_client.
    {
        // Background worker for executing clipping jobs in separate thread.
        // Uses std::thread + restart loop to recover from unexpected panics.
        let worker_state = shared_state.clone();
        std::thread::spawn(move || loop {
            let state = worker_state.clone();
            let handle = std::thread::spawn(move || {
                let rt = tokio::runtime::Runtime::new()
                    .expect("Failed to create tokio runtime for worker");
                rt.block_on(
                    async move { jobs::clipping_worker::run_clipping_worker_loop(state).await },
                );
            });
            match handle.join() {
                Ok(_) => {
                    tracing::warn!("⚠️ Clipping worker exited unexpectedly. Restarting in 5s...");
                }
                Err(e) => {
                    tracing::error!(
                        "💥 Clipping worker thread panicked: {:?}. Restarting in 5s...",
                        e
                    );
                }
            }
            std::thread::sleep(std::time::Duration::from_secs(5));
        });

        // Independent stuck-job detection task — runs every 60s in the main tokio runtime.
        let stuck_detect_state = shared_state.clone();
        tokio::spawn(async move {
            let mut interval = tokio::time::interval(tokio::time::Duration::from_secs(60));
            interval.tick().await; // skip first immediate tick
            loop {
                interval.tick().await;
                if let Err(e) =
                    jobs::clipping_worker::run_stuck_job_detection(&stuck_detect_state).await
                {
                    tracing::warn!("Stuck job detection error: {}", e);
                }
            }
        });

        // Worker process-level heartbeat — 30s interval, independent of job execution.
        // Health endpoint reads this to determine if the worker process is alive.
        let hb_state = shared_state.clone();
        let hb_worker_id = {
            let config = jobs::worker_config::WorkerConfig::from_env();
            config.worker_id.clone()
        };
        tokio::spawn(async move {
            let mut interval = tokio::time::interval(tokio::time::Duration::from_secs(30));
            loop {
                interval.tick().await;
                jobs::clipping_worker::update_worker_heartbeat(&hb_state, &hb_worker_id, None)
                    .await;
            }
        });

        // DB-aware clipping supervisor — investigates queue pathologies, suppresses
        // duplicates, annotates quota/capacity waits, and escalates fallback deliveries.
        let supervisor_state = shared_state.clone();
        tokio::spawn(async move {
            jobs::clipping_supervisor::run_clipping_supervisor_loop(supervisor_state).await;
        });

        tracing::info!(
            "✅ Clipping worker enabled — auto-retry, stuck detection, supervisor, and heartbeat active"
        );
    }

    // Spawn Twitch → YouTube channel auto-mapper cron (10-minute interval)
    if shared_state.twitch_client.is_some() {
        let twitch_mapper_state = shared_state.clone();
        tokio::spawn(async move {
            tracing::info!("📺 Starting Twitch channel auto-mapper cron (10-minute interval)...");
            jobs::twitch_mapper_job::run_twitch_mapping_cron(twitch_mapper_state).await;
        });
        tracing::info!("✅ Twitch mapper cron started");
    } else {
        tracing::info!("Twitch mapper cron disabled (Twitch client not configured)");
    }

    // Spawn YouTube Analytics sync job (runs every 6 hours)
    // Syncs view/like/comment counts from YouTube Analytics API for all published clips,
    // then refreshes viral_factor_performance and duration_performance tables.
    {
        let analytics_db = shared_state.db_pool.clone();
        tokio::spawn(async move {
            tracing::info!("📊 Starting YouTube Analytics sync job (every 6 hours)...");
            jobs::AnalyticsSyncJob::new(analytics_db).start().await;
        });
        tracing::info!("✅ Analytics sync job started");
    }

    // Run the server with ConnectInfo to provide socket addresses for rate limiting
    axum::serve(
        listener,
        app.into_make_service_with_connect_info::<std::net::SocketAddr>(),
    )
    .await
    .unwrap();
}

// Production-grade logging configuration
fn init_logging() -> Result<(), Box<dyn std::error::Error>> {
    use tracing_subscriber::{
        fmt, layer::SubscriberExt, util::SubscriberInitExt, EnvFilter, Layer,
    };

    // Get log level from environment or default to INFO for production
    let log_level = std::env::var("RUST_LOG").unwrap_or_else(|_| {
        if cfg!(debug_assertions) {
            "debug,video_editor=trace,sqlx=info,reqwest=info,hyper=info,tower=info".to_string()
        } else {
            "info,video_editor=info,sqlx=warn,reqwest=warn,hyper=warn,tower=warn".to_string()
        }
    });

    let env_filter =
        EnvFilter::try_from_default_env().or_else(|_| EnvFilter::try_new(&log_level))?;

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
    tracing::info!(
        "Build mode: {}",
        if cfg!(debug_assertions) {
            "development"
        } else {
            "production"
        }
    );
    tracing::info!("Log level: {}", log_level);

    // Log environment configuration
    let gemini_configured = std::env::var("GEMINI_API_KEY").is_ok();
    let qdrant_configured = std::env::var("QDRANT_API_KEY").is_ok();
    let astra_configured = std::env::var("ASTRA_DB_API_ENDPOINT").is_ok()
        && std::env::var("ASTRA_DB_APPLICATION_TOKEN").is_ok();
    let db_configured = std::env::var("DATABASE_URL").is_ok();

    tracing::info!(
        "Configuration - Database: {}, Gemini AI: {}, Qdrant: {}, AstraDB: {}",
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
async fn api_status(
    Extension(state): Extension<Arc<AppState>>,
) -> axum::response::Json<serde_json::Value> {
    use serde_json::json;

    let db_status = match sqlx::query("SELECT 1").fetch_one(&state.db_pool).await {
        Ok(_) => "healthy",
        Err(_) => "unhealthy",
    };

    let gemini_status = if state.gemini_client.is_some() {
        "configured"
    } else {
        "not_configured"
    };
    let claude_status = if state.claude_client.is_some() {
        "configured"
    } else {
        "not_configured"
    };
    let qdrant_status = if state.qdrant_client.is_some() {
        "configured"
    } else {
        "not_configured"
    };
    let astra_status = if state.vector_db.is_some() {
        "configured"
    } else {
        "not_configured"
    };
    let elevenlabs_status = if state.elevenlabs_client.is_some() {
        "configured"
    } else {
        "not_configured"
    };
    let vibevoice_status = if state.vibevoice_client.is_some() {
        "configured"
    } else {
        "not_configured"
    };

    axum::response::Json(json!({
        "status": "operational",
        "version": env!("CARGO_PKG_VERSION"),
        "services": {
            "database": db_status,
            "claude_ai": claude_status,
            "gemini_ai": gemini_status,
            "elevenlabs_audio": elevenlabs_status,
            "vibevoice_audio": vibevoice_status,
            "qdrant_vector_db": qdrant_status,
            "astra_vector_db": astra_status
        },
        "features": {
            "video_editing_tools": 327,
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
