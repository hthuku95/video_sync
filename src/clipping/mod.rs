// YouTube Clipping Module
// Handles monitoring external channels, downloading videos, AI clip extraction, and posting to YouTube

pub mod models;
pub mod ytdlp_client;
pub mod rustube_client; // Pure Rust YouTube downloader (no Python dependency)
pub mod monitor;
pub mod ai_clipper;
pub mod uploader;
pub mod performance_tracker;
pub mod thumbnail_generator;

// Re-export commonly used types
pub use models::*;
pub use ytdlp_client::YtDlpClient;
pub use rustube_client::RustubeClient; // Recommended for production
pub use monitor::ChannelMonitor;
pub use ai_clipper::AiClipper;
pub use uploader::ClipUploader;
pub use performance_tracker::PerformanceTracker;
pub use thumbnail_generator::ThumbnailGenerator;
