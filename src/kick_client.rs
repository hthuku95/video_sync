use serde::{Deserialize, Serialize};
use std::sync::Arc;
use tokio::sync::RwLock;
use urlencoding;

#[derive(Debug, Clone)]
pub struct KickClient {
    client: reqwest::Client,
    client_id: String,
    client_secret: String,
    token: Arc<RwLock<Option<AccessTokenResponse>>>,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
struct AccessTokenResponse {
    access_token: String,
    expires_in: i64,
    #[serde(rename = "token_type")]
    _token_type: String,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
struct KickApiResponse<T> {
    data: T,
    message: Option<String>,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct KickChannel {
    pub broadcaster_user_id: i64,
    pub slug: String,
    pub stream_title: Option<String>,
    pub channel_description: Option<String>,
    pub banner_picture: Option<String>,
    pub category: Option<KickCategory>,
    pub stream: Option<KickStream>,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct KickCategory {
    pub id: i64,
    pub name: String,
    pub thumbnail: Option<String>,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct KickStream {
    pub is_live: bool,
    pub key: Option<String>,
    pub language: Option<String>,
    pub viewer_count: Option<i64>,
    pub thumbnail: Option<String>,
    pub url: Option<String>,
    pub start_time: Option<String>,
    pub is_mature: Option<bool>,
    pub custom_tags: Option<Vec<String>>,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct KickLivestream {
    pub broadcaster_user_id: i64,
    pub slug: String,
    pub stream_title: Option<String>,
    pub category: Option<KickCategory>,
    pub language: Option<String>,
    pub viewer_count: Option<i64>,
    pub thumbnail: Option<String>,
    pub profile_picture: Option<String>,
    pub started_at: Option<String>,
    pub has_mature_content: Option<bool>,
    pub custom_tags: Option<Vec<String>>,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct KickVideo {
    pub uuid: String,
    #[serde(default)]
    pub session_title: Option<String>,
    #[serde(default)]
    pub is_live: Option<bool>,
    #[serde(default)]
    pub duration: Option<i64>,
    #[serde(default)]
    pub created_at: Option<String>,
}

impl KickClient {
    /// Recent videos (VODs) for a channel, from Kick's public website API.
    ///
    /// The OAuth API (`api.kick.com/public/v1`) exposes live channels only.
    /// Clipping needs a finished VOD, and the HLS playback URL that live
    /// resolution returns is signed and expires within hours — a Kick source
    /// cached on 16 Oct was dead by the 20th. VOD *pages*
    /// (`kick.com/{slug}/videos/{uuid}`) stay valid, so discovery must return
    /// those.
    pub async fn get_recent_videos(&self, slug: &str) -> Result<Vec<KickVideo>, String> {
        let url = format!("https://kick.com/api/v2/channels/{slug}/videos");
        let resp = self
            .client
            .get(&url)
            .header("Accept", "application/json")
            .header(
                "User-Agent",
                "Mozilla/5.0 (X11; Linux x86_64) AppleWebKit/537.36 (KHTML, like Gecko) Chrome/124.0 Safari/537.36",
            )
            .send()
            .await
            .map_err(|e| format!("Kick videos request failed: {e}"))?;

        if !resp.status().is_success() {
            return Err(format!("Kick videos API returned {}", resp.status()));
        }

        // The endpoint answers with a bare array; older responses wrapped it in
        // `{ "data": [...] }`. Accept both.
        let value: serde_json::Value = resp
            .json()
            .await
            .map_err(|e| format!("Kick videos parse failed: {e}"))?;
        let items = value.get("data").cloned().unwrap_or(value);
        serde_json::from_value(items).map_err(|e| format!("Kick videos decode failed: {e}"))
    }

    /// Newest finished VOD as a durable page URL, if the channel has one.
    pub async fn latest_vod_url(&self, slug: &str) -> Result<Option<String>, String> {
        let videos = self.get_recent_videos(slug).await?;
        Ok(videos
            .into_iter()
            .find(|v| v.is_live != Some(true) && !v.uuid.trim().is_empty())
            .map(|v| format!("https://kick.com/{slug}/videos/{}", v.uuid)))
    }
    pub fn new(client_id: String, client_secret: String) -> Self {
        Self {
            client: reqwest::Client::new(),
            client_id,
            client_secret,
            token: Arc::new(RwLock::new(None)),
        }
    }

    pub fn is_configured(&self) -> bool {
        !self.client_id.is_empty() && !self.client_secret.is_empty()
    }

    async fn ensure_token(&self) -> Result<String, String> {
        {
            let token_guard = self.token.read().await;
            if let Some(token) = token_guard.as_ref() {
                if token.expires_in > 60 {
                    return Ok(token.access_token.clone());
                }
            }
        }

        let params = [
            ("client_id", self.client_id.as_str()),
            ("client_secret", self.client_secret.as_str()),
            ("grant_type", "client_credentials"),
        ];

        let resp = self
            .client
            .post("https://id.kick.com/oauth/token")
            .form(&params)
            .send()
            .await
            .map_err(|e| format!("Kick OAuth request failed: {}", e))?;

        let token_response: AccessTokenResponse = resp
            .json()
            .await
            .map_err(|e| format!("Kick OAuth parse failed: {}", e))?;

        let token = token_response.access_token.clone();
        {
            let mut token_guard = self.token.write().await;
            *token_guard = Some(token_response);
        }

        Ok(token)
    }

    pub async fn get_channel_by_slug(&self, slug: &str) -> Result<Option<KickChannel>, String> {
        let token = self.ensure_token().await?;
        let url = format!("https://api.kick.com/public/v1/channels?slug={}", slug);

        let resp = self
            .client
            .get(&url)
            .header("Authorization", format!("Bearer {}", token))
            .send()
            .await
            .map_err(|e| format!("Kick channel request failed: {}", e))?;

        if resp.status() == 404 {
            return Ok(None);
        }

        let api_resp: KickApiResponse<Vec<KickChannel>> = resp
            .json()
            .await
            .map_err(|e| format!("Kick channel parse failed: {}", e))?;

        Ok(api_resp.data.into_iter().next())
    }

    pub async fn get_livestreams(
        &self,
        category_id: Option<i64>,
        language: Option<&str>,
        limit: Option<i64>,
    ) -> Result<Vec<KickLivestream>, String> {
        let token = self.ensure_token().await?;

        let mut url = "https://api.kick.com/public/v1/livestreams".to_string();
        let mut params: Vec<String> = Vec::new();

        if let Some(cat) = category_id {
            params.push(format!("category_id={}", cat));
        }
        if let Some(lang) = language {
            params.push(format!("language={}", lang));
        }
        params.push(format!("limit={}", limit.unwrap_or(25)));

        if !params.is_empty() {
            url.push('?');
            url.push_str(&params.join("&"));
        }

        let resp = self
            .client
            .get(&url)
            .header("Authorization", format!("Bearer {}", token))
            .send()
            .await
            .map_err(|e| format!("Kick livestreams request failed: {}", e))?;

        let api_resp: KickApiResponse<Vec<KickLivestream>> = resp
            .json()
            .await
            .map_err(|e| format!("Kick livestreams parse failed: {}", e))?;

        Ok(api_resp.data)
    }

    pub async fn search_livestreams_by_category_name(
        &self,
        category_name: &str,
    ) -> Result<Vec<KickLivestream>, String> {
        let token = self.ensure_token().await?;

        let categories_url = format!(
            "https://api.kick.com/public/v1/categories?q={}&page=1",
            urlencoding::encode(category_name)
        );
        let resp = self
            .client
            .get(&categories_url)
            .header("Authorization", format!("Bearer {}", token))
            .send()
            .await
            .map_err(|e| format!("Kick categories request failed: {}", e))?;

        #[derive(Deserialize)]
        struct CategoriesResponse {
            data: Vec<CategoryItem>,
        }
        #[derive(Deserialize)]
        struct CategoryItem {
            id: i64,
            name: String,
        }

        let categories: CategoriesResponse = resp
            .json()
            .await
            .map_err(|e| format!("Kick categories parse failed: {}", e))?;

        let cat = categories
            .data
            .into_iter()
            .find(|c| c.name.to_lowercase() == category_name.to_lowercase());

        match cat {
            Some(category) => self.get_livestreams(Some(category.id), None, Some(100)).await,
            None => Ok(Vec::new()),
        }
    }
}
