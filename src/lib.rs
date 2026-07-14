// lib.rs - Main library file that exports all modules

use std::sync::Arc;
use tokio::sync::Semaphore;

// Core modules
pub mod agent;
pub mod ai_tool_selector;
pub mod bedrock_client;
pub mod browserbase_client;
pub mod blender_mcp_client;
pub mod blender_quality;
pub mod claude_client;
pub mod clipping;
pub mod cloud_storage;
pub mod db;
pub mod deepseek_client;
pub mod email;
pub mod elevenlabs_client;
pub mod gemini_client;
pub mod handlers;
pub mod jobs;
pub mod kick_client;
pub mod kick_vod_scraper;
pub mod llm_utils;
pub mod middleware;
pub mod models;
pub mod nvidia_nim_client;
pub mod ollama_client;
pub mod pexels_client;
pub mod phantombuster_client;
pub mod product_hunt_client;
pub mod portfolio_samples;
pub mod qdrant_client;
pub mod r2_client;
pub mod render_review;
pub mod services;
pub mod sketchfab_client;
pub mod telegram_bot;
pub mod telegram_client;
pub mod token_manager;
pub mod tool_registry;
pub mod twitch_client;
pub mod ffmpeg_mcp_client;
pub mod utils;
pub mod vector_db;
pub mod vertex_multimodal_embeddings;
pub mod vibevoice_client;
pub mod voyage_embeddings;
pub mod x402;
pub mod youtube_analytics_client;
pub mod youtube_client;
pub mod zernio_client;

// Video processing modules
pub mod advanced;
pub mod audio;
pub mod core;
pub mod export;
pub mod manual_clipping_tests;
pub mod portfolio_tests;
pub mod transform;
pub mod types;
pub mod visual;
pub mod workflows;

// Re-export commonly used types for convenience
pub use advanced::*;
pub use audio::*;
pub use core::*;
pub use export::*;
pub use transform::*;
pub use types::*;
pub use visual::*;

// AppState struct for integration tests
pub struct AppState {
    pub db_pool: sqlx::PgPool,
    pub vector_db: Option<vector_db::AstraDBClient>,
    pub qdrant_client: Option<qdrant_client::QdrantClient>,
    pub gemini_client: Option<gemini_client::GeminiClient>,
    pub manual_clipping_gemini_client: Option<gemini_client::GeminiClient>,
    pub video_gemini_client: Option<gemini_client::GeminiClient>,
    pub gemma_client: Option<gemini_client::GeminiClient>,
    pub bedrock_client: Option<Arc<bedrock_client::BedrockClient>>,
    pub nvidia_nim_client: Option<nvidia_nim_client::NvidiaNimClient>,
    pub nvidia_nim_vision_client: Option<nvidia_nim_client::NvidiaNimClient>,
    pub deepseek_client: Option<deepseek_client::DeepSeekClient>,
    pub ollama_client: Option<ollama_client::OllamaClient>,
    pub ollama_fast_client: Option<ollama_client::OllamaClient>,
    pub claude_client: Option<claude_client::ClaudeClient>,
    pub vertex_multimodal_embeddings:
        Option<vertex_multimodal_embeddings::VertexMultimodalEmbeddingsClient>,
    pub voyage_embeddings: Option<voyage_embeddings::VoyageEmbeddings>,
    pub pexels_client: Option<pexels_client::PexelsClient>,
    pub elevenlabs_client: Option<elevenlabs_client::ElevenLabsClient>,
    pub vibevoice_client: Option<vibevoice_client::VibeVoiceClient>,
    pub blender_mcp_client: Option<blender_mcp_client::BlenderMCPClient>,
    pub r2_client: Option<std::sync::Arc<r2_client::R2Client>>,
    pub youtube_client: Option<youtube_client::YouTubeClient>,
    pub youtube_analytics_client: Option<youtube_analytics_client::YouTubeAnalyticsClient>,
    pub google_oauth_client_id: Option<String>,
    pub google_oauth_client_secret: Option<String>,
    pub job_manager: jobs::SharedJobManager,
    pub token_manager: Option<Arc<token_manager::TokenManager>>,
    pub twitch_client: Option<Arc<twitch_client::TwitchClient>>,
    pub download_semaphore: Arc<Semaphore>,
    pub delivery_render_semaphore: Arc<Semaphore>,
    pub phantombuster_client: Option<phantombuster_client::PhantomBusterClient>,
    pub active_agent_channels:
        std::sync::Arc<tokio::sync::RwLock<std::collections::HashMap<String, tokio::sync::mpsc::UnboundedSender<String>>>>,
    pub kick_client: Option<kick_client::KickClient>,
    pub zernio_client: Option<zernio_client::ZernioClient>,
}
