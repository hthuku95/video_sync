// YouTube Data API v3 client for video uploads and channel management
// Docs: https://developers.google.com/youtube/v3

use reqwest::Client;
use serde::{Deserialize, Serialize};
use serde_json::json;
use std::path::Path;

#[derive(Debug, Clone)]
pub struct YouTubeClient {
    client: Client,
    api_key: String,
}

// ============================================================================
// OAuth and Channel Structures
// ============================================================================

#[derive(Debug, Serialize, Deserialize, Clone)]
pub struct YouTubeChannel {
    pub id: String,
    pub title: String,
    pub description: String,
    pub thumbnail_url: Option<String>,
    pub subscriber_count: Option<i64>,
    pub video_count: Option<i64>,
}

#[derive(Debug, Deserialize)]
pub struct ChannelListResponse {
    pub items: Vec<ChannelItem>,
}

#[derive(Debug, Deserialize)]
pub struct ChannelItem {
    pub id: String,
    pub snippet: ChannelSnippet,
    pub statistics: Option<ChannelStatistics>,
}

#[derive(Debug, Deserialize)]
pub struct ChannelSnippet {
    pub title: String,
    pub description: String,
    pub thumbnails: Option<Thumbnails>,
}

#[derive(Debug, Deserialize)]
pub struct Thumbnails {
    pub default: Option<ThumbnailInfo>,
    pub medium: Option<ThumbnailInfo>,
    pub high: Option<ThumbnailInfo>,
}

#[derive(Debug, Deserialize)]
pub struct ThumbnailInfo {
    pub url: String,
}

#[derive(Debug, Deserialize)]
pub struct ChannelStatistics {
    #[serde(rename = "subscriberCount")]
    pub subscriber_count: Option<String>,
    #[serde(rename = "videoCount")]
    pub video_count: Option<String>,
}

// ============================================================================
// Video Upload Structures
// ============================================================================

#[derive(Debug, Serialize)]
pub struct VideoSnippet {
    pub title: String,
    pub description: String,
    #[serde(rename = "categoryId")]
    pub category_id: String,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub tags: Option<Vec<String>>,
}

#[derive(Debug, Serialize)]
pub struct VideoStatus {
    #[serde(rename = "privacyStatus")]
    pub privacy_status: String, // "public", "private", "unlisted"
}

#[derive(Debug, Serialize)]
pub struct VideoResource {
    pub snippet: VideoSnippet,
    pub status: VideoStatus,
}

#[derive(Debug, Deserialize)]
pub struct VideoUploadResponse {
    pub id: String,
    pub snippet: VideoResponseSnippet,
}

#[derive(Debug, Deserialize)]
pub struct VideoResponseSnippet {
    pub title: String,
    #[serde(rename = "publishedAt")]
    pub published_at: String,
}

// ============================================================================
// YouTube Client Implementation
// ============================================================================

impl YouTubeClient {
    pub fn new(api_key: String) -> Self {
        Self {
            client: Client::new(),
            api_key,
        }
    }

    /// List user's YouTube channels using OAuth access token
    pub async fn list_channels(&self, access_token: &str) -> Result<Vec<YouTubeChannel>, Box<dyn std::error::Error + Send + Sync>> {
        let url = "https://www.googleapis.com/youtube/v3/channels";

        let response = self.client
            .get(url)
            .query(&[
                ("part", "snippet,statistics"),
                ("mine", "true"),
            ])
            .header("Authorization", format!("Bearer {}", access_token))
            .send()
            .await?;

        if !response.status().is_success() {
            let error_text = response.text().await?;
            return Err(format!("Failed to list YouTube channels: {}", error_text).into());
        }

        let channel_response: ChannelListResponse = response.json().await?;

        Ok(channel_response.items.into_iter().map(|item| {
            let thumbnail_url = item.snippet.thumbnails
                .and_then(|t| t.high.or(t.medium).or(t.default))
                .map(|t| t.url);

            let subscriber_count = item.statistics
                .as_ref()
                .and_then(|s| s.subscriber_count.as_ref())
                .and_then(|c| c.parse().ok());

            let video_count = item.statistics
                .as_ref()
                .and_then(|s| s.video_count.as_ref())
                .and_then(|c| c.parse().ok());

            YouTubeChannel {
                id: item.id,
                title: item.snippet.title,
                description: item.snippet.description,
                thumbnail_url,
                subscriber_count,
                video_count,
            }
        }).collect())
    }

    /// Upload video to YouTube
    pub async fn upload_video(
        &self,
        access_token: &str,
        video_path: &str,
        title: &str,
        description: &str,
        privacy_status: &str,
        category_id: Option<&str>,
        tags: Option<Vec<String>>,
    ) -> Result<VideoUploadResponse, Box<dyn std::error::Error + Send + Sync>> {
        // Validate privacy status
        if !["public", "private", "unlisted"].contains(&privacy_status) {
            return Err("Invalid privacy status. Must be 'public', 'private', or 'unlisted'".into());
        }

        // Read video file
        let video_data = tokio::fs::read(video_path).await?;
        let file_name = Path::new(video_path)
            .file_name()
            .and_then(|n| n.to_str())
            .unwrap_or("video.mp4");

        // Create metadata
        let metadata = VideoResource {
            snippet: VideoSnippet {
                title: title.to_string(),
                description: description.to_string(),
                category_id: category_id.unwrap_or("22").to_string(), // Default: People & Blogs
                tags,
            },
            status: VideoStatus {
                privacy_status: privacy_status.to_string(),
            },
        };

        // Create multipart form
        let metadata_json = serde_json::to_string(&metadata)?;

        let form = reqwest::multipart::Form::new()
            .part(
                "snippet",
                reqwest::multipart::Part::text(metadata_json.clone())
                    .mime_str("application/json")?
            )
            .part(
                "media",
                reqwest::multipart::Part::bytes(video_data)
                    .file_name(file_name.to_string())
                    .mime_str("video/*")?
            );

        // Upload using resumable upload endpoint
        let upload_url = "https://www.googleapis.com/upload/youtube/v3/videos";

        let response = self.client
            .post(upload_url)
            .query(&[
                ("part", "snippet,status"),
                ("uploadType", "multipart"),
            ])
            .header("Authorization", format!("Bearer {}", access_token))
            .multipart(form)
            .send()
            .await?;

        if !response.status().is_success() {
            let error_text = response.text().await?;
            tracing::error!("YouTube upload failed: {}", error_text);
            return Err(format!("Failed to upload video: {}", error_text).into());
        }

        let upload_response: VideoUploadResponse = response.json().await?;

        tracing::info!("✅ Video uploaded to YouTube: {} (ID: {})", upload_response.snippet.title, upload_response.id);

        Ok(upload_response)
    }

    /// Refresh an expired access token using refresh token
    pub async fn refresh_access_token(
        &self,
        refresh_token: &str,
        client_id: &str,
        client_secret: &str,
    ) -> Result<TokenRefreshResponse, Box<dyn std::error::Error + Send + Sync>> {
        let url = "https://oauth2.googleapis.com/token";

        let params = json!({
            "client_id": client_id,
            "client_secret": client_secret,
            "refresh_token": refresh_token,
            "grant_type": "refresh_token"
        });

        let response = self.client
            .post(url)
            .json(&params)
            .send()
            .await?;

        if !response.status().is_success() {
            let error_text = response.text().await?;
            return Err(format!("Failed to refresh token: {}", error_text).into());
        }

        let token_response: TokenRefreshResponse = response.json().await?;
        Ok(token_response)
    }

    // ========================================================================
    // Video Management Methods
    // ========================================================================

    /// Delete a video from YouTube
    ///
    /// Required scope: https://www.googleapis.com/auth/youtube.force-ssl
    pub async fn delete_video(
        &self,
        access_token: &str,
        video_id: &str,
    ) -> Result<(), Box<dyn std::error::Error + Send + Sync>> {
        let url = "https://www.googleapis.com/youtube/v3/videos";

        tracing::info!("📹 Deleting YouTube video: {}", video_id);

        let response = self
            .client
            .delete(url)
            .query(&[("id", video_id)])
            .header("Authorization", format!("Bearer {}", access_token))
            .send()
            .await?;

        if !response.status().is_success() {
            let error_text = response.text().await?;
            tracing::error!("❌ Failed to delete video {}: {}", video_id, error_text);
            return Err(format!("Failed to delete video: {}", error_text).into());
        }

        tracing::info!("✅ Video deleted from YouTube: {}", video_id);
        Ok(())
    }

    /// Update video metadata (title, description, privacy, tags)
    ///
    /// Required scope: https://www.googleapis.com/auth/youtube.force-ssl
    pub async fn update_video(
        &self,
        access_token: &str,
        video_id: &str,
        title: Option<&str>,
        description: Option<&str>,
        privacy_status: Option<&str>,
        category_id: Option<&str>,
        tags: Option<Vec<String>>,
    ) -> Result<VideoUpdateResponse, Box<dyn std::error::Error + Send + Sync>> {
        let url = "https://www.googleapis.com/youtube/v3/videos";

        tracing::info!("📝 Updating YouTube video metadata: {}", video_id);

        // Build the update payload
        let mut snippet = json!({});
        if let Some(t) = title {
            snippet["title"] = json!(t);
        }
        if let Some(d) = description {
            snippet["description"] = json!(d);
        }
        if let Some(cat) = category_id {
            snippet["categoryId"] = json!(cat);
        }
        if let Some(t) = tags {
            snippet["tags"] = json!(t);
        }

        let mut status = json!({});
        if let Some(p) = privacy_status {
            status["privacyStatus"] = json!(p);
        }

        let body = json!({
            "id": video_id,
            "snippet": snippet,
            "status": status,
        });

        let response = self
            .client
            .put(url)
            .query(&[("part", "snippet,status")])
            .header("Authorization", format!("Bearer {}", access_token))
            .header("Content-Type", "application/json")
            .json(&body)
            .send()
            .await?;

        if !response.status().is_success() {
            let error_text = response.text().await?;
            tracing::error!("❌ Failed to update video {}: {}", video_id, error_text);
            return Err(format!("Failed to update video: {}", error_text).into());
        }

        let update_response: VideoUpdateResponse = response.json().await?;
        tracing::info!("✅ Video metadata updated: {}", video_id);

        Ok(update_response)
    }

    /// Upload a custom thumbnail for a video
    ///
    /// Required scope: https://www.googleapis.com/auth/youtube.force-ssl
    ///
    /// # Arguments
    /// * `access_token` - OAuth access token
    /// * `video_id` - YouTube video ID
    /// * `image_data` - Thumbnail image bytes (JPEG, PNG, etc.)
    /// * `content_type` - MIME type (e.g., "image/jpeg")
    pub async fn upload_thumbnail(
        &self,
        access_token: &str,
        video_id: &str,
        image_data: Vec<u8>,
        content_type: &str,
    ) -> Result<ThumbnailUploadResponse, Box<dyn std::error::Error + Send + Sync>> {
        let url = format!("https://www.googleapis.com/upload/youtube/v3/thumbnails/set?videoId={}", video_id);

        tracing::info!("🖼️ Uploading custom thumbnail for video: {}", video_id);

        let response = self
            .client
            .post(&url)
            .header("Authorization", format!("Bearer {}", access_token))
            .header("Content-Type", content_type)
            .body(image_data)
            .send()
            .await?;

        if !response.status().is_success() {
            let error_text = response.text().await?;
            tracing::error!("❌ Failed to upload thumbnail for {}: {}", video_id, error_text);
            return Err(format!("Failed to upload thumbnail: {}", error_text).into());
        }

        let thumb_response: ThumbnailUploadResponse = response.json().await?;
        tracing::info!("✅ Thumbnail uploaded for video: {}", video_id);

        Ok(thumb_response)
    }

    // ========================================================================
    // Playlist Management Methods
    // ========================================================================

    /// Create a new playlist
    ///
    /// Required scope: https://www.googleapis.com/auth/youtube.force-ssl
    pub async fn create_playlist(
        &self,
        access_token: &str,
        title: &str,
        description: Option<&str>,
        privacy_status: &str,
    ) -> Result<PlaylistResponse, Box<dyn std::error::Error + Send + Sync>> {
        let url = "https://www.googleapis.com/youtube/v3/playlists";

        tracing::info!("📋 Creating YouTube playlist: {}", title);

        let body = json!({
            "snippet": {
                "title": title,
                "description": description.unwrap_or(""),
            },
            "status": {
                "privacyStatus": privacy_status,
            }
        });

        let response = self
            .client
            .post(url)
            .query(&[("part", "snippet,status")])
            .header("Authorization", format!("Bearer {}", access_token))
            .header("Content-Type", "application/json")
            .json(&body)
            .send()
            .await?;

        if !response.status().is_success() {
            let error_text = response.text().await?;
            tracing::error!("❌ Failed to create playlist: {}", error_text);
            return Err(format!("Failed to create playlist: {}", error_text).into());
        }

        let playlist_response: PlaylistResponse = response.json().await?;
        tracing::info!("✅ Playlist created: {} (ID: {})", title, playlist_response.id);

        Ok(playlist_response)
    }

    /// List user's playlists
    pub async fn list_playlists(
        &self,
        access_token: &str,
    ) -> Result<PlaylistListResponse, Box<dyn std::error::Error + Send + Sync>> {
        let url = "https://www.googleapis.com/youtube/v3/playlists";

        let response = self
            .client
            .get(url)
            .query(&[
                ("part", "snippet,contentDetails,status"),
                ("mine", "true"),
                ("maxResults", "50"),
            ])
            .header("Authorization", format!("Bearer {}", access_token))
            .send()
            .await?;

        if !response.status().is_success() {
            let error_text = response.text().await?;
            return Err(format!("Failed to list playlists: {}", error_text).into());
        }

        let playlists: PlaylistListResponse = response.json().await?;
        Ok(playlists)
    }

    /// Update playlist metadata
    pub async fn update_playlist(
        &self,
        access_token: &str,
        playlist_id: &str,
        title: Option<&str>,
        description: Option<&str>,
        privacy_status: Option<&str>,
    ) -> Result<PlaylistResponse, Box<dyn std::error::Error + Send + Sync>> {
        let url = "https://www.googleapis.com/youtube/v3/playlists";

        let mut snippet = json!({});
        if let Some(t) = title {
            snippet["title"] = json!(t);
        }
        if let Some(d) = description {
            snippet["description"] = json!(d);
        }

        let mut status = json!({});
        if let Some(p) = privacy_status {
            status["privacyStatus"] = json!(p);
        }

        let body = json!({
            "id": playlist_id,
            "snippet": snippet,
            "status": status,
        });

        let response = self
            .client
            .put(url)
            .query(&[("part", "snippet,status")])
            .header("Authorization", format!("Bearer {}", access_token))
            .header("Content-Type", "application/json")
            .json(&body)
            .send()
            .await?;

        if !response.status().is_success() {
            let error_text = response.text().await?;
            return Err(format!("Failed to update playlist: {}", error_text).into());
        }

        let playlist_response: PlaylistResponse = response.json().await?;
        Ok(playlist_response)
    }

    /// Delete a playlist
    pub async fn delete_playlist(
        &self,
        access_token: &str,
        playlist_id: &str,
    ) -> Result<(), Box<dyn std::error::Error + Send + Sync>> {
        let url = "https://www.googleapis.com/youtube/v3/playlists";

        tracing::info!("🗑️ Deleting YouTube playlist: {}", playlist_id);

        let response = self
            .client
            .delete(url)
            .query(&[("id", playlist_id)])
            .header("Authorization", format!("Bearer {}", access_token))
            .send()
            .await?;

        if !response.status().is_success() {
            let error_text = response.text().await?;
            tracing::error!("❌ Failed to delete playlist {}: {}", playlist_id, error_text);
            return Err(format!("Failed to delete playlist: {}", error_text).into());
        }

        tracing::info!("✅ Playlist deleted: {}", playlist_id);
        Ok(())
    }

    /// Add a video to a playlist
    pub async fn add_video_to_playlist(
        &self,
        access_token: &str,
        playlist_id: &str,
        video_id: &str,
        position: Option<i32>,
    ) -> Result<PlaylistItemResponse, Box<dyn std::error::Error + Send + Sync>> {
        let url = "https://www.googleapis.com/youtube/v3/playlistItems";

        let mut resource_id = json!({
            "kind": "youtube#video",
            "videoId": video_id,
        });

        let mut snippet = json!({
            "playlistId": playlist_id,
            "resourceId": resource_id,
        });

        if let Some(pos) = position {
            snippet["position"] = json!(pos);
        }

        let body = json!({
            "snippet": snippet,
        });

        let response = self
            .client
            .post(url)
            .query(&[("part", "snippet")])
            .header("Authorization", format!("Bearer {}", access_token))
            .header("Content-Type", "application/json")
            .json(&body)
            .send()
            .await?;

        if !response.status().is_success() {
            let error_text = response.text().await?;
            return Err(format!("Failed to add video to playlist: {}", error_text).into());
        }

        let item_response: PlaylistItemResponse = response.json().await?;
        Ok(item_response)
    }

    /// Remove a video from a playlist
    pub async fn remove_video_from_playlist(
        &self,
        access_token: &str,
        playlist_item_id: &str,
    ) -> Result<(), Box<dyn std::error::Error + Send + Sync>> {
        let url = "https://www.googleapis.com/youtube/v3/playlistItems";

        let response = self
            .client
            .delete(url)
            .query(&[("id", playlist_item_id)])
            .header("Authorization", format!("Bearer {}", access_token))
            .send()
            .await?;

        if !response.status().is_success() {
            let error_text = response.text().await?;
            return Err(format!("Failed to remove video from playlist: {}", error_text).into());
        }

        Ok(())
    }

    // ========================================================================
    // Search & Discovery Methods
    // ========================================================================

    /// Search YouTube videos
    ///
    /// Required scope: https://www.googleapis.com/auth/youtube.readonly
    pub async fn search_videos(
        &self,
        access_token: Option<&str>,
        query: &str,
        max_results: i32,
        order: Option<&str>,
    ) -> Result<SearchResponse, Box<dyn std::error::Error + Send + Sync>> {
        let url = "https://www.googleapis.com/youtube/v3/search";

        let mut query_params = vec![
            ("part", "snippet".to_string()),
            ("q", query.to_string()),
            ("type", "video".to_string()),
            ("maxResults", max_results.to_string()),
        ];

        if let Some(ord) = order {
            query_params.push(("order", ord.to_string()));
        }

        let mut request = self.client.get(url).query(&query_params);

        // Use OAuth token if provided, otherwise use API key
        if let Some(token) = access_token {
            request = request.header("Authorization", format!("Bearer {}", token));
        } else {
            request = request.query(&[("key", &self.api_key)]);
        }

        let response = request.send().await?;

        if !response.status().is_success() {
            let error_text = response.text().await?;
            return Err(format!("Failed to search videos: {}", error_text).into());
        }

        let search_response: SearchResponse = response.json().await?;
        Ok(search_response)
    }

    /// Search for YouTube channels
    ///
    /// Required scope: https://www.googleapis.com/auth/youtube.readonly
    pub async fn search_channels(
        &self,
        access_token: Option<&str>,
        query: &str,
        max_results: i32,
        order: Option<&str>,
    ) -> Result<ChannelSearchResponse, Box<dyn std::error::Error + Send + Sync>> {
        let url = "https://www.googleapis.com/youtube/v3/search";

        let mut query_params = vec![
            ("part", "snippet".to_string()),
            ("q", query.to_string()),
            ("type", "channel".to_string()),  // Search for channels instead of videos
            ("maxResults", max_results.to_string()),
        ];

        if let Some(ord) = order {
            query_params.push(("order", ord.to_string()));
        }

        let mut request = self.client.get(url).query(&query_params);

        // Use OAuth token if provided, otherwise use API key
        if let Some(token) = access_token {
            request = request.header("Authorization", format!("Bearer {}", token));
        } else {
            request = request.query(&[("key", &self.api_key)]);
        }

        let response = request.send().await?;

        if !response.status().is_success() {
            let error_text = response.text().await?;
            return Err(format!("Failed to search channels: {}", error_text).into());
        }

        let search_response: ChannelSearchResponse = response.json().await?;
        Ok(search_response)
    }

    /// Get trending videos by region and category
    pub async fn get_trending_videos(
        &self,
        region_code: Option<&str>,
        category_id: Option<&str>,
        max_results: i32,
    ) -> Result<TrendingVideosResponse, Box<dyn std::error::Error + Send + Sync>> {
        let url = "https://www.googleapis.com/youtube/v3/videos";

        let mut query_params = vec![
            ("part", "snippet,statistics".to_string()),
            ("chart", "mostPopular".to_string()),
            ("maxResults", max_results.to_string()),
            ("key", self.api_key.clone()),
        ];

        if let Some(region) = region_code {
            query_params.push(("regionCode", region.to_string()));
        }

        if let Some(category) = category_id {
            query_params.push(("videoCategoryId", category.to_string()));
        }

        let response = self
            .client
            .get(url)
            .query(&query_params)
            .send()
            .await?;

        if !response.status().is_success() {
            let error_text = response.text().await?;
            return Err(format!("Failed to get trending videos: {}", error_text).into());
        }

        let trending_response: TrendingVideosResponse = response.json().await?;
        Ok(trending_response)
    }

    /// Get videos related to a specific video
    pub async fn get_related_videos(
        &self,
        video_id: &str,
        max_results: i32,
    ) -> Result<SearchResponse, Box<dyn std::error::Error + Send + Sync>> {
        let url = "https://www.googleapis.com/youtube/v3/search";

        let response = self
            .client
            .get(url)
            .query(&[
                ("part", "snippet"),
                ("relatedToVideoId", video_id),
                ("type", "video"),
                ("maxResults", &max_results.to_string()),
                ("key", &self.api_key),
            ])
            .send()
            .await?;

        if !response.status().is_success() {
            let error_text = response.text().await?;
            return Err(format!("Failed to get related videos: {}", error_text).into());
        }

        let related_response: SearchResponse = response.json().await?;
        Ok(related_response)
    }

    /// Get real-time statistics for a video (uses Data API, not Analytics API)
    pub async fn get_video_realtime_stats(
        &self,
        access_token: &str,
        video_id: &str,
    ) -> Result<VideoStatsResponse, Box<dyn std::error::Error + Send + Sync>> {
        let url = "https://www.googleapis.com/youtube/v3/videos";

        let response = self
            .client
            .get(url)
            .query(&[
                ("part", "statistics,snippet"),
                ("id", video_id),
            ])
            .header("Authorization", format!("Bearer {}", access_token))
            .send()
            .await?;

        if !response.status().is_success() {
            let error_text = response.text().await?;
            return Err(format!("Failed to get video stats: {}", error_text).into());
        }

        let stats_response: VideoStatsResponse = response.json().await?;
        Ok(stats_response)
    }

    // ========================================================================
    // Comment Moderation Methods
    // ========================================================================

    /// Get comments for a video
    ///
    /// Required scope: https://www.googleapis.com/auth/youtube.force-ssl
    pub async fn get_video_comments(
        &self,
        access_token: &str,
        video_id: &str,
        max_results: i32,
    ) -> Result<CommentThreadsResponse, Box<dyn std::error::Error + Send + Sync>> {
        let url = "https://www.googleapis.com/youtube/v3/commentThreads";

        let response = self
            .client
            .get(url)
            .query(&[
                ("part", "snippet,replies"),
                ("videoId", video_id),
                ("maxResults", &max_results.to_string()),
                ("order", "time"),  // Most recent first
            ])
            .header("Authorization", format!("Bearer {}", access_token))
            .send()
            .await?;

        if !response.status().is_success() {
            let error_text = response.text().await?;
            return Err(format!("Failed to get comments: {}", error_text).into());
        }

        let comments_response: CommentThreadsResponse = response.json().await?;
        Ok(comments_response)
    }

    /// Reply to a comment
    pub async fn reply_to_comment(
        &self,
        access_token: &str,
        parent_comment_id: &str,
        text: &str,
    ) -> Result<CommentResponse, Box<dyn std::error::Error + Send + Sync>> {
        let url = "https://www.googleapis.com/youtube/v3/comments";

        let body = json!({
            "snippet": {
                "parentId": parent_comment_id,
                "textOriginal": text,
            }
        });

        let response = self
            .client
            .post(url)
            .query(&[("part", "snippet")])
            .header("Authorization", format!("Bearer {}", access_token))
            .header("Content-Type", "application/json")
            .json(&body)
            .send()
            .await?;

        if !response.status().is_success() {
            let error_text = response.text().await?;
            return Err(format!("Failed to reply to comment: {}", error_text).into());
        }

        let comment_response: CommentResponse = response.json().await?;
        Ok(comment_response)
    }

    /// Delete a comment
    pub async fn delete_comment(
        &self,
        access_token: &str,
        comment_id: &str,
    ) -> Result<(), Box<dyn std::error::Error + Send + Sync>> {
        let url = "https://www.googleapis.com/youtube/v3/comments";

        tracing::info!("🗑️ Deleting comment: {}", comment_id);

        let response = self
            .client
            .delete(url)
            .query(&[("id", comment_id)])
            .header("Authorization", format!("Bearer {}", access_token))
            .send()
            .await?;

        if !response.status().is_success() {
            let error_text = response.text().await?;
            tracing::error!("❌ Failed to delete comment: {}", error_text);
            return Err(format!("Failed to delete comment: {}", error_text).into());
        }

        tracing::info!("✅ Comment deleted: {}", comment_id);
        Ok(())
    }

    // ========================================================================
    // Caption Management Methods
    // ========================================================================

    /// List caption tracks for a video
    pub async fn list_captions(
        &self,
        access_token: &str,
        video_id: &str,
    ) -> Result<CaptionListResponse, Box<dyn std::error::Error + Send + Sync>> {
        let url = "https://www.googleapis.com/youtube/v3/captions";

        let response = self
            .client
            .get(url)
            .query(&[
                ("part", "snippet"),
                ("videoId", video_id),
            ])
            .header("Authorization", format!("Bearer {}", access_token))
            .send()
            .await?;

        if !response.status().is_success() {
            let error_text = response.text().await?;
            return Err(format!("Failed to list captions: {}", error_text).into());
        }

        let captions: CaptionListResponse = response.json().await?;
        Ok(captions)
    }

    /// Upload a caption file (SRT, VTT, etc.)
    ///
    /// Required scope: https://www.googleapis.com/auth/youtube.force-ssl
    pub async fn upload_caption(
        &self,
        access_token: &str,
        video_id: &str,
        language: &str,
        name: &str,
        caption_data: Vec<u8>,
    ) -> Result<CaptionResponse, Box<dyn std::error::Error + Send + Sync>> {
        let url = "https://www.googleapis.com/upload/youtube/v3/captions";

        tracing::info!("📄 Uploading caption for video {}: {} ({})", video_id, name, language);

        let metadata = json!({
            "snippet": {
                "videoId": video_id,
                "language": language,
                "name": name,
                "isDraft": false,
            }
        });

        let metadata_json = serde_json::to_string(&metadata)?;

        let form = reqwest::multipart::Form::new()
            .part(
                "snippet",
                reqwest::multipart::Part::text(metadata_json)
                    .mime_str("application/json")?,
            )
            .part(
                "media",
                reqwest::multipart::Part::bytes(caption_data)
                    .file_name("caption.srt")
                    .mime_str("application/octet-stream")?,
            );

        let response = self
            .client
            .post(url)
            .query(&[
                ("part", "snippet"),
                ("uploadType", "multipart"),
            ])
            .header("Authorization", format!("Bearer {}", access_token))
            .multipart(form)
            .send()
            .await?;

        if !response.status().is_success() {
            let error_text = response.text().await?;
            tracing::error!("❌ Failed to upload caption: {}", error_text);
            return Err(format!("Failed to upload caption: {}", error_text).into());
        }

        let caption_response: CaptionResponse = response.json().await?;
        tracing::info!("✅ Caption uploaded: {}", caption_response.id);

        Ok(caption_response)
    }

    /// Delete a caption track
    pub async fn delete_caption(
        &self,
        access_token: &str,
        caption_id: &str,
    ) -> Result<(), Box<dyn std::error::Error + Send + Sync>> {
        let url = "https://www.googleapis.com/youtube/v3/captions";

        tracing::info!("🗑️ Deleting caption: {}", caption_id);

        let response = self
            .client
            .delete(url)
            .query(&[("id", caption_id)])
            .header("Authorization", format!("Bearer {}", access_token))
            .send()
            .await?;

        if !response.status().is_success() {
            let error_text = response.text().await?;
            tracing::error!("❌ Failed to delete caption: {}", error_text);
            return Err(format!("Failed to delete caption: {}", error_text).into());
        }

        tracing::info!("✅ Caption deleted: {}", caption_id);
        Ok(())
    }

    // ========================================================================
    // Resumable Upload Methods
    // ========================================================================

    /// Initiate a resumable upload session for large video files
    ///
    /// Returns an upload session URL where chunks should be sent
    ///
    /// Required scope: https://www.googleapis.com/auth/youtube.upload
    pub async fn initiate_resumable_upload(
        &self,
        access_token: &str,
        title: &str,
        description: &str,
        privacy_status: &str,
        category_id: Option<&str>,
        tags: Option<Vec<String>>,
        file_size: i64,
    ) -> Result<ResumableUploadSessionResponse, Box<dyn std::error::Error + Send + Sync>> {
        let url = "https://www.googleapis.com/upload/youtube/v3/videos";

        tracing::info!("🎬 Initiating resumable upload: {} ({} bytes)", title, file_size);

        let metadata = json!({
            "snippet": {
                "title": title,
                "description": description,
                "categoryId": category_id.unwrap_or("22"),
                "tags": tags.unwrap_or_default(),
            },
            "status": {
                "privacyStatus": privacy_status,
            }
        });

        let response = self
            .client
            .post(url)
            .query(&[
                ("uploadType", "resumable"),
                ("part", "snippet,status"),
            ])
            .header("Authorization", format!("Bearer {}", access_token))
            .header("Content-Type", "application/json")
            .header("X-Upload-Content-Length", file_size.to_string())
            .header("X-Upload-Content-Type", "video/*")
            .json(&metadata)
            .send()
            .await?;

        if !response.status().is_success() {
            let error_text = response.text().await?;
            tracing::error!("❌ Failed to initiate resumable upload: {}", error_text);
            return Err(format!("Failed to initiate resumable upload: {}", error_text).into());
        }

        // Extract the upload session URL from Location header
        let session_url = response
            .headers()
            .get("Location")
            .and_then(|v| v.to_str().ok())
            .ok_or("No upload session URL in response")?
            .to_string();

        tracing::info!("✅ Resumable upload session initiated: {}", session_url);

        Ok(ResumableUploadSessionResponse { session_url })
    }

    /// Upload a chunk of video data to a resumable upload session
    ///
    /// # Arguments
    /// * `session_url` - The upload session URL from initiate_resumable_upload
    /// * `chunk_data` - Chunk of video data
    /// * `start_byte` - Starting byte position (0-indexed)
    /// * `end_byte` - Ending byte position (inclusive)
    /// * `total_bytes` - Total file size
    pub async fn upload_resumable_chunk(
        &self,
        session_url: &str,
        chunk_data: Vec<u8>,
        start_byte: i64,
        end_byte: i64,
        total_bytes: i64,
    ) -> Result<ResumableChunkResponse, Box<dyn std::error::Error + Send + Sync>> {
        tracing::debug!("📤 Uploading chunk: bytes {}-{}/{}", start_byte, end_byte, total_bytes);

        let content_range = format!("bytes {}-{}/{}", start_byte, end_byte, total_bytes);

        let response = self
            .client
            .put(session_url)
            .header("Content-Length", chunk_data.len().to_string())
            .header("Content-Range", content_range)
            .header("Content-Type", "video/*")
            .body(chunk_data)
            .send()
            .await?;

        let status = response.status();

        // 308 Resume Incomplete = chunk uploaded successfully, more chunks expected
        // 200 OK or 201 Created = upload complete
        if status == 308 {
            tracing::debug!("✅ Chunk uploaded: {}-{}", start_byte, end_byte);
            return Ok(ResumableChunkResponse {
                complete: false,
                video_response: None,
            });
        }

        if status.is_success() {
            // Upload complete
            let video_response: VideoUploadApiResponse = response.json().await?;
            tracing::info!("✅ Resumable upload complete: {}", video_response.id);
            return Ok(ResumableChunkResponse {
                complete: true,
                video_response: Some(video_response),
            });
        }

        // Error occurred
        let error_text = response.text().await?;
        tracing::error!("❌ Failed to upload chunk: {}", error_text);
        Err(format!("Failed to upload chunk: {}", error_text).into())
    }
}

#[derive(Debug, Deserialize)]
pub struct TokenRefreshResponse {
    pub access_token: String,
    pub expires_in: i64,
    pub token_type: String,
}

// ============================================================================
// Google OAuth Helpers
// ============================================================================

/// Build Google OAuth authorization URL
pub fn build_google_oauth_url(
    client_id: &str,
    redirect_uri: &str,
    scopes: &[&str],
    state: &str,
) -> String {
    let scope_string = scopes.join(" ");

    // Using prompt=select_account allows users to:
    // 1. See all Google accounts signed into Chrome
    // 2. Pick an existing account OR sign in with a new one
    // 3. Connect YouTube channels from ANY Google account (not just the one they used to sign up)
    format!(
        "https://accounts.google.com/o/oauth2/v2/auth?client_id={}&redirect_uri={}&response_type=code&scope={}&access_type=offline&state={}&prompt=select_account",
        urlencoding::encode(client_id),
        urlencoding::encode(redirect_uri),
        urlencoding::encode(&scope_string),
        urlencoding::encode(state)
    )
}

/// Exchange authorization code for access token
pub async fn exchange_code_for_token(
    client: &Client,
    code: &str,
    client_id: &str,
    client_secret: &str,
    redirect_uri: &str,
) -> Result<GoogleTokenResponse, Box<dyn std::error::Error + Send + Sync>> {
    let url = "https://oauth2.googleapis.com/token";

    let params = json!({
        "code": code,
        "client_id": client_id,
        "client_secret": client_secret,
        "redirect_uri": redirect_uri,
        "grant_type": "authorization_code"
    });

    let response = client
        .post(url)
        .json(&params)
        .send()
        .await?;

    if !response.status().is_success() {
        let error_text = response.text().await?;
        return Err(format!("Failed to exchange code: {}", error_text).into());
    }

    let token_response: GoogleTokenResponse = response.json().await?;
    Ok(token_response)
}

#[derive(Debug, Deserialize)]
pub struct GoogleTokenResponse {
    pub access_token: String,
    pub refresh_token: Option<String>,
    pub expires_in: i64,
    pub token_type: String,
    pub scope: String,
}

/// Get user info from Google OAuth
pub async fn get_google_user_info(
    client: &Client,
    access_token: &str,
) -> Result<GoogleUserInfo, Box<dyn std::error::Error + Send + Sync>> {
    let url = "https://www.googleapis.com/oauth2/v2/userinfo";

    let response = client
        .get(url)
        .header("Authorization", format!("Bearer {}", access_token))
        .send()
        .await?;

    if !response.status().is_success() {
        let error_text = response.text().await?;
        return Err(format!("Failed to get user info: {}", error_text).into());
    }

    let user_info: GoogleUserInfo = response.json().await?;
    Ok(user_info)
}

#[derive(Debug, Deserialize)]
pub struct GoogleUserInfo {
    pub id: String,
    pub email: String,
    pub name: String,
    pub picture: Option<String>,
    pub verified_email: bool,
}

// ============================================================================
// Video Management Response Structures
// ============================================================================

#[derive(Debug, Deserialize, Serialize)]
pub struct VideoUpdateResponse {
    pub id: String,
    pub snippet: VideoSnippetResponse,
    pub status: VideoStatusResponse,
}

#[derive(Debug, Deserialize, Serialize)]
pub struct VideoSnippetResponse {
    pub title: String,
    pub description: String,
    #[serde(rename = "categoryId")]
    pub category_id: String,
    pub tags: Option<Vec<String>>,
}

#[derive(Debug, Deserialize, Serialize)]
pub struct VideoStatusResponse {
    #[serde(rename = "privacyStatus")]
    pub privacy_status: String,
}

#[derive(Debug, Deserialize, Serialize)]
pub struct ThumbnailUploadResponse {
    pub kind: String,
    pub items: Vec<ThumbnailItem>,
}

#[derive(Debug, Deserialize, Serialize)]
pub struct ThumbnailItem {
    pub url: String,
    pub width: i32,
    pub height: i32,
}

// ============================================================================
// Playlist Response Structures
// ============================================================================

#[derive(Debug, Deserialize, Serialize)]
pub struct PlaylistResponse {
    pub id: String,
    pub snippet: PlaylistSnippet,
    pub status: PlaylistStatus,
    #[serde(rename = "contentDetails")]
    pub content_details: Option<PlaylistContentDetails>,
}

#[derive(Debug, Deserialize, Serialize)]
pub struct PlaylistSnippet {
    pub title: String,
    pub description: String,
    #[serde(rename = "publishedAt")]
    pub published_at: String,
    pub thumbnails: Option<serde_json::Value>,
}

#[derive(Debug, Deserialize, Serialize)]
pub struct PlaylistStatus {
    #[serde(rename = "privacyStatus")]
    pub privacy_status: String,
}

#[derive(Debug, Deserialize, Serialize)]
pub struct PlaylistContentDetails {
    #[serde(rename = "itemCount")]
    pub item_count: i32,
}

#[derive(Debug, Deserialize, Serialize)]
pub struct PlaylistListResponse {
    pub kind: String,
    pub items: Vec<PlaylistResponse>,
    #[serde(rename = "pageInfo")]
    pub page_info: PageInfo,
}

#[derive(Debug, Deserialize, Serialize)]
pub struct PlaylistItemResponse {
    pub id: String,
    pub snippet: PlaylistItemSnippet,
}

#[derive(Debug, Deserialize, Serialize)]
pub struct PlaylistItemSnippet {
    #[serde(rename = "playlistId")]
    pub playlist_id: String,
    #[serde(rename = "resourceId")]
    pub resource_id: ResourceId,
    pub position: i32,
    pub title: Option<String>,
}

#[derive(Debug, Deserialize, Serialize)]
pub struct ResourceId {
    pub kind: String,
    #[serde(rename = "videoId")]
    pub video_id: String,
}

// ============================================================================
// Search & Discovery Response Structures
// ============================================================================

#[derive(Debug, Deserialize, Serialize)]
pub struct SearchResponse {
    pub kind: String,
    pub items: Vec<SearchResultItem>,
    #[serde(rename = "pageInfo")]
    pub page_info: PageInfo,
    #[serde(rename = "nextPageToken")]
    pub next_page_token: Option<String>,
}

#[derive(Debug, Clone, Deserialize, Serialize)]
pub struct SearchResultItem {
    pub id: SearchResultId,
    pub snippet: SearchResultSnippet,
}

#[derive(Debug, Clone, Deserialize, Serialize)]
pub struct SearchResultId {
    pub kind: String,
    #[serde(rename = "videoId")]
    pub video_id: String,
}

#[derive(Debug, Clone, Deserialize, Serialize)]
pub struct SearchResultSnippet {
    pub title: String,
    pub description: String,
    #[serde(rename = "channelId")]
    pub channel_id: String,
    #[serde(rename = "channelTitle")]
    pub channel_title: String,
    pub thumbnails: serde_json::Value,
    #[serde(rename = "publishedAt")]
    pub published_at: String,
}

// Channel search response structures
#[derive(Debug, Deserialize, Serialize)]
pub struct ChannelSearchResponse {
    pub kind: String,
    pub items: Vec<ChannelSearchItem>,
    #[serde(rename = "pageInfo")]
    pub page_info: PageInfo,
    #[serde(rename = "nextPageToken")]
    pub next_page_token: Option<String>,
}

#[derive(Debug, Deserialize, Serialize)]
pub struct ChannelSearchItem {
    pub id: ChannelSearchId,
    pub snippet: ChannelSearchSnippet,
}

#[derive(Debug, Deserialize, Serialize)]
pub struct ChannelSearchId {
    pub kind: String,
    #[serde(rename = "channelId")]
    pub channel_id: String,
}

#[derive(Debug, Deserialize, Serialize)]
pub struct ChannelSearchSnippet {
    pub title: String,
    pub description: String,
    #[serde(rename = "channelId")]
    pub channel_id: String,
    pub thumbnails: serde_json::Value,
    #[serde(rename = "publishedAt")]
    pub published_at: String,
}

#[derive(Debug, Deserialize, Serialize)]
pub struct TrendingVideosResponse {
    pub kind: String,
    pub items: Vec<TrendingVideoItem>,
}

#[derive(Debug, Deserialize, Serialize)]
pub struct TrendingVideoItem {
    pub id: String,
    pub snippet: SearchResultSnippet,
    pub statistics: VideoStatistics,
}

#[derive(Debug, Deserialize, Serialize)]
pub struct VideoStatistics {
    #[serde(rename = "viewCount")]
    pub view_count: String,
    #[serde(rename = "likeCount")]
    pub like_count: Option<String>,
    #[serde(rename = "commentCount")]
    pub comment_count: Option<String>,
}

#[derive(Debug, Deserialize, Serialize)]
pub struct VideoStatsResponse {
    pub kind: String,
    pub items: Vec<VideoStatsItem>,
}

#[derive(Debug, Deserialize, Serialize)]
pub struct VideoStatsItem {
    pub id: String,
    pub snippet: SearchResultSnippet,
    pub statistics: VideoStatistics,
}

#[derive(Debug, Deserialize, Serialize)]
pub struct PageInfo {
    #[serde(rename = "totalResults")]
    pub total_results: i32,
    #[serde(rename = "resultsPerPage")]
    pub results_per_page: i32,
}

// ============================================================================
// Comment Response Structures
// ============================================================================

#[derive(Debug, Deserialize, Serialize)]
pub struct CommentThreadsResponse {
    pub kind: String,
    pub items: Vec<CommentThread>,
    #[serde(rename = "pageInfo")]
    pub page_info: PageInfo,
}

#[derive(Debug, Deserialize, Serialize)]
pub struct CommentThread {
    pub id: String,
    pub snippet: CommentThreadSnippet,
    pub replies: Option<CommentReplies>,
}

#[derive(Debug, Deserialize, Serialize)]
pub struct CommentThreadSnippet {
    #[serde(rename = "videoId")]
    pub video_id: String,
    #[serde(rename = "topLevelComment")]
    pub top_level_comment: Comment,
    #[serde(rename = "canReply")]
    pub can_reply: bool,
    #[serde(rename = "totalReplyCount")]
    pub total_reply_count: i32,
}

#[derive(Debug, Deserialize, Serialize)]
pub struct CommentReplies {
    pub comments: Vec<Comment>,
}

#[derive(Debug, Deserialize, Serialize)]
pub struct Comment {
    pub id: String,
    pub snippet: CommentSnippet,
}

#[derive(Debug, Deserialize, Serialize)]
pub struct CommentSnippet {
    #[serde(rename = "authorDisplayName")]
    pub author_display_name: String,
    #[serde(rename = "authorChannelId")]
    pub author_channel_id: Option<serde_json::Value>,
    #[serde(rename = "textDisplay")]
    pub text_display: String,
    #[serde(rename = "textOriginal")]
    pub text_original: String,
    #[serde(rename = "likeCount")]
    pub like_count: i32,
    #[serde(rename = "publishedAt")]
    pub published_at: String,
    #[serde(rename = "updatedAt")]
    pub updated_at: String,
}

#[derive(Debug, Deserialize, Serialize)]
pub struct CommentResponse {
    pub id: String,
    pub snippet: CommentSnippet,
}

// ============================================================================
// Caption Response Structures
// ============================================================================

#[derive(Debug, Deserialize, Serialize)]
pub struct CaptionListResponse {
    pub kind: String,
    pub items: Vec<CaptionTrack>,
}

#[derive(Debug, Deserialize, Serialize)]
pub struct CaptionTrack {
    pub id: String,
    pub snippet: CaptionSnippet,
}

#[derive(Debug, Deserialize, Serialize)]
pub struct CaptionSnippet {
    #[serde(rename = "videoId")]
    pub video_id: String,
    pub language: String,
    pub name: String,
    #[serde(rename = "trackKind")]
    pub track_kind: String,
    #[serde(rename = "isAutoSynced")]
    pub is_auto_synced: bool,
    #[serde(rename = "isDraft")]
    pub is_draft: bool,
}

#[derive(Debug, Deserialize, Serialize)]
pub struct CaptionResponse {
    pub id: String,
    pub snippet: CaptionSnippet,
}

// ============================================================================
// Resumable Upload Response Structures
// ============================================================================

#[derive(Debug, Deserialize, Serialize)]
pub struct ResumableUploadSessionResponse {
    pub session_url: String,
}

#[derive(Debug, Deserialize, Serialize)]
pub struct ResumableChunkResponse {
    pub complete: bool,
    pub video_response: Option<VideoUploadApiResponse>,
}

#[derive(Debug, Deserialize, Serialize)]
pub struct VideoUploadApiResponse {
    pub id: String,
    pub snippet: VideoSnippetResponse,
}
