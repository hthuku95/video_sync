// Periodic cron job: auto-map unmapped YouTube source channels to Twitch.
//
// Runs every 10 minutes. Processes at most 3 unmapped channels per cycle to
// stay well within the Gemini free-tier RPM limit (15s delay between calls).

use crate::clipping::models::SourceChannel;
use crate::services::twitch_mapper::{auto_map_youtube_to_twitch, MappingResult};
use crate::AppState;
use std::sync::Arc;
use tokio::time::Duration;

pub async fn run_twitch_mapping_cron(app_state: Arc<AppState>) {
    let mut interval = tokio::time::interval(Duration::from_secs(600)); // 10 minutes
    interval.tick().await; // skip immediate first tick

    loop {
        interval.tick().await;

        if let Some(twitch) = &app_state.twitch_client {
            run_mapping_pass(twitch, &app_state).await;
        }
    }
}

async fn run_mapping_pass(
    twitch: &crate::twitch_client::TwitchClient,
    app_state: &AppState,
) {
    let unmapped: Vec<SourceChannel> = match sqlx::query_as(
        "SELECT * FROM youtube_source_channels
         WHERE twitch_mapping_status = 'unmapped' AND is_active = true
         ORDER BY created_at ASC
         LIMIT 3",
    )
    .fetch_all(db)
    .await
    {
        Ok(rows) => rows,
        Err(e) => {
            tracing::warn!(
                "Twitch mapper cron: failed to fetch unmapped channels: {}",
                e
            );
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
        match auto_map_youtube_to_twitch(channel, twitch, app_state, &app_state.db_pool).await {
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

        // 15-second delay between Gemini calls — free tier is 10 RPM shared with clipping worker
        tokio::time::sleep(Duration::from_secs(15)).await;
    }
}
