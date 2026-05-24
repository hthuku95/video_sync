use reqwest::Client;
use serde::Deserialize;
use std::path::Path;
use tokio_util::io::ReaderStream;

#[derive(Debug, Clone)]
pub struct GcsClient {
    client: Client,
    bucket: String,
}

#[derive(Debug, Deserialize)]
struct MetadataTokenResponse {
    access_token: String,
}

impl GcsClient {
    pub fn from_env() -> Option<Self> {
        let bucket = std::env::var("VIDEO_SYNC_GCS_BUCKET")
            .or_else(|_| std::env::var("GCS_BUCKET"))
            .or_else(|_| std::env::var("GOOGLE_CLOUD_STORAGE_BUCKET"))
            .unwrap_or_else(|_| "videosync-481307-generated".to_string());

        Some(Self {
            client: Client::new(),
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
        let token = self.access_token().await?;
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

        let response = self
            .client
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

    pub async fn download_bytes(&self, object_key: &str) -> Result<Vec<u8>, String> {
        let token = self.access_token().await?;
        let encoded_key = urlencoding::encode(object_key);
        let url = format!(
            "https://storage.googleapis.com/storage/v1/b/{}/o/{}?alt=media",
            self.bucket, encoded_key
        );

        let response = self
            .client
            .get(url)
            .bearer_auth(token)
            .send()
            .await
            .map_err(|e| format!("GCS download request failed: {e:?}"))?;

        if response.status().is_success() {
            response
                .bytes()
                .await
                .map(|bytes| bytes.to_vec())
                .map_err(|e| format!("Failed to read GCS download body: {e}"))
        } else {
            let status = response.status();
            let body = response.text().await.unwrap_or_default();
            Err(format!("GCS download failed with {status}: {body}"))
        }
    }

    async fn access_token(&self) -> Result<String, String> {
        let response = self
            .client
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
            .map(|token| token.access_token)
            .map_err(|e| format!("Failed to parse GCP metadata token: {e}"))
    }
}
