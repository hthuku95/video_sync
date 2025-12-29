// Channel monitoring system for polling YouTube channels

use crate::clipping::models::{ChannelLinkage, SourceChannel};
use crate::youtube_client::YouTubeClient;
use chrono::Utc;
use sqlx::PgPool;
use std::sync::Arc;

pub struct ChannelMonitor {
    pub youtube_client: Arc<YouTubeClient>,
    pub db_pool: PgPool,
}

impl ChannelMonitor {
    pub fn new(youtube_client: Arc<YouTubeClient>, db_pool: PgPool) -> Self {
        Self {
            youtube_client,
            db_pool,
        }
    }

    /// Poll all active source channels for new videos
    pub async fn poll_all_channels(&self) -> Result<(), String> {
        tracing::info!("🔍 Starting channel polling cycle...");

        // Get all active source channels that are due for polling
        let channels = self.get_channels_due_for_poll().await?;

        if channels.is_empty() {
            tracing::info!("No channels due for polling");
            return Ok(());
        }

        tracing::info!("📺 Polling {} channels", channels.len());

        for channel in channels {
            if let Err(e) = self.poll_channel(&channel).await {
                tracing::error!(
                    "Failed to poll channel {} ({}): {}",
                    channel.channel_name,
                    channel.channel_id,
                    e
                );
                // Continue with other channels even if one fails
            }
        }

        tracing::info!("✅ Channel polling cycle completed");
        Ok(())
    }

    /// Poll a single channel for new videos
    async fn poll_channel(&self, channel: &SourceChannel) -> Result<(), String> {
        let start_time = std::time::Instant::now();

        tracing::info!(
            "Polling channel: {} ({})",
            channel.channel_name,
            channel.channel_id
        );

        // Mark as currently polling
        self.mark_polling(channel.id, true).await?;

        // Search YouTube for latest videos from this channel
        let query = format!("channel:{}", channel.channel_id);
        let videos = match self
            .youtube_client
            .search_videos(None, &query, 10, Some("date"))
            .await
        {
            Ok(response) => response.items,
            Err(e) => {
                self.mark_polling(channel.id, false).await?;
                self.increment_failure_count(channel.id).await?;
                return Err(format!("YouTube API search failed: {}", e));
            }
        };

        // Filter for new videos not yet processed
        let new_videos = self.filter_new_videos(channel, &videos).await?;

        tracing::info!("Found {} new videos", new_videos.len());

        // Create clipping jobs for new videos
        for video in &new_videos {
            if let Err(e) = self.create_clipping_job(channel, video).await {
                tracing::error!("Failed to create clipping job for video {}: {}", video.id.video_id, e);
            }
        }

        // Update last_polled_at and last_video_checked
        if let Some(latest_video) = new_videos.first() {
            self.update_poll_timestamp(channel.id, &latest_video.id.video_id)
                .await?;
        } else {
            // No new videos, just update timestamp
            self.update_poll_timestamp(channel.id, &channel.last_video_checked.clone().unwrap_or_default())
                .await?;
        }

        // Mark as no longer polling
        self.mark_polling(channel.id, false).await?;

        // Reset consecutive failures on success
        self.reset_failure_count(channel.id).await?;

        let duration = start_time.elapsed();
        tracing::info!(
            "✅ Completed polling {} in {:?}",
            channel.channel_name,
            duration
        );

        Ok(())
    }

    /// Get channels that are due for polling
    async fn get_channels_due_for_poll(&self) -> Result<Vec<SourceChannel>, String> {
        let channels = sqlx::query_as::<_, SourceChannel>(
            "SELECT * FROM youtube_source_channels
             WHERE is_active = true
               AND (last_polled_at IS NULL
                    OR last_polled_at < NOW() - (polling_interval_minutes * INTERVAL '1 minute'))
             ORDER BY last_polled_at ASC NULLS FIRST
             LIMIT 10",
        )
        .fetch_all(&self.db_pool)
        .await
        .map_err(|e| format!("Database error: {}", e))?;

        Ok(channels)
    }

    /// Filter videos to find new ones not yet processed
    async fn filter_new_videos(
        &self,
        channel: &SourceChannel,
        videos: &[crate::youtube_client::SearchResultItem],
    ) -> Result<Vec<crate::youtube_client::SearchResultItem>, String> {
        let mut new_videos = Vec::new();

        for video in videos {
            // Skip if we've already processed this video
            if let Some(ref last_checked) = channel.last_video_checked {
                if video.id.video_id == *last_checked {
                    break; // All videos after this are older
                }
            }

            // Check if we already have a clipping job for this video
            let exists = sqlx::query_scalar::<_, bool>(
                "SELECT EXISTS(SELECT 1 FROM clipping_jobs WHERE source_video_id = $1)",
            )
            .bind(&video.id.video_id)
            .fetch_one(&self.db_pool)
            .await
            .unwrap_or(false);

            if !exists {
                new_videos.push(video.clone());
            }
        }

        Ok(new_videos)
    }

    /// Create clipping jobs for all linkages of this source channel
    async fn create_clipping_job(
        &self,
        channel: &SourceChannel,
        video: &crate::youtube_client::SearchResultItem,
    ) -> Result<(), String> {
        // Find all active linkages for this source channel
        let linkages = sqlx::query_as::<_, ChannelLinkage>(
            "SELECT * FROM youtube_channel_linkages
             WHERE source_channel_id = $1 AND is_active = true",
        )
        .bind(channel.id)
        .fetch_all(&self.db_pool)
        .await
        .map_err(|e| format!("Database error: {}", e))?;

        if linkages.is_empty() {
            tracing::warn!(
                "No active linkages found for source channel {}",
                channel.channel_name
            );
            return Ok(());
        }

        // Create a clipping job for each linkage
        for linkage in linkages {
            sqlx::query(
                "INSERT INTO clipping_jobs
                 (linkage_id, source_video_id, source_video_title, status)
                 VALUES ($1, $2, $3, 'pending')",
            )
            .bind(linkage.id)
            .bind(&video.id.video_id)
            .bind(&video.snippet.title)
            .execute(&self.db_pool)
            .await
            .map_err(|e| format!("Failed to create clipping job: {}", e))?;

            tracing::info!(
                "Created clipping job for video '{}' (linkage: {})",
                video.snippet.title,
                linkage.id
            );
        }

        Ok(())
    }

    /// Mark channel as currently polling or not
    async fn mark_polling(&self, channel_id: i32, is_polling: bool) -> Result<(), String> {
        sqlx::query(
            "UPDATE clipping_poll_schedule
             SET is_polling = $1
             WHERE source_channel_id = $2",
        )
        .bind(is_polling)
        .bind(channel_id)
        .execute(&self.db_pool)
        .await
        .map_err(|e| format!("Failed to update polling status: {}", e))?;

        Ok(())
    }

    /// Update last polled timestamp and last checked video
    async fn update_poll_timestamp(
        &self,
        channel_id: i32,
        last_video_id: &str,
    ) -> Result<(), String> {
        sqlx::query(
            "UPDATE youtube_source_channels
             SET last_polled_at = $1, last_video_checked = $2
             WHERE id = $3",
        )
        .bind(Utc::now())
        .bind(last_video_id)
        .bind(channel_id)
        .execute(&self.db_pool)
        .await
        .map_err(|e| format!("Failed to update poll timestamp: {}", e))?;

        Ok(())
    }

    /// Increment consecutive failure count
    async fn increment_failure_count(&self, channel_id: i32) -> Result<(), String> {
        sqlx::query(
            "UPDATE clipping_poll_schedule
             SET consecutive_failures = consecutive_failures + 1
             WHERE source_channel_id = $1",
        )
        .bind(channel_id)
        .execute(&self.db_pool)
        .await
        .map_err(|e| format!("Failed to increment failure count: {}", e))?;

        Ok(())
    }

    /// Reset consecutive failure count
    async fn reset_failure_count(&self, channel_id: i32) -> Result<(), String> {
        sqlx::query(
            "UPDATE clipping_poll_schedule
             SET consecutive_failures = 0
             WHERE source_channel_id = $1",
        )
        .bind(channel_id)
        .execute(&self.db_pool)
        .await
        .map_err(|e| format!("Failed to reset failure count: {}", e))?;

        Ok(())
    }
}
