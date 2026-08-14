// Channel monitoring system for polling YouTube channels

use crate::clipping::models::{ChannelLinkage, SourceChannel};
use crate::youtube_client::YouTubeClient;
use aws_config::BehaviorVersion;
use chrono::Utc;
use sqlx::PgPool;
use std::sync::Arc;

const ACTIVE_CLIPPING_JOB_STATUSES: &[&str] = &[
    "pending",
    "downloading",
    "analyzing",
    "extracting_clips",
    "posting",
];

pub struct ChannelMonitor {
    pub youtube_client: Arc<YouTubeClient>,
    pub db_pool: PgPool,
}

async fn create_clipping_workflow(
    pool: &PgPool,
    linkage: &ChannelLinkage,
    source_video_id: &str,
    source_video_title: &str,
) -> Result<uuid::Uuid, String> {
    let workflow_runtime = crate::services::WorkflowRuntime::new(pool.clone());
    let workflow_id = workflow_runtime
        .create_or_reuse_workflow(crate::services::NewWorkflow {
            idempotency_key: Some(format!("auto-clipping:{}:{}", source_video_id, linkage.id)),
            workflow_type: "auto_clipping_job".to_string(),
            status: crate::services::WorkflowStatus::Queued,
            session_uuid: None,
            user_id: Some(linkage.user_id),
            source_table: Some("clipping_jobs".to_string()),
            source_record_id: None,
            request_summary: format!(
                "Auto clipping for video {} on linkage {}",
                source_video_title, linkage.id
            )
            .chars()
            .take(200)
            .collect::<String>(),
            current_step: Some("job_created".to_string()),
            metadata: serde_json::json!({
                "linkage_id": linkage.id,
                "source_video_id": source_video_id,
                "source_video_title": source_video_title,
                "destination_channel_id": linkage.destination_channel_id,
            }),
            artifact_requirements: serde_json::json!([
                {
                    "kind": "uploaded_clips",
                    "required": true,
                    "must_create_extracted_clip_records": true
                }
            ]),
        })
        .await?;

    let _ = workflow_runtime
        .append_event(
            workflow_id,
            "queued",
            Some("job_created"),
            "Auto clipping job created and queued for the clipping worker.",
            serde_json::json!({
                "linkage_id": linkage.id,
                "source_video_id": source_video_id,
            }),
        )
        .await;

    Ok(workflow_id)
}

async fn find_active_clipping_job(
    pool: &PgPool,
    linkage_id: i32,
    source_video_id: &str,
) -> Result<Option<i32>, String> {
    sqlx::query_scalar::<_, i32>(
        "SELECT id
         FROM clipping_jobs
         WHERE linkage_id = $1
           AND source_video_id = $2
           AND status = ANY($3)
         ORDER BY
             CASE WHEN claimed_by IS NOT NULL THEN 0 ELSE 1 END,
             created_at ASC
         LIMIT 1",
    )
    .bind(linkage_id)
    .bind(source_video_id)
    .bind(ACTIVE_CLIPPING_JOB_STATUSES)
    .fetch_optional(pool)
    .await
    .map_err(|e| format!("Failed to lookup active clipping job: {}", e))
}

async fn enqueue_clipping_job(
    pool: &PgPool,
    linkage: &ChannelLinkage,
    source_video_id: &str,
    source_video_title: &str,
) -> Result<(i32, bool), String> {
    if let Some(existing_job_id) =
        find_active_clipping_job(pool, linkage.id, source_video_id).await?
    {
        return Ok((existing_job_id, false));
    }

    let workflow_id =
        create_clipping_workflow(pool, linkage, source_video_id, source_video_title).await?;

    match sqlx::query_scalar::<_, i32>(
        "INSERT INTO clipping_jobs
         (linkage_id, source_video_id, source_video_title, status, workflow_id)
         VALUES ($1, $2, $3, 'pending', $4)
         RETURNING id",
    )
    .bind(linkage.id)
    .bind(source_video_id)
    .bind(source_video_title)
    .bind(workflow_id)
    .fetch_one(pool)
    .await
    {
            Ok(job_id) => {
                let _ = sqlx::query(
                    "UPDATE app_workflows
                     SET metadata = jsonb_set(
                             COALESCE(metadata, '{}'::jsonb),
                             '{clipping_job_id}',
                             to_jsonb($1::int),
                             true
                         ),
                         updated_at = NOW()
                     WHERE id = $2
                       AND source_table = 'clipping_jobs'",
                )
                .bind(job_id)
                .bind(workflow_id)
                .execute(pool)
                .await;
                tokio::spawn(async move {
                    if let Err(e) = sqs_enqueue_clipping_job(job_id).await {
                        tracing::warn!("SQS enqueue failed (non-fatal): {}", e);
                    }
                });
                Ok((job_id, true))
        }
        Err(sqlx::Error::Database(db_err)) if db_err.is_unique_violation() => {
            let existing_job_id = find_active_clipping_job(pool, linkage.id, source_video_id)
                .await?
                .ok_or_else(|| {
                    "Active clipping job appeared to exist after unique violation, but it could not be fetched".to_string()
                })?;
            Ok((existing_job_id, false))
        }
        Err(e) => Err(format!("Failed to create clipping job: {}", e)),
    }
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

        // NEW: Process pending videos first (from previous blocked attempts)
        if let Err(e) = self.process_pending_videos().await {
            tracing::error!("Failed to process pending videos: {}", e);
        }

        // Get all active source channels that are due for polling
        let channels = self.get_channels_due_for_poll().await?;

        if channels.is_empty() {
            tracing::info!("No channels due for polling");
            return Ok(());
        }

        tracing::info!("📺 Polling {} channels", channels.len());

        let mut success_count = 0i32;
        let mut fail_count = 0i32;

        for channel in &channels {
            if let Err(e) = self.poll_channel(channel).await {
                fail_count += 1;
                tracing::error!(
                    "Failed to poll channel {} ({}): {}",
                    channel.channel_name,
                    channel.channel_id,
                    e
                );
            } else {
                success_count += 1;
            }
        }

        if fail_count > 0 && success_count == 0 {
            tracing::error!(
                "🚨 ALL {} channel polls failed this cycle — verify YOUTUBE_API_KEY is valid on Render",
                fail_count
            );
        } else if fail_count > 0 {
            tracing::warn!(
                "{}/{} channel polls failed this cycle",
                fail_count,
                channels.len()
            );
        }

        tracing::info!(
            "✅ Channel polling cycle completed ({} succeeded, {} failed)",
            success_count,
            fail_count
        );
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

        // Update last_polled_at immediately so the timestamp always reflects the most recent
        // attempt, even if the YouTube API call below fails. Without this, a persistent API
        // failure leaves last_polled_at stale (e.g., 5 days old) making it impossible to tell
        // from the DB whether the monitor is running at all.
        let _ =
            sqlx::query("UPDATE youtube_source_channels SET last_polled_at = NOW() WHERE id = $1")
                .bind(channel.id)
                .execute(&self.db_pool)
                .await;

        // CRITICAL FIX: Use playlistItems API instead of search API to save quota
        // The YouTube Data API v3 charges:
        // - search.list() = 100 quota units per call
        // - playlistItems.list() = 1 quota unit per call
        // With 10,000 daily quota, search API allows only 100 calls/day (10 channels × 48 times = 480 calls needed)
        // Using playlistItems allows 10,000 calls/day which is more than sufficient

        // CHANGED: Get videos from channel's upload playlist instead of using search
        // Every YouTube channel has an "uploads" playlist with ID starting with "UU" + rest of channel ID
        let playlist_id = if channel.channel_id.starts_with("UC") {
            format!("UU{}", &channel.channel_id[2..])
        } else {
            tracing::error!("Invalid channel ID format: {}", channel.channel_id);
            self.mark_polling(channel.id, false).await?;
            return Err(format!("Invalid channel ID format: {}", channel.channel_id));
        };

        let videos = match self
            .youtube_client
            .get_channel_uploads(&playlist_id, 10)
            .await
        {
            Ok(response) => response.items,
            Err(e) => {
                let error_str = e.to_string();

                // Check if this is a quota exhaustion error
                if error_str.contains("quotaExceeded") {
                    tracing::error!(
                        "⚠️ YouTube API quota exceeded for channel {} ({}). System will pause polling.",
                        channel.channel_name,
                        channel.channel_id
                    );
                    self.mark_polling(channel.id, false).await?;
                    self.increment_failure_count(channel.id).await?;
                    return Err(format!("Quota exceeded, will retry later: {}", e));
                }

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
                tracing::error!(
                    "Failed to create clipping job for video {}: {}",
                    video.id.video_id,
                    e
                );
            }
        }

        // Update last_polled_at and last_video_checked
        if let Some(latest_video) = new_videos.first() {
            self.update_poll_timestamp(channel.id, &latest_video.id.video_id)
                .await?;
        } else {
            // No new videos, just update timestamp
            self.update_poll_timestamp(
                channel.id,
                &channel.last_video_checked.clone().unwrap_or_default(),
            )
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
    /// NOW: Uses clipped_source_videos table for backward scanning
    async fn filter_new_videos(
        &self,
        channel: &SourceChannel,
        videos: &[crate::youtube_client::SearchResultItem],
    ) -> Result<Vec<crate::youtube_client::SearchResultItem>, String> {
        // Get all clipped video IDs for this source channel
        let clipped_video_ids: Vec<String> = sqlx::query_scalar(
            "SELECT video_id FROM clipped_source_videos WHERE source_channel_id = $1",
        )
        .bind(channel.id)
        .fetch_all(&self.db_pool)
        .await
        .unwrap_or_default();

        let mut new_videos = Vec::new();

        for video in videos {
            // Skip if already clipped
            if clipped_video_ids.contains(&video.id.video_id) {
                continue;
            }

            // Also check for active/completed jobs (job may exist but not in clipped table yet).
            // Exclude 'failed' and 'cancelled' statuses so videos with failed jobs can be re-queued.
            let job_exists = sqlx::query_scalar::<_, bool>(
                "SELECT EXISTS(
                    SELECT 1 FROM clipping_jobs
                    WHERE source_video_id = $1
                      AND status NOT IN ('failed', 'cancelled')
                )",
            )
            .bind(&video.id.video_id)
            .fetch_one(&self.db_pool)
            .await
            .unwrap_or(false);

            if !job_exists {
                new_videos.push(video.clone());
            }
        }

        Ok(new_videos)
    }

    /// Create clipping jobs for all linkages of this source channel
    /// NOW: Includes 24-hour cooldown, 4-clip daily limit, and session memory
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

        // Process each linkage
        for linkage in linkages {
            // CHECK 1: 24-hour cooldown
            if !self.is_linkage_eligible_for_session(linkage.id).await? {
                tracing::info!(
                    "Linkage {} in cooldown - storing video {} for later",
                    linkage.id,
                    video.id.video_id
                );
                self.remember_pending_video(linkage.id, video).await?;
                continue;
            }

            // CHECK 2: Daily 4-clip limit
            let clips_today = self
                .count_clips_posted_today(linkage.destination_channel_id)
                .await?;
            if clips_today >= 4 {
                tracing::info!(
                    "Destination channel {} reached daily limit (4 clips) - storing video for tomorrow",
                    linkage.destination_channel_id
                );
                self.remember_pending_video(linkage.id, video).await?;
                continue;
            }

            // CHECK 3: Active job doesn't already exist
            if find_active_clipping_job(&self.db_pool, linkage.id, &video.id.video_id)
                .await?
                .is_some()
            {
                tracing::debug!(
                    "Active job already exists for video {} on linkage {}",
                    video.id.video_id,
                    linkage.id
                );
                continue;
            }

            // ALL CHECKS PASSED - Create job
            let (job_id, created) = enqueue_clipping_job(
                &self.db_pool,
                &linkage,
                &video.id.video_id,
                &video.snippet.title,
            )
            .await?;

            // NOTE: clipped_source_videos is NOT inserted here.
            // It is only written upon successful job completion in execute_clipping_job().
            // This allows the monitor to re-queue a video whose job previously failed or
            // was cancelled, since filter_new_videos() checks clipped_source_videos first.

            if created {
                tracing::info!(
                    "✅ Created clipping job {} for video '{}' (linkage: {})",
                    job_id,
                    video.snippet.title,
                    linkage.id
                );
            } else {
                tracing::info!(
                    "♻️ Reused active clipping job {} for video '{}' (linkage: {})",
                    job_id,
                    video.snippet.title,
                    linkage.id
                );
            }
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

    /// Check if a linkage is eligible for a clipping session (24-hour cooldown)
    async fn is_linkage_eligible_for_session(&self, linkage_id: i32) -> Result<bool, String> {
        let linkage = sqlx::query_as::<_, ChannelLinkage>(
            "SELECT * FROM youtube_channel_linkages WHERE id = $1",
        )
        .bind(linkage_id)
        .fetch_one(&self.db_pool)
        .await
        .map_err(|e| format!("Failed to fetch linkage: {}", e))?;

        if let Some(last_session) = linkage.last_clipping_session_at {
            let cooldown = chrono::Duration::hours(linkage.clipping_cooldown_hours as i64);
            let next_allowed = last_session + cooldown;

            if Utc::now() < next_allowed {
                tracing::debug!(
                    "Linkage {} in cooldown (last: {}, next: {})",
                    linkage_id,
                    last_session,
                    next_allowed
                );
                return Ok(false);
            }
        }

        Ok(true)
    }

    /// Count clips posted in last 24 hours for a destination channel
    async fn count_clips_posted_today(&self, destination_channel_id: i32) -> Result<i32, String> {
        let count: i64 = sqlx::query_scalar(
            "SELECT COUNT(*) FROM extracted_clips
             WHERE destination_channel_id = $1
             AND upload_status = 'published'
             AND published_at >= NOW() - INTERVAL '24 hours'",
        )
        .bind(destination_channel_id)
        .fetch_one(&self.db_pool)
        .await
        .map_err(|e| format!("Failed to count clips: {}", e))?;

        Ok(count as i32)
    }

    /// Store unclipped video for later when cooldown blocks session
    async fn remember_pending_video(
        &self,
        linkage_id: i32,
        video: &crate::youtube_client::SearchResultItem,
    ) -> Result<(), String> {
        sqlx::query(
            "INSERT INTO pending_unclipped_videos
             (linkage_id, video_id, video_title, video_published_at)
             VALUES ($1, $2, $3, $4)
             ON CONFLICT (linkage_id, video_id) DO NOTHING",
        )
        .bind(linkage_id)
        .bind(&video.id.video_id)
        .bind(&video.snippet.title)
        .bind(&video.snippet.published_at)
        .execute(&self.db_pool)
        .await
        .map_err(|e| format!("Failed to store pending: {}", e))?;

        Ok(())
    }

    /// Retrieve pending videos for eligible linkages
    async fn get_pending_videos(&self, linkage_id: i32) -> Result<Vec<String>, String> {
        let video_ids: Vec<String> = sqlx::query_scalar(
            "SELECT video_id FROM pending_unclipped_videos
             WHERE linkage_id = $1
             ORDER BY discovered_at ASC",
        )
        .bind(linkage_id)
        .fetch_all(&self.db_pool)
        .await
        .map_err(|e| format!("Failed to fetch pending: {}", e))?;

        Ok(video_ids)
    }

    /// Clear pending video after job creation
    async fn clear_pending_video(&self, linkage_id: i32, video_id: &str) -> Result<(), String> {
        sqlx::query("DELETE FROM pending_unclipped_videos WHERE linkage_id = $1 AND video_id = $2")
            .bind(linkage_id)
            .bind(video_id)
            .execute(&self.db_pool)
            .await
            .map_err(|e| format!("Failed to clear pending: {}", e))?;

        Ok(())
    }

    /// Process pending videos that were blocked by cooldown/limits
    /// Called at start of each poll cycle
    async fn process_pending_videos(&self) -> Result<(), String> {
        tracing::info!("🔄 Processing pending videos from previous scans...");

        // Get all active linkages
        let linkages = sqlx::query_as::<_, ChannelLinkage>(
            "SELECT * FROM youtube_channel_linkages WHERE is_active = true",
        )
        .fetch_all(&self.db_pool)
        .await
        .map_err(|e| format!("Failed to fetch linkages: {}", e))?;

        for linkage in linkages {
            // Check if linkage is now eligible
            if !self.is_linkage_eligible_for_session(linkage.id).await? {
                continue; // Still in cooldown
            }

            // Check daily limit
            let clips_today = self
                .count_clips_posted_today(linkage.destination_channel_id)
                .await?;
            if clips_today >= 4 {
                continue; // Daily limit reached
            }

            // Get pending videos for this linkage
            let pending_videos = self.get_pending_videos(linkage.id).await?;

            if pending_videos.is_empty() {
                continue;
            }

            tracing::info!(
                "Found {} pending videos for linkage {}",
                pending_videos.len(),
                linkage.id
            );

            // Take first pending video (FIFO order)
            if let Some(video_id) = pending_videos.first() {
                let (job_id, created) =
                    enqueue_clipping_job(&self.db_pool, &linkage, video_id, "Pending Video")
                        .await?;

                // Clear from pending
                self.clear_pending_video(linkage.id, video_id).await?;

                if created {
                    tracing::info!("✅ Created job {} from pending video {}", job_id, video_id);
                } else {
                    tracing::info!(
                        "♻️ Pending video {} already has active clipping job {}",
                        video_id,
                        job_id
                    );
                }
            }
        }

        Ok(())
    }
}

/// Fire-and-forget SQS enqueue for clipping jobs.
/// Reads CLIPPING_SQS_QUEUE_URL env var; no-ops if unset.
async fn sqs_enqueue_clipping_job(job_id: i32) -> Result<(), String> {
    let queue_url = match std::env::var("CLIPPING_SQS_QUEUE_URL") {
        Ok(url) if !url.is_empty() => url,
        _ => return Ok(()),
    };
    let config = aws_config::defaults(BehaviorVersion::latest())
        .region(aws_config::Region::new(
            &std::env::var("AWS_REGION").unwrap_or_else(|_| "us-east-1".to_string()),
        ))
        .load()
        .await;
    let client = aws_sdk_sqs::Client::new(&config);
    let body = serde_json::json!({"job_id": job_id});
    client
        .send_message()
        .queue_url(&queue_url)
        .message_body(body.to_string())
        .send()
        .await
        .map_err(|e| format!("SQS send_message failed: {}", e))?;
    Ok(())
}
