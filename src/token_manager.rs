use crate::models::youtube::ConnectedYouTubeChannel;
use crate::youtube_client::YouTubeClient;
use chrono::{Duration, Utc};
use sqlx::PgPool;
use backoff::{ExponentialBackoff, backoff::Backoff};
use std::time::Duration as StdDuration;

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

    /// Check if an error is retryable (network/transient errors)
    fn is_retryable_error(error_str: &str) -> bool {
        // Don't retry on permanent errors
        if error_str.contains("REFRESH_TOKEN_EXPIRED")
            || error_str.contains("invalid_grant")
            || error_str.contains("INVALID_CREDENTIALS")
            || error_str.contains("invalid_client") {
            return false;
        }

        // Retry on network errors, timeouts, 5xx errors
        error_str.contains("network")
            || error_str.contains("timeout")
            || error_str.contains("connection")
            || error_str.contains("500")
            || error_str.contains("502")
            || error_str.contains("503")
            || error_str.contains("504")
    }

    /// Refresh token with exponential backoff retry
    async fn refresh_with_retry(
        &self,
        refresh_token: &str,
        channel_name: &str,
    ) -> Result<crate::youtube_client::TokenRefreshResponse, Box<dyn std::error::Error + Send + Sync>> {
        let mut backoff = ExponentialBackoff {
            max_elapsed_time: Some(StdDuration::from_secs(30)),
            ..Default::default()
        };

        let mut attempt = 1;
        loop {
            match self.youtube_client.refresh_access_token(
                refresh_token,
                &self.oauth_client_id,
                &self.oauth_client_secret,
            ).await {
                Ok(response) => {
                    if attempt > 1 {
                        tracing::info!("✅ Token refresh succeeded on attempt {} for channel: {}", attempt, channel_name);
                    }
                    return Ok(response);
                }
                Err(e) => {
                    let error_str = e.to_string();

                    // Check if error is retryable
                    if !Self::is_retryable_error(&error_str) {
                        tracing::error!("🔴 TokenManager: Non-retryable error for channel {}: {}", channel_name, error_str);
                        return Err(e);
                    }

                    // Check if we should retry
                    if let Some(duration) = backoff.next_backoff() {
                        tracing::warn!(
                            "⚠️ TokenManager: Attempt {} failed for channel {}. Retrying in {:?}...",
                            attempt,
                            channel_name,
                            duration
                        );
                        tokio::time::sleep(duration).await;
                        attempt += 1;
                    } else {
                        tracing::error!(
                            "🔴 TokenManager: Max retries exceeded for channel {}: {}",
                            channel_name,
                            error_str
                        );
                        return Err(e);
                    }
                }
            }
        }
    }

    /// Ensure token is fresh, refresh if expiring within 30 minutes
    /// Returns (did_refresh, access_token)
    pub async fn ensure_fresh_token(
        &self,
        channel: &mut ConnectedYouTubeChannel,
    ) -> Result<bool, Box<dyn std::error::Error + Send + Sync>> {
        let now = Utc::now();
        let expires_soon = channel.token_expiry < now + Duration::minutes(30);

        if expires_soon {
            tracing::info!(
                "🔄 Refreshing expired token for channel: {}",
                channel.channel_name
            );

            // Use retry wrapper for token refresh
            let token_response = self.refresh_with_retry(
                &channel.refresh_token,
                &channel.channel_name,
            ).await?;

            // Update in-memory channel
            channel.access_token = token_response.access_token.clone();
            channel.token_expiry = now + Duration::seconds(token_response.expires_in);

            // CRITICAL: If Google provides a new refresh token, we MUST use it
            // This is part of Google's token rotation security policy
            if let Some(new_refresh_token) = &token_response.refresh_token {
                tracing::info!(
                    "🔄 Google provided new refresh token for channel: {} - updating storage",
                    channel.channel_name
                );
                channel.refresh_token = new_refresh_token.clone();
            }

            // Update database and clear requires_reauth flag on successful refresh
            sqlx::query(
                "UPDATE connected_youtube_channels
                 SET access_token = $1,
                     token_expiry = $2,
                     refresh_token = $3,
                     requires_reauth = false,
                     reauth_reason = NULL,
                     updated_at = NOW()
                 WHERE id = $4",
            )
            .bind(&channel.access_token)
            .bind(channel.token_expiry)
            .bind(&channel.refresh_token)
            .bind(channel.id)
            .execute(&self.db_pool)
            .await?;

            tracing::info!("✅ Token refreshed successfully for channel: {}", channel.channel_name);
            return Ok(true);
        }

        Ok(false)
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

        self.ensure_fresh_token(&mut channel).await?;
        Ok(channel.access_token.clone())
    }

    /// Refresh tokens for all channels that expire within the next hour
    /// Returns the count of successfully refreshed tokens
    pub async fn refresh_all_expiring_tokens(&self) -> Result<usize, String> {
        let now = Utc::now();
        let threshold = now + Duration::hours(1);

        // Fetch all channels with tokens expiring soon
        let channels = sqlx::query_as::<_, ConnectedYouTubeChannel>(
            "SELECT * FROM connected_youtube_channels
             WHERE is_active = true
             AND token_expiry < $1
             ORDER BY token_expiry ASC"
        )
        .bind(threshold)
        .fetch_all(&self.db_pool)
        .await
        .map_err(|e| format!("Failed to fetch expiring channels: {}", e))?;

        if channels.is_empty() {
            return Ok(0);
        }

        tracing::info!("🔄 Found {} channels with tokens expiring within 1 hour", channels.len());

        let mut refreshed_count = 0;

        for mut channel in channels {
            match self.ensure_fresh_token(&mut channel).await {
                Ok(true) => {
                    refreshed_count += 1;
                    tracing::info!(
                        "✅ Proactively refreshed token for channel: {} ({})",
                        channel.channel_name,
                        channel.channel_id
                    );
                }
                Ok(false) => {
                    // Token still fresh, skip (shouldn't happen with our query but handle anyway)
                }
                Err(e) => {
                    let error_str = e.to_string();
                    tracing::warn!(
                        "⚠️ Failed to refresh token for channel {} ({}): {}",
                        channel.channel_name,
                        channel.channel_id,
                        error_str
                    );

                    // Mark channel as requiring re-auth when refresh token is permanently invalid
                    if error_str.contains("invalid_grant")
                        || error_str.contains("REFRESH_TOKEN_EXPIRED")
                        || error_str.contains("INVALID_CREDENTIALS")
                        || error_str.contains("token has been expired or revoked")
                    {
                        let _ = sqlx::query(
                            "UPDATE connected_youtube_channels
                             SET requires_reauth = true,
                                 reauth_reason = 'YouTube authorization expired. Please reconnect your channel.',
                                 updated_at = NOW()
                             WHERE id = $1",
                        )
                        .bind(channel.id)
                        .execute(&self.db_pool)
                        .await;

                        tracing::warn!(
                            "🔴 Marked channel {} (id={}) as requires_reauth=true (invalid_grant)",
                            channel.channel_name,
                            channel.id
                        );
                    }
                    // Don't fail the entire batch - continue with other channels
                }
            }
        }

        Ok(refreshed_count)
    }
}
