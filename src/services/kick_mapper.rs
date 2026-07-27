use crate::kick_client::KickClient;
use crate::AppState;
use sqlx::PgPool;

/// Attempt to find a Kick.com account for a creator by their brand name.
/// Uses Gemini to guess the Kick slug, then verifies it via the Kick API.
pub async fn auto_map_kick_channel(
    state: &AppState,
    creator_name: &str,
    platform: &str,       // "youtube" or "twitch"
    source_channel_id: i32,
) -> Result<String, String> {
    let kick_client = state
        .kick_client
        .as_ref()
        .ok_or_else(|| "Kick client not configured".to_string())?;

    let prompt = format!(
        r#"Given a content creator known as "{}", what is their most likely Kick.com channel slug?
A Kick.com slug is the part after kick.com/ in their profile URL (e.g., "xqc" for https://kick.com/xqc).

Rules:
- Return ONLY the slug, nothing else — no explanation, no markdown.
- If you are unsure, return "NONE".
- The slug is usually the creator's known username or brand name.
- Kick.com is popular among streamers and gaming creators."#,
        creator_name
    );

    let guessed_slug = crate::llm_utils::generate_text_fast(
        state.ollama_client.as_ref(),
        state.deepseek_client.as_ref(),
        state.video_gemini_client.as_ref().or(state.gemini_client.as_ref()),
        &prompt,
    )
    .await
    .map_err(|e| format!("LLM slug guess failed: {}", e))?
    .trim()
    .to_string();

    if guessed_slug.eq_ignore_ascii_case("NONE") || guessed_slug.is_empty() {
        return Err("Gemini could not guess a Kick slug".to_string());
    }

    // Verify the slug exists on Kick
    let channel = kick_client.get_channel_by_slug(&guessed_slug).await?;

    let (slug, display_name, broadcaster_user_id) = match channel {
        Some(c) => {
            let slug = c.slug.clone();
            let display = slug.clone();
            (slug, display, Some(c.broadcaster_user_id))
        }
        None => {
            // Try common variations
            let lower = guessed_slug.to_lowercase();
            let variations = vec![
                lower.clone(),
                format!("{}tv", lower),
                format!("{}live", lower),
            ];
            let mut found = None;
            for var in &variations {
                if let Ok(Some(c)) = kick_client.get_channel_by_slug(var).await {
                    let slug = c.slug.clone();
                    let display = slug.clone();
                    found = Some((slug, display, Some(c.broadcaster_user_id)));
                    break;
                }
            }
            match found {
                Some(f) => f,
                None => {
                    return Err(format!("Kick slug '{}' not found on Kick.com", guessed_slug));
                }
            }
        }
    };

    // Upsert into kick_source_channels
    let kick_id: i32 = sqlx::query_scalar(
        r#"INSERT INTO kick_source_channels (slug, display_name, broadcaster_user_id)
           VALUES ($1, $2, $3)
           ON CONFLICT (slug) DO UPDATE SET display_name = EXCLUDED.display_name, updated_at = NOW()
           RETURNING id"#,
    )
    .bind(&slug)
    .bind(&display_name)
    .bind(broadcaster_user_id)
    .fetch_one(&state.db_pool)
    .await
    .map_err(|e| format!("Failed to upsert Kick channel: {}", e))?;

    // Create the mapping
    match platform {
        "youtube" => {
            sqlx::query(
                r#"INSERT INTO youtube_kick_channel_mappings (youtube_source_channel_id, kick_source_channel_id)
                   VALUES ($1, $2)
                   ON CONFLICT (youtube_source_channel_id) DO UPDATE SET kick_source_channel_id = EXCLUDED.kick_source_channel_id"#,
            )
            .bind(source_channel_id)
            .bind(kick_id)
            .execute(&state.db_pool)
            .await
            .map_err(|e| format!("Failed to create YouTube-Kick mapping: {}", e))?;

            sqlx::query(
                "UPDATE youtube_source_channels SET kick_mapping_status = 'mapped' WHERE id = $1",
            )
            .bind(source_channel_id)
            .execute(&state.db_pool)
            .await
            .map_err(|e| format!("Failed to update kick_mapping_status: {}", e))?;
        }
        "twitch" => {
            sqlx::query(
                r#"INSERT INTO twitch_kick_channel_mappings (twitch_source_channel_id, kick_source_channel_id)
                   VALUES ($1, $2)
                   ON CONFLICT (twitch_source_channel_id) DO UPDATE SET kick_source_channel_id = EXCLUDED.kick_source_channel_id"#,
            )
            .bind(source_channel_id)
            .bind(kick_id)
            .execute(&state.db_pool)
            .await
            .map_err(|e| format!("Failed to create Twitch-Kick mapping: {}", e))?;

            sqlx::query(
                "UPDATE twitch_source_channels SET kick_mapping_status = 'mapped' WHERE id = $1",
            )
            .bind(source_channel_id)
            .execute(&state.db_pool)
            .await
            .map_err(|e| format!("Failed to update kick_mapping_status: {}", e))?;
        }
        _ => return Err(format!("Unknown platform: {}", platform)),
    }

    Ok(slug)
}

/// Check if a YouTube source channel has all three platform mappings.
pub async fn has_complete_three_way_mapping(
    pool: &PgPool,
    youtube_source_channel_id: i32,
) -> Result<bool, sqlx::Error> {
    let twitch_ok: bool = sqlx::query_scalar(
        "SELECT EXISTS(SELECT 1 FROM youtube_twitch_channel_mappings WHERE youtube_source_channel_id = $1)",
    )
    .bind(youtube_source_channel_id)
    .fetch_one(pool)
    .await?;

    let kick_ok: bool = sqlx::query_scalar(
        "SELECT EXISTS(SELECT 1 FROM youtube_kick_channel_mappings WHERE youtube_source_channel_id = $1)",
    )
    .bind(youtube_source_channel_id)
    .fetch_one(pool)
    .await?;

    Ok(twitch_ok && kick_ok)
}
