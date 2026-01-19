// Performance Tracking & Learning System
// Implements Recommendation 5: Feedback Loop for Continuous Learning
// This system learns from clip performance data to optimize future clip selection

use crate::youtube_analytics_client::{YouTubeAnalyticsClient, ClipPerformanceMetrics, ShortsTrafficSources};
use chrono::{DateTime, Duration, Utc};
use serde::{Deserialize, Serialize};
use sqlx::PgPool;
use std::collections::HashMap;

pub struct PerformanceTracker {
    db_pool: PgPool,
    analytics_client: YouTubeAnalyticsClient,
}

impl PerformanceTracker {
    pub fn new(db_pool: PgPool) -> Self {
        Self {
            db_pool,
            analytics_client: YouTubeAnalyticsClient::new(),
        }
    }

    /// Main entry point: Sync analytics for all published clips
    /// Call this periodically (e.g., every 6 hours) via background job
    pub async fn sync_all_clip_analytics(&self) -> Result<AnalyticsSyncReport, String> {
        tracing::info!("🔄 Starting analytics sync for all clips");

        let start_time = std::time::Instant::now();

        // Step 1: Get all clips that need analytics updates
        let clips_to_sync = self.get_clips_needing_sync().await?;

        if clips_to_sync.is_empty() {
            tracing::info!("No clips need analytics sync");
            return Ok(AnalyticsSyncReport {
                clips_synced: 0,
                clips_failed: 0,
                duration_seconds: start_time.elapsed().as_secs(),
            });
        }

        tracing::info!("Found {} clips needing sync", clips_to_sync.len());

        // Step 2: Group clips by destination channel (for auth token)
        let clips_by_channel = self.group_clips_by_channel(&clips_to_sync).await?;

        let mut total_synced = 0;
        let mut total_failed = 0;

        // Step 3: Sync each channel's clips in batch
        for (channel_id, clips) in clips_by_channel {
            match self.sync_channel_clips(channel_id, &clips).await {
                Ok(count) => {
                    total_synced += count;
                    tracing::info!("✅ Synced {} clips for channel {}", count, channel_id);
                }
                Err(e) => {
                    total_failed += clips.len();
                    tracing::error!("Failed to sync clips for channel {}: {}", channel_id, e);
                }
            }
        }

        // Step 4: Refresh viral factor performance statistics
        if total_synced > 0 {
            if let Err(e) = self.refresh_viral_factor_stats().await {
                tracing::error!("Failed to refresh viral factor stats: {}", e);
            }

            // Step 5: Generate learning recommendations
            if let Err(e) = self.generate_learning_recommendations().await {
                tracing::error!("Failed to generate recommendations: {}", e);
            }
        }

        let duration = start_time.elapsed().as_secs();
        tracing::info!(
            "✅ Analytics sync completed: {} synced, {} failed, {} seconds",
            total_synced,
            total_failed,
            duration
        );

        Ok(AnalyticsSyncReport {
            clips_synced: total_synced,
            clips_failed: total_failed,
            duration_seconds: duration,
        })
    }

    /// Get clips that haven't been synced recently or never synced
    async fn get_clips_needing_sync(&self) -> Result<Vec<ClipToSync>, String> {
        let query = "
            SELECT
                ec.id as clip_id,
                ec.youtube_video_id,
                ec.destination_channel_id,
                ec.published_at,
                cah.data_fetched_at as last_synced_at
            FROM extracted_clips ec
            LEFT JOIN LATERAL (
                SELECT data_fetched_at
                FROM clip_analytics_history
                WHERE clip_id = ec.id
                ORDER BY data_fetched_at DESC
                LIMIT 1
            ) cah ON true
            WHERE ec.upload_status = 'published'
              AND ec.youtube_video_id IS NOT NULL
              AND ec.destination_channel_id IS NOT NULL
              AND (
                  -- Never synced
                  cah.data_fetched_at IS NULL
                  -- Or synced more than 6 hours ago
                  OR cah.data_fetched_at < NOW() - INTERVAL '6 hours'
              )
            ORDER BY ec.published_at DESC
            LIMIT 100
        ";

        sqlx::query_as::<_, ClipToSync>(query)
            .fetch_all(&self.db_pool)
            .await
            .map_err(|e| format!("Database error fetching clips: {}", e))
    }

    /// Group clips by destination channel for batch processing
    async fn group_clips_by_channel(
        &self,
        clips: &[ClipToSync],
    ) -> Result<HashMap<i32, Vec<ClipToSync>>, String> {
        let mut grouped: HashMap<i32, Vec<ClipToSync>> = HashMap::new();

        for clip in clips {
            grouped
                .entry(clip.destination_channel_id)
                .or_insert_with(Vec::new)
                .push(clip.clone());
        }

        Ok(grouped)
    }

    /// Sync analytics for all clips from a specific channel
    async fn sync_channel_clips(
        &self,
        channel_id: i32,
        clips: &[ClipToSync],
    ) -> Result<usize, String> {
        // Get channel OAuth token
        let access_token = self.get_channel_access_token(channel_id).await?;

        // Batch fetch analytics (much more efficient than individual calls)
        let video_ids: Vec<String> = clips
            .iter()
            .map(|c| c.youtube_video_id.clone())
            .collect();

        // Calculate date range (from earliest publish date to today)
        let start_date = clips
            .iter()
            .map(|c| c.published_at)
            .min()
            .unwrap_or_else(Utc::now)
            .format("%Y-%m-%d")
            .to_string();

        let end_date = Utc::now().format("%Y-%m-%d").to_string();

        // Fetch batch analytics
        let analytics_results = self
            .analytics_client
            .get_batch_clip_analytics(&access_token, &video_ids, &start_date, &end_date)
            .await
            .map_err(|e| format!("Analytics API error: {}", e))?;

        // Store results in database
        let mut synced_count = 0;
        for clip in clips {
            // Find analytics for this clip
            if let Some(analytics) = analytics_results
                .iter()
                .find(|a| a.video_id == clip.youtube_video_id)
            {
                // Fetch traffic sources separately (not available in batch)
                let traffic_sources = self
                    .analytics_client
                    .get_shorts_traffic_sources(
                        &access_token,
                        &clip.youtube_video_id,
                        &start_date,
                        &end_date,
                    )
                    .await
                    .unwrap_or_default();

                // Store in database
                if let Ok(_) = self
                    .store_clip_analytics(clip.clip_id, analytics, &traffic_sources)
                    .await
                {
                    // Update extracted_clips table with latest metrics
                    self.update_clip_metrics(clip.clip_id, analytics).await?;
                    synced_count += 1;
                }
            }
        }

        Ok(synced_count)
    }

    /// Get OAuth access token for a channel
    async fn get_channel_access_token(&self, channel_id: i32) -> Result<String, String> {
        let query = "SELECT access_token FROM connected_youtube_channels WHERE id = $1";

        sqlx::query_scalar::<_, String>(query)
            .bind(channel_id)
            .fetch_one(&self.db_pool)
            .await
            .map_err(|e| format!("Failed to get access token: {}", e))
    }

    /// Store analytics data in clip_analytics_history table
    async fn store_clip_analytics(
        &self,
        clip_id: i32,
        analytics: &ClipPerformanceMetrics,
        traffic_sources: &ShortsTrafficSources,
    ) -> Result<(), String> {
        // Calculate hours since published
        let clip_age_hours = sqlx::query_scalar::<_, i32>(
            "SELECT EXTRACT(EPOCH FROM (NOW() - published_at)) / 3600 FROM extracted_clips WHERE id = $1"
        )
        .bind(clip_id)
        .fetch_one(&self.db_pool)
        .await
        .unwrap_or(0);

        let query = "
            INSERT INTO clip_analytics_history (
                clip_id,
                youtube_video_id,
                views,
                likes,
                dislikes,
                comments,
                shares,
                like_rate,
                comment_rate,
                avg_watch_percentage,
                traffic_source_browse_features,
                traffic_source_suggested_videos,
                traffic_source_shorts_feed,
                traffic_source_external,
                hours_since_published
            ) VALUES ($1, $2, $3, $4, $5, $6, $7, $8, $9, $10, $11, $12, $13, $14, $15)
        ";

        sqlx::query(query)
            .bind(clip_id)
            .bind(&analytics.video_id)
            .bind(analytics.views)
            .bind(analytics.likes)
            .bind(analytics.dislikes)
            .bind(analytics.comments)
            .bind(analytics.shares)
            .bind(analytics.like_rate)
            .bind(analytics.comment_rate)
            .bind(analytics.avg_watch_percentage)
            .bind(traffic_sources.browse_features_pct)
            .bind(traffic_sources.suggested_videos_pct)
            .bind(traffic_sources.shorts_feed_pct)
            .bind(traffic_sources.external_pct)
            .bind(clip_age_hours)
            .execute(&self.db_pool)
            .await
            .map_err(|e| format!("Failed to store analytics: {}", e))?;

        Ok(())
    }

    /// Update extracted_clips table with latest metrics
    async fn update_clip_metrics(
        &self,
        clip_id: i32,
        analytics: &ClipPerformanceMetrics,
    ) -> Result<(), String> {
        let query = "
            UPDATE extracted_clips
            SET views_24h = $1, likes_24h = $2, comments_24h = $3, updated_at = NOW()
            WHERE id = $4
        ";

        sqlx::query(query)
            .bind(analytics.views)
            .bind(analytics.likes)
            .bind(analytics.comments)
            .bind(clip_id)
            .execute(&self.db_pool)
            .await
            .map_err(|e| format!("Failed to update clip metrics: {}", e))?;

        Ok(())
    }

    /// Refresh viral factor performance statistics (calls SQL function)
    async fn refresh_viral_factor_stats(&self) -> Result<(), String> {
        tracing::info!("📊 Refreshing viral factor performance statistics");

        sqlx::query("SELECT refresh_viral_factor_performance()")
            .execute(&self.db_pool)
            .await
            .map_err(|e| format!("Failed to refresh stats: {}", e))?;

        tracing::info!("✅ Viral factor statistics refreshed");
        Ok(())
    }

    /// Generate AI-powered learning recommendations based on performance data
    async fn generate_learning_recommendations(&self) -> Result<(), String> {
        tracing::info!("🤖 Generating learning recommendations");

        // Get top-performing viral factors
        let top_factors = self.get_top_viral_factors(10).await?;

        // Get poor-performing factors
        let poor_factors = self.get_poor_viral_factors(5).await?;

        // Get optimal duration range
        let optimal_duration = self.get_optimal_clip_duration().await?;

        // Generate recommendations
        let mut recommendations = Vec::new();

        // Recommendation 1: Focus on top-performing viral factors
        if !top_factors.is_empty() {
            let top_list = top_factors
                .iter()
                .map(|vf| format!("{} ({:.0} avg views)", vf.viral_factor, vf.avg_views))
                .collect::<Vec<_>>()
                .join(", ");

            let supporting_data = serde_json::json!({
                "top_factors": top_factors,
            });

            recommendations.push(LearningRecommendation {
                recommendation_type: "viral_factor".to_string(),
                recommendation: format!(
                    "Prioritize these high-performing viral factors: {}. \
                     These consistently drive {}% more views than average.",
                    top_list,
                    ((top_factors[0].avg_views / 10000.0) * 100.0) as i32
                ),
                confidence: 0.90,
                supporting_data,
            });
        }

        // Recommendation 2: Avoid poor-performing factors
        if !poor_factors.is_empty() {
            let poor_list = poor_factors
                .iter()
                .map(|vf| vf.viral_factor.clone())
                .collect::<Vec<_>>()
                .join(", ");

            recommendations.push(LearningRecommendation {
                recommendation_type: "viral_factor".to_string(),
                recommendation: format!(
                    "Reduce usage of these underperforming factors: {}. \
                     Consider replacing with higher-performing alternatives.",
                    poor_list
                ),
                confidence: 0.75,
                supporting_data: serde_json::json!({"poor_factors": poor_factors}),
            });
        }

        // Recommendation 3: Optimal duration
        if let Some(duration_data) = optimal_duration {
            recommendations.push(LearningRecommendation {
                recommendation_type: "duration".to_string(),
                recommendation: format!(
                    "Optimal clip duration is {}-{} seconds (avg {} views vs {} views for other durations). \
                     Target this range for maximum engagement.",
                    duration_data.duration_min,
                    duration_data.duration_max,
                    duration_data.avg_views as i32,
                    (duration_data.avg_views * 0.6) as i32
                ),
                confidence: 0.85,
                supporting_data: serde_json::json!({"optimal_duration": duration_data}),
            });
        }

        // Store recommendations in database
        let recommendations_count = recommendations.len();
        for rec in recommendations {
            self.store_recommendation(&rec).await?;
        }

        tracing::info!("✅ Generated {} learning recommendations", recommendations_count);
        Ok(())
    }

    /// Get top-performing viral factors
    async fn get_top_viral_factors(&self, limit: i32) -> Result<Vec<ViralFactorStats>, String> {
        let query = "
            SELECT viral_factor, avg_views, avg_like_rate, avg_comment_rate, performance_score
            FROM viral_factor_performance
            WHERE total_clips >= 3
            ORDER BY performance_score DESC
            LIMIT $1
        ";

        sqlx::query_as::<_, ViralFactorStats>(query)
            .bind(limit)
            .fetch_all(&self.db_pool)
            .await
            .map_err(|e| format!("Failed to fetch top factors: {}", e))
    }

    /// Get poor-performing viral factors
    async fn get_poor_viral_factors(&self, limit: i32) -> Result<Vec<ViralFactorStats>, String> {
        let query = "
            SELECT viral_factor, avg_views, avg_like_rate, avg_comment_rate, performance_score
            FROM viral_factor_performance
            WHERE total_clips >= 3
            ORDER BY performance_score ASC
            LIMIT $1
        ";

        sqlx::query_as::<_, ViralFactorStats>(query)
            .bind(limit)
            .fetch_all(&self.db_pool)
            .await
            .map_err(|e| format!("Failed to fetch poor factors: {}", e))
    }

    /// Get optimal clip duration based on performance data
    async fn get_optimal_clip_duration(&self) -> Result<Option<DurationStats>, String> {
        let query = "
            SELECT duration_bucket, duration_min, duration_max, avg_views, performance_score
            FROM duration_performance_analysis
            WHERE total_clips >= 5
            ORDER BY performance_score DESC
            LIMIT 1
        ";

        sqlx::query_as::<_, DurationStats>(query)
            .fetch_optional(&self.db_pool)
            .await
            .map_err(|e| format!("Failed to fetch duration stats: {}", e))
    }

    /// Store a learning recommendation
    async fn store_recommendation(&self, rec: &LearningRecommendation) -> Result<(), String> {
        let query = "
            INSERT INTO learning_recommendations (
                recommendation_type,
                recommendation,
                confidence,
                supporting_data,
                is_active
            ) VALUES ($1, $2, $3, $4, true)
            ON CONFLICT (recommendation_type) DO UPDATE
            SET recommendation = EXCLUDED.recommendation,
                confidence = EXCLUDED.confidence,
                supporting_data = EXCLUDED.supporting_data,
                updated_at = NOW()
        ";

        sqlx::query(query)
            .bind(&rec.recommendation_type)
            .bind(&rec.recommendation)
            .bind(rec.confidence)
            .bind(&rec.supporting_data)
            .execute(&self.db_pool)
            .await
            .map_err(|e| format!("Failed to store recommendation: {}", e))?;

        Ok(())
    }

    /// Get learned recommendations for clip selection
    /// This is called by the AI clipper to optimize clip selection
    pub async fn get_optimized_viral_factors(&self) -> Result<Vec<String>, String> {
        let query = "
            SELECT viral_factor
            FROM viral_factor_performance
            WHERE total_clips >= 3
            ORDER BY performance_score DESC
            LIMIT 10
        ";

        sqlx::query_scalar::<_, String>(query)
            .fetch_all(&self.db_pool)
            .await
            .map_err(|e| format!("Failed to fetch optimized factors: {}", e))
    }

    /// Get optimal clip duration range
    pub async fn get_optimal_duration_range(&self) -> Result<(i32, i32), String> {
        let query = "
            SELECT duration_min, duration_max
            FROM duration_performance_analysis
            WHERE total_clips >= 5
            ORDER BY performance_score DESC
            LIMIT 1
        ";

        sqlx::query_as::<_, (i32, i32)>(query)
            .fetch_optional(&self.db_pool)
            .await
            .map_err(|e| format!("Failed to fetch duration range: {}", e))?
            .ok_or_else(|| "No duration data available yet".to_string())
    }

    /// Get performance score for a specific viral factor
    /// Used during clip selection to prioritize high-performing factors
    pub async fn get_viral_factor_score(&self, factor: &str) -> Result<f64, String> {
        let query = "
            SELECT performance_score
            FROM viral_factor_performance
            WHERE viral_factor = $1
        ";

        sqlx::query_scalar::<_, f64>(query)
            .bind(factor)
            .fetch_optional(&self.db_pool)
            .await
            .map_err(|e| format!("Failed to fetch factor score: {}", e))?
            .ok_or_else(|| format!("No score data for factor: {}", factor))
    }
}

// Supporting structs

#[derive(Debug, Clone, sqlx::FromRow)]
struct ClipToSync {
    clip_id: i32,
    youtube_video_id: String,
    destination_channel_id: i32,
    published_at: DateTime<Utc>,
    last_synced_at: Option<DateTime<Utc>>,
}

#[derive(Debug, Clone, Serialize, Deserialize, sqlx::FromRow)]
struct ViralFactorStats {
    viral_factor: String,
    avg_views: f64,
    avg_like_rate: f64,
    avg_comment_rate: f64,
    performance_score: f64,
}

#[derive(Debug, Clone, Serialize, Deserialize, sqlx::FromRow)]
struct DurationStats {
    duration_bucket: String,
    duration_min: i32,
    duration_max: i32,
    avg_views: f64,
    performance_score: f64,
}

#[derive(Debug, Clone)]
struct LearningRecommendation {
    recommendation_type: String,
    recommendation: String,
    confidence: f64,
    supporting_data: serde_json::Value,
}

#[derive(Debug, Serialize)]
pub struct AnalyticsSyncReport {
    pub clips_synced: usize,
    pub clips_failed: usize,
    pub duration_seconds: u64,
}
