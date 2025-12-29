// src/pexels_client.rs
use reqwest::Client;
use serde::{Deserialize, Serialize};
use std::collections::HashMap;
use tokio::fs;
use tokio::io::AsyncWriteExt;
use tracing::{error, info};

#[derive(Debug, Clone)]
pub struct PexelsClient {
    client: Client,
    api_key: String,
    base_url: String,
}

#[derive(Debug, Deserialize, Serialize)]
pub struct PexelsVideoResponse {
    pub page: i32,
    pub per_page: i32,
    pub total_results: i32,
    pub videos: Vec<PexelsVideo>,
}

#[derive(Debug, Deserialize, Serialize, Clone)]
pub struct PexelsVideo {
    pub id: i64,
    pub width: i32,
    pub height: i32,
    pub duration: i32,
    pub user: PexelsUser,
    pub video_files: Vec<PexelsVideoFile>,
    pub video_pictures: Vec<PexelsVideoPicture>,
}

#[derive(Debug, Deserialize, Serialize, Clone)]
pub struct PexelsVideoFile {
    pub id: i64,
    pub quality: String,
    pub file_type: String,
    pub width: Option<i32>,
    pub height: Option<i32>,
    pub fps: Option<f64>,
    pub link: String,
}

#[derive(Debug, Deserialize, Serialize, Clone)]
pub struct PexelsVideoPicture {
    pub id: i64,
    pub picture: String,
    pub nr: i32,
}

#[derive(Debug, Deserialize, Serialize, Clone)]
pub struct PexelsUser {
    pub id: i64,
    pub name: String,
    pub url: String,
}

#[derive(Debug, Deserialize, Serialize)]
pub struct PexelsPhotoResponse {
    pub page: i32,
    pub per_page: i32,
    pub total_results: i32,
    pub photos: Vec<PexelsPhoto>,
}

#[derive(Debug, Deserialize, Serialize, Clone)]
pub struct PexelsPhoto {
    pub id: i64,
    pub width: i32,
    pub height: i32,
    pub url: String,
    pub photographer: String,
    pub photographer_url: String,
    pub photographer_id: i64,
    pub avg_color: String,
    pub src: PexelsPhotoSrc,
    pub liked: bool,
    pub alt: String,
}

#[derive(Debug, Deserialize, Serialize, Clone)]
pub struct PexelsPhotoSrc {
    pub original: String,
    pub large2x: String,
    pub large: String,
    pub medium: String,
    pub small: String,
    pub portrait: String,
    pub landscape: String,
    pub tiny: String,
}

impl PexelsClient {
    pub fn new(api_key: String) -> Self {
        Self {
            client: Client::new(),
            api_key,
            base_url: "https://api.pexels.com".to_string(),
        }
    }

    /// Search for videos on Pexels
    pub async fn search_videos(
        &self,
        query: &str,
        per_page: Option<i32>,
        page: Option<i32>,
        min_width: Option<i32>,
        min_height: Option<i32>,
        min_duration: Option<i32>,
        max_duration: Option<i32>,
    ) -> Result<PexelsVideoResponse, Box<dyn std::error::Error + Send + Sync>> {
        let mut params = HashMap::new();
        params.insert("query", query.to_string());
        
        if let Some(pp) = per_page {
            params.insert("per_page", pp.to_string());
        }
        if let Some(p) = page {
            params.insert("page", p.to_string());
        }
        if let Some(mw) = min_width {
            params.insert("min_width", mw.to_string());
        }
        if let Some(mh) = min_height {
            params.insert("min_height", mh.to_string());
        }
        if let Some(mind) = min_duration {
            params.insert("min_duration", mind.to_string());
        }
        if let Some(maxd) = max_duration {
            params.insert("max_duration", maxd.to_string());
        }

        info!("🎬 Searching Pexels for videos: '{}' with {} results per page", query, per_page.unwrap_or(15));

        let response = self.client
            .get(&format!("{}/videos/search", self.base_url))
            .header("Authorization", &self.api_key)
            .query(&params)
            .send()
            .await?;

        if !response.status().is_success() {
            let error_text = response.text().await?;
            error!("Pexels API error: {}", error_text);
            return Err(format!("Pexels API error: {}", error_text).into());
        }

        let videos = response.json::<PexelsVideoResponse>().await?;
        info!("✅ Found {} videos for query: '{}'", videos.videos.len(), query);
        
        Ok(videos)
    }

    /// Search for photos on Pexels
    pub async fn search_photos(
        &self,
        query: &str,
        per_page: Option<i32>,
        page: Option<i32>,
        size: Option<&str>, // "large", "medium", "small"
        color: Option<&str>, // "red", "orange", "yellow", etc.
        orientation: Option<&str>, // "landscape", "portrait", "square"
    ) -> Result<PexelsPhotoResponse, Box<dyn std::error::Error + Send + Sync>> {
        let mut params = HashMap::new();
        params.insert("query", query.to_string());
        
        if let Some(pp) = per_page {
            params.insert("per_page", pp.to_string());
        }
        if let Some(p) = page {
            params.insert("page", p.to_string());
        }
        if let Some(s) = size {
            params.insert("size", s.to_string());
        }
        if let Some(c) = color {
            params.insert("color", c.to_string());
        }
        if let Some(o) = orientation {
            params.insert("orientation", o.to_string());
        }

        info!("📸 Searching Pexels for photos: '{}' with {} results per page", query, per_page.unwrap_or(15));

        let response = self.client
            .get(&format!("{}/v1/search", self.base_url))
            .header("Authorization", &self.api_key)
            .query(&params)
            .send()
            .await?;

        if !response.status().is_success() {
            let error_text = response.text().await?;
            error!("Pexels API error: {}", error_text);
            return Err(format!("Pexels API error: {}", error_text).into());
        }

        let photos = response.json::<PexelsPhotoResponse>().await?;
        info!("✅ Found {} photos for query: '{}'", photos.photos.len(), query);
        
        Ok(photos)
    }

    /// Download a video file from Pexels
    pub async fn download_video(
        &self,
        video_file: &PexelsVideoFile,
        download_path: &str,
    ) -> Result<String, Box<dyn std::error::Error + Send + Sync>> {
        info!("⬇️ Downloading video: {} ({}x{} - {})", video_file.link, 
              video_file.width.unwrap_or(0), video_file.height.unwrap_or(0), video_file.quality);

        let response = self.client
            .get(&video_file.link)
            .header("Authorization", &self.api_key)
            .send()
            .await?;

        if !response.status().is_success() {
            return Err(format!("Failed to download video: {}", response.status()).into());
        }

        let bytes = response.bytes().await?;
        
        // Ensure directory exists
        if let Some(parent) = std::path::Path::new(download_path).parent() {
            fs::create_dir_all(parent).await?;
        }

        let mut file = fs::File::create(download_path).await?;
        file.write_all(&bytes).await?;
        file.flush().await?;

        info!("✅ Downloaded video to: {}", download_path);
        Ok(download_path.to_string())
    }

    /// Download a photo file from Pexels
    pub async fn download_photo(
        &self,
        photo: &PexelsPhoto,
        size: &str, // "original", "large", "medium", "small"
        download_path: &str,
    ) -> Result<String, Box<dyn std::error::Error + Send + Sync>> {
        let image_url = match size {
            "original" => &photo.src.original,
            "large" => &photo.src.large,
            "medium" => &photo.src.medium,
            "small" => &photo.src.small,
            "portrait" => &photo.src.portrait,
            "landscape" => &photo.src.landscape,
            "tiny" => &photo.src.tiny,
            _ => &photo.src.large, // default
        };

        info!("⬇️ Downloading photo: {} ({}x{} - {})", image_url, photo.width, photo.height, size);

        let response = self.client
            .get(image_url)
            .header("Authorization", &self.api_key)
            .send()
            .await?;

        if !response.status().is_success() {
            return Err(format!("Failed to download photo: {}", response.status()).into());
        }

        let bytes = response.bytes().await?;
        
        // Ensure directory exists
        if let Some(parent) = std::path::Path::new(download_path).parent() {
            fs::create_dir_all(parent).await?;
        }

        let mut file = fs::File::create(download_path).await?;
        file.write_all(&bytes).await?;
        file.flush().await?;

        info!("✅ Downloaded photo to: {}", download_path);
        Ok(download_path.to_string())
    }

    /// Get trending videos
    pub async fn get_trending_videos(
        &self,
        per_page: Option<i32>,
        page: Option<i32>,
    ) -> Result<PexelsVideoResponse, Box<dyn std::error::Error + Send + Sync>> {
        let mut params = HashMap::new();
        
        if let Some(pp) = per_page {
            params.insert("per_page", pp.to_string());
        }
        if let Some(p) = page {
            params.insert("page", p.to_string());
        }

        info!("🔥 Fetching trending videos from Pexels");

        let response = self.client
            .get(&format!("{}/videos/popular", self.base_url))
            .header("Authorization", &self.api_key)
            .query(&params)
            .send()
            .await?;

        if !response.status().is_success() {
            let error_text = response.text().await?;
            return Err(format!("Pexels API error: {}", error_text).into());
        }

        let videos = response.json::<PexelsVideoResponse>().await?;
        info!("✅ Found {} trending videos", videos.videos.len());
        
        Ok(videos)
    }

    /// Get curated photos
    pub async fn get_curated_photos(
        &self,
        per_page: Option<i32>,
        page: Option<i32>,
    ) -> Result<PexelsPhotoResponse, Box<dyn std::error::Error + Send + Sync>> {
        let mut params = HashMap::new();
        
        if let Some(pp) = per_page {
            params.insert("per_page", pp.to_string());
        }
        if let Some(p) = page {
            params.insert("page", p.to_string());
        }

        info!("🎨 Fetching curated photos from Pexels");

        let response = self.client
            .get(&format!("{}/v1/curated", self.base_url))
            .header("Authorization", &self.api_key)
            .query(&params)
            .send()
            .await?;

        if !response.status().is_success() {
            let error_text = response.text().await?;
            return Err(format!("Pexels API error: {}", error_text).into());
        }

        let photos = response.json::<PexelsPhotoResponse>().await?;
        info!("✅ Found {} curated photos", photos.photos.len());
        
        Ok(photos)
    }
}