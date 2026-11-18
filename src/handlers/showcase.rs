// src/handlers/showcase.rs
//! Public marketing showcase — serves scene clips + full renders from the
//! fallback (B2) bucket so the TypeScript apps can display real generated
//! videos on their marketing surfaces (owner directive Nov 17 2026, modeled
//! on higgsfield.ai's video-first homepage: hero background videos, showcase
//! walls, scene cards).
//!
//! Design:
//! - Curated scene clips live under `portfolio_scenes/<name>.mp4` in the
//!   fallback bucket with a `portfolio_scenes/manifest.json` describing them.
//! - The manifest is fetched and cached for 1 hour, so re-uploading a new
//!   manifest auto-enriches every app without a redeploy.
//! - Full renders: any completed delivery whose stored media lives on the B2
//!   fallback bucket is included automatically (auto-enriching as renders
//!   complete — no pipeline changes needed).
//! - URLs are presigned for 7 days; every page load re-signs, so links never
//!   go stale for visitors.
//!
//! NOTE: this endpoint is PUBLIC (no auth) — it only ever exposes media that
//! is already shareable via public delivery links, never user PII.

use axum::{Extension, Json};
use serde_json::{json, Value};
use sqlx::Row;
use std::sync::Arc;
use std::time::{Duration, Instant};
use tokio::sync::Mutex;

use crate::AppState;

const SCENES_PREFIX: &str = "portfolio_scenes";
const MANIFEST_KEY: &str = "portfolio_scenes/manifest.json";
const MANIFEST_TTL: Duration = Duration::from_secs(3600);
const PRESIGN_SECS: u64 = 7 * 24 * 3600;

type ManifestCache = Mutex<Option<(Instant, Vec<Value>)>>;

fn manifest_cache() -> &'static ManifestCache {
    use std::sync::OnceLock;
    static CACHE: OnceLock<ManifestCache> = OnceLock::new();
    CACHE.get_or_init(|| Mutex::new(None))
}

/// Extract the object key from a B2 fallback URL
/// (`https://s3.<region>.backblazeb2.com/<bucket>/<key>...`).
fn fallback_object_key(url: &str, fallback_bucket: &str) -> Option<String> {
    let marker = format!("{fallback_bucket}/");
    if let Some(pos) = url.find(&marker) {
        let rest = &url[pos + marker.len()..];
        // strip query string (X-Amz-* params etc.)
        let key = rest.split(['?', '&']).next().unwrap_or(rest);
        let key = percent_decode(key);
        if key.is_empty() {
            None
        } else {
            Some(key)
        }
    } else {
        None
    }
}

fn percent_decode(s: &str) -> String {
    let bytes = s.as_bytes();
    let mut out = Vec::with_capacity(bytes.len());
    let mut i = 0;
    while i < bytes.len() {
        if bytes[i] == b'%' && i + 2 < bytes.len() {
            if let Ok(v) = u8::from_str_radix(&s[i + 1..i + 3], 16) {
                out.push(v);
                i += 3;
                continue;
            }
        }
        out.push(bytes[i]);
        i += 1;
    }
    String::from_utf8_lossy(&out).into_owned()
}

/// Public accessor for the admin showcase page (same 1h cache).
pub async fn load_manifest_public(state: &Arc<AppState>) -> Vec<Value> {
    load_manifest(state).await
}

async fn load_manifest(state: &Arc<AppState>) -> Vec<Value> {
    let mut cache = manifest_cache().lock().await;
    if let Some((at, scenes)) = cache.as_ref() {
        if at.elapsed() < MANIFEST_TTL {
            return scenes.clone();
        }
    }
    let scenes = match &state.r2_client {
        Some(router) => {
            let tmp = std::env::temp_dir().join("vs_showcase_manifest.json");
            match router
                .download(
                    MANIFEST_KEY,
                    tmp.to_str().unwrap_or("/tmp/vs_showcase_manifest.json"),
                )
                .await
            {
                Ok(()) => {
                    let raw = std::fs::read_to_string(&tmp).unwrap_or_default();
                    let parsed: Value = serde_json::from_str(&raw).unwrap_or(Value::Null);
                    parsed
                        .get("scenes")
                        .and_then(|s| s.as_array())
                        .cloned()
                        .unwrap_or_default()
                }
                Err(_) => Vec::new(), // no manifest yet — empty wall until uploaded
            }
        }
        None => Vec::new(),
    };
    *cache = Some((Instant::now(), scenes.clone()));
    scenes
}

/// GET /api/showcase — public, no auth.
pub async fn api_showcase(Extension(state): Extension<Arc<AppState>>) -> Json<Value> {
    let mut scenes_out = Vec::new();

    // 1. Curated scene clips from the manifest.
    if let Some(router) = state.r2_client.as_ref() {
        let manifest_scenes = load_manifest(&state).await;
        for s in &manifest_scenes {
            let key = match s.get("key").and_then(|k| k.as_str()) {
                Some(k) if k.starts_with(SCENES_PREFIX) => k.to_string(),
                _ => continue,
            };
            match router.presign_get(&key, PRESIGN_SECS).await {
                Ok(url) => scenes_out.push(json!({
                    "kind": "scene",
                    "url": url,
                    "source": s.get("source").cloned().unwrap_or(json!("sample")),
                    "theme": s.get("theme").cloned().unwrap_or(json!("sample")),
                    "orientation": s.get("orientation").cloned().unwrap_or(json!("landscape")),
                    "duration": s.get("duration").cloned().unwrap_or(json!(7)),
                    "title": s.get("title").cloned().unwrap_or_else(|| s.get("source").cloned().unwrap_or(json!("Sample scene"))),
                })),
                Err(_) => continue,
            }
        }
    }

    // 2. Full renders — auto-enriching: every completed delivery stored on the
    //    B2 fallback bucket becomes a wall entry (muted background material +
    //    watchable sample).
    let mut videos_out = Vec::new();
    if let Some(router) = state.r2_client.as_ref() {
        let rows = sqlx::query(
            "SELECT id, title, gig_type, output_r2_url, extra_args \
             FROM deliveries \
             WHERE status = 'completed' \
               AND output_r2_url LIKE '%backblazeb2%' \
               AND length(output_r2_url) > 40 \
             ORDER BY created_at DESC \
             LIMIT 24",
        )
        .fetch_all(&state.db_pool)
        .await
        .unwrap_or_default();

        let fallback_bucket = std::env::var("S3_FALLBACK_BUCKET").unwrap_or_default();
        for row in rows {
            let stored_url: String = row.get("output_r2_url");
            let Some(key) = fallback_object_key(&stored_url, &fallback_bucket) else {
                continue;
            };
            match router.presign_get(&key, PRESIGN_SECS).await {
                Ok(url) => {
                    let extra: Value = row.try_get("extra_args").unwrap_or(Value::Null);
                    let delivery_id: uuid::Uuid = row.get("id");
                    let orientation = if extra
                        .get("aspect_ratio")
                        .and_then(|v| v.as_str())
                        .map(|a| a.contains("9:16") || a.contains("vertical"))
                        .unwrap_or(false)
                    {
                        "portrait"
                    } else {
                        "landscape"
                    };
                    videos_out.push(json!({
                        "kind": "video",
                        "url": url,
                        "title": row.get::<String, _>("title"),
                        "gig_type": row.get::<String, _>("gig_type"),
                        "orientation": orientation,
                        "delivery": format!("/delivery/{}", delivery_id),
                    }));
                }
                Err(_) => continue,
            }
        }
    }

    Json(json!({
        "success": true,
        "scenes": scenes_out,
        "videos": videos_out,
        "generated_at": chrono::Utc::now().to_rfc3339(),
    }))
}
