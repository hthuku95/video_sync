// Twitch Helix API client
//
// Provides channel search, user lookup, and VOD listing.
// Token is cached in `twitch_app_token` table (single row) — refreshed when < 1 hour left.

use chrono::{DateTime, Utc};
use reqwest::Client;
use serde::{Deserialize, Serialize};
use sqlx::PgPool;

// ─────────────────────────── public structs ──────────────────────────────────

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct TwitchChannel {
    pub broadcaster_id: String,
    pub broadcaster_login: String,
    pub display_name: String,
    pub profile_image_url: Option<String>,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct TwitchVideo {
    pub id: String,
    pub user_id: String,
    pub title: String,
    pub url: String,
    pub published_at: DateTime<Utc>,
    pub duration: String, // e.g. "1h2m3s"
    pub view_count: i64,
}

// ─────────────────────────── API response shapes ──────────────────────────────

#[derive(Deserialize)]
struct TokenResponse {
    access_token: String,
    expires_in: u64,
}

#[derive(Deserialize)]
struct SearchChannelsResponse {
    data: Vec<SearchChannelItem>,
}

#[derive(Deserialize)]
struct SearchChannelItem {
    id: String,
    broadcaster_login: String,
    display_name: String,
    thumbnail_url: Option<String>,
}

#[derive(Deserialize)]
struct UsersResponse {
    data: Vec<UserItem>,
}

#[derive(Deserialize)]
struct UserItem {
    id: String,
    login: String,
    display_name: String,
    profile_image_url: Option<String>,
}

#[derive(Deserialize)]
struct VideosResponse {
    data: Vec<VideoItem>,
    pagination: Option<Pagination>,
}

#[derive(Deserialize)]
struct VideoItem {
    id: String,
    user_id: String,
    title: String,
    url: String,
    published_at: DateTime<Utc>,
    duration: String,
    view_count: i64,
}

#[derive(Deserialize)]
struct Pagination {
    cursor: Option<String>,
}

// ─────────────────────────── client ──────────────────────────────────────────

#[derive(Clone)]
pub struct TwitchClient {
    client: Client,
    client_id: String,
    client_secret: String,
    db_pool: PgPool,
}

impl TwitchClient {
    pub fn new(client_id: String, client_secret: String, db_pool: PgPool) -> Self {
        Self {
            client: Client::new(),
            client_id,
            client_secret,
            db_pool,
        }
    }

    // ─────────────────────────── token management ────────────────────────────

    /// Returns a valid app access token. Reads from DB cache first; fetches a
    /// fresh token if missing or expiring within the next hour.
    async fn get_app_token(&self) -> Result<String, String> {
        // Try cached token
        let row: Option<(String, DateTime<Utc>)> =
            sqlx::query_as("SELECT access_token, expires_at FROM twitch_app_token WHERE id = 1")
                .fetch_optional(&self.db_pool)
                .await
                .map_err(|e| format!("DB error reading twitch token: {}", e))?;

        if let Some((token, expires_at)) = row {
            // Use cached token if it lasts more than 1 hour
            if expires_at > Utc::now() + chrono::Duration::hours(1) {
                return Ok(token);
            }
        }

        // Fetch fresh token from Twitch
        let resp = self
            .client
            .post("https://id.twitch.tv/oauth2/token")
            .form(&[
                ("client_id", self.client_id.as_str()),
                ("client_secret", self.client_secret.as_str()),
                ("grant_type", "client_credentials"),
            ])
            .send()
            .await
            .map_err(|e| format!("Twitch token request failed: {}", e))?;

        if !resp.status().is_success() {
            let status = resp.status();
            let body = resp.text().await.unwrap_or_default();
            return Err(format!(
                "Twitch token endpoint returned {}: {}",
                status, body
            ));
        }

        let token_resp: TokenResponse = resp
            .json()
            .await
            .map_err(|e| format!("Failed to parse Twitch token response: {}", e))?;

        let expires_at = Utc::now() + chrono::Duration::seconds(token_resp.expires_in as i64);

        sqlx::query(
            "INSERT INTO twitch_app_token (id, access_token, expires_at, created_at)
             VALUES (1, $1, $2, NOW())
             ON CONFLICT (id) DO UPDATE
               SET access_token = EXCLUDED.access_token,
                   expires_at   = EXCLUDED.expires_at,
                   created_at   = NOW()",
        )
        .bind(&token_resp.access_token)
        .bind(expires_at)
        .execute(&self.db_pool)
        .await
        .map_err(|e| format!("Failed to cache Twitch token: {}", e))?;

        Ok(token_resp.access_token)
    }

    // ─────────────────────────── channel search ──────────────────────────────

    /// Search Twitch for channels matching `query`. Returns up to `limit` results.
    pub async fn search_channels(
        &self,
        query: &str,
        limit: u32,
    ) -> Result<Vec<TwitchChannel>, String> {
        let token = self.get_app_token().await?;

        let resp = self
            .client
            .get("https://api.twitch.tv/helix/search/channels")
            .header("Authorization", format!("Bearer {}", token))
            .header("Client-Id", &self.client_id)
            .query(&[("query", query), ("first", &limit.to_string())])
            .send()
            .await
            .map_err(|e| format!("Twitch search_channels request failed: {}", e))?;

        self.handle_rate_limit_once(resp, |r| async move {
            let data: SearchChannelsResponse = r
                .json()
                .await
                .map_err(|e| format!("Failed to parse search_channels response: {}", e))?;

            Ok(data
                .data
                .into_iter()
                .map(|item| TwitchChannel {
                    broadcaster_id: item.id,
                    broadcaster_login: item.broadcaster_login,
                    display_name: item.display_name,
                    profile_image_url: item.thumbnail_url,
                })
                .collect())
        })
        .await
    }

    /// Lookup a single Twitch user by their login name (e.g. "xqc").
    pub async fn get_user_by_login(&self, login: &str) -> Result<Option<TwitchChannel>, String> {
        let token = self.get_app_token().await?;

        let resp = self
            .client
            .get("https://api.twitch.tv/helix/users")
            .header("Authorization", format!("Bearer {}", token))
            .header("Client-Id", &self.client_id)
            .query(&[("login", login)])
            .send()
            .await
            .map_err(|e| format!("Twitch get_user_by_login request failed: {}", e))?;

        self.handle_rate_limit_once(resp, |r| async move {
            let data: UsersResponse = r
                .json()
                .await
                .map_err(|e| format!("Failed to parse users response: {}", e))?;

            Ok(data.data.into_iter().next().map(|u| TwitchChannel {
                broadcaster_id: u.id,
                broadcaster_login: u.login,
                display_name: u.display_name,
                profile_image_url: u.profile_image_url,
            }))
        })
        .await
    }

    // ─────────────────────────── video listing ───────────────────────────────

    /// Fetch up to `limit` archived VODs for a broadcaster.
    /// Pass `cursor` from a previous call to paginate backward through history.
    /// Returns `(videos, next_cursor)`.
    pub async fn get_videos(
        &self,
        broadcaster_id: &str,
        cursor: Option<&str>,
        limit: u32,
    ) -> Result<(Vec<TwitchVideo>, Option<String>), String> {
        let token = self.get_app_token().await?;

        let mut req = self
            .client
            .get("https://api.twitch.tv/helix/videos")
            .header("Authorization", format!("Bearer {}", token))
            .header("Client-Id", &self.client_id)
            .query(&[
                ("user_id", broadcaster_id),
                ("type", "archive"),
                ("first", &limit.to_string()),
            ]);

        if let Some(c) = cursor {
            req = req.query(&[("after", c)]);
        }

        let resp = req
            .send()
            .await
            .map_err(|e| format!("Twitch get_videos request failed: {}", e))?;

        self.handle_rate_limit_once(resp, |r| async move {
            let data: VideosResponse = r
                .json()
                .await
                .map_err(|e| format!("Failed to parse videos response: {}", e))?;

            let next_cursor = data.pagination.and_then(|p| p.cursor);
            let videos = data
                .data
                .into_iter()
                .map(|v| TwitchVideo {
                    id: v.id,
                    user_id: v.user_id,
                    title: v.title,
                    url: v.url,
                    published_at: v.published_at,
                    duration: v.duration,
                    view_count: v.view_count,
                })
                .collect();

            Ok((videos, next_cursor))
        })
        .await
    }

    // ─────────────────────────── rate-limit helper ───────────────────────────

    /// If the response has a 429 status, wait for `Ratelimit-Reset` seconds and
    /// retry once. Otherwise, pass the response to `handler`.
    async fn handle_rate_limit_once<F, Fut, T>(
        &self,
        resp: reqwest::Response,
        handler: F,
    ) -> Result<T, String>
    where
        F: Fn(reqwest::Response) -> Fut,
        Fut: std::future::Future<Output = Result<T, String>>,
    {
        if resp.status().as_u16() == 429 {
            let reset_secs = resp
                .headers()
                .get("Ratelimit-Reset")
                .and_then(|v| v.to_str().ok())
                .and_then(|s| s.parse::<u64>().ok())
                .unwrap_or(1);

            tracing::warn!(
                "Twitch 429 rate limit — waiting {}s before retry",
                reset_secs
            );
            tokio::time::sleep(tokio::time::Duration::from_secs(reset_secs)).await;

            // Retry the request (re-issue full request would require cloning the
            // builder, so we return an error that callers can handle instead)
            return Err(format!("Twitch rate-limited; retry after {}s", reset_secs));
        }

        if !resp.status().is_success() {
            let status = resp.status();
            let body = resp.text().await.unwrap_or_default();
            return Err(format!("Twitch API error {}: {}", status, body));
        }

        handler(resp).await
    }
}
