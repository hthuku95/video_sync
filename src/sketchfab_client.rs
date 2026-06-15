/// Sketchfab client — Data API v3 for searching 3D models.
/// API: https://docs.sketchfab.com/data-api/v3/index.html
///
/// Configurable via SKETCHFAB_API_KEY env var (API Token or OAuth2).
use reqwest::Client;
use serde::{Deserialize, Serialize};
use std::collections::HashMap;

pub const SKETCHFAB_API_URL: &str = "https://api.sketchfab.com/v3";

#[derive(Debug, Clone)]
pub struct SketchfabClient {
    client: Client,
    api_key: String,
}

// ── Search Response Types ─────────────────────────────────────────────

#[derive(Debug, Deserialize, Serialize)]
pub struct SketchfabSearchResponse {
    pub results: Vec<SketchfabModel>,
    pub next: Option<String>,
    pub previous: Option<String>,
    pub cursors: Option<SketchfabCursors>,
}

#[derive(Debug, Deserialize, Serialize)]
pub struct SketchfabCursors {
    pub next: Option<String>,
    pub previous: Option<String>,
}

#[derive(Debug, Deserialize, Serialize, Clone)]
pub struct SketchfabModel {
    pub uid: String,
    pub name: String,
    #[serde(default)]
    pub description: String,
    pub viewer_url: Option<String>,
    pub thumbnails: Option<Vec<SketchfabThumbnail>>,
    pub user: Option<SketchfabUser>,
    #[serde(default)]
    pub tags: Vec<SketchfabTag>,
    #[serde(default)]
    pub categories: Vec<SketchfabCategory>,
    pub like_count: Option<i64>,
    pub view_count: Option<i64>,
    pub comment_count: Option<i64>,
    pub license: Option<SketchfabLicense>,
    pub published_at: Option<String>,
    pub created_at: Option<String>,
    pub is_animated: Option<bool>,
    pub is_staff_picked: Option<bool>,
    pub vertex_count: Option<i64>,
    pub face_count: Option<i64>,
}

#[derive(Debug, Deserialize, Serialize, Clone)]
pub struct SketchfabThumbnail {
    pub url: String,
    pub width: Option<i64>,
    pub height: Option<i64>,
}

#[derive(Debug, Deserialize, Serialize, Clone)]
pub struct SketchfabUser {
    pub uid: String,
    pub username: String,
    pub display_name: Option<String>,
    pub profile_url: Option<String>,
    pub avatar: Option<SketchfabThumbnail>,
}

#[derive(Debug, Deserialize, Serialize, Clone)]
pub struct SketchfabTag {
    pub name: String,
    pub slug: String,
}

#[derive(Debug, Deserialize, Serialize, Clone)]
pub struct SketchfabCategory {
    pub name: String,
    pub slug: String,
}

#[derive(Debug, Deserialize, Serialize, Clone)]
pub struct SketchfabLicense {
    pub label: String,
    pub slug: String,
    pub url: Option<String>,
}

// ── Model Detail Types ────────────────────────────────────────────────

#[derive(Debug, Deserialize, Serialize)]
pub struct SketchfabModelDetail {
    pub uid: String,
    pub name: String,
    pub description: String,
    pub viewer_url: Option<String>,
    pub embed_url: Option<String>,
    pub thumbnails: Option<Vec<SketchfabThumbnail>>,
    pub user: Option<SketchfabUser>,
    #[serde(default)]
    pub tags: Vec<SketchfabTag>,
    #[serde(default)]
    pub categories: Vec<SketchfabCategory>,
    pub license: Option<SketchfabLicense>,
    pub like_count: Option<i64>,
    pub view_count: Option<i64>,
    pub comment_count: Option<i64>,
    pub published_at: Option<String>,
    pub created_at: Option<String>,
    pub updated_at: Option<String>,
    pub is_animated: Option<bool>,
    pub is_staff_picked: Option<bool>,
    pub vertex_count: Option<i64>,
    pub face_count: Option<i64>,
    pub animation_count: Option<i64>,
    pub sound_count: Option<i64>,
    pub is_age_filtered: Option<bool>,
    pub is_purchased: Option<bool>,
    pub is_published: Option<bool>,
    pub password_protected: Option<bool>,
}

// ── Error Types ───────────────────────────────────────────────────────

#[derive(Debug)]
pub enum SketchfabError {
    NoApiKey,
    Network(String),
    Api { status: u16, body: String },
    Parse(String),
}

impl std::fmt::Display for SketchfabError {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        match self {
            Self::NoApiKey => write!(f, "SKETCHFAB_API_KEY environment variable not set"),
            Self::Network(e) => write!(f, "Network: {}", e),
            Self::Api { status, body } => write!(f, "API {}: {}", status, body),
            Self::Parse(e) => write!(f, "Parse: {}", e),
        }
    }
}

impl std::error::Error for SketchfabError {}

impl SketchfabClient {
    pub fn from_env() -> Result<Self, SketchfabError> {
        let api_key = std::env::var("SKETCHFAB_API_KEY")
            .map_err(|_| SketchfabError::NoApiKey)?;
        Ok(Self {
            client: Client::builder()
                .timeout(std::time::Duration::from_secs(30))
                .build()
                .map_err(|e| SketchfabError::Network(e.to_string()))?,
            api_key,
        })
    }

    pub fn new(api_key: String) -> Self {
        Self {
            client: Client::builder()
                .timeout(std::time::Duration::from_secs(30))
                .build()
                .unwrap_or_default(),
            api_key,
        }
    }

    /// Search for 3D models on Sketchfab.
    ///
    /// Supported filters:
    /// - `query`: search keyword
    /// - `categories`: comma-separated category slugs (e.g. "animals-pets,architecture")
    /// - `sort_by`: "-likeCount", "-viewCount", "-createdAt", "-publishedAt"
    /// - `license`: license slug ("by", "by-sa", "by-nd", "by-nc", "by-nc-sa", "by-nc-nd", "cc0")
    /// - `animated`: filter for animated models only ("true"/"false")
    /// - `staffpicked`: filter for staff-picked only ("true"/"false")
    /// - `downloadable`: filter for downloadable only ("true"/"false")
    /// - `count`: results per page (max 24, default 24)
    pub async fn search(
        &self,
        query: &str,
        categories: Option<&str>,
        sort_by: Option<&str>,
        license: Option<&str>,
        animated: Option<bool>,
        staffpicked: Option<bool>,
        downloadable: Option<bool>,
        count: Option<i32>,
    ) -> Result<SketchfabSearchResponse, SketchfabError> {
        let mut params: HashMap<&str, String> = HashMap::new();
        params.insert("q", query.to_string());
        if let Some(c) = categories {
            params.insert("categories", c.to_string());
        }
        if let Some(s) = sort_by {
            params.insert("sort_by", s.to_string());
        }
        if let Some(l) = license {
            params.insert("license", l.to_string());
        }
        if let Some(a) = animated {
            params.insert("animated", a.to_string());
        }
        if let Some(s) = staffpicked {
            params.insert("staffpicked", s.to_string());
        }
        if let Some(d) = downloadable {
            params.insert("downloadable", d.to_string());
        }
        if let Some(c) = count {
            params.insert("count", c.to_string());
        }

        let resp = self
            .client
            .get(format!("{}/models", SKETCHFAB_API_URL))
            .header("Authorization", format!("Token {}", self.api_key))
            .query(&params)
            .send()
            .await
            .map_err(|e| SketchfabError::Network(e.to_string()))?;

        let status = resp.status();
        if !status.is_success() {
            let body = resp.text().await.unwrap_or_default();
            return Err(SketchfabError::Api {
                status: status.as_u16(),
                body,
            });
        }

        resp.json::<SketchfabSearchResponse>()
            .await
            .map_err(|e| SketchfabError::Parse(e.to_string()))
    }

    /// Get detailed information about a specific model by UID.
    pub async fn get_model(
        &self,
        uid: &str,
    ) -> Result<SketchfabModelDetail, SketchfabError> {
        let resp = self
            .client
            .get(format!("{}/models/{}", SKETCHFAB_API_URL, uid))
            .header("Authorization", format!("Token {}", self.api_key))
            .send()
            .await
            .map_err(|e| SketchfabError::Network(e.to_string()))?;

        let status = resp.status();
        if !status.is_success() {
            let body = resp.text().await.unwrap_or_default();
            return Err(SketchfabError::Api {
                status: status.as_u16(),
                body,
            });
        }

        resp.json::<SketchfabModelDetail>()
            .await
            .map_err(|e| SketchfabError::Parse(e.to_string()))
    }

    /// Format search results into a human-readable agent response.
    pub fn format_search_results(results: &SketchfabSearchResponse) -> String {
        if results.results.is_empty() {
            return "No models found.".to_string();
        }
        let mut out = format!("Found {} model(s):\n\n", results.results.len());
        for model in &results.results {
            out.push_str(&format!(
                "  - {} (UID: {})\n    by {}\n    {} likes | {} views\n    {}\n\n",
                model.name,
                model.uid,
                model.user.as_ref().map(|u| &u.username).unwrap_or("unknown"),
                model.like_count.unwrap_or(0),
                model.view_count.unwrap_or(0),
                model.viewer_url.as_deref().unwrap_or("no URL"),
            ));
        }
        out
    }
}
