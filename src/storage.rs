// Multi-backend object storage router.
//
// Primary backend  = Cloudflare R2 (`R2_*` env vars) — all historic media lives here.
// Fallback backend = AWS S3 (`S3_FALLBACK_BUCKET` + `S3_REGION`, task role creds) —
//                    used automatically whenever R2 is unavailable (e.g. billing
//                    block / NotEntitled) so renders keep flowing.
//
// The router mirrors the public surface of `R2Client`, so existing call sites
// (`state.r2_client.upload(...)`, `.presign_get(...)`, …) keep working while
// gaining transparent failover. Callers that persist a media row should prefer
// the `*_tracked` variants, which report which backend actually stored the
// object, so `storage_backend` can be recorded as 'r2' or 's3'.
//
// Every pre-existing object is on R2, so every back-filled `storage_backend`
// column defaults to 'r2' — reads for current media keep hitting R2, and new
// media records whichever backend served the write.

use std::sync::atomic::{AtomicU64, Ordering};
use std::sync::Arc;
use std::time::{SystemTime, UNIX_EPOCH};

use aws_sdk_s3::primitives::ByteStream;
use tokio::io::AsyncRead;

use crate::r2_client::R2Client;

/// Which physical bucket an object lives in.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum StorageBackend {
    R2,
    S3,
}

impl StorageBackend {
    pub const fn as_str(self) -> &'static str {
        match self {
            StorageBackend::R2 => "r2",
            StorageBackend::S3 => "s3",
        }
    }

    /// Parse a `storage_backend` column value. Unknown/empty defaults to R2
    /// (every legacy object predates S3 and lives on R2).
    pub fn from_db(value: &str) -> Self {
        match value.trim().to_ascii_lowercase().as_str() {
            "s3" | "aws" | "aws_s3" => StorageBackend::S3,
            _ => StorageBackend::R2,
        }
    }
}

impl std::fmt::Display for StorageBackend {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.write_str(self.as_str())
    }
}

/// Seconds to stop hammering a backend after a hard failure before retrying it.
const PRIMARY_COOLDOWN_SECS: u64 = 300;

fn now_epoch_secs() -> u64 {
    SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .map(|d| d.as_secs())
        .unwrap_or(0)
}

#[derive(Clone)]
pub struct StorageRouter {
    primary: Arc<R2Client>,
    fallback: Option<Arc<R2Client>>,
    /// Primary (R2) bucket — public field so `state.r2_client.bucket` call
    /// sites keep compiling.
    pub bucket: String,
    /// Primary (R2) endpoint — mirrors `R2Client.endpoint`.
    pub endpoint: String,
    /// Epoch seconds until which the primary (R2) is considered down. `0` = healthy.
    primary_down_until: Arc<AtomicU64>,
}

impl StorageRouter {
    pub fn new(primary: Arc<R2Client>, fallback: Option<Arc<R2Client>>) -> Self {
        let bucket = primary.bucket.clone();
        let endpoint = primary.endpoint.clone();
        Self {
            primary,
            fallback,
            bucket,
            endpoint,
            primary_down_until: Arc::new(AtomicU64::new(0)),
        }
    }

    pub fn has_fallback(&self) -> bool {
        self.fallback.is_some()
    }

    /// Bucket backing a given backend (S3 returns the fallback bucket).
    pub fn bucket_of(&self, backend: StorageBackend) -> &str {
        match backend {
            StorageBackend::R2 => &self.primary.bucket,
            StorageBackend::S3 => self
                .fallback
                .as_ref()
                .map(|c| c.bucket.as_str())
                .unwrap_or(&self.primary.bucket),
        }
    }

    fn primary_healthy(&self) -> bool {
        now_epoch_secs() >= self.primary_down_until.load(Ordering::Relaxed)
    }

    fn mark_primary_down(&self) {
        if self.fallback.is_some() {
            let until = now_epoch_secs() + PRIMARY_COOLDOWN_SECS;
            self.primary_down_until.store(until, Ordering::Relaxed);
            tracing::warn!(
                "⛔ R2 marked unavailable for {PRIMARY_COOLDOWN_SECS}s — routing writes to S3 fallback"
            );
        }
    }

    fn mark_primary_up(&self) {
        if self.primary_down_until.swap(0, Ordering::Relaxed) != 0 {
            tracing::info!("✅ R2 recovered — primary storage restored");
        }
    }

    fn client_for(&self, backend: StorageBackend) -> &Arc<R2Client> {
        match backend {
            StorageBackend::R2 => &self.primary,
            StorageBackend::S3 => self.fallback.as_ref().unwrap_or(&self.primary),
        }
    }

    // ------------------------------------------------------------------
    // Writes — primary first, then fallback; `*_tracked` report the winner.
    // ------------------------------------------------------------------

    pub async fn upload(&self, local_path: &str, key: &str) -> Result<(), String> {
        self.upload_tracked(local_path, key).await.map(|_| ())
    }

    pub async fn upload_tracked(
        &self,
        local_path: &str,
        key: &str,
    ) -> Result<StorageBackend, String> {
        if self.primary_healthy() {
            match self.primary.upload(local_path, key).await {
                Ok(()) => {
                    self.mark_primary_up();
                    return Ok(StorageBackend::R2);
                }
                Err(e) => {
                    tracing::warn!("R2 upload failed ({e}) — trying S3 fallback");
                    self.mark_primary_down();
                }
            }
        }

        match self.fallback.as_ref() {
            Some(fb) => {
                fb.upload(local_path, key).await?;
                tracing::info!(key = %key, "☁️ stored via S3 fallback (upload)");
                Ok(StorageBackend::S3)
            }
            None => Err(format!(
                "primary storage (R2) unavailable and no S3 fallback configured for {key}"
            )),
        }
    }

    pub async fn upload_bytes(&self, key: &str, data: &[u8], content_type: &str) -> Result<String, String> {
        self.upload_bytes_tracked(key, data, content_type).await.map(|(_, url)| url)
    }

    pub async fn upload_bytes_tracked(
        &self,
        key: &str,
        data: &[u8],
        content_type: &str,
    ) -> Result<(StorageBackend, String), String> {
        if self.primary_healthy() {
            match self.primary.upload_bytes(key, data, content_type).await {
                Ok(url) => {
                    self.mark_primary_up();
                    return Ok((StorageBackend::R2, url));
                }
                Err(e) => {
                    tracing::warn!("R2 upload_bytes failed ({e}) — trying S3 fallback");
                    self.mark_primary_down();
                }
            }
        }

        match self.fallback.as_ref() {
            Some(fb) => {
                let url = fb.upload_bytes(key, data, content_type).await?;
                tracing::info!(key = %key, "☁️ stored via S3 fallback (upload_bytes)");
                Ok((StorageBackend::S3, url))
            }
            None => Err(format!(
                "primary storage (R2) unavailable and no S3 fallback configured for {key}"
            )),
        }
    }

    pub async fn upload_file(&self, local_path: &str, key: &str) -> Result<String, String> {
        self.upload_file_tracked(local_path, key).await.map(|(_, url)| url)
    }

    pub async fn upload_file_tracked(
        &self,
        local_path: &str,
        key: &str,
    ) -> Result<(StorageBackend, String), String> {
        let backend = self.upload_tracked(local_path, key).await?;
        // Presign from the backend we just wrote to — no probe needed.
        let url = self.client_for(backend).presign_get(key, 7 * 24 * 3600).await?;
        Ok((backend, url))
    }

    /// Stream upload. The reader cannot be rewound, so we can't retry after a
    /// partial primary failure — we only route to S3 when the primary is
    /// already known-down (circuit breaker tripped by an earlier write).
    pub async fn upload_stream(
        &self,
        key: &str,
        reader: &mut (dyn AsyncRead + Unpin + Send),
    ) -> Result<(), String> {
        self.upload_stream_tracked(key, reader).await.map(|_| ())
    }

    pub async fn upload_stream_tracked(
        &self,
        key: &str,
        reader: &mut (dyn AsyncRead + Unpin + Send),
    ) -> Result<StorageBackend, String> {
        if self.primary_healthy() {
            match self.primary.upload_stream(key, reader).await {
                Ok(()) => {
                    self.mark_primary_up();
                    return Ok(StorageBackend::R2);
                }
                Err(e) => {
                    tracing::warn!("R2 upload_stream failed ({e}) — primary marked down");
                    self.mark_primary_down();
                    return Err(e);
                }
            }
        }

        match self.fallback.as_ref() {
            Some(fb) => {
                fb.upload_stream(key, reader).await?;
                tracing::info!(key = %key, "☁️ stored via S3 fallback (upload_stream)");
                Ok(StorageBackend::S3)
            }
            None => Err(format!(
                "primary storage (R2) unavailable and no S3 fallback configured for {key}"
            )),
        }
    }

    // ------------------------------------------------------------------
    // Reads
    // ------------------------------------------------------------------

    /// Presign a GET URL. Prefers R2 when the object is there (historic media),
    /// otherwise falls back to S3. Callers holding a `storage_backend` column
    /// value should use [`presign_get_backend`] to skip the probe.
    pub async fn presign_get(&self, key: &str, expires_secs: u64) -> Result<String, String> {
        if self.primary_healthy() && self.primary.exists(key).await {
            self.mark_primary_up();
            return self.primary.presign_get(key, expires_secs).await;
        }
        if let Some(fb) = self.fallback.as_ref() {
            if fb.exists(key).await {
                return fb.presign_get(key, expires_secs).await;
            }
        }
        // Last resort: presign from primary (returns a URL even if the object
        // is missing, matching the previous single-backend behaviour).
        self.primary.presign_get(key, expires_secs).await
    }

    pub async fn presign_get_backend(
        &self,
        key: &str,
        backend: StorageBackend,
        expires_secs: u64,
    ) -> Result<String, String> {
        self.client_for(backend).presign_get(key, expires_secs).await
    }

    pub async fn stream_object(
        &self,
        key: &str,
        range: Option<&str>,
    ) -> Result<(u16, Vec<(String, String)>, ByteStream), String> {
        if self.primary_healthy() {
            match self.primary.stream_object(key, range).await {
                Ok(v) => {
                    self.mark_primary_up();
                    return Ok(v);
                }
                Err(e) => {
                    tracing::warn!("R2 stream_object failed for {key}: {e}");
                }
            }
        }
        match self.fallback.as_ref() {
            Some(fb) => fb.stream_object(key, range).await,
            None => self.primary.stream_object(key, range).await,
        }
    }

    pub async fn download(&self, key: &str, local_path: &str) -> Result<(), String> {
        if self.primary_healthy() {
            match self.primary.download(key, local_path).await {
                Ok(()) => {
                    self.mark_primary_up();
                    return Ok(());
                }
                Err(e) => {
                    tracing::warn!("R2 download failed for {key}: {e} — trying S3 fallback");
                    self.mark_primary_down();
                }
            }
        }
        match self.fallback.as_ref() {
            Some(fb) => fb.download(key, local_path).await,
            None => self.primary.download(key, local_path).await,
        }
    }

    pub async fn exists(&self, key: &str) -> bool {
        if self.primary_healthy() && self.primary.exists(key).await {
            return true;
        }
        match self.fallback.as_ref() {
            Some(fb) => fb.exists(key).await,
            None => false,
        }
    }

    pub async fn delete(&self, key: &str) -> Result<(), String> {
        let mut last_err: Option<String> = None;
        if self.primary_healthy() {
            match self.primary.delete(key).await {
                Ok(()) => return Ok(()),
                Err(e) => last_err = Some(e),
            }
        }
        if let Some(fb) = self.fallback.as_ref() {
            match fb.delete(key).await {
                Ok(()) => return Ok(()),
                Err(e) => last_err = Some(e),
            }
        }
        Err(last_err.unwrap_or_else(|| format!("delete failed for {key}: no backend available")))
    }

    pub async fn list_keys(&self, prefix: Option<&str>, max_keys: i32) -> Result<Vec<String>, String> {
        self.primary.list_keys(prefix, max_keys).await
    }

    pub async fn health_check(&self) -> bool {
        if self.primary.health_check().await {
            self.mark_primary_up();
            return true;
        }
        // Trip the breaker so the first write after a failed startup check goes
        // straight to the fallback instead of waiting on R2's retry loop.
        self.mark_primary_down();
        match self.fallback.as_ref() {
            Some(fb) => fb.health_check().await,
            None => false,
        }
    }
}

/// Build a storage router from environment variables, mirroring the startup
/// wiring in `main.rs`. Returns `None` when the R2 primary is not configured.
/// Used by code paths that need a client without an `AppState` handle.
pub async fn router_from_env() -> Option<Arc<StorageRouter>> {
    let endpoint = std::env::var("R2_ENDPOINT").ok().filter(|v| !v.is_empty())?;
    let access_key = std::env::var("R2_ACCESS_KEY_ID").ok().filter(|v| !v.is_empty())?;
    let secret_key = std::env::var("R2_SECRET_ACCESS_KEY").ok().filter(|v| !v.is_empty())?;
    let bucket = std::env::var("R2_BUCKET").ok().filter(|v| !v.is_empty())?;

    let primary = Arc::new(R2Client::new(&endpoint, &access_key, &secret_key, &bucket).await.ok()?);

    let fallback = match std::env::var("S3_FALLBACK_BUCKET").ok().filter(|b| !b.is_empty()) {
        Some(s3_bucket) => {
            let region = std::env::var("S3_REGION")
                .ok()
                .or_else(|| std::env::var("AWS_REGION").ok())
                .filter(|r| !r.is_empty())
                .unwrap_or_else(|| "us-east-1".to_string());
            R2Client::new_aws_s3(&s3_bucket, &region).await.ok().map(Arc::new)
        }
        None => None,
    };

    Some(Arc::new(StorageRouter::new(primary, fallback)))
}

/// Best-effort backend inference from a stored media URL (used when a caller
/// only has a URL, not the upload result). AWS S3 URLs contain `amazonaws.com`.
pub fn backend_from_url(url: &str) -> StorageBackend {
    match reqwest::Url::parse(url)
        .ok()
        .and_then(|u| u.host_str().map(|h| h.to_ascii_lowercase()))
    {
        Some(h) if h.contains("amazonaws.com") => StorageBackend::S3,
        _ => StorageBackend::R2,
    }
}

/// Extract an object key from a presigned/virtual-hosted S3-compatible URL,
/// for either R2 (`*.r2.cloudflarestorage.com`) or real AWS S3
/// (`<bucket>.s3.<region>.amazonaws.com`, `s3.<region>.amazonaws.com/<bucket>/…`).
///
/// `known_buckets` are stripped when they appear as the first path segment
/// (path-style addressing). Unknown buckets still yield their key so presigned
/// refreshes keep working after a backend switch.
pub fn object_key_from_url(url: &str, known_buckets: &[&str]) -> Option<String> {
    let parsed = reqwest::Url::parse(url).ok()?;
    let host = parsed.host_str().unwrap_or_default().to_ascii_lowercase();

    let segments: Vec<&str> = parsed
        .path_segments()?
        .filter(|s| !s.is_empty())
        .collect();
    if segments.is_empty() {
        return None;
    }

    // 1) Virtual-hosted: "<bucket>.<...>" — bucket is the host prefix, key is the path.
    for b in known_buckets {
        if !b.is_empty() && host.starts_with(&format!("{}.", b.to_ascii_lowercase())) {
            return Some(segments.join("/"));
        }
    }

    // 2) Path-style with a known bucket as the first path segment.
    if let Some(first) = segments.first() {
        for b in known_buckets {
            if !b.is_empty() && first.eq_ignore_ascii_case(b) {
                if segments.len() < 2 {
                    return None;
                }
                return Some(segments[1..].join("/"));
            }
        }
    }

    // 3) Path-style AWS S3 endpoint (host is the service, not a bucket):
    //    s3.amazonaws.com | s3.<region>.amazonaws.com | s3-<region>.amazonaws.com
    let host_is_path_style_s3 = host == "s3.amazonaws.com"
        || host.starts_with("s3.")
        || host.starts_with("s3-");
    if host_is_path_style_s3 && segments.len() >= 2 {
        return Some(segments[1..].join("/"));
    }

    // 4) Path-style R2: "<account>.r2.cloudflarestorage.com/<bucket>/<key>".
    if host.ends_with(".r2.cloudflarestorage.com") && segments.len() >= 2 {
        return Some(segments[1..].join("/"));
    }

    // 5) Virtual-hosted anything else: the whole path is the key.
    Some(segments.join("/"))
}

/// Return `(bucket, key)` for an S3-compatible URL, covering both path-style
/// (`<endpoint>/<bucket>/<key>`) and virtual-hosted (`<bucket>.<endpoint>/<key>`).
pub fn bucket_and_key_from_url(url: &str) -> Option<(String, String)> {
    let parsed = reqwest::Url::parse(url).ok()?;
    let host = parsed.host_str().unwrap_or_default().to_ascii_lowercase();
    let segments: Vec<&str> = parsed
        .path_segments()?
        .filter(|s| !s.is_empty())
        .collect();
    if segments.is_empty() {
        return None;
    }

    // Path-style R2 / S3 service endpoint → bucket is the first path segment.
    let path_style = host.ends_with(".r2.cloudflarestorage.com")
        || host == "s3.amazonaws.com"
        || host.starts_with("s3.")
        || host.starts_with("s3-");
    if path_style {
        if segments.len() < 2 {
            return None;
        }
        let bucket = segments[0].to_string();
        let key = segments[1..].join("/");
        return if key.is_empty() { None } else { Some((bucket, key)) };
    }

    // Virtual-hosted "<bucket>.s3..."/"<bucket>.<account>.r2..." → bucket = host label.
    let bucket = host.split('.').next().unwrap_or_default().to_string();
    if bucket.is_empty() {
        return None;
    }
    Some((bucket, segments.join("/")))
}
