// Database models for YouTube Clipping feature

use chrono::{DateTime, Utc};
use serde::{Deserialize, Serialize};
use sqlx::FromRow;

/// Source channel to monitor (e.g., Mr Beast)
#[derive(Debug, Clone, Serialize, Deserialize, FromRow)]
pub struct SourceChannel {
    pub id: i32,
    pub channel_id: String,
    pub channel_name: String,
    pub channel_thumbnail_url: Option<String>,
    pub subscriber_count: Option<i64>,
    pub is_active: bool,
    pub polling_interval_minutes: i32,
    pub last_polled_at: Option<DateTime<Utc>>,
    pub last_video_checked: Option<String>,
    pub created_at: DateTime<Utc>,
    pub updated_at: DateTime<Utc>,
}

/// Linkage between source channel and destination channel
#[derive(Debug, Clone, Serialize, Deserialize, FromRow)]
pub struct ChannelLinkage {
    pub id: i32,
    pub user_id: i32,
    pub source_channel_id: i32,
    pub destination_channel_id: i32,
    pub is_active: bool,
    pub clips_per_video: i32,
    pub min_clip_duration_seconds: i32,
    pub max_clip_duration_seconds: i32,
    pub total_clips_generated: i32,
    pub total_clips_posted: i32,
    pub last_clip_generated_at: Option<DateTime<Utc>>,
    pub created_at: DateTime<Utc>,
    pub updated_at: DateTime<Utc>,
}

/// Clipping job tracking
#[derive(Debug, Clone, Serialize, Deserialize, FromRow)]
pub struct ClippingJob {
    pub id: i32,
    pub linkage_id: i32,
    pub source_video_id: String,
    pub source_video_title: Option<String>,
    pub source_video_duration_seconds: Option<i32>,
    pub local_video_path: Option<String>,
    pub status: String,
    pub current_step: Option<String>,
    pub progress_percent: i32,
    pub error_message: Option<String>,
    pub started_at: Option<DateTime<Utc>>,
    pub completed_at: Option<DateTime<Utc>>,
    pub created_at: DateTime<Utc>,
    pub updated_at: DateTime<Utc>,
}

/// Extracted clip from long-form video
#[derive(Debug, Clone, Serialize, Deserialize, FromRow)]
pub struct ExtractedClip {
    pub id: i32,
    pub clipping_job_id: i32,
    pub clip_number: i32,
    pub local_clip_path: String,
    pub start_time_seconds: f64,
    pub end_time_seconds: f64,
    pub duration_seconds: f64,
    pub ai_title: Option<String>,
    pub ai_description: Option<String>,
    pub ai_tags: Option<Vec<String>>,
    pub ai_confidence_score: Option<f64>,
    pub viral_factors: Option<Vec<String>>,
    pub youtube_video_id: Option<String>,
    pub youtube_url: Option<String>,
    pub upload_status: String,
    pub published_at: Option<DateTime<Utc>>,
    pub upload_error: Option<String>,
    pub views_24h: i32,
    pub likes_24h: i32,
    pub comments_24h: i32,
    pub created_at: DateTime<Utc>,
    pub updated_at: DateTime<Utc>,
}

/// Polling schedule for source channels
#[derive(Debug, Clone, Serialize, Deserialize, FromRow)]
pub struct PollSchedule {
    pub id: i32,
    pub source_channel_id: i32,
    pub next_poll_at: DateTime<Utc>,
    pub is_polling: bool,
    pub last_poll_duration_ms: Option<i32>,
    pub consecutive_failures: i32,
    pub created_at: DateTime<Utc>,
    pub updated_at: DateTime<Utc>,
}

// Request/Response DTOs

#[derive(Debug, Deserialize)]
pub struct AddSourceChannelRequest {
    pub channel_id: String,
    pub polling_interval_minutes: Option<i32>,
}

#[derive(Debug, Deserialize)]
pub struct CreateLinkageRequest {
    pub source_channel_id: i32,
    pub destination_channel_id: i32,
    pub clips_per_video: Option<i32>,
    pub min_clip_duration_seconds: Option<i32>,
    pub max_clip_duration_seconds: Option<i32>,
}

#[derive(Debug, Deserialize)]
pub struct UpdateLinkageRequest {
    pub is_active: Option<bool>,
    pub clips_per_video: Option<i32>,
    pub min_clip_duration_seconds: Option<i32>,
    pub max_clip_duration_seconds: Option<i32>,
}

#[derive(Debug, Serialize)]
pub struct ClippingJobResponse {
    pub id: i32,
    pub linkage_id: i32,
    pub source_video_id: String,
    pub source_video_title: Option<String>,
    pub status: String,
    pub current_step: Option<String>,
    pub progress_percent: i32,
    pub error_message: Option<String>,
    pub created_at: DateTime<Utc>,
    pub updated_at: DateTime<Utc>,
}

#[derive(Debug, Serialize)]
pub struct ExtractedClipResponse {
    pub id: i32,
    pub clip_number: i32,
    pub ai_title: Option<String>,
    pub ai_description: Option<String>,
    pub duration_seconds: f64,
    pub youtube_video_id: Option<String>,
    pub youtube_url: Option<String>,
    pub upload_status: String,
    pub published_at: Option<DateTime<Utc>>,
    pub views_24h: i32,
    pub likes_24h: i32,
    pub comments_24h: i32,
}

/// Configuration for AI clipping
#[derive(Debug, Clone)]
pub struct ClippingConfig {
    pub clips_per_video: i32,
    pub min_clip_duration_seconds: i32,
    pub max_clip_duration_seconds: i32,
}

/// AI-identified clip candidate
#[derive(Debug, Clone)]
pub struct ClipCandidate {
    pub start_time: f64,
    pub end_time: f64,
    pub title: String,
    pub description: String,
    pub tags: Vec<String>,
    pub confidence: f64,
    pub viral_factors: Vec<String>,
    pub criteria: String,
}

/// Review result for extracted clip
#[derive(Debug)]
pub struct ReviewResult {
    pub passed: bool,
    pub feedback: String,
}
