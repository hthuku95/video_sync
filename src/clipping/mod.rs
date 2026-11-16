// Clipping primitives shared by the ZERNIO campaign trio
// (kick_auto_clipper / twitch_clipping / youtube_clipping — campaign engine,
// generate_clip_compilation tool). The legacy YouTube-API auto-clipping
// system (monitor, worker, phase pipeline, manual clipping) was DELETED
// Nov 14-15 2026 per owner directive — replaced by education posting.

pub mod apify_client; // Primary downloader with 5-tier fallback system
pub mod gemini_video_analyzer; // Single-call YouTube URL analysis
pub mod models;
pub mod rusty_ytdl_client; // Pure Rust YouTube downloader (last resort)
pub mod smart_clipping;
pub mod ytdlp_api_client; // FastAPI yt-dlp microservice (HTTP API)

pub use rusty_ytdl_client::RustyYtdlClient;
pub use ytdlp_api_client::YtdlpApiClient;
