// YouTube Clipping Module
// Handles monitoring external channels, downloading videos, AI clip extraction, and posting to YouTube

pub mod models;
pub mod ytdlp_client;
pub mod monitor;
pub mod ai_clipper;
pub mod uploader;

// Re-export commonly used types
pub use models::*;
pub use ytdlp_client::YtDlpClient;
pub use monitor::ChannelMonitor;
pub use ai_clipper::AiClipper;
pub use uploader::ClipUploader;
