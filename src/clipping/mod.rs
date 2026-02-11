// YouTube Clipping Module
// Handles monitoring external channels, downloading videos, AI clip extraction, and posting to YouTube

pub mod models;
pub mod apify_client;          // Primary downloader (Apify + rusty_ytdl fallback)
pub mod rusty_ytdl_client;     // Pure Rust YouTube downloader (no Python dependency)
pub mod monitor;
pub mod ai_clipper;
pub mod uploader;
pub mod performance_tracker;
pub mod thumbnail_generator;

// Re-export commonly used types
pub use models::*;
pub use rusty_ytdl_client::RustyYtdlClient; // Production-ready pure Rust downloader
pub use monitor::ChannelMonitor;
pub use ai_clipper::AiClipper;
pub use uploader::ClipUploader;
pub use performance_tracker::PerformanceTracker;
pub use thumbnail_generator::ThumbnailGenerator;
