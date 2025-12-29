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
