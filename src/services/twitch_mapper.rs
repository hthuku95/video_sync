// Gemini-powered YouTube → Twitch channel auto-mapper.
//
// For each unmapped YouTube source channel, ask Gemini whether the creator has
// an official Twitch channel. If yes, look them up on Twitch and create the
// 1:1 mapping automatically.

use crate::clipping::models::{SourceChannel, TwitchSourceChannel};
use crate::twitch_client::TwitchClient;
use crate::AppState;
use sqlx::PgPool;

pub enum MappingResult {
    Mapped(TwitchSourceChannel),
    NoEquivalent,
}

/// Auto-map a single YouTube source channel to its Twitch equivalent (if any).
pub async fn auto_map_youtube_to_twitch(
    youtube_channel: &SourceChannel,
    twitch_client: &TwitchClient,
    state: &AppState,
    db_pool: &PgPool,
) -> Result<MappingResult, String> {
    // 1. Ask LLM (Ollama first via fallback chain) for the Twitch login name
    let prompt = format!(
        "You are a social media research assistant.\n\
         Given this YouTube channel:\n\
         Name: {name}\n\
         Determine whether this creator or brand has an official Twitch channel.\n\
         If yes, return ONLY their Twitch login name (lowercase, no spaces, no punctuation).\n\
         If no, return exactly: NONE\n\
         Do not explain. Just the login name or NONE.",
        name = youtube_channel.channel_name,
    );

    let response = crate::llm_utils::generate_text_fast(
        state.ollama_client.as_ref(),
        state.deepseek_client.as_ref(),
        state.gemini_client.as_ref(),
        &prompt,
    )
    .await
    .map_err(|e| format!("LLM error during mapping: {}", e))?;

    let twitch_login = response.trim().to_lowercase();

    if twitch_login == "none" || twitch_login.is_empty() {
        // Gemini says no Twitch channel
        sqlx::query(
            "UPDATE youtube_source_channels
             SET twitch_mapping_status = 'no_twitch_equivalent'
             WHERE id = $1",
        )
        .bind(youtube_channel.id)
        .execute(db_pool)
        .await
        .map_err(|e| format!("DB update failed: {}", e))?;

        return Ok(MappingResult::NoEquivalent);
    }

    // 2. Verify the Twitch login actually exists
    let twitch_user = match twitch_client.get_user_by_login(&twitch_login).await {
        Ok(Some(u)) => u,
        Ok(None) => {
            tracing::warn!(
                "Gemini suggested Twitch login '{}' for '{}' but it doesn't exist",
                twitch_login,
                youtube_channel.channel_name
            );
            sqlx::query(
                "UPDATE youtube_source_channels
                 SET twitch_mapping_status = 'no_twitch_equivalent'
                 WHERE id = $1",
            )
            .bind(youtube_channel.id)
            .execute(db_pool)
            .await
            .ok();
            return Ok(MappingResult::NoEquivalent);
        }
        Err(e) => return Err(format!("Twitch user lookup failed: {}", e)),
    };

    // 3. Upsert into twitch_source_channels
    let twitch_channel: TwitchSourceChannel = sqlx::query_as(
        "INSERT INTO twitch_source_channels
             (broadcaster_id, broadcaster_login, display_name, profile_image_url)
         VALUES ($1, $2, $3, $4)
         ON CONFLICT (broadcaster_id) DO UPDATE
           SET broadcaster_login  = EXCLUDED.broadcaster_login,
               display_name       = EXCLUDED.display_name,
               profile_image_url  = EXCLUDED.profile_image_url,
               updated_at         = NOW()
         RETURNING *",
    )
    .bind(&twitch_user.broadcaster_id)
    .bind(&twitch_user.broadcaster_login)
    .bind(&twitch_user.display_name)
    .bind(&twitch_user.profile_image_url)
    .fetch_one(db_pool)
    .await
    .map_err(|e| format!("Failed to upsert twitch_source_channels: {}", e))?;

    // 4. Create mapping (ignore if already exists)
    sqlx::query(
        "INSERT INTO youtube_twitch_channel_mappings
             (youtube_source_channel_id, twitch_source_channel_id)
         VALUES ($1, $2)
         ON CONFLICT (youtube_source_channel_id) DO NOTHING",
    )
    .bind(youtube_channel.id)
    .bind(twitch_channel.id)
    .execute(db_pool)
    .await
    .map_err(|e| format!("Failed to insert mapping: {}", e))?;

    // 5. Mark YouTube channel as mapped
    sqlx::query(
        "UPDATE youtube_source_channels
         SET twitch_mapping_status = 'mapped'
         WHERE id = $1",
    )
    .bind(youtube_channel.id)
    .execute(db_pool)
    .await
    .map_err(|e| format!("Failed to update mapping status: {}", e))?;

    Ok(MappingResult::Mapped(twitch_channel))
}
