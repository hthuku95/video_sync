#![allow(dead_code, unused_imports)]
// Database models for YouTube Clipping feature

use chrono::{DateTime, Utc};
use serde::{Deserialize, Serialize};
use sqlx::FromRow;
use uuid::Uuid;

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
    pub last_clipping_session_at: Option<DateTime<Utc>>,
    pub clipping_cooldown_hours: i32,
    /// When true, generated clips are queued for human review instead of being auto-published.
    #[serde(default)]
    pub requires_human_approval: bool,
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
    pub workflow_id: Option<Uuid>,
    pub started_at: Option<DateTime<Utc>>,
    pub completed_at: Option<DateTime<Utc>>,
    pub retry_count: i32,
    pub last_retry_at: Option<DateTime<Utc>>,
    pub stuck_detection_count: i32,
    pub created_at: DateTime<Utc>,
    pub updated_at: DateTime<Utc>,
    /// Full VideoAnalysis from Phase A stored as JSONB; allows retries to skip Gemini re-analysis.
    pub viral_moments_json: Option<serde_json::Value>,
    /// Overall video quality score from Phase A (0.0–1.0).
    pub analysis_quality: Option<f64>,
    /// Phase resume hint set by auto_retry_failed_jobs; cleared at the start of execution.
    /// Values: "analyzed" (skip to Phase B), "downloaded" (skip to Phase C),
    /// "clips_extracted" (skip to Phase E), or NULL (start from Phase A).
    pub resume_from: Option<String>,
    /// True when this job used a Twitch VOD instead of the YouTube source video.
    pub used_twitch_fallback: bool,
    /// Twitch video ID used as fallback source (if any).
    pub twitch_video_id: Option<String>,
    /// Overrides source_video_id URL when Twitch fallback is active.
    pub active_video_url: Option<String>,
    /// True when this job used a Kick.com broadcast instead of the YouTube source video.
    pub used_kick_fallback: bool,
    /// Kick channel slug resolved from three-way mapping.
    pub kick_channel_slug: Option<String>,
    /// URL of the Kick broadcast to download.
    pub kick_video_url: Option<String>,
    /// Generated longform fallback delivery row created when download fails.
    pub fallback_delivery_id: Option<Uuid>,
    /// Fallback mode used for this job, e.g. "generated_summary_delivery".
    pub fallback_strategy: Option<String>,
    /// When the fallback path was activated.
    pub fallback_activated_at: Option<DateTime<Utc>>,
    /// High-level supervisor health label for queue triage/remediation.
    #[serde(default)]
    pub supervisor_status: String,
    /// Human-readable reason for the current supervisor state.
    pub supervisor_reason: Option<String>,
    /// Last action the clipping supervisor took for this job.
    pub supervisor_last_action: Option<String>,
    /// Last time the clipping supervisor evaluated this job.
    pub supervisor_last_run_at: Option<DateTime<Utc>>,
    /// Canonical active job that this duplicate or blocked job is waiting behind.
    pub blocked_by_job_id: Option<i32>,
}

// ─────────────────────────── Twitch models ────────────────────────────────────

/// A Twitch broadcaster account added as a potential fallback source.
#[derive(Debug, Clone, Serialize, Deserialize, FromRow)]
pub struct TwitchSourceChannel {
    pub id: i32,
    pub broadcaster_id: String,
    pub broadcaster_login: String,
    pub display_name: String,
    pub profile_image_url: Option<String>,
    pub is_active: bool,
    pub last_polled_at: Option<DateTime<Utc>>,
    pub last_video_checked: Option<String>,
    pub created_at: DateTime<Utc>,
    pub updated_at: DateTime<Utc>,
}

/// 1:1 mapping between a YouTube source channel and its Twitch equivalent.
#[derive(Debug, Clone, Serialize, Deserialize, FromRow)]
pub struct YoutubeTwitchMapping {
    pub id: i32,
    pub youtube_source_channel_id: i32,
    pub twitch_source_channel_id: i32,
    pub created_at: DateTime<Utc>,
}

// ─────────────────────────── Twitch request DTOs ──────────────────────────────

#[derive(Debug, Deserialize)]
pub struct SearchTwitchChannelsRequest {
    pub query: String,
}

#[derive(Debug, Deserialize)]
pub struct AddTwitchSourceChannelRequest {
    /// Twitch broadcaster_id (numeric string) returned by the search endpoint.
    pub broadcaster_id: String,
}

#[derive(Debug, Deserialize)]
pub struct CreateTwitchMappingRequest {
    pub youtube_source_channel_id: i32,
    pub twitch_source_channel_id: i32,
}

/// Extracted clip from long-form video
#[derive(Debug, Clone, Serialize, Deserialize, FromRow)]
pub struct ExtractedClip {
    pub id: i32,
    pub clipping_job_id: i32,
    pub clip_number: i32,
    pub local_clip_path: String,
    pub r2_clip_key: Option<String>,
    pub r2_thumb_key: Option<String>,
    pub r2_clip_url: Option<String>,
    pub r2_clip_url_expires_at: Option<DateTime<Utc>>,
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
    #[serde(default)]
    pub qa_status: String,
    pub qa_score: Option<i32>,
    pub qa_feedback: Option<String>,
    pub qa_retry_hint: Option<String>,
    pub views_24h: i32,
    pub likes_24h: i32,
    pub comments_24h: i32,
    pub created_at: DateTime<Utc>,
    pub updated_at: DateTime<Utc>,
    // Review system fields (added by migration 20260313000001)
    #[serde(default)]
    pub review_status: String,
    pub proposed_title: Option<String>,
    pub proposed_description: Option<String>,
    pub reviewed_by: Option<i32>,
    pub reviewed_at: Option<DateTime<Utc>>,
    pub review_notes: Option<String>,
    // Phase C+ enhancement tracking (added by migration 20260318000000)
    #[serde(default)]
    pub enhancement_applied: bool,
    #[serde(default)]
    pub enhancement_tools: Vec<String>,
    pub enhancement_reasoning: Option<String>,
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
    /// YouTube channel URL (e.g., https://www.youtube.com/@handle) - preferred for content_machine
    pub channel_url: Option<String>,
    /// YouTube channel ID or handle (e.g., @handle or UCxxx) - for backward compatibility with embedded UI
    pub channel_id: Option<String>,
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
    pub retry_count: i32,
    pub last_retry_at: Option<DateTime<Utc>>,
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

/// Clipped source video tracking record
#[derive(Debug, Clone, Serialize, Deserialize, FromRow)]
pub struct ClippedSourceVideo {
    pub id: i32,
    pub source_channel_id: i32,
    pub video_id: String,
    pub video_title: Option<String>,
    pub video_published_at: Option<DateTime<Utc>>,
    pub first_clipped_at: DateTime<Utc>,
    pub created_at: DateTime<Utc>,
}

/// Pending unclipped video (session memory)
#[derive(Debug, Clone, Serialize, Deserialize, FromRow)]
pub struct PendingUnclippedVideo {
    pub id: i32,
    pub linkage_id: i32,
    pub video_id: String,
    pub video_title: Option<String>,
    pub video_published_at: Option<DateTime<Utc>>,
    pub discovered_at: DateTime<Utc>,
    pub created_at: DateTime<Utc>,
}

/// A Kick.com broadcaster account.
#[derive(Debug, Clone, Serialize, Deserialize, FromRow)]
pub struct KickSourceChannel {
    pub id: i32,
    pub slug: String,
    pub display_name: String,
    pub profile_picture: Option<String>,
    pub is_active: bool,
    pub broadcaster_user_id: Option<i64>,
    pub created_at: DateTime<Utc>,
    pub updated_at: DateTime<Utc>,
}

/// Mapping from a YouTube source channel to its Kick.com equivalent.
#[derive(Debug, Clone, Serialize, Deserialize, FromRow)]
pub struct YoutubeKickMapping {
    pub id: i32,
    pub youtube_source_channel_id: i32,
    pub kick_source_channel_id: i32,
    pub created_at: DateTime<Utc>,
}

/// Mapping from a Twitch source channel to its Kick.com equivalent.
#[derive(Debug, Clone, Serialize, Deserialize, FromRow)]
pub struct TwitchKickMapping {
    pub id: i32,
    pub twitch_source_channel_id: i32,
    pub kick_source_channel_id: i32,
    pub created_at: DateTime<Utc>,
}
