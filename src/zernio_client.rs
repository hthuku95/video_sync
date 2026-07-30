use reqwest::Client;
use serde::{Deserialize, Serialize};
use std::collections::HashMap;
use tracing::{error, info};

#[derive(Debug, Clone)]
pub struct ZernioClient {
    client: Client,
    api_key: String,
    base_url: String,
}

// ── Request types ──────────────────────────────────────────────────────────

#[derive(Debug, Serialize)]
pub struct CreateProfileRequest {
    pub name: String,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub description: Option<String>,
}

#[derive(Debug, Deserialize, Serialize)]
pub struct CreateProfileResponse {
    pub profile: ZernioProfile,
}

#[derive(Debug, Clone, Deserialize, Serialize)]
pub struct ZernioProfile {
    #[serde(rename = "_id")]
    pub id: String,
    pub name: String,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub description: Option<String>,
}

#[derive(Debug, Deserialize, Serialize)]
pub struct ListProfilesResponse {
    pub profiles: Vec<ZernioProfile>,
}

#[derive(Debug, Deserialize, Serialize)]
pub struct ConnectUrlResponse {
    #[serde(rename = "authUrl")]
    pub auth_url: String,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub state: Option<String>,
}

#[derive(Debug, Deserialize, Serialize)]
pub struct ListAccountsResponse {
    pub accounts: Vec<ZernioAccount>,
}

#[derive(Debug, Deserialize, Serialize)]
pub struct ZernioAccount {
    #[serde(rename = "_id")]
    pub id: String,
    pub platform: String,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub username: Option<String>,
    #[serde(rename = "displayName", skip_serializing_if = "Option::is_none")]
    pub display_name: Option<String>,
    #[serde(rename = "profileId", skip_serializing_if = "Option::is_none")]
    pub profile_id: Option<String>,
    #[serde(rename = "isActive")]
    pub is_active: bool,
    #[serde(rename = "profilePicture", skip_serializing_if = "Option::is_none")]
    pub profile_picture: Option<String>,
    #[serde(rename = "profileUrl", skip_serializing_if = "Option::is_none")]
    pub profile_url: Option<String>,
}

#[derive(Debug, Serialize)]
pub struct PlatformTarget {
    pub platform: String,
    pub accountId: String,
}

#[derive(Debug, Serialize)]
pub struct MediaItem {
    pub r#type: String,
    pub url: String,
}

#[derive(Debug, Serialize)]
pub struct CreatePostRequest {
    #[serde(skip_serializing_if = "Option::is_none")]
    pub content: Option<String>,
    pub platforms: Vec<PlatformTarget>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub profileId: Option<String>,
    #[serde(rename = "mediaItems", skip_serializing_if = "Option::is_none")]
    pub media_items: Option<Vec<MediaItem>>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub scheduledFor: Option<String>,
    pub publishNow: bool,
}

#[derive(Debug, Deserialize, Serialize)]
pub struct CreatePostResponse {
    pub post: ZernioPost,
}

#[derive(Debug, Clone, Deserialize, Serialize)]
pub struct GetPostResponse {
    pub post: ZernioPostDetail,
}

#[derive(Debug, Clone, Deserialize, Serialize)]
pub struct ZernioPost {
    #[serde(rename = "_id")]
    pub id: String,
    pub status: Option<String>,
}

/// Full post detail with per-platform status, errors, and URLs.
#[derive(Debug, Clone, Deserialize, Serialize)]
pub struct ZernioPostDetail {
    #[serde(rename = "_id")]
    pub id: String,
    pub status: Option<String>,
    pub content: Option<String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub scheduledFor: Option<String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub timezone: Option<String>,
    pub platforms: Vec<PlatformStatus>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub publishedAt: Option<String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub createdAt: Option<String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub updatedAt: Option<String>,
}

/// Per-platform publishing status with error details and post URLs.
#[derive(Debug, Clone, Deserialize, Serialize)]
pub struct PlatformStatus {
    pub platform: String,
    pub accountId: PlatformAccountId,
    pub status: String,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub platformPostId: Option<String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub platformPostUrl: Option<String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub errorMessage: Option<String>,
    /// Machine-readable error type: account_issue, platform_rejected, platform_error, system_error, unknown
    #[serde(skip_serializing_if = "Option::is_none")]
    pub errorType: Option<String>,
}

/// Per-platform state constants
impl PlatformStatus {
    pub const PENDING: &'static str = "pending";
    pub const PROCESSING: &'static str = "processing";
    pub const UPLOADING: &'static str = "uploading";
    pub const PUBLISHED: &'static str = "published";
    pub const FAILED: &'static str = "failed";
    pub const CANCELLED: &'static str = "cancelled";
}

/// Post-level state constants
pub struct PostStatus;
impl PostStatus {
    pub const DRAFT: &'static str = "draft";
    pub const SCHEDULED: &'static str = "scheduled";
    pub const PUBLISHING: &'static str = "publishing";
    pub const PUBLISHED: &'static str = "published";
    pub const PARTIAL: &'static str = "partial";
    pub const FAILED: &'static str = "failed";
    pub const CANCELLED: &'static str = "cancelled";
}

#[derive(Debug, Clone, Deserialize, Serialize)]
pub struct PlatformAccountId {
    #[serde(rename = "_id")]
    pub id: String,
    pub platform: String,
    pub username: Option<String>,
    #[serde(rename = "displayName")]
    pub display_name: Option<String>,
    #[serde(rename = "isActive")]
    pub is_active: bool,
}

#[derive(Debug, Deserialize, Serialize)]
pub struct RetryPostResponse {
    pub message: String,
    pub post: ZernioPostDetail,
}

#[derive(Debug, Serialize)]
pub struct UploadMediaRequest {
    pub url: String,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub accountId: Option<String>,
}

#[derive(Debug, Deserialize, Serialize)]
pub struct UploadMediaResponse {
    pub media: ZernioMedia,
}

#[derive(Debug, Deserialize, Serialize)]
pub struct ZernioMedia {
    #[serde(rename = "_id")]
    pub id: String,
    pub url: Option<String>,
    #[serde(rename = "type")]
    pub media_type: Option<String>,
}

// ── Error type ─────────────────────────────────────────────────────────────

#[derive(Debug)]
pub enum ZernioError {
    Http(reqwest::StatusCode, String),
    Network(String),
    Deserialize(String),
}

impl std::fmt::Display for ZernioError {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        match self {
            ZernioError::Http(code, body) => write!(f, "Zernio HTTP {}: {}", code, body),
            ZernioError::Network(msg) => write!(f, "Zernio network error: {}", msg),
            ZernioError::Deserialize(msg) => write!(f, "Zernio deserialize error: {}", msg),
        }
    }
}

impl std::error::Error for ZernioError {}

pub type Result<T> = std::result::Result<T, ZernioError>;

// ── Client implementation ──────────────────────────────────────────────────

impl ZernioClient {
    pub fn new(api_key: String) -> Self {
        Self {
            client: Client::new(),
            api_key,
            base_url: "https://zernio.com/api/v1".to_string(),
        }
    }

    fn auth_header(&self) -> String {
        format!("Bearer {}", self.api_key)
    }

    async fn get_json<T: for<'de> Deserialize<'de>>(&self, path: &str, params: Option<&HashMap<&str, String>>) -> Result<T> {
        let mut req = self
            .client
            .get(format!("{}{}", self.base_url, path))
            .header("Authorization", self.auth_header())
            .header("Content-Type", "application/json");

        if let Some(p) = params {
            req = req.query(p);
        }

        let resp = req.send().await.map_err(|e| ZernioError::Network(e.to_string()))?;
        let status = resp.status();
        let body = resp.text().await.map_err(|e| ZernioError::Network(e.to_string()))?;

        if !status.is_success() {
            return Err(ZernioError::Http(status, body));
        }

        serde_json::from_str(&body).map_err(|e| ZernioError::Deserialize(format!("{} body: {}", e, &body[..body.len().min(200)])))
    }

    async fn post_json<T: for<'de> Deserialize<'de>, B: Serialize>(&self, path: &str, body: &B) -> Result<T> {
        let resp = self
            .client
            .post(format!("{}{}", self.base_url, path))
            .header("Authorization", self.auth_header())
            .header("Content-Type", "application/json")
            .json(body)
            .send()
            .await
            .map_err(|e| ZernioError::Network(e.to_string()))?;

        let status = resp.status();
        let body_text = resp.text().await.map_err(|e| ZernioError::Network(e.to_string()))?;

        if !status.is_success() {
            return Err(ZernioError::Http(status, body_text));
        }

        serde_json::from_str(&body_text).map_err(|e| ZernioError::Deserialize(format!("{} body: {}", e, &body_text[..body_text.len().min(200)])))
    }

    // ── Profiles ───────────────────────────────────────────────────────────

    pub async fn create_profile(&self, name: &str, description: Option<&str>) -> Result<CreateProfileResponse> {
        info!("Creating Zernio profile: {}", name);
        let body = CreateProfileRequest {
            name: name.to_string(),
            description: description.map(|s| s.to_string()),
        };
        self.post_json("/profiles", &body).await
    }

    pub async fn list_profiles(&self) -> Result<ListProfilesResponse> {
        info!("Listing Zernio profiles");
        self.get_json::<ListProfilesResponse>("/profiles", None).await
    }

    // ── Connect / OAuth ────────────────────────────────────────────────────

    pub async fn get_connect_url(&self, platform: &str, profile_id: &str, redirect_url: Option<&str>) -> Result<ConnectUrlResponse> {
        info!("Getting Zernio connect URL for platform {} profile {}", platform, profile_id);
        let mut params = HashMap::new();
        params.insert("profileId", profile_id.to_string());
        if let Some(url) = redirect_url {
            params.insert("redirectUrl", url.to_string());
        }
        self.get_json::<ConnectUrlResponse>(&format!("/connect/{}", platform), Some(&params)).await
    }

    // ── Accounts ───────────────────────────────────────────────────────────

    pub async fn list_accounts(&self, profile_id: Option<&str>) -> Result<ListAccountsResponse> {
        info!("Listing Zernio accounts");
        let params = profile_id.map(|pid| {
            let mut p = HashMap::new();
            p.insert("profileId", pid.to_string());
            p
        });
        self.get_json::<ListAccountsResponse>("/accounts", params.as_ref()).await
    }

    // ── Posts ──────────────────────────────────────────────────────────────

    pub async fn create_post(&self, req: &CreatePostRequest) -> Result<CreatePostResponse> {
        let target_count = req.platforms.len();
        info!("Creating Zernio post for {} platform(s), publishNow: {}", target_count, req.publishNow);
        self.post_json("/posts", req).await
    }

    /// Get a post by ID (full detail with per-platform status).
    pub async fn get_post(&self, post_id: &str) -> Result<GetPostResponse> {
        info!("Fetching Zernio post: {}", post_id);
        self.get_json::<GetPostResponse>(&format!("/posts/{}", post_id), None).await
    }

    /// Retry a failed or partially-published post.
    pub async fn retry_post(&self, post_id: &str) -> Result<RetryPostResponse> {
        info!("Retrying Zernio post: {}", post_id);
        self.post_json::<RetryPostResponse, serde_json::Value>(&format!("/posts/{}/retry", post_id), &serde_json::json!({})).await
    }

    /// Convenience: publish text+media to multiple accounts immediately.
    pub async fn publish_to_accounts(
        &self,
        profile_id: &str,
        text: &str,
        account_targets: Vec<PlatformTarget>,
        media_urls: Vec<String>,
    ) -> Result<CreatePostResponse> {
        let media_items: Vec<MediaItem> = media_urls
            .into_iter()
            .map(|url| {
                let is_video = url.contains(".mp4")
                    || url.contains(".webm")
                    || url.contains(".mov")
                    || url.contains("video");
                MediaItem {
                    r#type: if is_video { "video".to_string() } else { "image".to_string() },
                    url,
                }
            })
            .collect();
        let req = CreatePostRequest {
            content: Some(text.to_string()),
            platforms: account_targets,
            profileId: Some(profile_id.to_string()),
            media_items: Some(media_items),
            scheduledFor: None,
            publishNow: true,
        };
        self.create_post(&req).await
    }

    // ── Media ──────────────────────────────────────────────────────────────

    pub async fn upload_media(&self, url: &str, account_id: Option<&str>) -> Result<UploadMediaResponse> {
        info!("Uploading media to Zernio from: {}", url);
        let body = UploadMediaRequest {
            url: url.to_string(),
            accountId: account_id.map(|s| s.to_string()),
        };
        self.post_json("/media/upload", &body).await
    }

    // ── Health ─────────────────────────────────────────────────────────────

    pub async fn health_check(&self) -> bool {
        match self.client.get("https://zernio.com/api/v1/user")
            .header("Authorization", self.auth_header())
            .send()
            .await
        {
            Ok(resp) => resp.status().is_success(),
            Err(e) => {
                error!("Zernio health check failed: {}", e);
                false
            }
        }
    }
}
