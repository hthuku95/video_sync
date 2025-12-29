use axum::{
    routing::get,
    Router,
    Extension,
    response::IntoResponse,
};
use std::sync::Arc;

use crate::AppState;
use super::background::{get_background_image, get_background_info};

pub fn background_routes() -> Router {
    Router::new()
        .route("/api/background/image", get(background_image_handler))
        .route("/api/background/info", get(get_background_info))
}

async fn background_image_handler(Extension(state): Extension<Arc<AppState>>) -> axum::response::Response {
    match &state.gemini_client {
        Some(gemini_client) => {
            get_background_image(Arc::new(gemini_client.clone())).await
        }
        None => {
            use axum::response::Json;
            use serde_json::json;
            
            // Return fallback gradient when Gemini is not configured
            (
                axum::http::StatusCode::OK,
                [(axum::http::header::CONTENT_TYPE, "application/json")],
                Json(json!({
                    "fallback": true,
                    "gradient": "linear-gradient(135deg, #1a1a2e 0%, #16213e 50%, #0f1419 100%)",
                    "message": "Gemini client not configured, using fallback gradient"
                }))
            ).into_response()
        }
    }
}