use axum::{
    http::{header, StatusCode},
    response::{IntoResponse, Response, Json},
};
use serde_json::json;
use std::sync::Arc;
use tokio::sync::RwLock;
use chrono::{DateTime, Utc};

use crate::gemini_client::GeminiClient;

// Cache structure for background images
#[derive(Debug, Clone)]
pub struct BackgroundCache {
    pub image_data: Vec<u8>,
    pub generated_at: DateTime<Utc>,
    pub theme: String,
}

// Global cache for background images
lazy_static::lazy_static! {
    static ref BACKGROUND_CACHE: Arc<RwLock<Option<BackgroundCache>>> = Arc::new(RwLock::new(None));
}

pub async fn get_background_image(gemini_client: Arc<GeminiClient>) -> Response {
    // Check if we have a cached image that's less than 5 minutes old
    {
        let cache_guard = BACKGROUND_CACHE.read().await;
        if let Some(cache) = cache_guard.as_ref() {
            let age = Utc::now().signed_duration_since(cache.generated_at);
            if age.num_minutes() < 5 {
                // Return cached image with appropriate content type
                let content_type = if std::str::from_utf8(&cache.image_data)
                    .map(|s| s.starts_with("<svg"))
                    .unwrap_or(false) {
                    "image/svg+xml"
                } else if cache.image_data.starts_with(&[0x89, 0x50, 0x4E, 0x47]) { // PNG signature
                    "image/png"
                } else if cache.image_data.starts_with(&[0xFF, 0xD8, 0xFF]) { // JPEG signature
                    "image/jpeg"
                } else if cache.image_data.starts_with(&[0x47, 0x49, 0x46]) { // GIF signature
                    "image/gif"
                } else if cache.image_data.starts_with(&[0x52, 0x49, 0x46, 0x46]) { // WebP signature (RIFF)
                    "image/webp"
                } else {
                    "image/png" // Default fallback
                };

                return (
                    StatusCode::OK,
                    [(header::CONTENT_TYPE, content_type)],
                    cache.image_data.clone(),
                ).into_response();
            }
        }
    }

    // Generate new background image
    match generate_new_background(&gemini_client).await {
        Ok(image_data) => {
            // Cache the new image
            let new_cache = BackgroundCache {
                image_data: image_data.clone(),
                generated_at: Utc::now(),
                theme: "dynamic".to_string(),
            };
            
            {
                let mut cache_guard = BACKGROUND_CACHE.write().await;
                *cache_guard = Some(new_cache);
            }

            // Determine content type based on data
            let content_type = if std::str::from_utf8(&image_data)
                .map(|s| s.starts_with("<svg"))
                .unwrap_or(false) {
                "image/svg+xml"
            } else if image_data.starts_with(&[0x89, 0x50, 0x4E, 0x47]) { // PNG signature
                "image/png"
            } else if image_data.starts_with(&[0xFF, 0xD8, 0xFF]) { // JPEG signature
                "image/jpeg"
            } else if image_data.starts_with(&[0x47, 0x49, 0x46]) { // GIF signature
                "image/gif"
            } else if image_data.starts_with(&[0x52, 0x49, 0x46, 0x46]) { // WebP signature (RIFF)
                "image/webp"
            } else {
                "image/png" // Default fallback
            };

            (
                StatusCode::OK,
                [(header::CONTENT_TYPE, content_type)],
                image_data,
            ).into_response()
        }
        Err(e) => {
            tracing::error!("Failed to generate background image: {}", e);
            
            // Return a fallback CSS gradient as JSON
            (
                StatusCode::OK,
                [(header::CONTENT_TYPE, "application/json")],
                Json(json!({
                    "fallback": true,
                    "gradient": "linear-gradient(135deg, #1a1a2e 0%, #16213e 50%, #0f1419 100%)"
                }))
            ).into_response()
        }
    }
}

async fn generate_new_background(gemini_client: &GeminiClient) -> Result<Vec<u8>, Box<dyn std::error::Error + Send + Sync>> {
    let prompt = GeminiClient::create_background_image_prompt("dynamic");
    
    tracing::info!("Generating new background image with prompt: {}", prompt);
    
    let image_data = gemini_client.generate_image(&prompt, None, None).await?;
    
    // Validate that we got actual image data
    if image_data.len() < 100 {
        return Err("Generated image data too small".into());
    }
    
    tracing::info!("Successfully generated background image ({} bytes)", image_data.len());
    
    Ok(image_data)
}

pub async fn get_background_info() -> Json<serde_json::Value> {
    let cache_guard = BACKGROUND_CACHE.read().await;
    
    if let Some(cache) = cache_guard.as_ref() {
        let age_minutes = Utc::now().signed_duration_since(cache.generated_at).num_minutes();
        let next_refresh_minutes = 5 - age_minutes;
        
        Json(json!({
            "cached": true,
            "generated_at": cache.generated_at,
            "theme": cache.theme,
            "age_minutes": age_minutes,
            "next_refresh_minutes": next_refresh_minutes.max(0),
            "image_size_bytes": cache.image_data.len()
        }))
    } else {
        Json(json!({
            "cached": false,
            "message": "No background image cached yet"
        }))
    }
}