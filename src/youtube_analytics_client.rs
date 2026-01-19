use reqwest::Client;
use serde::{Deserialize, Serialize};
use serde_json::json;

/// YouTube Analytics API client for fetching video and channel analytics data
///
/// Note: This is SEPARATE from YouTube Data API v3
/// Base URL: https://youtubeanalytics.googleapis.com/v2/reports
/// Required OAuth Scope: https://www.googleapis.com/auth/yt-analytics.readonly
#[derive(Debug, Clone)]
pub struct YouTubeAnalyticsClient {
    client: Client,
}

impl YouTubeAnalyticsClient {
    pub fn new() -> Self {
        Self {
            client: Client::new(),
        }
    }

    /// Fetch video analytics for a specific date range
    ///
    /// Metrics: views, estimatedMinutesWatched, averageViewDuration, averageViewPercentage,
    ///          likes, dislikes, comments, shares, subscribersGained, subscribersLost
    ///
    /// # Arguments
    /// * `access_token` - OAuth access token with yt-analytics.readonly scope
    /// * `video_id` - YouTube video ID
    /// * `start_date` - Start date in YYYY-MM-DD format
    /// * `end_date` - End date in YYYY-MM-DD format
    pub async fn get_video_analytics(
        &self,
        access_token: &str,
        video_id: &str,
        start_date: &str,
        end_date: &str,
    ) -> Result<VideoAnalyticsApiResponse, Box<dyn std::error::Error + Send + Sync>> {
        let url = "https://youtubeanalytics.googleapis.com/v2/reports";

        let metrics = "views,estimatedMinutesWatched,averageViewDuration,averageViewPercentage,\
                      likes,dislikes,comments,shares,subscribersGained,subscribersLost";

        let response = self
            .client
            .get(url)
            .query(&[
                ("ids", format!("channel==MINE")),
                ("startDate", start_date.to_string()),
                ("endDate", end_date.to_string()),
                ("metrics", metrics.to_string()),
                ("dimensions", "video".to_string()),
                ("filters", format!("video=={}", video_id)),
                ("sort", "-estimatedMinutesWatched".to_string()),
            ])
            .header("Authorization", format!("Bearer {}", access_token))
            .send()
            .await?;

        if !response.status().is_success() {
            let error_text = response.text().await?;
            tracing::error!("YouTube Analytics API error for video {}: {}", video_id, error_text);
            return Err(format!("Failed to fetch video analytics: {}", error_text).into());
        }

        let analytics: VideoAnalyticsApiResponse = response.json().await?;

        // Log view count if available
        if let Some(row) = analytics.rows.first() {
            if let Some(views) = row.first().and_then(|v| v.as_i64()) {
                tracing::debug!("Fetched analytics for video {}: {} views", video_id, views);
            }
        }

        Ok(analytics)
    }

    /// Fetch channel-level analytics with demographics and traffic sources
    ///
    /// # Arguments
    /// * `access_token` - OAuth access token
    /// * `start_date` - Start date in YYYY-MM-DD format
    /// * `end_date` - End date in YYYY-MM-DD format
    pub async fn get_channel_analytics(
        &self,
        access_token: &str,
        start_date: &str,
        end_date: &str,
    ) -> Result<ChannelAnalyticsApiResponse, Box<dyn std::error::Error + Send + Sync>> {
        let url = "https://youtubeanalytics.googleapis.com/v2/reports";

        let metrics = "views,estimatedMinutesWatched,subscribersGained,subscribersLost";

        let response = self
            .client
            .get(url)
            .query(&[
                ("ids", "channel==MINE".to_string()),
                ("startDate", start_date.to_string()),
                ("endDate", end_date.to_string()),
                ("metrics", metrics.to_string()),
            ])
            .header("Authorization", format!("Bearer {}", access_token))
            .send()
            .await?;

        if !response.status().is_success() {
            let error_text = response.text().await?;
            tracing::error!("YouTube Analytics API error for channel: {}", error_text);
            return Err(format!("Failed to fetch channel analytics: {}", error_text).into());
        }

        let analytics: ChannelAnalyticsApiResponse = response.json().await?;
        Ok(analytics)
    }

    /// Fetch demographic data (age groups, gender, geography)
    ///
    /// Returns breakdown by: ageGroup (13-17, 18-24, 25-34, 35-44, 45-54, 55-64, 65+)
    ///                       gender (female, male)
    ///                       country (US, GB, etc.)
    pub async fn get_channel_demographics(
        &self,
        access_token: &str,
        start_date: &str,
        end_date: &str,
    ) -> Result<DemographicsApiResponse, Box<dyn std::error::Error + Send + Sync>> {
        // Age and Gender Demographics
        let age_gender_data = self.fetch_demographics_dimension(
            access_token,
            start_date,
            end_date,
            "ageGroup,gender",
        ).await?;

        // Geographic Demographics
        let geography_data = self.fetch_demographics_dimension(
            access_token,
            start_date,
            end_date,
            "country",
        ).await?;

        Ok(DemographicsApiResponse {
            age_gender: age_gender_data,
            geography: geography_data,
        })
    }

    /// Fetch traffic source breakdown
    ///
    /// Shows how viewers find your content:
    /// - ADVERTISING: From YouTube ads
    /// - ANNOTATION: From video annotations
    /// - EXT_URL: External websites
    /// - NO_LINK_EMBEDDED: Embedded players without link
    /// - NO_LINK_OTHER: Other unattributed sources
    /// - PROMOTED: Promoted content
    /// - RELATED_VIDEO: Suggested/related videos
    /// - SUBSCRIBER: Subscriber feeds
    /// - YT_CHANNEL: Channel page
    /// - YT_OTHER_PAGE: Other YouTube pages
    /// - YT_SEARCH: YouTube search results
    pub async fn get_traffic_sources(
        &self,
        access_token: &str,
        start_date: &str,
        end_date: &str,
    ) -> Result<TrafficSourcesApiResponse, Box<dyn std::error::Error + Send + Sync>> {
        let url = "https://youtubeanalytics.googleapis.com/v2/reports";

        let response = self
            .client
            .get(url)
            .query(&[
                ("ids", "channel==MINE".to_string()),
                ("startDate", start_date.to_string()),
                ("endDate", end_date.to_string()),
                ("metrics", "views".to_string()),
                ("dimensions", "insightTrafficSourceType".to_string()),
                ("sort", "-views".to_string()),
            ])
            .header("Authorization", format!("Bearer {}", access_token))
            .send()
            .await?;

        if !response.status().is_success() {
            let error_text = response.text().await?;
            return Err(format!("Failed to fetch traffic sources: {}", error_text).into());
        }

        let data: TrafficSourcesApiResponse = response.json().await?;
        Ok(data)
    }

    /// Fetch device type breakdown (mobile, desktop, TV, tablet)
    pub async fn get_device_types(
        &self,
        access_token: &str,
        start_date: &str,
        end_date: &str,
    ) -> Result<DeviceTypesApiResponse, Box<dyn std::error::Error + Send + Sync>> {
        let url = "https://youtubeanalytics.googleapis.com/v2/reports";

        let response = self
            .client
            .get(url)
            .query(&[
                ("ids", "channel==MINE".to_string()),
                ("startDate", start_date.to_string()),
                ("endDate", end_date.to_string()),
                ("metrics", "views".to_string()),
                ("dimensions", "deviceType".to_string()),
                ("sort", "-views".to_string()),
            ])
            .header("Authorization", format!("Bearer {}", access_token))
            .send()
            .await?;

        if !response.status().is_success() {
            let error_text = response.text().await?;
            return Err(format!("Failed to fetch device types: {}", error_text).into());
        }

        let data: DeviceTypesApiResponse = response.json().await?;
        Ok(data)
    }

    /// Helper method to fetch demographic dimensions
    async fn fetch_demographics_dimension(
        &self,
        access_token: &str,
        start_date: &str,
        end_date: &str,
        dimensions: &str,
    ) -> Result<AnalyticsApiResponse, Box<dyn std::error::Error + Send + Sync>> {
        let url = "https://youtubeanalytics.googleapis.com/v2/reports";

        let response = self
            .client
            .get(url)
            .query(&[
                ("ids", "channel==MINE".to_string()),
                ("startDate", start_date.to_string()),
                ("endDate", end_date.to_string()),
                ("metrics", "viewerPercentage".to_string()),
                ("dimensions", dimensions.to_string()),
                ("sort", "-viewerPercentage".to_string()),
            ])
            .header("Authorization", format!("Bearer {}", access_token))
            .send()
            .await?;

        if !response.status().is_success() {
            let error_text = response.text().await?;
            return Err(format!("Failed to fetch demographics dimension {}: {}", dimensions, error_text).into());
        }

        let data: AnalyticsApiResponse = response.json().await?;
        Ok(data)
    }

    /// Batch fetch analytics for multiple clips (OPTIMIZATION for performance tracking)
    /// This is much more efficient than individual calls for each clip
    pub async fn get_batch_clip_analytics(
        &self,
        access_token: &str,
        video_ids: &[String],
        start_date: &str,
        end_date: &str,
    ) -> Result<Vec<ClipPerformanceMetrics>, Box<dyn std::error::Error + Send + Sync>> {
        if video_ids.is_empty() {
            return Ok(Vec::new());
        }

        tracing::info!("Fetching batch analytics for {} clips", video_ids.len());

        let url = "https://youtubeanalytics.googleapis.com/v2/reports";
        let video_filter = format!("video=={}", video_ids.join(","));

        let response = self
            .client
            .get(url)
            .query(&[
                ("ids", "channel==MINE".to_string()),
                ("startDate", start_date.to_string()),
                ("endDate", end_date.to_string()),
                ("metrics", "views,likes,dislikes,comments,shares,averageViewPercentage".to_string()),
                ("dimensions", "video".to_string()),
                ("filters", video_filter),
            ])
            .header("Authorization", format!("Bearer {}", access_token))
            .send()
            .await?;

        if !response.status().is_success() {
            let error_text = response.text().await?;
            return Err(format!("Batch analytics failed: {}", error_text).into());
        }

        let analytics_response: AnalyticsApiResponse = response.json().await?;

        // Parse each row as a separate clip's metrics
        let mut results = Vec::new();
        for row in analytics_response.rows {
            let video_id = row.get(0).and_then(|v| v.as_str()).unwrap_or("unknown").to_string();
            let views = row.get(1).and_then(|v| v.as_i64()).unwrap_or(0) as i32;
            let likes = row.get(2).and_then(|v| v.as_i64()).unwrap_or(0) as i32;
            let dislikes = row.get(3).and_then(|v| v.as_i64()).unwrap_or(0) as i32;
            let comments = row.get(4).and_then(|v| v.as_i64()).unwrap_or(0) as i32;
            let shares = row.get(5).and_then(|v| v.as_i64()).unwrap_or(0) as i32;
            let avg_watch_pct = row.get(6).and_then(|v| v.as_f64()).unwrap_or(0.0);

            // Calculate engagement rates
            let like_rate = if views > 0 { likes as f64 / views as f64 } else { 0.0 };
            let comment_rate = if views > 0 { comments as f64 / views as f64 } else { 0.0 };

            results.push(ClipPerformanceMetrics {
                video_id,
                views,
                likes,
                dislikes,
                comments,
                shares,
                like_rate,
                comment_rate,
                avg_watch_percentage: avg_watch_pct,
            });
        }

        Ok(results)
    }

    /// Get traffic sources specifically for a Shorts clip
    /// Optimized for understanding Shorts performance
    pub async fn get_shorts_traffic_sources(
        &self,
        access_token: &str,
        video_id: &str,
        start_date: &str,
        end_date: &str,
    ) -> Result<ShortsTrafficSources, Box<dyn std::error::Error + Send + Sync>> {
        let url = "https://youtubeanalytics.googleapis.com/v2/reports";

        let response = self
            .client
            .get(url)
            .query(&[
                ("ids", "channel==MINE".to_string()),
                ("startDate", start_date.to_string()),
                ("endDate", end_date.to_string()),
                ("metrics", "views".to_string()),
                ("dimensions", "insightTrafficSourceType".to_string()),
                ("filters", format!("video=={}", video_id)),
                ("sort", "-views".to_string()),
            ])
            .header("Authorization", format!("Bearer {}", access_token))
            .send()
            .await?;

        if !response.status().is_success() {
            tracing::warn!("Traffic sources not available for video {}", video_id);
            return Ok(ShortsTrafficSources::default());
        }

        let data: TrafficSourcesApiResponse = response.json().await?;

        // Parse into our specialized structure
        let mut sources = std::collections::HashMap::new();
        let mut total_views = 0i32;

        for row in data.rows {
            let source_type = row.get(0).and_then(|v| v.as_str()).unwrap_or("UNKNOWN");
            let views = row.get(1).and_then(|v| v.as_i64()).unwrap_or(0) as i32;
            sources.insert(source_type.to_string(), views);
            total_views += views;
        }

        // Calculate percentages
        let calc_pct = |views: i32| -> f64 {
            if total_views > 0 {
                (views as f64 / total_views as f64) * 100.0
            } else {
                0.0
            }
        };

        Ok(ShortsTrafficSources {
            shorts_feed_pct: calc_pct(*sources.get("SHORTS").unwrap_or(&0)),
            suggested_videos_pct: calc_pct(*sources.get("RELATED_VIDEO").unwrap_or(&0)),
            browse_features_pct: calc_pct(*sources.get("BROWSE").unwrap_or(&0)),
            search_pct: calc_pct(*sources.get("YT_SEARCH").unwrap_or(&0)),
            external_pct: calc_pct(*sources.get("EXT_URL").unwrap_or(&0)),
        })
    }
}

// ============================================================================
// API Response Structures
// ============================================================================

#[derive(Debug, Serialize, Deserialize)]
pub struct VideoAnalyticsApiResponse {
    pub kind: String,
    #[serde(rename = "columnHeaders")]
    pub column_headers: Vec<ColumnHeader>,
    pub rows: Vec<Vec<serde_json::Value>>,
}

#[derive(Debug, Serialize, Deserialize)]
pub struct ChannelAnalyticsApiResponse {
    pub kind: String,
    #[serde(rename = "columnHeaders")]
    pub column_headers: Vec<ColumnHeader>,
    pub rows: Vec<Vec<serde_json::Value>>,
}

#[derive(Debug, Serialize, Deserialize)]
pub struct AnalyticsApiResponse {
    pub kind: String,
    #[serde(rename = "columnHeaders")]
    pub column_headers: Vec<ColumnHeader>,
    pub rows: Vec<Vec<serde_json::Value>>,
}

#[derive(Debug, Serialize, Deserialize)]
pub struct DemographicsApiResponse {
    pub age_gender: AnalyticsApiResponse,
    pub geography: AnalyticsApiResponse,
}

#[derive(Debug, Serialize, Deserialize)]
pub struct TrafficSourcesApiResponse {
    pub kind: String,
    #[serde(rename = "columnHeaders")]
    pub column_headers: Vec<ColumnHeader>,
    pub rows: Vec<Vec<serde_json::Value>>,
}

#[derive(Debug, Serialize, Deserialize)]
pub struct DeviceTypesApiResponse {
    pub kind: String,
    #[serde(rename = "columnHeaders")]
    pub column_headers: Vec<ColumnHeader>,
    pub rows: Vec<Vec<serde_json::Value>>,
}

#[derive(Debug, Serialize, Deserialize)]
pub struct ColumnHeader {
    pub name: String,
    #[serde(rename = "columnType")]
    pub column_type: String,
    #[serde(rename = "dataType")]
    pub data_type: String,
}

// ============================================================================
// Helper Functions for Parsing Analytics Responses
// ============================================================================

impl VideoAnalyticsApiResponse {
    /// Parse the raw API response into structured metrics
    pub fn to_metrics(&self) -> Option<ParsedVideoMetrics> {
        let row = self.rows.first()?;

        // Column order from API:
        // [0] views, [1] estimatedMinutesWatched, [2] averageViewDuration,
        // [3] averageViewPercentage, [4] likes, [5] dislikes, [6] comments,
        // [7] shares, [8] subscribersGained, [9] subscribersLost

        Some(ParsedVideoMetrics {
            views: row.get(0).and_then(|v| v.as_i64()).unwrap_or(0),
            watch_time_minutes: row.get(1).and_then(|v| v.as_i64()).unwrap_or(0),
            average_view_duration: row.get(2).and_then(|v| v.as_i64()).unwrap_or(0) as i32,
            average_view_percentage: row.get(3).and_then(|v| v.as_f64()).unwrap_or(0.0),
            likes: row.get(4).and_then(|v| v.as_i64()).unwrap_or(0) as i32,
            dislikes: row.get(5).and_then(|v| v.as_i64()).unwrap_or(0) as i32,
            comments: row.get(6).and_then(|v| v.as_i64()).unwrap_or(0) as i32,
            shares: row.get(7).and_then(|v| v.as_i64()).unwrap_or(0) as i32,
            subscribers_gained: row.get(8).and_then(|v| v.as_i64()).unwrap_or(0) as i32,
            subscribers_lost: row.get(9).and_then(|v| v.as_i64()).unwrap_or(0) as i32,
        })
    }
}

impl ChannelAnalyticsApiResponse {
    /// Parse channel analytics response
    pub fn to_metrics(&self) -> Option<ParsedChannelMetrics> {
        let row = self.rows.first()?;

        // Column order: [0] views, [1] estimatedMinutesWatched,
        //               [2] subscribersGained, [3] subscribersLost

        Some(ParsedChannelMetrics {
            views: row.get(0).and_then(|v| v.as_i64()).unwrap_or(0),
            watch_time_minutes: row.get(1).and_then(|v| v.as_i64()).unwrap_or(0),
            subscribers_gained: row.get(2).and_then(|v| v.as_i64()).unwrap_or(0) as i32,
            subscribers_lost: row.get(3).and_then(|v| v.as_i64()).unwrap_or(0) as i32,
        })
    }
}

#[derive(Debug, Serialize, Deserialize)]
pub struct ParsedVideoMetrics {
    pub views: i64,
    pub watch_time_minutes: i64,
    pub average_view_duration: i32,
    pub average_view_percentage: f64,
    pub likes: i32,
    pub dislikes: i32,
    pub comments: i32,
    pub shares: i32,
    pub subscribers_gained: i32,
    pub subscribers_lost: i32,
}

#[derive(Debug, Serialize, Deserialize)]
pub struct ParsedChannelMetrics {
    pub views: i64,
    pub watch_time_minutes: i64,
    pub subscribers_gained: i32,
    pub subscribers_lost: i32,
}

// ============================================================================
// Performance Tracking Structures (for learning system)
// ============================================================================

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct ClipPerformanceMetrics {
    pub video_id: String,
    pub views: i32,
    pub likes: i32,
    pub dislikes: i32,
    pub comments: i32,
    pub shares: i32,
    pub like_rate: f64,
    pub comment_rate: f64,
    pub avg_watch_percentage: f64,
}

#[derive(Debug, Clone, Serialize, Deserialize, Default)]
pub struct ShortsTrafficSources {
    pub shorts_feed_pct: f64,      // % from Shorts feed (most important for Shorts)
    pub suggested_videos_pct: f64,  // % from suggested/related
    pub browse_features_pct: f64,   // % from browse/homepage
    pub search_pct: f64,            // % from search
    pub external_pct: f64,          // % from external sites
}
