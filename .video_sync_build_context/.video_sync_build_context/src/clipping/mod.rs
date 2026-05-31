// YouTube Clipping Module
// Handles monitoring external channels, downloading videos, AI clip extraction, and posting to YouTube

pub mod ai_clipper;
pub mod apify_client; // Primary downloader with 5-tier fallback system
pub mod clip_enhancer;
pub mod gemini_video_analyzer; // Single-call YouTube URL analysis (replaces frame-by-frame pipeline)
pub mod models;
pub mod monitor;
pub mod performance_tracker;
pub mod rust_yt_downloader_client; // Strategy #4: Feature-rich yt-dlp wrapper
pub mod rustube_client; // Strategy #2: Pure Rust downloader (no external deps)
pub mod rusty_ytdl_client; // Strategy #5: Pure Rust YouTube downloader (last resort)
pub mod thumbnail_generator;
pub mod uploader;
pub mod ytdlp_api_client; // Strategy #3: FastAPI yt-dlp microservice (HTTP API)
pub mod ytdlp_client; // Legacy: yt-dlp CLI wrapper (deprecated, kept for compatibility)

// Re-export commonly used types
#[allow(unused_imports)] // Exported for external use
pub use ai_clipper::AiClipper;
#[allow(unused_imports)] // Exported for external use
pub use monitor::ChannelMonitor;
#[allow(unused_imports)] // Exported for external use
pub use performance_tracker::PerformanceTracker;
#[allow(unused_imports)] // Exported for external use
pub use rust_yt_downloader_client::RustYtDownloaderClient;
#[allow(unused_imports)] // Exported for external use
pub use rustube_client::RustubeClient;
#[allow(unused_imports)] // Exported for external use
pub use rusty_ytdl_client::RustyYtdlClient;
#[allow(unused_imports)] // Exported for external use
pub use thumbnail_generator::ThumbnailGenerator;
#[allow(unused_imports)] // Exported for external use
pub use uploader::ClipUploader;
#[allow(unused_imports)] // Exported for external use
pub use ytdlp_api_client::YtdlpApiClient;
#[allow(unused_imports)] // Exported for external use
pub use ytdlp_client::YtDlpClient;
