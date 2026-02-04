use crate::models::youtube::ConnectedYouTubeChannel;
use crate::youtube_client::YouTubeClient;
use chrono::{Duration, Utc};
use sqlx::PgPool;

/// Token manager for centralized YouTube OAuth token refresh
pub struct TokenManager {
    youtube_client: YouTubeClient,
    oauth_client_id: String,
    oauth_client_secret: String,
    db_pool: PgPool,
}

impl TokenManager {
    pub fn new(
        youtube_client: YouTubeClient,
        oauth_client_id: String,
        oauth_client_secret: String,
        db_pool: PgPool,
    ) -> Self {
        Self {
            youtube_client,
            oauth_client_id,
            oauth_client_secret,
            db_pool,
        }
    }

    /// Ensure token is fresh, refresh if expiring within 5 minutes
    /// Returns the valid access token
    pub async fn ensure_fresh_token(
        &self,
        channel: &mut ConnectedYouTubeChannel,
    ) -> Result<String, Box<dyn std::error::Error + Send + Sync>> {
        let now = Utc::now();
        let expires_soon = channel.token_expiry < now + Duration::minutes(5);

        if expires_soon {
            tracing::info!(
                "🔄 Refreshing expired token for channel: {}",
                channel.channel_name
            );

            let token_response = self
                .youtube_client
                .refresh_access_token(
                    &channel.refresh_token,
                    &self.oauth_client_id,
                    &self.oauth_client_secret,
                )
                .await?;

            // Update in-memory channel
            channel.access_token = token_response.access_token.clone();
            channel.token_expiry = now + Duration::seconds(token_response.expires_in);

            // Update database
            sqlx::query(
                "UPDATE connected_youtube_channels
                 SET access_token = $1, token_expiry = $2, updated_at = NOW()
                 WHERE id = $3",
            )
            .bind(&channel.access_token)
            .bind(channel.token_expiry)
            .bind(channel.id)
            .execute(&self.db_pool)
            .await?;

            tracing::info!("✅ Token refreshed successfully for channel: {}", channel.channel_name);
        }

        Ok(channel.access_token.clone())
    }

    /// Refresh a token by channel ID, returns the fresh token
    pub async fn refresh_token_by_id(
        &self,
        channel_id: i32,
    ) -> Result<String, Box<dyn std::error::Error + Send + Sync>> {
        let mut channel = sqlx::query_as::<_, ConnectedYouTubeChannel>(
            "SELECT * FROM connected_youtube_channels WHERE id = $1"
        )
        .bind(channel_id)
        .fetch_one(&self.db_pool)
        .await?;

        self.ensure_fresh_token(&mut channel).await
    }
}
