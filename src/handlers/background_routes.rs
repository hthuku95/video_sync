use axum::{response::IntoResponse, routing::get, Extension, Router};
use std::sync::Arc;

use super::background::{get_background_image, get_background_info};
use crate::AppState;

pub fn background_routes() -> Router {
    Router::new()
        .route("/api/background/image", get(background_image_handler))
        .route("/api/background/info", get(get_background_info))
}

async fn background_image_handler(
    Extension(state): Extension<Arc<AppState>>,
) -> axum::response::Response {
    // Generation chain (see CLAUDE.md §35.3): NVIDIA NIM FLUX.1-dev is the PRIMARY
    // provider (free tier). A dedicated Gemini key (GEMINI_BACKGROUND_API_KEY) acts
    // as fallback for when NIM fails; shared client as last Gemini option.
    let gemini_client: Option<std::sync::Arc<crate::gemini_client::GeminiClient>> =
        if let Ok(bg_key) = std::env::var("GEMINI_BACKGROUND_API_KEY") {
            if !bg_key.trim().is_empty() {
                Some(std::sync::Arc::new(
                    crate::gemini_client::GeminiClient::new(bg_key.trim().to_string()),
                ))
            } else {
                None
            }
        } else {
            state
                .video_gemini_client
                .as_ref()
                .or(state.gemini_client.as_ref())
                .map(|c| std::sync::Arc::new(c.clone()))
        };

    get_background_image(gemini_client).await
}
