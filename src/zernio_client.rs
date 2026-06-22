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
    pub url: String,
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
    pub username: Option<String>,
    pub profile_id: Option<String>,
    pub connected: bool,
}

#[derive(Debug, Serialize)]
pub struct PlatformTarget {
    pub platform: String,
    pub accountId: String,
}

#[derive(Debug, Serialize)]
pub struct CreatePostRequest {
    #[serde(skip_serializing_if = "Option::is_none")]
    pub content: Option<String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub text: Option<String>,
    pub platforms: Vec<PlatformTarget>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub profileId: Option<String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub mediaUrls: Option<Vec<String>>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub scheduledFor: Option<String>,
    pub publishNow: bool,
}

#[derive(Debug, Deserialize, Serialize)]
pub struct CreatePostResponse {
    pub post: ZernioPost,
}

#[derive(Debug, Deserialize, Serialize)]
pub struct ZernioPost {
    #[serde(rename = "_id")]
    pub id: String,
    pub status: Option<String>,
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

    pub async fn get_connect_url(&self, platform: &str, profile_id: &str) -> Result<ConnectUrlResponse> {
        info!("Getting Zernio connect URL for platform {} profile {}", platform, profile_id);
        let mut params = HashMap::new();
        params.insert("profileId", profile_id.to_string());
        self.get_json::<ConnectUrlResponse>(&format!("/connect/{}", platform), Some(&params)).await
    }

    // ── Accounts ───────────────────────────────────────────────────────────

    pub async fn list_accounts(&self) -> Result<ListAccountsResponse> {
        info!("Listing Zernio accounts");
        self.get_json::<ListAccountsResponse>("/accounts", None).await
    }

    // ── Posts ──────────────────────────────────────────────────────────────

    pub async fn create_post(&self, req: &CreatePostRequest) -> Result<CreatePostResponse> {
        let target_count = req.platforms.len();
        info!("Creating Zernio post for {} platform(s), publishNow: {}", target_count, req.publishNow);
        self.post_json("/posts", req).await
    }

    /// Convenience: publish text+media to multiple accounts immediately.
    pub async fn publish_to_accounts(
        &self,
        profile_id: &str,
        text: &str,
        account_targets: Vec<PlatformTarget>,
        media_urls: Vec<String>,
    ) -> Result<CreatePostResponse> {
        let req = CreatePostRequest {
            content: Some(text.to_string()),
            text: None,
            platforms: account_targets,
            profileId: Some(profile_id.to_string()),
            mediaUrls: Some(media_urls),
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
