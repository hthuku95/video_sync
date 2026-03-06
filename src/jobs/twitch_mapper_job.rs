// Periodic cron job: auto-map unmapped YouTube source channels to Twitch.
//
// Runs every 10 minutes. Processes at most 10 unmapped channels per cycle to
// avoid burning Gemini quota (2s delay between calls).

use crate::AppState;
use crate::clipping::models::SourceChannel;
use crate::services::twitch_mapper::{MappingResult, auto_map_youtube_to_twitch};
use std::sync::Arc;
use tokio::time::Duration;

pub async fn run_twitch_mapping_cron(app_state: Arc<AppState>) {
    let mut interval = tokio::time::interval(Duration::from_secs(600)); // 10 minutes
    interval.tick().await; // skip immediate first tick

    loop {
        interval.tick().await;

        match (&app_state.twitch_client, &app_state.gemini_client) {
            (Some(twitch), Some(gemini)) => {
                run_mapping_pass(twitch, gemini, &app_state.db_pool).await;
            }
            _ => {
                // Either Twitch or Gemini is not configured — skip silently
            }
        }
    }
}

async fn run_mapping_pass(
    twitch: &crate::twitch_client::TwitchClient,
    gemini: &crate::gemini_client::GeminiClient,
    db: &sqlx::PgPool,
) {
    let unmapped: Vec<SourceChannel> = match sqlx::query_as(
        "SELECT * FROM youtube_source_channels
         WHERE twitch_mapping_status = 'unmapped' AND is_active = true
         ORDER BY created_at ASC
         LIMIT 10",
    )
    .fetch_all(db)
    .await
    {
        Ok(rows) => rows,
        Err(e) => {
            tracing::warn!("Twitch mapper cron: failed to fetch unmapped channels: {}", e);
            return;
        }
    };

    if unmapped.is_empty() {
        tracing::debug!("Twitch mapper cron: no unmapped YouTube channels");
        return;
    }

    tracing::info!(
        "Twitch mapper cron: processing {} unmapped YouTube channel(s)",
        unmapped.len()
    );

    for channel in &unmapped {
        match auto_map_youtube_to_twitch(channel, twitch, gemini, db).await {
            Ok(MappingResult::Mapped(tc)) => {
                tracing::info!(
                    "Twitch mapper: mapped '{}' → Twitch:{} ({})",
                    channel.channel_name,
                    tc.broadcaster_login,
                    tc.display_name
                );
            }
            Ok(MappingResult::NoEquivalent) => {
                tracing::info!(
                    "Twitch mapper: no Twitch equivalent for '{}'",
                    channel.channel_name
                );
            }
            Err(e) => {
                tracing::warn!(
                    "Twitch mapper: failed to map '{}': {}",
                    channel.channel_name,
                    e
                );
            }
        }

        // 2-second delay between Gemini calls to avoid 429
        tokio::time::sleep(Duration::from_secs(2)).await;
    }
}
