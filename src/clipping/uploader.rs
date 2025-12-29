// Clip upload manager for posting to YouTube

use crate::clipping::ai_clipper::ExtractedClipData;
use crate::models::youtube::ConnectedYouTubeChannel;
use crate::youtube_client::YouTubeClient;
use chrono::Utc;
use sqlx::PgPool;
use std::sync::Arc;

pub struct ClipUploader {
    pub youtube_client: Arc<YouTubeClient>,
    pub db_pool: PgPool,
    pub oauth_client_id: String,
    pub oauth_client_secret: String,
}

impl ClipUploader {
    pub fn new(
        youtube_client: Arc<YouTubeClient>,
        db_pool: PgPool,
        oauth_client_id: String,
        oauth_client_secret: String,
    ) -> Self {
        Self {
            youtube_client,
            db_pool,
            oauth_client_id,
            oauth_client_secret,
        }
    }

    /// Upload a clip to YouTube as a Short
    pub async fn upload_clip(
        &self,
        clip: &ExtractedClipData,
        clip_db_id: i32,
        destination_channel: &ConnectedYouTubeChannel,
    ) -> Result<YouTubeUploadResult, String> {
        tracing::info!(
            "📤 Uploading clip '{}' to YouTube channel {}",
            clip.ai_title,
            destination_channel.channel_name
        );

        // Step 1: Ensure access token is valid
        let access_token = self.ensure_valid_token(destination_channel).await?;

        // Step 2: Prepare metadata optimized for YouTube Shorts
        let title = self.optimize_title(&clip.ai_title);
        let description = self.format_description(&clip.ai_description, &clip.ai_tags);

        tracing::debug!("Title: {}", title);
        tracing::debug!("Description: {}", description);

        // Step 3: Upload to YouTube
        let upload_result = self
            .youtube_client
            .upload_video(
                &access_token,
                &clip.local_clip_path,
                &title,
                &description,
                "public",
                Some("24"), // Category: Entertainment
                Some(clip.ai_tags.clone()),
            )
            .await
            .map_err(|e| format!("YouTube upload failed: {}", e))?;

        tracing::info!("✅ Clip uploaded successfully: {}", upload_result.id);

        // Construct YouTube URL
        let youtube_url = format!("https://youtube.com/shorts/{}", upload_result.id);

        // Step 4: Update database with YouTube video ID and URL
        self.update_clip_upload_status(
            clip_db_id,
            &upload_result.id,
            &youtube_url,
        )
        .await?;

        Ok(YouTubeUploadResult {
            video_id: upload_result.id,
            url: youtube_url,
        })
    }

    /// Ensure access token is valid, refresh if necessary
    async fn ensure_valid_token(
        &self,
        channel: &ConnectedYouTubeChannel,
    ) -> Result<String, String> {
        // Check if token expires within 5 minutes (same logic as existing code)
        let now = Utc::now();
        let expires_soon = channel.token_expiry < now + chrono::Duration::minutes(5);

        if expires_soon {
            tracing::info!("Access token expiring soon, refreshing...");

            // Refresh token using YouTube client
            let new_token = self
                .youtube_client
                .refresh_access_token(
                    &channel.refresh_token,
                    &self.oauth_client_id,
                    &self.oauth_client_secret,
                )
                .await
                .map_err(|e| format!("Token refresh failed: {}", e))?;

            // Update database with new token
            sqlx::query(
                "UPDATE connected_youtube_channels
                 SET access_token = $1, token_expiry = $2
                 WHERE id = $3",
            )
            .bind(&new_token.access_token)
            .bind(now + chrono::Duration::seconds(new_token.expires_in))
            .bind(channel.id)
            .execute(&self.db_pool)
            .await
            .map_err(|e| format!("Failed to update token: {}", e))?;

            Ok(new_token.access_token)
        } else {
            Ok(channel.access_token.clone())
        }
    }

    /// Optimize title for YouTube Shorts (max 100 chars)
    fn optimize_title(&self, title: &str) -> String {
        let mut optimized = title.trim().to_string();

        // Add #Shorts hashtag if not present
        if !optimized.to_lowercase().contains("#shorts") {
            optimized.push_str(" #Shorts");
        }

        // Truncate if too long
        if optimized.len() > 100 {
            optimized = optimized[..97].to_string();
            optimized.push_str("...");
        }

        optimized
    }

    /// Format description with hashtags
    fn format_description(&self, description: &str, tags: &[String]) -> String {
        let mut formatted = description.trim().to_string();

        // Add newline before hashtags
        formatted.push_str("\n\n");

        // Add #Shorts if not present
        if !formatted.to_lowercase().contains("#shorts") {
            formatted.push_str("#Shorts ");
        }

        // Add tags as hashtags
        for tag in tags.iter().take(10) {
            // YouTube limits total tags
            let tag_clean = tag.replace(" ", "").replace("#", "");
            formatted.push_str(&format!("#{} ", tag_clean));
        }

        formatted
    }

    /// Update clip record in database with upload result
    async fn update_clip_upload_status(
        &self,
        clip_id: i32,
        video_id: &str,
        url: &str,
    ) -> Result<(), String> {
        sqlx::query(
            "UPDATE extracted_clips
             SET youtube_video_id = $1, youtube_url = $2,
                 upload_status = 'published', published_at = NOW()
             WHERE id = $3",
        )
        .bind(video_id)
        .bind(url)
        .bind(clip_id)
        .execute(&self.db_pool)
        .await
        .map_err(|e| format!("Failed to update clip status: {}", e))?;

        Ok(())
    }

    /// Mark clip upload as failed
    pub async fn mark_upload_failed(&self, clip_id: i32, error: &str) -> Result<(), String> {
        sqlx::query(
            "UPDATE extracted_clips
             SET upload_status = 'failed', upload_error = $1
             WHERE id = $2",
        )
        .bind(error)
        .bind(clip_id)
        .execute(&self.db_pool)
        .await
        .map_err(|e| format!("Failed to mark upload as failed: {}", e))?;

        Ok(())
    }
}

/// Result of YouTube upload
#[derive(Debug)]
pub struct YouTubeUploadResult {
    pub video_id: String,
    pub url: String,
}
