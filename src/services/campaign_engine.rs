use crate::services::agentic_service_pipeline::{AgenticServicePipeline, ServiceInput, ServiceType};
use crate::zernio_client::{self, PlatformTarget};
use crate::AppState;
use regex::Regex;
use serde_json::json;
use std::sync::Arc;
use uuid::Uuid;

/// Run one cycle of campaign processing. Should be called every ~10-15 minutes.
pub async fn process_campaigns(state: &Arc<AppState>) {
    let active = match fetch_active_campaigns(state).await {
        Ok(c) => c,
        Err(e) => {
            tracing::error!("campaign_engine: failed to fetch active campaigns: {e}");
            return;
        }
    };

    for campaign in active {
        process_campaign(state, campaign).await;
    }
}

// ── Data types ──────────────────────────────────────────────────────────────

struct CampaignRow {
    id: Uuid,
    user_id: i32,
    name: String,
    service_type: String,
    brief: String,
    style: String,
    duration: f64,
    schedule: serde_json::Value,
    platforms: serde_json::Value,
    posts_per_day: i32,
    start_date: chrono::DateTime<chrono::Utc>,
    zernio_profile_id: Option<String>,
    source_url: Option<String>,
}

struct PostRow {
    id: Uuid,
    day_number: i32,
    slot_index: i32,
    scheduled_at: chrono::DateTime<chrono::Utc>,
}

// ── Step 1: Fetch active campaigns ──────────────────────────────────────────

async fn fetch_active_campaigns(state: &Arc<AppState>) -> Result<Vec<CampaignRow>, sqlx::Error> {
    sqlx::query_as::<_, (Uuid, i32, String, String, String, String, f64, serde_json::Value, serde_json::Value, i32, chrono::DateTime<chrono::Utc>, Option<String>, Option<String>)>(
        "SELECT id, user_id, name, service_type, brief, style, duration, schedule, platforms, \
                posts_per_day, start_date, zernio_profile_id, source_url \
         FROM campaigns \
         WHERE status = 'active' AND start_date <= NOW() AND end_date >= NOW() \
           AND paid_until IS NOT NULL AND paid_until > NOW()",
    )
    .fetch_all(&state.db_pool)
    .await
    .map(|rows| {
        rows.into_iter()
            .map(|(id, user_id, name, service_type, brief, style, duration, schedule, platforms, posts_per_day, start_date, zernio_profile_id, source_url)| {
                CampaignRow { id, user_id, name, service_type, brief, style, duration, schedule, platforms, posts_per_day, start_date, zernio_profile_id, source_url }
            })
            .collect()
    })
}

// ── Step 2: Process a single campaign ───────────────────────────────────────

async fn process_campaign(state: &Arc<AppState>, campaign: CampaignRow) {
    // Fill today's unfilled slots
    if let Err(e) = fill_today_slots(state, &campaign).await {
        tracing::error!("campaign[{}]: fill_today_slots failed: {e}", campaign.id);
    }

    // Process pending_generation posts → create delivery, kick off rendering
    match fetch_posts_by_status(state, campaign.id, "pending_generation", 10).await {
        Ok(posts) => {
            for post in &posts {
                process_pending_post(state, &campaign, post).await;
            }
        }
        Err(e) => tracing::error!("campaign[{}]: fetch pending failed: {e}", campaign.id),
    }

    // Check rendering posts for completed deliveries
    match fetch_posts_by_status(state, campaign.id, "rendering", 50).await {
        Ok(posts) => {
            for post in &posts {
                check_rendering_post(state, &campaign, post).await;
            }
        }
        Err(e) => tracing::error!("campaign[{}]: fetch rendering failed: {e}", campaign.id),
    }

    update_campaign_counts(state, campaign.id).await;
}

// ── Schedule slots ──────────────────────────────────────────────────────────

#[derive(serde::Deserialize, Clone)]
struct ScheduleSlot {
    time: String,
    #[serde(default)]
    platform: String,
    #[serde(default)]
    index: Option<i32>,
}

async fn fill_today_slots(state: &Arc<AppState>, campaign: &CampaignRow) -> Result<(), sqlx::Error> {
    let schedule: Vec<ScheduleSlot> = serde_json::from_value(campaign.schedule.clone())
        .unwrap_or_default();
    if schedule.is_empty() {
        return Ok(());
    }

    let now = chrono::Utc::now();
    let day = days_since_start(campaign.start_date);

    for (idx, slot) in schedule.iter().enumerate() {
        let parts: Vec<&str> = slot.time.split(':').collect();
        let hour: u32 = parts.first().and_then(|s| s.parse().ok()).unwrap_or(0);
        let minute: u32 = parts.get(1).and_then(|s| s.parse().ok()).unwrap_or(0);

        let scheduled_at = now
            .date_naive()
            .and_hms_opt(hour, minute, 0)
            .map(|d| chrono::DateTime::from_naive_utc_and_offset(d, chrono::Utc))
            .unwrap_or(now);

        if scheduled_at <= now {
            continue; // Only create future posts
        }

        let slot_idx = slot.index.unwrap_or(idx as i32);

        sqlx::query(
            "INSERT INTO campaign_posts (campaign_id, day_number, slot_index, scheduled_at, status) \
             VALUES ($1, $2, $3, $4, 'pending_generation') \
             ON CONFLICT (campaign_id, day_number, slot_index) DO NOTHING",
        )
        .bind(campaign.id)
        .bind(day)
        .bind(slot_idx)
        .bind(scheduled_at)
        .execute(&state.db_pool)
        .await?;
    }

    Ok(())
}

fn days_since_start(start_date: chrono::DateTime<chrono::Utc>) -> i32 {
    let days = (chrono::Utc::now() - start_date).num_days();
    days.max(0) as i32 + 1
}

// ── Fetch posts by status ──────────────────────────────────────────────────

async fn fetch_posts_by_status(
    state: &Arc<AppState>,
    campaign_id: Uuid,
    status: &str,
    limit: i32,
) -> Result<Vec<PostRow>, sqlx::Error> {
    sqlx::query_as::<_, (Uuid, i32, i32, chrono::DateTime<chrono::Utc>)>(
        "SELECT id, day_number, slot_index, scheduled_at \
         FROM campaign_posts \
         WHERE campaign_id = $1 AND status = $2 \
         ORDER BY day_number, slot_index \
         LIMIT $3",
    )
    .bind(campaign_id)
    .bind(status)
    .bind(limit)
    .fetch_all(&state.db_pool)
    .await
    .map(|rows| {
        rows.into_iter()
            .map(|(id, day_number, slot_index, scheduled_at)| PostRow {
                id,
                day_number,
                slot_index,
                scheduled_at,
            })
            .collect()
    })
}

// ── Process pending post: generate variation + kick off rendering ──────────

async fn process_pending_post(state: &Arc<AppState>, campaign: &CampaignRow, post: &PostRow) {
    // 1. Generate variation from brief
    let variation = generate_variation(state, &campaign.brief, post.day_number, post.slot_index).await;
    let variation_text = match variation {
        Ok(v) => v,
        Err(e) => {
            tracing::error!("campaign[{}] post[{}]: variation gen failed: {}", campaign.id, post.id, e);
            mark_post_failed(state, post.id, &format!("Variation generation failed: {e}")).await;
            return;
        }
    };

    // 2. Create a delivery record for the pipeline to work with
    let extra = json!({
        "campaign_id": campaign.id.to_string(),
        "campaign_post_id": post.id.to_string(),
        "service_slug": campaign.service_type,
    });

    let delivery_id = match sqlx::query_scalar::<_, Uuid>(
        "INSERT INTO deliveries (client_ref, title, gig_type, prompt, style, duration, extra_args, status) \
         VALUES ($1, $2, $3, $4, $5, $6, $7, 'pending') RETURNING id",
    )
    .bind(format!("campaign:{}", campaign.id))
    .bind(format!("{} — Day {} Slot {}", campaign.name, post.day_number, post.slot_index + 1))
    .bind(&campaign.service_type)
    .bind(&variation_text)
    .bind(&campaign.style)
    .bind(campaign.duration)
    .bind(&extra)
    .fetch_one(&state.db_pool)
    .await
    {
        Ok(id) => id,
        Err(e) => {
            tracing::error!("campaign[{}] post[{}]: delivery create failed: {e}", campaign.id, post.id);
            mark_post_failed(state, post.id, &format!("Delivery create failed: {e}")).await;
            return;
        }
    };

    // 3. Link delivery to campaign post
    let _ = sqlx::query(
        "UPDATE campaign_posts SET variation_prompt = $1, delivery_id = $2, status = 'rendering' WHERE id = $3",
    )
    .bind(&variation_text)
    .bind(delivery_id)
    .bind(post.id)
    .execute(&state.db_pool)
    .await;

    // 4. Kick off rendering via AgenticServicePipeline
    let service_type = match campaign.service_type.as_str() {
        "landing_page" => ServiceType::LandingPage,
        "education" => ServiceType::Education,
        "kick_auto_clipper" | "clipping" => ServiceType::Clipping,
        "manim_explainer" => ServiceType::ManimExplainer,
        "whiteboard_animation" => ServiceType::WhiteboardAnimation,
        "kinetic_typography" => ServiceType::KineticTypography,
        "animated_infographic" => ServiceType::AnimatedInfographic,
        "algorithm_viz" => ServiceType::AlgorithmViz,
        "investor_pitch" => ServiceType::InvestorPitch,
        "year_in_review" => ServiceType::YearInReview,
        "isometric_explainer" => ServiceType::IsometricExplainer,
        _ => ServiceType::Clipping,
    };

    // Resolve the source URL to the latest video (for clipping/Kick campaigns)
    let source_url = if let Some(ref url) = campaign.source_url {
        let resolved = resolve_latest_video_url(state, url).await.unwrap_or_else(|e| {
            tracing::warn!("campaign[{}] post[{}]: resolve_source_url failed: {e}", campaign.id, post.id);
            url.clone()
        });
        if resolved.is_empty() { None } else { Some(resolved) }
    } else {
        None
    };

    // Inject relevant skills into the brief as LESSONS LEARNED
    let enriched_brief = {
        let campaign_id = campaign.id;
        let campaign_service_type = &campaign.service_type;
        let skills = crate::services::skills::get_relevant_skills(
            &state.db_pool, Some(campaign_service_type), Some(campaign_id),
            Some(campaign.user_id), 5,
        ).await.unwrap_or_default();
        let skills_context = crate::services::skills::format_skills_context(&skills);
        if skills_context.is_empty() {
            variation_text.clone()
        } else {
            format!("{}\n\n{}", skills_context, variation_text)
        }
    };

    let input = ServiceInput {
        title: format!("{} — Day {} Slot {}", campaign.name, post.day_number, post.slot_index + 1),
        brief: enriched_brief,
        source_url,
        style: campaign.style.clone(),
        duration_seconds: campaign.duration,
        delivery_id,
        prospect_id: None,
        session_uuid: None,
        user_id: Some(campaign.user_id),
        source_table: Some("deliveries".to_string()),
        source_record_id: Some(delivery_id),
        idempotency_key: None,
        reference_images: get_campaign_file_urls(state, campaign.id).await,
    };

    match AgenticServicePipeline::start(state.clone(), service_type, input).await {
        Ok(_) => tracing::info!(
            "campaign[{}] post[{}] delivery[{}]: rendering started",
            campaign.id, post.id, delivery_id
        ),
        Err(e) => {
            tracing::error!(
                "campaign[{}] post[{}]: failed to start rendering: {e}",
                campaign.id, post.id
            );
            mark_post_failed(state, post.id, &format!("Failed to start rendering: {e}")).await;
        }
    }
}

// ── Get campaign file reference images ─────────────────────────────────────

async fn get_campaign_file_urls(state: &Arc<AppState>, campaign_id: Uuid) -> Vec<String> {
    sqlx::query_scalar::<_, String>("SELECT r2_url FROM campaign_files WHERE campaign_id = $1 ORDER BY uploaded_at")
        .bind(campaign_id)
        .fetch_all(&state.db_pool)
        .await
        .unwrap_or_default()
}

// ── Generate variation ──────────────────────────────────────────────────────

async fn generate_variation(
    state: &Arc<AppState>,
    brief: &str,
    day_number: i32,
    slot_index: i32,
) -> Result<String, String> {
    let prompt = format!(
        "You are a content creator. Generate a unique video brief variation for day {day_number}, \
         post #{slot_index} of a daily content campaign.\n\n\
         Original brief: {brief}\n\n\
         Rules:\n\
         - Make this variation unique but clearly on-topic\n\
         - Include specific details that would make this a different video from yesterday's\n\
         - Keep it concise (2-4 sentences)\n\
         - Return ONLY the variation text, no explanations"
    );

    crate::llm_utils::generate_text_fast(
        state.ollama_client.as_ref(),
        state.deepseek_client.as_ref(),
        state.gemini_client.as_ref(),
        &prompt,
    )
    .await
    .map_err(|e| format!("LLM error: {e}"))
}

// ── Check rendering post for completion ────────────────────────────────────

async fn check_rendering_post(state: &Arc<AppState>, campaign: &CampaignRow, post: &PostRow) {
    // Read delivery_id from campaign post
    let delivery_id: Option<Uuid> = sqlx::query_scalar(
        "SELECT delivery_id FROM campaign_posts WHERE id = $1",
    )
    .bind(post.id)
    .fetch_optional(&state.db_pool)
    .await
    .ok()
    .flatten()
    .flatten();

    let Some(delivery_id) = delivery_id else {
        return;
    };

    // Check delivery status
    let delivery = sqlx::query_as::<_, (String, Option<String>, Option<String>, Option<chrono::DateTime<chrono::Utc>>)>(
        "SELECT status, output_r2_url, error_message, updated_at FROM deliveries WHERE id = $1",
    )
    .bind(delivery_id)
    .fetch_optional(&state.db_pool)
    .await;

    let Some((status, output_url, error_msg, updated_at)) = match delivery {
        Ok(Some(r)) => Some(r),
        _ => None,
    } else {
        return;
    };

    // Handle failed deliveries
    if status == "failed" {
        let err = error_msg.as_deref().unwrap_or("Unknown delivery failure");
        mark_post_failed(state, post.id, err).await;
        return;
    }

    // Handle stale pending deliveries (stuck >1 hour — likely lost on restart)
    if status == "pending" {
        if let Some(updated) = updated_at {
            if chrono::Utc::now() - updated > chrono::Duration::hours(1) {
                mark_post_failed(state, post.id, "Delivery stalled (pending >1h — pipeline may have been interrupted)").await;
                return;
            }
        }
        return; // Still within grace period
    }

    // Completed — check output URL
    let Some(url) = output_url else {
        return;
    };

    // Store the media URL on the post
    let _ = sqlx::query(
        "UPDATE campaign_posts SET media_r2_url = $1 WHERE id = $2",
    )
    .bind(&url)
    .bind(post.id)
    .execute(&state.db_pool)
    .await;

    // Schedule via Zernio if configured
    if let Some(ref profile_id) = campaign.zernio_profile_id {
        schedule_via_zernio(state, profile_id, post, &url, &campaign.platforms).await;
    }

    let new_status = if campaign.zernio_profile_id.is_some() {
        "scheduled"
    } else {
        "published"
    };

    let _ = sqlx::query(
        "UPDATE campaign_posts SET status = $1, published_at = NOW() WHERE id = $2",
    )
    .bind(new_status)
    .bind(post.id)
    .execute(&state.db_pool)
    .await;

    tracing::info!(
        "campaign post {} → {} (delivery={}, url={})",
        post.id, new_status, delivery_id, url
    );

    // Store embedding in Qdrant for campaign learning
    store_campaign_post_embedding(state, campaign, post, &url, &delivery_id).await;

    // Create skill from successful workflow
    create_skill_from_workflow(state, campaign, post, &delivery_id).await;
}

// ── Skill creation from successful workflow ───────────────────────────────────

async fn create_skill_from_workflow(
    state: &Arc<AppState>,
    campaign: &CampaignRow,
    _post: &PostRow,
    delivery_id: &Uuid,
) {
    // Get workflow_id from delivery
    let workflow_id: Option<Uuid> = sqlx::query_scalar(
        "SELECT workflow_id FROM deliveries WHERE id = $1 AND workflow_id IS NOT NULL",
    )
    .bind(delivery_id)
    .fetch_optional(&state.db_pool)
    .await
    .ok()
    .flatten()
    .flatten();

    let Some(workflow_id) = workflow_id else {
        return;
    };

    // Load workflow nodes
    let runtime = crate::services::workflow_runtime::WorkflowRuntime::new(state.db_pool.clone());
    let nodes = match runtime.list_nodes(workflow_id).await {
        Ok(n) => n,
        Err(e) => {
            tracing::warn!("create_skill_from_workflow: list_nodes failed: {}", e);
            return;
        }
    };

    let completed_nodes: Vec<&crate::services::workflow_runtime::WorkflowNode> = nodes
        .iter()
        .filter(|n| n.status == "completed" && !n.node_type.is_empty())
        .collect();

    if completed_nodes.is_empty() {
        return;
    }

    // Build tool sequence summary from completed nodes
    let tool_seq: Vec<serde_json::Value> = completed_nodes
        .iter()
        .map(|n| {
            json!({
                "node_key": n.node_key,
                "node_type": n.node_type,
                "input": n.input,
            })
        })
        .collect();

    let tool_seq_json = serde_json::to_string_pretty(&tool_seq).unwrap_or_default();

    // LLM prompt to extract a reusable skill
    let prompt = format!(
        "You are analyzing a successful content generation workflow. Below is the sequence \
         of tool calls that produced a high-quality output for a '{}' campaign with brief: '{}'.\n\n\
         Tool sequence:\n{}\n\n\
         Extract a reusable skill from this workflow. Respond in JSON format:\n\
         {{\n\
           \"name\": \"Short skill name (max 60 chars)\",\n\
           \"description\": \"What this pattern does and when to apply it (1-2 sentences)\",\n\
           \"trigger_conditions\": {{\"brief_contains\": \"keyword or leave empty\"}}\n\
         }}",
        campaign.service_type, campaign.brief, tool_seq_json
    );

    let text = match crate::llm_utils::generate_text_fast(
        state.ollama_client.as_ref(),
        state.deepseek_client.as_ref(),
        state.gemini_client.as_ref(),
        &prompt,
    )
    .await
    {
        Ok(t) => t,
        Err(e) => {
            tracing::warn!("create_skill_from_workflow: LLM call failed: {}", e);
            return;
        }
    };

    let parsed: serde_json::Value = match serde_json::from_str(&text) {
        Ok(v) => v,
        Err(_) => {
            // Try to extract JSON from the response
            let re = regex::Regex::new(r"\{[^}]*\}").unwrap();
            if let Some(cap) = re.find(&text) {
                serde_json::from_str(cap.as_str()).unwrap_or(json!({}))
            } else {
                json!({})
            }
        }
    };

    let name = parsed["name"].as_str().unwrap_or("Workflow pattern");
    let description = parsed["description"]
        .as_str()
        .unwrap_or("Learned from successful workflow");
    let trigger_conditions = parsed
        .get("trigger_conditions")
        .cloned()
        .unwrap_or(json!({}));

    if let Err(e) = crate::services::skills::store_skill(
        &state.db_pool,
        state.qdrant_client.as_ref(),
        state.gemini_client.as_ref(),
        Some(campaign.user_id),
        Some(&campaign.service_type),
        Some(campaign.id),
        name,
        description,
        trigger_conditions,
        json!(tool_seq),
        "successful_workflow",
        None,
        "campaign",
    )
    .await
    {
        tracing::warn!("create_skill_from_workflow: store_skill failed: {}", e);
    }
}

// ── Schedule via Zernio ────────────────────────────────────────────────────

async fn schedule_via_zernio(
    state: &Arc<AppState>,
    profile_id: &str,
    post: &PostRow,
    media_url: &str,
    platforms_json: &serde_json::Value,
) {
    let Some(zernio) = state.zernio_client.clone() else {
        return;
    };

    let targets: Vec<PlatformTarget> = match platforms_json {
        serde_json::Value::Array(arr) => arr
            .iter()
            .filter_map(|v| {
                let obj = v.as_object()?;
                Some(PlatformTarget {
                    platform: obj.get("platform")?.as_str()?.to_string(),
                    accountId: obj.get("account_id")?.as_str()?.to_string(),
                })
            })
            .collect(),
        _ => return,
    };

    if targets.is_empty() {
        return;
    }

    let text = format!("Daily content — Day {}", post.day_number);
    let scheduled_for = post.scheduled_at.format("%Y-%m-%dT%H:%M:%SZ").to_string();
    let scheduled_for_clone = scheduled_for.clone();

    let media_items = vec![zernio_client::MediaItem {
        r#type: "video".to_string(),
        url: media_url.to_string(),
    }];
    let req = zernio_client::CreatePostRequest {
        content: Some(text),
        platforms: targets,
        profileId: Some(profile_id.to_string()),
        media_items: Some(media_items),
        scheduledFor: Some(scheduled_for),
        publishNow: false,
    };

    match zernio.create_post(&req).await {
        Ok(resp) => {
            let _ = sqlx::query(
                "UPDATE campaign_posts SET zernio_post_id = $1 WHERE id = $2",
            )
            .bind(&resp.post.id)
            .bind(post.id)
            .execute(&state.db_pool)
            .await;
            tracing::info!(
                "campaign post {} scheduled via Zernio (post_id={}, at={})",
                post.id, resp.post.id, scheduled_for_clone
            );
        }
        Err(e) => {
            tracing::warn!("campaign post {} Zernio schedule failed: {e}", post.id);
        }
    }
}

// ── Update campaign counters ───────────────────────────────────────────────

async fn update_campaign_counts(state: &Arc<AppState>, campaign_id: Uuid) {
    let counts = sqlx::query_as::<_, (i64, i64)>(
        "SELECT COUNT(*), \
                COALESCE(SUM(CASE WHEN status IN ('scheduled','published') THEN 1 ELSE 0 END), 0) \
         FROM campaign_posts WHERE campaign_id = $1",
    )
    .bind(campaign_id)
    .fetch_one(&state.db_pool)
    .await;

    if let Ok((total, published)) = counts {
        let _ = sqlx::query(
            "UPDATE campaigns SET total_posts_planned = $1, total_posts_published = $2, \
             updated_at = NOW() WHERE id = $3",
        )
        .bind(total as i32)
        .bind(published as i32)
        .bind(campaign_id)
        .execute(&state.db_pool)
        .await;
    }
}

// ── Resolve channel URL → latest video URL ─────────────────────────────────

/// Takes a channel/streamer URL and returns the latest video URL from that source.
/// - YouTube channels → fetches most recent upload via uploads playlist API
/// - Twitch channels → fetches most recent VOD via Helix API
/// - Kick streamers → returns as-is (yt-dlp resolves latest broadcast natively)
/// - Direct video URLs → returned unchanged
async fn resolve_latest_video_url(state: &Arc<AppState>, url: &str) -> Result<String, String> {
    let trimmed = url.trim();

    // Direct video URLs — pass through (already a specific video)
    if is_direct_video_url(trimmed) {
        return Ok(trimmed.to_string());
    }

    // Kick: https://kick.com/{slug} or https://kick.com/{slug}/videos/{uuid}
    if let Some(slug) = parse_kick_slug(trimmed) {
        // Verify the streamer exists via Kick API
        if let Some(ref client) = state.kick_client {
            match client.get_channel_by_slug(&slug).await {
                Ok(Some(_)) => {} // streamer confirmed
                Ok(None) => return Err(format!("Kick streamer '{}' not found", slug)),
                Err(e) => return Err(format!("Kick API error: {}", e)),
            }
        }

        // Resolve to HLS stream URL (Browserbase → VOD page → HLS m3u8)
        let resolved = crate::kick_vod_scraper::resolve_url_to_hls(trimmed).await;
        if resolved != trimmed {
            tracing::info!("Kick '{}' resolved to HLS stream URL", slug);
            return Ok(resolved);
        }

        // Fallback: return the channel URL for yt-dlp
        return Ok(format!("https://kick.com/{}", slug));
    }

    // Twitch: https://twitch.tv/{channel}
    if let Some(channel) = parse_twitch_channel(trimmed) {
        if let Some(ref client) = state.twitch_client {
            let user = client.get_user_by_login(&channel).await
                .map_err(|e| format!("Twitch API error: {}", e))?
                .ok_or_else(|| format!("Twitch channel '{}' not found", channel))?;
            let (videos, _) = client.get_videos(&user.broadcaster_id, None, 1).await
                .map_err(|e| format!("Twitch videos error: {}", e))?;
            return videos.into_iter().next()
                .map(|v| v.url)
                .ok_or_else(|| format!("No VODs found for Twitch channel '{}'", channel));
        }
        return Err("Twitch client not configured".to_string());
    }

    // YouTube: https://youtube.com/channel/{id} or youtube.com/@{handle} or youtube.com/c/{name}
    if let Some(channel_id) = parse_youtube_channel_id(trimmed) {
        if let Some(ref client) = state.youtube_client {
            let uploads_id = uploads_playlist_id_for_channel(&channel_id);
            let resp = client.get_channel_uploads(&uploads_id, 1).await
                .map_err(|e| format!("YouTube API error: {}", e))?;
            return resp.items.into_iter().next()
                .map(|item| format!("https://youtube.com/watch?v={}", item.id.video_id))
                .ok_or_else(|| format!("No uploads found for YouTube channel '{}'", channel_id));
        }
        return Err("YouTube client not configured".to_string());
    }

    // YouTube handle: youtube.com/@handle — needs a channel search first
    if let Some(handle) = parse_youtube_handle(trimmed) {
        if let Some(ref client) = state.youtube_client {
            let resp = client.search_channels(None, &handle, 1, None).await
                .map_err(|e| format!("YouTube search error: {}", e))?;
            let channel = resp.items.into_iter().next()
                .ok_or_else(|| format!("YouTube channel '@{}' not found", handle))?;
            let uploads_id = uploads_playlist_id_for_channel(&channel.id.channel_id);
            let uploads = client.get_channel_uploads(&uploads_id, 1).await
                .map_err(|e| format!("YouTube uploads error: {}", e))?;
            return uploads.items.into_iter().next()
                .map(|item| format!("https://youtube.com/watch?v={}", item.id.video_id))
                .ok_or_else(|| format!("No uploads found for YouTube channel '@{}'", handle));
        }
        return Err("YouTube client not configured".to_string());
    }

    // Unknown URL format — return as-is, let the pipeline try it
    Ok(trimmed.to_string())
}

fn is_direct_video_url(url: &str) -> bool {
    // Direct video URLs have a video ID parameter (watch?v=, /videos/, /video/)
    url.contains("watch?v=")
        || url.contains("youtu.be/")
        || url.contains("twitch.tv/videos/")
        || url.contains("kick.com/video/")
        || url.contains("kick.com/videos/")
        || url.contains("vimeo.com/")
        || url.ends_with(".mp4")
        || url.ends_with(".webm")
        || url.ends_with(".mov")
}

fn parse_kick_slug(url: &str) -> Option<String> {
    // Match kick.com/{slug} optionally followed by /videos/{uuid}
    // The slug is always the first path segment after kick.com
    let re = Regex::new(r"kick\.com/([a-zA-Z0-9_-]+)(?:/videos?/[a-f0-9-]+)?").ok()?;
    re.captures(url)
        .and_then(|c| c.get(1).map(|m| m.as_str().to_lowercase()))
}

fn parse_twitch_channel(url: &str) -> Option<String> {
    let re = Regex::new(r"twitch\.tv/([a-zA-Z0-9_]+)").ok()?;
    re.captures(url)
        .and_then(|c| c.get(1).map(|m| m.as_str().to_lowercase()))
}

fn parse_youtube_channel_id(url: &str) -> Option<String> {
    let re = Regex::new(r"youtube\.com/channel/(UC[a-zA-Z0-9_-]+)").ok()?;
    re.captures(url)
        .and_then(|c| c.get(1).map(|m| m.as_str().to_string()))
}

fn parse_youtube_handle(url: &str) -> Option<String> {
    let re = Regex::new(r"youtube\.com/@([a-zA-Z0-9_-]+)").ok()?;
    re.captures(url)
        .and_then(|c| c.get(1).map(|m| m.as_str().to_string()))
}

fn uploads_playlist_id_for_channel(channel_id: &str) -> String {
    // YouTube uploads playlist: replace "UC" prefix with "UU"
    if channel_id.starts_with("UC") {
        format!("UU{}", &channel_id[2..])
    } else {
        format!("UU{}", channel_id)
    }
}

// ── Campaign learning: store post embeddings ───────────────────────────────

/// After a campaign post publishes, embed its content in Qdrant so the
/// campaign manager agent can learn from past posts via semantic search.
async fn store_campaign_post_embedding(
    state: &Arc<AppState>,
    campaign: &CampaignRow,
    post: &PostRow,
    output_url: &str,
    delivery_id: &Uuid,
) {
    let Some(ref qdrant) = state.qdrant_client else { return };
    let gemini = state.video_gemini_client.as_ref().or(state.gemini_client.as_ref());
    let Some(ref gemini) = gemini else { return };

    // Read the variation prompt and caption from the post/delivery
    let prompt_caption: (Option<String>, Option<String>) = sqlx::query_as(
        "SELECT cp.variation_prompt, cp.caption \
         FROM campaign_posts cp WHERE cp.id = $1",
    )
    .bind(post.id)
    .fetch_optional(&state.db_pool)
    .await
    .ok()
    .flatten()
    .unwrap_or((None, None));

    let variation = prompt_caption.0.as_deref().unwrap_or("");
    let caption = prompt_caption.1.as_deref().unwrap_or("");

    // Build a rich text for embedding
    let embed_text = format!(
        "Campaign: {} | Service: {} | Day {} | Variation: {} | Caption: {} | Output: {}",
        campaign.name, campaign.service_type, post.day_number, variation, caption, output_url
    );

    let mut context = std::collections::HashMap::new();
    context.insert("campaign_name".to_string(), serde_json::json!(campaign.name));
    context.insert("service_type".to_string(), serde_json::json!(campaign.service_type));
    context.insert("day_number".to_string(), serde_json::json!(post.day_number));
    context.insert("output_url".to_string(), serde_json::json!(output_url));
    context.insert("delivery_id".to_string(), serde_json::json!(delivery_id.to_string()));

    if let Err(e) = qdrant
        .store_chat_memory_with_gemini2(
            &campaign.id.to_string(), // session_id = campaign_id
            Some(&campaign.user_id.to_string()),
            &embed_text,
            &caption,
            vec![output_url.to_string()],
            context,
            gemini,
            Some("campaign_post"),
        )
        .await
    {
        tracing::warn!("campaign[{}] post[{}]: embedding store failed: {e}", campaign.id, post.id);
    }
}

// ── Helpers ─────────────────────────────────────────────────────────────────

async fn mark_post_failed(state: &Arc<AppState>, post_id: Uuid, error: &str) {
    let _ = sqlx::query(
        "UPDATE campaign_posts SET status = 'failed', error_message = $1 WHERE id = $2",
    )
    .bind(error)
    .bind(post_id)
    .execute(&state.db_pool)
    .await;
    tracing::warn!("campaign post {post_id} failed: {error}");
}
