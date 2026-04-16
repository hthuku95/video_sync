// lib.rs - Main library file that exports all modules

use std::sync::Arc;
use tokio::sync::Semaphore;

// Core modules
pub mod agent;
pub mod db;
pub mod gemini_client;
pub mod nvidia_nim_client;
pub mod llm_utils;
pub mod claude_client;
pub mod voyage_embeddings;
pub mod elevenlabs_client;
pub mod blender_mcp_client;
pub mod r2_client;
pub mod phantombuster_client;
pub mod youtube_client;
pub mod youtube_analytics_client;
pub mod twitch_client;
pub mod handlers;
pub mod jobs;
pub mod workflow;
pub mod middleware;
pub mod models;
pub mod pexels_client;
pub mod qdrant_client;
pub mod services;
pub mod vector_db;
pub mod clipping;
pub mod tool_selector;
pub mod ai_tool_selector;
pub mod utils;
pub mod token_manager;
pub mod x402;
pub mod telegram_bot;

// Video processing modules
pub mod types;
pub mod core;
pub mod audio;
pub mod visual;
pub mod transform;
pub mod advanced;
pub mod export;
pub mod workflows;
pub mod portfolio_tests;
pub mod manual_clipping_tests;

// Re-export commonly used types for convenience
pub use types::*;
pub use core::*;
pub use audio::*;
pub use visual::*;
pub use transform::*;
pub use advanced::*;
pub use export::*;

// AppState struct for integration tests
pub struct AppState {
    pub db_pool: sqlx::PgPool,
    pub vector_db: Option<vector_db::AstraDBClient>,
    pub qdrant_client: Option<qdrant_client::QdrantClient>,
    pub gemini_client: Option<gemini_client::GeminiClient>,
    pub manual_clipping_gemini_client: Option<gemini_client::GeminiClient>,
    pub video_gemini_client: Option<gemini_client::GeminiClient>,
    pub gemma_client: Option<gemini_client::GeminiClient>,
    pub nvidia_nim_client: Option<nvidia_nim_client::NvidiaNimClient>,
    pub claude_client: Option<claude_client::ClaudeClient>,
    pub voyage_embeddings: Option<voyage_embeddings::VoyageEmbeddings>,
    pub pexels_client: Option<pexels_client::PexelsClient>,
    pub elevenlabs_client: Option<elevenlabs_client::ElevenLabsClient>,
    pub blender_mcp_client: Option<blender_mcp_client::BlenderMCPClient>,
    pub r2_client: Option<std::sync::Arc<r2_client::R2Client>>,
    pub youtube_client: Option<youtube_client::YouTubeClient>,
    pub youtube_analytics_client: Option<youtube_analytics_client::YouTubeAnalyticsClient>,
    pub google_oauth_client_id: Option<String>,
    pub google_oauth_client_secret: Option<String>,
    pub job_manager: jobs::SharedJobManager,
    pub workflow_checkpointer: Option<workflow::checkpoint::WorkflowCheckpointer>,
    pub token_manager: Option<Arc<token_manager::TokenManager>>,
    pub twitch_client: Option<Arc<twitch_client::TwitchClient>>,
    pub download_semaphore: Arc<Semaphore>,
    pub phantombuster_client: Option<phantombuster_client::PhantomBusterClient>,
}