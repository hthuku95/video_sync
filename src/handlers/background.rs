use axum::{
    http::{header, StatusCode},
    response::{IntoResponse, Json, Response},
};
use chrono::{DateTime, Utc};
use serde_json::json;
use std::sync::Arc;
use tokio::sync::RwLock;

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

fn detect_content_type(data: &[u8]) -> &'static str {
    if std::str::from_utf8(data)
        .map(|s| s.starts_with("<svg"))
        .unwrap_or(false)
    {
        "image/svg+xml"
    } else if data.starts_with(&[0x89, 0x50, 0x4E, 0x47]) {
        "image/png"
    } else if data.starts_with(&[0xFF, 0xD8, 0xFF]) {
        "image/jpeg"
    } else if data.starts_with(&[0x47, 0x49, 0x46]) {
        "image/gif"
    } else if data.starts_with(&[0x52, 0x49, 0x46, 0x46]) {
        "image/webp"
    } else {
        "image/png"
    }
}

/// Return any cached image regardless of age, or None if no cache exists.
async fn read_expired_cache() -> Option<BackgroundCache> {
    let cache_guard = BACKGROUND_CACHE.read().await;
    cache_guard.as_ref().cloned()
}

pub async fn get_background_image(gemini_client: Option<Arc<GeminiClient>>) -> Response {
    // Fast path: return fresh cache (<5 min) without acquiring write lock
    {
        let cache_guard = BACKGROUND_CACHE.read().await;
        if let Some(cache) = cache_guard.as_ref() {
            let age = Utc::now().signed_duration_since(cache.generated_at);
            if age.num_minutes() < 5 {
                return (
                    StatusCode::OK,
                    [(header::CONTENT_TYPE, detect_content_type(&cache.image_data))],
                    cache.image_data.clone(),
                )
                    .into_response();
            }
        }
    }

    // Provider chain: NVIDIA NIM (free) → Gemini (prepaid credits required) →
    // stale cache → CSS gradient.
    let prompt = GeminiClient::create_background_image_prompt("dynamic");

    let mut generated: Option<(Vec<u8>, &'static str)> = None;

    match generate_nim_background(&prompt).await {
        Ok(image_data) => generated = Some((image_data, "nvidia-flux.1-dev")),
        Err(nim_err) => {
            tracing::warn!("NIM background generation failed: {} — trying Gemini fallback", nim_err);
            if let Some(client) = gemini_client.as_ref() {
                match generate_new_background(client, &prompt).await {
                    Ok(image_data) => generated = Some((image_data, "gemini-nano-banana-2-lite")),
                    Err(e) => tracing::error!(
                        "Gemini background generation also failed: {} — falling back to stale cache/gradient",
                        e
                    ),
                }
            }
        }
    }

    if let Some((image_data, provider)) = generated {
        tracing::info!("Background image generated via {}", provider);

        let new_cache = BackgroundCache {
            image_data: image_data.clone(),
            generated_at: Utc::now(),
            theme: format!("dynamic:{}", provider),
        };

        {
            let mut cache_guard = BACKGROUND_CACHE.write().await;
            *cache_guard = Some(new_cache);
        }

        return (
            StatusCode::OK,
            [(header::CONTENT_TYPE, detect_content_type(&image_data))],
            image_data,
        )
            .into_response();
    }

    // Stale-cache fallback: serve the last good image even if expired
    if let Some(stale) = read_expired_cache().await {
        tracing::warn!(
            "Serving expired cache (age: {}s) — generation failed",
            Utc::now()
                .signed_duration_since(stale.generated_at)
                .num_seconds()
        );
        return (
            StatusCode::OK,
            [(header::CONTENT_TYPE, detect_content_type(&stale.image_data))],
            stale.image_data,
        )
            .into_response();
    }

    // Last resort: CSS gradient
    (
        StatusCode::OK,
        [(header::CONTENT_TYPE, "application/json")],
        Json(json!({
            "fallback": true,
            "gradient": "linear-gradient(135deg, #1a1a2e 0%, #16213e 50%, #0f1419 100%)"
        })),
    )
        .into_response()
}

/// Generate a background via NVIDIA FLUX.1-dev (free tier). Primary provider.
async fn generate_nim_background(
    prompt: &str,
) -> Result<Vec<u8>, Box<dyn std::error::Error + Send + Sync>> {
    let api_key = std::env::var("NVIDIA_API_KEY").unwrap_or_default();
    crate::nvidia_nim_client::generate_image_flux(api_key.trim(), prompt)
        .await
        .map_err(Into::into)
}

async fn generate_new_background(
    gemini_client: &GeminiClient,
    prompt: &str,
) -> Result<Vec<u8>, Box<dyn std::error::Error + Send + Sync>> {
    tracing::info!("Generating new background image with prompt: {}", prompt);

    // Use 16:9 aspect ratio for widescreen displays (better for UI backgrounds)
    // Use 2K resolution for good quality without being too large
    // Model: Nano Banana 2 Lite (gemini-3.1-flash-lite-image) — fastest/cheapest of the
    // nano banana family (~4s/image, ~$0.025-0.033), fine quality for UI backgrounds.
    let image_data = gemini_client
        .generate_image(prompt, Some("16:9"), Some("2K"), Some("gemini-3.1-flash-lite-image"))
        .await?;

    // Validate that we got actual image data
    if image_data.len() < 100 {
        return Err("Generated image data too small".into());
    }

    tracing::info!(
        "Successfully generated background image ({} bytes)",
        image_data.len()
    );

    Ok(image_data)
}

pub async fn get_background_info() -> Json<serde_json::Value> {
    let cache_guard = BACKGROUND_CACHE.read().await;

    if let Some(cache) = cache_guard.as_ref() {
        let age_minutes = Utc::now()
            .signed_duration_since(cache.generated_at)
            .num_minutes();
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
