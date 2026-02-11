// YouTube Clipping Module
// Handles monitoring external channels, downloading videos, AI clip extraction, and posting to YouTube

pub mod models;
pub mod apify_client;          // Primary downloader with 5-tier fallback system
pub mod rusty_ytdl_client;     // Strategy #5: Pure Rust YouTube downloader (last resort)
pub mod rustube_client;        // Strategy #2: Pure Rust downloader (no external deps)
pub mod ytdlp_client;          // Strategy #3: yt-dlp CLI wrapper (battle-tested)
pub mod rust_yt_downloader_client; // Strategy #4: Feature-rich yt-dlp wrapper
pub mod monitor;
pub mod ai_clipper;
pub mod uploader;
pub mod performance_tracker;
pub mod thumbnail_generator;

// Re-export commonly used types
pub use models::*;
pub use rusty_ytdl_client::RustyYtdlClient;
pub use rustube_client::RustubeClient;
pub use ytdlp_client::YtDlpClient;
pub use rust_yt_downloader_client::RustYtDownloaderClient;
pub use monitor::ChannelMonitor;
pub use ai_clipper::AiClipper;
pub use uploader::ClipUploader;
pub use performance_tracker::PerformanceTracker;
pub use thumbnail_generator::ThumbnailGenerator;
