// Cloudflare R2 Storage Client
//
// S3-compatible object storage for all video_editor file I/O:
//   - Raw YouTube downloads      → raw/{user_id}/{video_id}.mp4
//   - Extracted clips            → clips/{job_id}/clip_{n}.mp4
//   - Thumbnails                 → clips/{job_id}/thumb_{n}.jpg
//   - Generated video outputs    → generated/{user_id}/video/{name}.mp4
//   - Generated audio            → generated/{user_id}/audio/{name}.mp3
//   - Blender renders            → blender/{user_id}/{name}.mp4
//
// Replaces ephemeral Render disk storage so files survive service restarts
// and are accessible across multiple services (API + workers).

use aws_config::BehaviorVersion;
use aws_credential_types::Credentials;
use aws_sdk_s3::config::Builder as S3ConfigBuilder;
use aws_sdk_s3::presigning::PresigningConfig;
use aws_sdk_s3::primitives::ByteStream;
use aws_sdk_s3::types::{CompletedMultipartUpload, CompletedPart};
use aws_sdk_s3::Client;
use std::path::Path;
use std::time::Duration;
use tokio::fs::File;
use tokio::io::{AsyncRead, AsyncReadExt};

const MULTIPART_THRESHOLD: u64 = 50 * 1024 * 1024; // 50 MB — use multipart above this
const PART_SIZE: usize = 50 * 1024 * 1024; // 50 MB parts

#[derive(Clone)]
pub struct R2Client {
    client: Client,
    http: reqwest::Client,
    pub bucket: String,
    pub endpoint: String,
}

impl R2Client {
    pub async fn new(
        endpoint_url: &str,
        access_key_id: &str,
        secret_access_key: &str,
        bucket: &str,
    ) -> Result<Self, String> {
        let endpoint = endpoint_url.to_string();

        let creds = Credentials::new(
            access_key_id,
            secret_access_key,
            None,
            None,
            "cloudflare-r2",
        );

        let sdk_config = aws_config::defaults(BehaviorVersion::latest())
            .credentials_provider(creds)
            .region(aws_config::Region::new("auto"))
            .load()
            .await;

        let s3_config = S3ConfigBuilder::from(&sdk_config)
            .endpoint_url(&endpoint)
            .force_path_style(true)
            .build();

        let client = Client::from_conf(s3_config);
        let http = reqwest::Client::builder()
            .timeout(Duration::from_secs(120))
            .build()
            .map_err(|e| format!("Failed to build HTTP client: {e}"))?;

        Ok(Self {
            client,
            http,
            bucket: bucket.to_string(),
            endpoint,
        })
    }

    // -------------------------------------------------------------------------
    // Upload — auto-selects simple vs multipart based on file size
    // -------------------------------------------------------------------------

    pub async fn upload(&self, local_path: &str, key: &str) -> Result<(), String> {
        let metadata = tokio::fs::metadata(local_path)
            .await
            .map_err(|e| format!("Cannot stat {local_path}: {e}"))?;

        let mut last_error = None;
        for attempt in 1..=3 {
            let result = if metadata.len() > MULTIPART_THRESHOLD {
                self.upload_multipart(local_path, key).await
            } else {
                // Use presigned PUT + reqwest for simple uploads to avoid
                // the AWS SDK HTTP client's connection-reset issues with R2.
                self.upload_via_presigned_put(local_path, key).await
            };

            match result {
                Ok(()) => return Ok(()),
                Err(error) => {
                    tracing::warn!(
                        key = %key,
                        attempt,
                        error = %error,
                        "R2 upload attempt failed"
                    );
                    last_error = Some(error);
                    tokio::time::sleep(Duration::from_secs(attempt * 2)).await;
                }
            }
        }

        Err(last_error.unwrap_or_else(|| format!("R2 upload failed for {key}")))
    }

    /// Upload bytes directly to R2 without writing to local disk.
    /// Uses presigned PUT URL + reqwest for files ≤ 50 MB.
    /// For larger files, writes to a temp file and uses multipart upload.
    pub async fn upload_bytes(&self, key: &str, data: &[u8], content_type: &str) -> Result<String, String> {
        if data.len() as u64 > MULTIPART_THRESHOLD {
            // Large file: write to temp, use multipart, clean up
            let tmp = format!("/tmp/opencode/r2_bytes_{}", key.replace('/', "_"));
            if let Some(parent) = std::path::Path::new(&tmp).parent() {
                tokio::fs::create_dir_all(parent).await
                    .map_err(|e| format!("Cannot create temp dir: {e}"))?;
            }
            tokio::fs::write(&tmp, data).await
                .map_err(|e| format!("Cannot write temp file: {e}"))?;
            self.upload_multipart(&tmp, key).await?;
            let _ = tokio::fs::remove_file(&tmp).await;
            return self.presign_get(key, 7 * 24 * 3600).await;
        }

        let config = PresigningConfig::builder()
            .expires_in(Duration::from_secs(3600))
            .build()
            .map_err(|e| format!("PresigningConfig error: {e}"))?;

        let url = self
            .client
            .put_object()
            .bucket(&self.bucket)
            .key(key)
            .content_type(content_type)
            .presigned(config)
            .await
            .map_err(|e| format!("Failed to presign PUT URL: {e}"))?
            .uri()
            .to_string();

        let body_len = data.len();

        let resp = self
            .http
            .put(&url)
            .header("Content-Type", content_type)
            .header("Content-Length", body_len.to_string())
            .body(data.to_vec())
            .send()
            .await
            .map_err(|e| format!("R2 presigned PUT failed: {e}"))?;

        if !resp.status().is_success() {
            let status = resp.status();
            let text = resp.text().await.unwrap_or_default();
            return Err(format!("R2 presigned PUT HTTP {status}: {text}"));
        }

        tracing::info!("R2 upload bytes → {key} ({size} bytes)", size = body_len);
        self.presign_get(key, 7 * 24 * 3600).await
    }

    /// Legacy compatibility helper for call sites that expect upload to
    /// return a downloadable URL in one step.
    pub async fn upload_file(&self, local_path: &str, key: &str) -> Result<String, String> {
        self.upload(local_path, key).await?;
        self.presign_get(key, 7 * 24 * 3600).await
    }

    /// Upload via presigned PUT URL (sign with SDK, upload with reqwest).
    /// Avoids the AWS SDK HTTP client's connectivity issues with R2.
    async fn upload_via_presigned_put(&self, local_path: &str, key: &str) -> Result<(), String> {
        let config = PresigningConfig::builder()
            .expires_in(Duration::from_secs(3600))
            .build()
            .map_err(|e| format!("PresigningConfig error: {e}"))?;

        let url = self
            .client
            .put_object()
            .bucket(&self.bucket)
            .key(key)
            .presigned(config)
            .await
            .map_err(|e| format!("Failed to presign PUT URL: {e}"))?
            .uri()
            .to_string();

        let body = tokio::fs::read(local_path)
            .await
            .map_err(|e| format!("Failed to read {local_path}: {e}"))?;

        let body_len = body.len();

        let resp = self
            .http
            .put(&url)
            .header("Content-Length", body_len.to_string())
            .body(body)
            .send()
            .await
            .map_err(|e| format!("R2 presigned PUT failed: {e}"))?;

        if !resp.status().is_success() {
            let status = resp.status();
            let text = resp.text().await.unwrap_or_default();
            return Err(format!("R2 presigned PUT HTTP {status}: {text}"));
        }

        tracing::info!("R2 upload: {local_path} → {key}");
        Ok(())
    }

    async fn upload_multipart(&self, local_path: &str, key: &str) -> Result<(), String> {
        let resp = self
            .client
            .create_multipart_upload()
            .bucket(&self.bucket)
            .key(key)
            .send()
            .await
            .map_err(|e| format!("create_multipart_upload failed: {e}"))?;

        let upload_id = resp
            .upload_id()
            .ok_or("No upload_id in multipart response")?
            .to_string();

        let mut file = File::open(local_path)
            .await
            .map_err(|e| format!("Cannot open {local_path}: {e}"))?;

        let mut part_number = 1i32;
        let mut completed_parts: Vec<CompletedPart> = Vec::new();

        loop {
            let mut buf = vec![0u8; PART_SIZE];
            let n = file
                .read(&mut buf)
                .await
                .map_err(|e| format!("Read error: {e}"))?;
            if n == 0 {
                break;
            }
            buf.truncate(n);

            let part = self
                .client
                .upload_part()
                .bucket(&self.bucket)
                .key(key)
                .upload_id(&upload_id)
                .part_number(part_number)
                .body(ByteStream::from(buf))
                .send()
                .await
                .map_err(|e| format!("upload_part {part_number} failed: {e}"))?;

            let etag = part
                .e_tag()
                .ok_or("No ETag in upload_part response")?
                .to_string();

            completed_parts.push(
                CompletedPart::builder()
                    .part_number(part_number)
                    .e_tag(etag)
                    .build(),
            );

            part_number += 1;
        }

        let completed = CompletedMultipartUpload::builder()
            .set_parts(Some(completed_parts))
            .build();

        self.client
            .complete_multipart_upload()
            .bucket(&self.bucket)
            .key(key)
            .upload_id(&upload_id)
            .multipart_upload(completed)
            .send()
            .await
            .map_err(|e| format!("complete_multipart_upload failed: {e}"))?;

        tracing::info!("R2 multipart upload: {local_path} → {key}");
        Ok(())
    }

    /// Multipart upload from an arbitrary async reader (e.g. subprocess stdout).
    /// No local disk needed — streams directly to R2.
    pub async fn upload_stream(
        &self,
        key: &str,
        reader: &mut (dyn AsyncRead + Unpin + Send),
    ) -> Result<(), String> {
        let resp = self
            .client
            .create_multipart_upload()
            .bucket(&self.bucket)
            .key(key)
            .send()
            .await
            .map_err(|e| format!("create_multipart_upload failed: {e}"))?;

        let upload_id = resp
            .upload_id()
            .ok_or("No upload_id in multipart response")?
            .to_string();

        let mut part_number = 1i32;
        let mut completed_parts: Vec<CompletedPart> = Vec::new();
        let mut buf = vec![0u8; PART_SIZE];

        loop {
            let n = reader
                .read(&mut buf)
                .await
                .map_err(|e| format!("Read error during stream upload: {e}"))?;
            if n == 0 {
                break;
            }

            let part = self
                .client
                .upload_part()
                .bucket(&self.bucket)
                .key(key)
                .upload_id(&upload_id)
                .part_number(part_number)
                .body(ByteStream::from(buf[..n].to_vec()))
                .send()
                .await
                .map_err(|e| format!("upload_part {part_number} failed: {e}"))?;

            let etag = part
                .e_tag()
                .ok_or("No ETag in upload_part response")?
                .to_string();

            completed_parts.push(
                CompletedPart::builder()
                    .part_number(part_number)
                    .e_tag(etag)
                    .build(),
            );

            part_number += 1;
        }

        let completed = CompletedMultipartUpload::builder()
            .set_parts(Some(completed_parts))
            .build();

        self.client
            .complete_multipart_upload()
            .bucket(&self.bucket)
            .key(key)
            .upload_id(&upload_id)
            .multipart_upload(completed)
            .send()
            .await
            .map_err(|e| format!("complete_multipart_upload failed: {e}"))?;

        tracing::info!("R2 stream upload complete: {key} ({part_number} parts)");
        Ok(())
    }

    // -------------------------------------------------------------------------
    // Download
    // -------------------------------------------------------------------------

    /// Stream an R2 object directly, supporting HTTP Range requests.
    /// Returns (status_code_u16, response_headers, byte_stream).
    pub async fn stream_object(
        &self,
        key: &str,
        range: Option<&str>,
    ) -> Result<(u16, Vec<(String, String)>, ByteStream), String> {
        let mut req = self.client.get_object().bucket(&self.bucket).key(key);
        if let Some(range_header) = range {
            req = req.range(range_header);
        }
        let resp = req.send().await.map_err(|e| format!("R2 stream failed for {key}: {e}"))?;

        let status: u16 = if range.is_some() { 206 } else { 200 };

        let mut headers = vec![
            ("accept-ranges".to_string(), "bytes".to_string()),
            ("content-type".to_string(), resp.content_type.unwrap_or_else(|| "video/mp4".to_string())),
        ];
        if let Some(len) = resp.content_length {
            headers.push(("content-length".to_string(), len.to_string()));
        }
        if let Some(cr) = resp.content_range {
            headers.push(("content-range".to_string(), cr));
        }

        Ok((status, headers, resp.body))
    }

    pub async fn download(&self, key: &str, local_path: &str) -> Result<(), String> {
        let resp = self
            .client
            .get_object()
            .bucket(&self.bucket)
            .key(key)
            .send()
            .await
            .map_err(|e| format!("R2 download failed for {key}: {e}"))?;

        let data = resp
            .body
            .collect()
            .await
            .map_err(|e| format!("Failed to read R2 body for {key}: {e}"))?;

        if let Some(parent) = Path::new(local_path).parent() {
            tokio::fs::create_dir_all(parent)
                .await
                .map_err(|e| format!("Cannot create dir {parent:?}: {e}"))?;
        }

        tokio::fs::write(local_path, data.into_bytes())
            .await
            .map_err(|e| format!("Cannot write {local_path}: {e}"))?;

        tracing::info!("R2 download: {key} → {local_path}");
        Ok(())
    }

    // -------------------------------------------------------------------------
    // Presigned URLs
    // -------------------------------------------------------------------------

    /// Presigned GET URL valid for `expires_secs` seconds (max 604 800 = 7 days).
    pub async fn presign_get(&self, key: &str, expires_secs: u64) -> Result<String, String> {
        let config = PresigningConfig::builder()
            .expires_in(Duration::from_secs(expires_secs))
            .build()
            .map_err(|e| format!("PresigningConfig error: {e}"))?;

        let mut last_error = None;
        for attempt in 1..=3 {
            match self
                .client
                .get_object()
                .bucket(&self.bucket)
                .key(key)
                .presigned(config.clone())
                .await
            {
                Ok(req) => return Ok(req.uri().to_string()),
                Err(error) => {
                    let message = format!("presign_get failed for {key}: {error}");
                    tracing::warn!(key = %key, attempt, error = %message, "R2 presign attempt failed");
                    last_error = Some(message);
                    tokio::time::sleep(Duration::from_secs(attempt * 2)).await;
                }
            }
        }

        Err(last_error.unwrap_or_else(|| format!("presign_get failed for {key}")))
    }

    // -------------------------------------------------------------------------
    // Key helpers — canonical paths for each asset type
    // -------------------------------------------------------------------------

    pub fn key_raw_download(user_id: i32, video_id: &str, ext: &str) -> String {
        format!("raw/{user_id}/{video_id}.{ext}")
    }

    pub fn key_clip(job_id: i32, clip_n: usize) -> String {
        format!("clips/{job_id}/clip_{clip_n}.mp4")
    }

    pub fn key_thumbnail(job_id: i32, clip_n: usize) -> String {
        format!("clips/{job_id}/thumb_{clip_n}.jpg")
    }

    pub fn key_generated_video(user_id: i32, name: &str) -> String {
        format!("generated/{user_id}/video/{name}")
    }

    pub fn key_generated_audio(user_id: i32, name: &str) -> String {
        format!("generated/{user_id}/audio/{name}")
    }

    pub fn key_blender(user_id: i32, name: &str) -> String {
        format!("blender/{user_id}/{name}")
    }

    // -------------------------------------------------------------------------
    // Existence / delete
    // -------------------------------------------------------------------------

    pub async fn exists(&self, key: &str) -> bool {
        self.client
            .head_object()
            .bucket(&self.bucket)
            .key(key)
            .send()
            .await
            .is_ok()
    }

    pub async fn delete(&self, key: &str) -> Result<(), String> {
        self.client
            .delete_object()
            .bucket(&self.bucket)
            .key(key)
            .send()
            .await
            .map_err(|e| format!("R2 delete failed for {key}: {e}"))?;
        Ok(())
    }

    // -------------------------------------------------------------------------
    // Health check
    // -------------------------------------------------------------------------

    pub async fn health_check(&self) -> bool {
        self.client
            .list_objects_v2()
            .bucket(&self.bucket)
            .max_keys(1)
            .send()
            .await
            .is_ok()
    }

    pub async fn list_keys(
        &self,
        prefix: Option<&str>,
        max_keys: i32,
    ) -> Result<Vec<String>, String> {
        let mut req = self.client.list_objects_v2().bucket(&self.bucket).max_keys(max_keys);
        if let Some(prefix) = prefix {
            req = req.prefix(prefix);
        }

        let resp = req
            .send()
            .await
            .map_err(|e| format!("R2 list_objects_v2 failed: {e}"))?;

        Ok(resp
            .contents()
            .iter()
            .filter_map(|obj| obj.key().map(str::to_string))
            .collect())
    }
}
