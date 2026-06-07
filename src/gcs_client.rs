use aws_config::BehaviorVersion;
use aws_credential_types::Credentials;
use aws_sdk_s3::config::Builder as S3ConfigBuilder;
use aws_sdk_s3::primitives::ByteStream;
use aws_sdk_s3::Client as S3Client;
use reqwest::Client;
use serde::Deserialize;
use std::path::Path;
use tokio_util::io::ReaderStream;

#[derive(Clone)]
enum GcsAuth {
    /// S3-compatible HMAC keys (works from anywhere — AWS, local dev, etc.)
    Hmac {
        s3_client: S3Client,
        bucket: String,
    },
    /// GCP Instance Metadata Server (only works on GCP VMs / Cloud Run)
    Metadata {
        http_client: Client,
    },
}

#[derive(Clone)]
pub struct GcsClient {
    auth: GcsAuth,
    bucket: String,
}

#[derive(Debug, Deserialize)]
struct MetadataTokenResponse {
    access_token: String,
}

impl GcsClient {
    pub async fn from_env() -> Option<Self> {
        let bucket = std::env::var("VIDEO_SYNC_GCS_BUCKET")
            .or_else(|_| std::env::var("GCS_BUCKET"))
            .or_else(|_| std::env::var("GOOGLE_CLOUD_STORAGE_BUCKET"))
            .unwrap_or_else(|_| "videosync-481307-generated".to_string());

        // 1. Try HMAC keys first (S3-compatible, works anywhere)
        if let (Ok(access_key), Ok(secret_key)) = (
            std::env::var("GCS_HMAC_ACCESS_KEY"),
            std::env::var("GCS_HMAC_SECRET_KEY"),
        ) {
            if !access_key.trim().is_empty() && !secret_key.trim().is_empty() {
                let creds = Credentials::new(
                    &access_key,
                    &secret_key,
                    None,
                    None,
                    "gcs-hmac",
                );

                let sdk_config = aws_config::defaults(BehaviorVersion::latest())
                    .credentials_provider(creds)
                    .region(aws_config::Region::new("auto"))
                    .load()
                    .await;

                let s3_config = S3ConfigBuilder::from(&sdk_config)
                    .endpoint_url("https://storage.googleapis.com")
                    .force_path_style(true)
                    .build();

                let s3_client = S3Client::from_conf(s3_config);

                return Some(Self {
                    auth: GcsAuth::Hmac {
                        s3_client,
                        bucket: bucket.clone(),
                    },
                    bucket,
                });
            }
        }

        // 2. Fall back to GCP metadata server (GCP-only)
        Some(Self {
            auth: GcsAuth::Metadata {
                http_client: Client::new(),
            },
            bucket,
        })
    }

    pub fn bucket(&self) -> &str {
        &self.bucket
    }

    pub async fn upload_file(
        &self,
        local_path: impl AsRef<Path>,
        object_key: &str,
        content_type: &str,
    ) -> Result<(), String> {
        match &self.auth {
            GcsAuth::Hmac { s3_client, bucket } => {
                let body = ByteStream::from_path(local_path.as_ref())
                    .await
                    .map_err(|e| format!("Failed to read file for GCS HMAC upload: {e}"))?;

                s3_client
                    .put_object()
                    .bucket(bucket)
                    .key(object_key)
                    .body(body)
                    .send()
                    .await
                    .map_err(|e| format!("GCS HMAC upload failed: {e:?}"))?;

                tracing::info!("GCS HMAC upload: {object_key}");
                Ok(())
            }
            GcsAuth::Metadata { http_client } => {
                let token = self.access_token_metadata(http_client).await?;
                let file = tokio::fs::File::open(local_path.as_ref())
                    .await
                    .map_err(|e| format!("Failed to open file for GCS upload: {e}"))?;
                let stream = ReaderStream::new(file);
                let body = reqwest::Body::wrap_stream(stream);
                let encoded_key = urlencoding::encode(object_key);
                let url = format!(
                    "https://storage.googleapis.com/upload/storage/v1/b/{}/o?uploadType=media&name={}",
                    self.bucket, encoded_key
                );

                let response = http_client
                    .post(url)
                    .bearer_auth(token)
                    .header(reqwest::header::CONTENT_TYPE, content_type)
                    .body(body)
                    .send()
                    .await
                    .map_err(|e| format!("GCS upload request failed: {e:?}"))?;

                if response.status().is_success() {
                    Ok(())
                } else {
                    let status = response.status();
                    let body = response.text().await.unwrap_or_default();
                    Err(format!("GCS upload failed with {status}: {body}"))
                }
            }
        }
    }

    pub async fn upload_bytes(
        &self,
        bytes: &[u8],
        object_key: &str,
        content_type: &str,
    ) -> Result<String, String> {
        match &self.auth {
            GcsAuth::Hmac { s3_client, bucket } => {
                let body = ByteStream::from(bytes.to_vec());

                s3_client
                    .put_object()
                    .bucket(bucket)
                    .key(object_key)
                    .body(body)
                    .send()
                    .await
                    .map_err(|e| format!("GCS HMAC upload failed: {e:?}"))?;

                let download_url = format!(
                    "https://storage.googleapis.com/{}/{}",
                    bucket, object_key
                );
                tracing::info!("GCS HMAC upload bytes: {object_key}");
                Ok(download_url)
            }
            GcsAuth::Metadata { http_client } => {
                let token = self.access_token_metadata(http_client).await?;
                let body = reqwest::Body::from(bytes.to_vec());
                let encoded_key = urlencoding::encode(object_key);
                let url = format!(
                    "https://storage.googleapis.com/upload/storage/v1/b/{}/o?uploadType=media&name={}",
                    self.bucket, encoded_key
                );

                let response = http_client
                    .post(url)
                    .bearer_auth(token)
                    .header(reqwest::header::CONTENT_TYPE, content_type)
                    .body(body)
                    .send()
                    .await
                    .map_err(|e| format!("GCS upload request failed: {e:?}"))?;

                if response.status().is_success() {
                    let download_url = format!(
                        "https://storage.googleapis.com/{}/{}",
                        self.bucket, encoded_key
                    );
                    Ok(download_url)
                } else {
                    let status = response.status();
                    let body = response.text().await.unwrap_or_default();
                    Err(format!("GCS upload failed with {status}: {body}"))
                }
            }
        }
    }

    pub async fn download_bytes(&self, object_key: &str) -> Result<Vec<u8>, String> {
        match &self.auth {
            GcsAuth::Hmac { s3_client, bucket } => {
                let resp = s3_client
                    .get_object()
                    .bucket(bucket)
                    .key(object_key)
                    .send()
                    .await
                    .map_err(|e| format!("GCS HMAC download failed: {e:?}"))?;

                let data = resp
                    .body
                    .collect()
                    .await
                    .map_err(|e| format!("Failed to read GCS HMAC body: {e}"))?;

                Ok(data.into_bytes().to_vec())
            }
            GcsAuth::Metadata { http_client } => {
                let token = self.access_token_metadata(http_client).await?;
                let encoded_key = urlencoding::encode(object_key);
                let url = format!(
                    "https://storage.googleapis.com/storage/v1/b/{}/o/{}?alt=media",
                    self.bucket, encoded_key
                );

                let response = http_client
                    .get(url)
                    .bearer_auth(token)
                    .send()
                    .await
                    .map_err(|e| format!("GCS download request failed: {e:?}"))?;

                if response.status().is_success() {
                    response
                        .bytes()
                        .await
                        .map(|b| b.to_vec())
                        .map_err(|e| format!("Failed to read GCS download body: {e}"))
                } else {
                    let status = response.status();
                    let body = response.text().await.unwrap_or_default();
                    Err(format!("GCS download failed with {status}: {body}"))
                }
            }
        }
    }

    async fn access_token_metadata(&self, http_client: &Client) -> Result<String, String> {
        let response = http_client
            .get("http://metadata.google.internal/computeMetadata/v1/instance/service-accounts/default/token")
            .header("Metadata-Flavor", "Google")
            .send()
            .await
            .map_err(|e| format!("Failed to fetch GCP metadata token: {e:?}"))?;

        if !response.status().is_success() {
            let status = response.status();
            let body = response.text().await.unwrap_or_default();
            return Err(format!("GCP metadata token failed with {status}: {body}"));
        }

        response
            .json::<MetadataTokenResponse>()
            .await
            .map(|t| t.access_token)
            .map_err(|e| format!("Failed to parse GCP metadata token: {e}"))
    }
}
