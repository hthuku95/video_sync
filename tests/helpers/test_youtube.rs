// Test YouTube helper for integration tests
// Provides utilities for YouTube API interactions during tests

use serde::Deserialize;

/// YouTube video details for verification
#[derive(Debug, Deserialize)]
pub struct YouTubeVideoDetails {
    pub id: String,
    pub snippet: VideoSnippet,
    pub status: VideoStatus,
}

#[derive(Debug, Deserialize)]
pub struct VideoSnippet {
    pub title: String,
    pub description: String,
    #[serde(rename = "channelId")]
    pub channel_id: String,
}

#[derive(Debug, Deserialize)]
pub struct VideoStatus {
    #[serde(rename = "uploadStatus")]
    pub upload_status: String,
    #[serde(rename = "privacyStatus")]
    pub privacy_status: String,
}

/// Test YouTube client for verification
pub struct TestYouTubeClient {
    access_token: String,
}

impl TestYouTubeClient {
    pub fn new(access_token: String) -> Self {
        Self { access_token }
    }

    /// Get video details from YouTube
    pub async fn get_video_details(
        &self,
        video_id: &str,
    ) -> Result<YouTubeVideoDetails, Box<dyn std::error::Error>> {
        let url = format!(
            "https://www.googleapis.com/youtube/v3/videos?id={}&part=snippet,status",
            video_id
        );

        let response = reqwest::Client::new()
            .get(&url)
            .header("Authorization", format!("Bearer {}", self.access_token))
            .send()
            .await?;

        if !response.status().is_success() {
            return Err(format!("YouTube API error: {}", response.status()).into());
        }

        let body: serde_json::Value = response.json().await?;
        let items = body["items"].as_array().ok_or("No items in response")?;

        if items.is_empty() {
            return Err("Video not found".into());
        }

        let video: YouTubeVideoDetails = serde_json::from_value(items[0].clone())?;
        Ok(video)
    }

    /// Delete a test video from YouTube (cleanup)
    pub async fn delete_video(&self, video_id: &str) -> Result<(), Box<dyn std::error::Error>> {
        let url = format!(
            "https://www.googleapis.com/youtube/v3/videos?id={}",
            video_id
        );

        let response = reqwest::Client::new()
            .delete(&url)
            .header("Authorization", format!("Bearer {}", self.access_token))
            .send()
            .await?;

        if !response.status().is_success() {
            return Err(format!("Failed to delete video: {}", response.status()).into());
        }

        Ok(())
    }

    /// Verify video is unlisted (for test safety)
    pub async fn verify_unlisted(
        &self,
        video_id: &str,
    ) -> Result<bool, Box<dyn std::error::Error>> {
        let details = self.get_video_details(video_id).await?;
        Ok(details.status.privacy_status == "unlisted")
    }
}

/// Test video URLs for integration testing
pub mod test_videos {
    /// Short public domain video for quick tests (18s - "Me at the zoo", first YouTube video, always public)
    pub const SHORT_VIDEO: &str = "https://www.youtube.com/watch?v=jNQXAC9IVRw";

    /// Medium length video for full workflow tests (3:33 - Rick Astley, reliably public)
    pub const MEDIUM_VIDEO: &str = "https://www.youtube.com/watch?v=dQw4w9WgXcQ";

    /// Long video for stress testing (10+ min)
    pub const LONG_VIDEO: &str = "https://www.youtube.com/watch?v=9bZkp7q19f0";
}

/// Assert that a video exists on YouTube with expected properties
pub async fn assert_video_uploaded(
    client: &TestYouTubeClient,
    video_id: &str,
    expected_title_contains: &str,
) -> Result<(), Box<dyn std::error::Error>> {
    let details = client.get_video_details(video_id).await?;

    assert_eq!(
        details.status.upload_status, "processed",
        "Video should be processed"
    );

    assert!(
        details.snippet.title.contains(expected_title_contains),
        "Video title should contain '{}'",
        expected_title_contains
    );

    assert!(
        details.snippet.description.contains("VideoSync"),
        "Video description should mention VideoSync"
    );

    Ok(())
}
