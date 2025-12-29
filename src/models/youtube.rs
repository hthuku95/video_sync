use serde::{Deserialize, Serialize};
use sqlx::FromRow;

#[derive(Debug, Serialize, Deserialize, FromRow, Clone)]
pub struct ConnectedYouTubeChannel {
    pub id: i32,
    pub user_id: i32,
    pub channel_id: String,
    pub channel_name: String,
    pub channel_description: Option<String>,
    pub channel_thumbnail_url: Option<String>,
    pub subscriber_count: Option<i64>,
    pub video_count: Option<i64>,
    pub access_token: String,
    pub refresh_token: String,
    pub token_expiry: chrono::DateTime<chrono::Utc>,
    pub granted_scopes: String,
    pub is_active: bool,
    pub last_sync_at: Option<chrono::DateTime<chrono::Utc>>,
    pub created_at: chrono::DateTime<chrono::Utc>,
    pub updated_at: chrono::DateTime<chrono::Utc>,
    pub requires_reauth: Option<bool>,  // NEW: OAuth scope migration flag
}

#[derive(Debug, Serialize, Deserialize)]
pub struct ConnectedChannelResponse {
    pub id: i32,
    pub channel_id: String,
    pub channel_name: String,
    pub channel_description: Option<String>,
    pub channel_thumbnail_url: Option<String>,
    pub subscriber_count: Option<i64>,
    pub video_count: Option<i64>,
    pub is_active: bool,
    pub connected_at: chrono::DateTime<chrono::Utc>,
}

impl From<ConnectedYouTubeChannel> for ConnectedChannelResponse {
    fn from(channel: ConnectedYouTubeChannel) -> Self {
        Self {
            id: channel.id,
            channel_id: channel.channel_id,
            channel_name: channel.channel_name,
            channel_description: channel.channel_description,
            channel_thumbnail_url: channel.channel_thumbnail_url,
            subscriber_count: channel.subscriber_count,
            video_count: channel.video_count,
            is_active: channel.is_active,
            connected_at: channel.created_at,
        }
    }
}

#[derive(Debug, Serialize, Deserialize, FromRow)]
pub struct YouTubeUpload {
    pub id: i32,
    pub user_id: i32,
    pub channel_id: i32,
    pub session_id: Option<i32>,
    pub local_video_path: String,
    pub local_file_id: Option<String>,
    pub youtube_video_id: Option<String>,
    pub video_title: String,
    pub video_description: Option<String>,
    pub video_category: Option<String>,
    pub privacy_status: Option<String>,
    pub upload_status: Option<String>,
    pub upload_progress: Option<i32>,
    pub error_message: Option<String>,
    pub youtube_url: Option<String>,
    pub published_at: Option<chrono::DateTime<chrono::Utc>>,
    pub created_at: chrono::DateTime<chrono::Utc>,
    pub updated_at: chrono::DateTime<chrono::Utc>,
    // Extended features columns
    pub deleted_at: Option<chrono::DateTime<chrono::Utc>>,
    pub metadata_updated_at: Option<chrono::DateTime<chrono::Utc>>,
    pub custom_thumbnail_path: Option<String>,
    pub scheduled_publish_at: Option<chrono::DateTime<chrono::Utc>>,
    pub is_scheduled: Option<bool>,
    pub upload_session_url: Option<String>,
    pub bytes_uploaded: Option<i64>,
    pub total_bytes: Option<i64>,
    pub is_resumable: Option<bool>,
}

#[derive(Debug, Serialize, Deserialize)]
pub struct UploadToYouTubeRequest {
    pub channel_id: i32,
    pub video_path: String,
    pub title: String,
    pub description: Option<String>,
    pub privacy_status: String, // "public", "private", "unlisted"
    pub category: Option<String>,
    pub tags: Option<Vec<String>>,
}

#[derive(Debug, Serialize, Deserialize)]
pub struct YouTubeUploadResponse {
    pub id: i32,
    pub youtube_video_id: String,
    pub youtube_url: String,
    pub title: String,
    pub privacy_status: String,
    pub published_at: String,
}

// ============================================================================
// Video Management Models
// ============================================================================

#[derive(Debug, Serialize, Deserialize)]
pub struct UpdateVideoRequest {
    pub title: Option<String>,
    pub description: Option<String>,
    pub privacy_status: Option<String>,
    pub category_id: Option<String>,
    pub tags: Option<Vec<String>>,
}

#[derive(Debug, Serialize, Deserialize)]
pub struct GenerateThumbnailRequest {
    pub timestamp: f64,  // Seconds into video
    pub width: Option<u32>,  // Defaults to 1280
    pub height: Option<u32>,  // Defaults to 720
}

// ============================================================================
// Playlist Models
// ============================================================================

#[derive(Debug, Serialize, Deserialize, FromRow, Clone)]
pub struct YouTubePlaylist {
    pub id: i32,
    pub user_id: i32,
    pub channel_id: i32,
    pub youtube_playlist_id: String,
    pub title: String,
    pub description: Option<String>,
    pub privacy_status: String,
    pub thumbnail_url: Option<String>,
    pub video_count: i32,
    pub created_at: chrono::DateTime<chrono::Utc>,
    pub updated_at: chrono::DateTime<chrono::Utc>,
}

#[derive(Debug, Serialize, Deserialize, FromRow)]
pub struct YouTubePlaylistItem {
    pub id: i32,
    pub playlist_id: i32,
    pub youtube_video_id: String,
    pub youtube_playlist_item_id: Option<String>,
    pub position: i32,
    pub video_title: Option<String>,
    pub video_thumbnail_url: Option<String>,
    pub added_at: chrono::DateTime<chrono::Utc>,
}

#[derive(Debug, Serialize, Deserialize)]
pub struct CreatePlaylistRequest {
    pub channel_id: i32,
    pub title: String,
    pub description: Option<String>,
    pub privacy_status: String,  // public, private, unlisted
}

#[derive(Debug, Serialize, Deserialize)]
pub struct UpdatePlaylistRequest {
    pub title: Option<String>,
    pub description: Option<String>,
    pub privacy_status: Option<String>,
}

#[derive(Debug, Serialize, Deserialize)]
pub struct AddVideoToPlaylistRequest {
    pub video_id: String,
    pub position: Option<i32>,
}

#[derive(Debug, Serialize, Deserialize)]
pub struct ReorderPlaylistRequest {
    pub video_positions: Vec<VideoPosition>,
}

#[derive(Debug, Serialize, Deserialize)]
pub struct VideoPosition {
    pub video_id: String,
    pub position: i32,
}

// ============================================================================
// Analytics Models
// ============================================================================

#[derive(Debug, Serialize, Deserialize, FromRow)]
pub struct YouTubeVideoAnalytics {
    pub id: i32,
    pub youtube_video_id: String,
    pub metric_date: chrono::NaiveDate,
    pub views: i64,
    pub watch_time_minutes: i64,
    pub average_view_duration: Option<i32>,
    pub average_view_percentage: Option<f64>,
    pub likes: i32,
    pub dislikes: i32,
    pub comments: i32,
    pub shares: i32,
    pub subscribers_gained: i32,
    pub subscribers_lost: i32,
    pub estimated_revenue: Option<f64>,
    pub fetched_at: chrono::DateTime<chrono::Utc>,
}

#[derive(Debug, Serialize, Deserialize, FromRow)]
pub struct YouTubeChannelAnalytics {
    pub id: i32,
    pub channel_id: i32,
    pub metric_date: chrono::NaiveDate,
    pub views: i64,
    pub watch_time_minutes: i64,
    pub subscribers_gained: i32,
    pub subscribers_lost: i32,
    pub estimated_revenue: Option<f64>,
    pub demographics: Option<serde_json::Value>,
    pub traffic_sources: Option<serde_json::Value>,
    pub device_types: Option<serde_json::Value>,
    pub fetched_at: chrono::DateTime<chrono::Utc>,
}

#[derive(Debug, Serialize, Deserialize)]
pub struct AnalyticsRequest {
    pub start_date: String,  // YYYY-MM-DD
    pub end_date: String,    // YYYY-MM-DD
}

#[derive(Debug, Serialize, Deserialize)]
pub struct VideoAnalyticsResponse {
    pub video_id: String,
    pub date_range: DateRange,
    pub metrics: VideoMetrics,
}

#[derive(Debug, Serialize, Deserialize)]
pub struct ChannelAnalyticsResponse {
    pub channel_id: i32,
    pub date_range: DateRange,
    pub metrics: ChannelMetrics,
    pub demographics: Option<serde_json::Value>,
    pub traffic_sources: Option<serde_json::Value>,
    pub device_types: Option<serde_json::Value>,
}

#[derive(Debug, Serialize, Deserialize)]
pub struct DateRange {
    pub start: String,
    pub end: String,
}

#[derive(Debug, Serialize, Deserialize)]
pub struct VideoMetrics {
    pub views: i64,
    pub watch_time_minutes: i64,
    pub average_view_duration: i32,
    pub average_view_percentage: f64,
    pub likes: i32,
    pub comments: i32,
    pub shares: i32,
    pub subscribers_gained: i32,
    pub estimated_revenue: Option<f64>,
}

#[derive(Debug, Serialize, Deserialize)]
pub struct ChannelMetrics {
    pub views: i64,
    pub watch_time_minutes: i64,
    pub subscribers_gained: i32,
    pub subscribers_lost: i32,
    pub estimated_revenue: Option<f64>,
}

// ============================================================================
// Search & Discovery Models
// ============================================================================

#[derive(Debug, Serialize, Deserialize)]
pub struct VideoSearchResult {
    pub video_id: String,
    pub title: String,
    pub description: String,
    pub channel_id: String,
    pub channel_title: String,
    pub thumbnail_url: String,
    pub published_at: String,
    pub view_count: Option<i64>,
    pub like_count: Option<i32>,
    pub comment_count: Option<i32>,
}

#[derive(Debug, Serialize, Deserialize)]
pub struct SearchRequest {
    pub query: String,
    pub max_results: Option<i32>,  // Max 50
    pub order: Option<String>,     // date, rating, relevance, title, viewCount
    pub published_after: Option<String>,  // ISO 8601
    pub published_before: Option<String>,
}

#[derive(Debug, Serialize, Deserialize)]
pub struct TrendingRequest {
    pub region_code: Option<String>,  // US, GB, etc.
    pub category_id: Option<String>,
    pub max_results: Option<i32>,
}

// ============================================================================
// Comment Models
// ============================================================================

#[derive(Debug, Serialize, Deserialize, FromRow)]
pub struct YouTubeComment {
    pub id: i32,
    pub youtube_comment_id: String,
    pub youtube_video_id: String,
    pub parent_comment_id: Option<String>,
    pub author_name: String,
    pub author_channel_id: String,
    pub author_profile_image_url: Option<String>,
    pub text_display: String,
    pub text_original: String,
    pub like_count: i32,
    pub can_reply: bool,
    pub moderation_status: Option<String>,
    pub published_at: chrono::DateTime<chrono::Utc>,
    pub updated_at: Option<chrono::DateTime<chrono::Utc>>,
    pub fetched_at: chrono::DateTime<chrono::Utc>,
}

#[derive(Debug, Serialize, Deserialize)]
pub struct ReplyToCommentRequest {
    pub text: String,
}

// ============================================================================
// Caption Models
// ============================================================================

#[derive(Debug, Serialize, Deserialize, FromRow)]
pub struct YouTubeCaption {
    pub id: i32,
    pub youtube_video_id: String,
    pub youtube_caption_id: String,
    pub language: String,
    pub name: Option<String>,
    pub track_kind: Option<String>,
    pub is_auto_generated: bool,
    pub is_cc: bool,
    pub is_draft: bool,
    pub local_file_path: Option<String>,
    pub created_at: chrono::DateTime<chrono::Utc>,
}

#[derive(Debug, Serialize, Deserialize)]
pub struct UploadCaptionRequest {
    pub language: String,  // ISO 639-1 code
    pub name: Option<String>,
    pub caption_file: String,  // Path to SRT/VTT file
}

// ============================================================================
// Scheduling Models
// ============================================================================

#[derive(Debug, Serialize, Deserialize)]
pub struct ScheduleVideoRequest {
    pub publish_at: String,  // ISO 8601 timestamp
}

// ============================================================================
// Resumable Upload Models
// ============================================================================

#[derive(Debug, Serialize, Deserialize)]
pub struct InitiateResumableUploadRequest {
    pub channel_id: i32,
    pub video_path: String,
    pub file_size: i64,
    pub title: String,
    pub description: Option<String>,
    pub privacy_status: String,
    pub category: Option<String>,
    pub tags: Option<Vec<String>>,
}

#[derive(Debug, Serialize, Deserialize)]
pub struct ResumableUploadSession {
    pub upload_id: i32,
    pub session_url: String,
    pub total_bytes: i64,
    pub bytes_uploaded: i64,
}

#[derive(Debug, Serialize, Deserialize)]
pub struct UploadChunkRequest {
    pub session_id: String,
    pub chunk_data: Vec<u8>,
    pub start_byte: i64,
    pub end_byte: i64,
}
