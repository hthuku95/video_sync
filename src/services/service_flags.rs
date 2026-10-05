//! Runtime on/off switches for AI-powered services (owner directive Sep 2026).
//!
//! Every entry point that spends LLM tokens checks [`service_enabled`] before
//! starting. In-run calls (QA review, embeddings, variations, skill writes)
//! inherit permission from the run — there are deliberately no separate flags
//! for them, so an enabled service can never end up half-gated.
//!
//! Rows live in the `service_flags` table (see migration
//! `20260928000000_service_flags.sql`) and flip at runtime via the admin UI or
//! `POST /api/admin/service-flags` — no redeploy. Unknown services default to
//! enabled (fail-open for unlisted legacy gig types; the 13 managed services
//! and system groups all have explicit rows).

/// The 13 managed campaign services plus system groups. Kept in sync with the
/// `service_flags` seed migrations (`20260928000000` + `20261005000000`).
/// `clipping` is the legacy pre-split key (YouTube-dominant, parked OFF);
/// `youtube_clipping` / `twitch_clipping` are its platform-specific successors.
pub const MANAGED_SERVICES: &[&str] = &[
    "clipping",
    "youtube_clipping",
    "twitch_clipping",
    "kick_auto_clipper",
    "landing_page",
    "education",
    "manim_explainer",
    "whiteboard_animation",
    "kinetic_typography",
    "animated_infographic",
    "algorithm_viz",
    "investor_pitch",
    "year_in_review",
    "isometric_explainer",
];

/// Services that must stay on for the three Zernio-powered clipping businesses.
/// Order is the constrain-fallback order (Kick first, then Twitch, then YouTube).
pub fn clipping_services() -> &'static [&'static str] {
    &["kick_auto_clipper", "twitch_clipping", "youtube_clipping"]
}

/// System groups beyond the 12 managed services.
pub const SYSTEM_GROUPS: &[&str] = &[
    "prospecting",
    "outreach",
    "sample_packs",
    "legacy_youtube_clipping",
    "chat_agents",
    "admin_tests",
];

/// Whether `service` is a known flaggable name (managed service or group).
/// The admin endpoint rejects anything else so junk rows can't accumulate.
pub fn is_known_flag(service: &str) -> bool {
    let s = service.to_ascii_lowercase();
    MANAGED_SERVICES.contains(&s.as_str()) || SYSTEM_GROUPS.contains(&s.as_str())
}

/// Whether `service` may currently spend LLM tokens. `service` is matched
/// case-insensitively against `service_flags.service`. Missing rows default to
/// enabled (fail-open); all managed services and groups have explicit rows, so
/// in practice only unlisted legacy gig types hit the default.
pub async fn service_enabled(pool: &sqlx::PgPool, service: &str) -> bool {
    let key = service.to_ascii_lowercase();
    let row: Option<(bool,)> =
        sqlx::query_as("SELECT enabled FROM service_flags WHERE service = $1")
            .bind(&key)
            .fetch_optional(pool)
            .await
            .unwrap_or(None);
    row.map(|(enabled,)| enabled).unwrap_or(true)
}

/// Enabled clipping-service slugs for constraining AI service menus (scoring
/// prompts, discovery filters). Returns only rows that are switched on.
pub async fn enabled_clipping_services(pool: &sqlx::PgPool) -> Vec<String> {
    let mut out = Vec::new();
    for service in clipping_services() {
        if service_enabled(pool, service).await {
            out.push(service.to_string());
        }
    }
    out
}

/// Full enabled-service menu (managed services only, no system groups).
pub async fn enabled_services(pool: &sqlx::PgPool) -> Vec<String> {
    let mut out = Vec::new();
    for service in MANAGED_SERVICES {
        if service_enabled(pool, service).await {
            out.push(service.to_string());
        }
    }
    out
}

/// One-line menu description per managed service (mirrors the studio offer).
/// Used to build AI scoring prompts dynamically from enabled flags.
pub fn service_menu_line(service: &str) -> Option<&'static str> {
    match service {
        "clipping" => Some(
            "- **clipping** — legacy combined clip service (superseded by youtube_clipping/twitch_clipping; do not pick for new prospects).",
        ),
        "youtube_clipping" => Some(
            "- **youtube_clipping** — turn long-form YouTube videos and podcasts into short-form clips with captions and thumbnails. Best fit: podcasters, long-form YouTubers. $297/mo.",
        ),
        "twitch_clipping" => Some(
            "- **twitch_clipping** — turn Twitch streams and VODs into short-form clips with captions and thumbnails, auto-posted daily. Best fit: Twitch streamers and their clip channels. $297/mo.",
        ),
        "kick_auto_clipper" => Some(
            "- **kick_auto_clipper** — automated Kick clip generation from VODs: branding, lower thirds, outro, watermark, with daily auto-posting. Best fit: clipping channels, Kick highlight reposters, stream compilations. $297-$899/mo.",
        ),
        "education" => Some(
            "- **education** — AI-driven animated explainer scenes, data visualizations, and motion graphics. Best fit: educators, finance/crypto creators, technical YouTubers. $75-$400 per asset.",
        ),
        "landing_page" => Some(
            "- **landing_page** — homepage hero videos, narrated product demos, launch cutdowns from your website or app. Best fit: indie founders, SaaS teams, launch marketers. $299-$1,500+.",
        ),
        "manim_explainer" => Some(
            "- **manim_explainer** — narrated Manim animated explainers with clean motion graphics, math/technical diagrams. Best fit: educators, course creators, math/finance channels. $75-$300 per asset.",
        ),
        "whiteboard_animation" => Some(
            "- **whiteboard_animation** — narrated whiteboard-style hand-drawn sketch explainer videos. Best fit: explainer channels, SaaS/startup explainers, how-to content. $75-$300 per asset.",
        ),
        "kinetic_typography" => Some(
            "- **kinetic_typography** — dynamic kinetic typography text animations with word-by-word reveals and bold visuals. Best fit: lyric videos, quote channels, brand taglines. $75-$250 per asset.",
        ),
        "animated_infographic" => Some(
            "- **animated_infographic** — animated data infographics with charts, counters, and data-driven visuals. Best fit: data journalism, finance channels, analytics dashboards. $75-$250 per asset.",
        ),
        "algorithm_viz" => Some(
            "- **algorithm_viz** — algorithm/technology visualizations with animated data structures and step-by-step execution. Best fit: coding channels, CS educators, tech docs. $150-$500 per asset.",
        ),
        "investor_pitch" => Some(
            "- **investor_pitch** — professional investor pitch deck videos with clean title cards, data charts, and motion graphics. Best fit: startups raising capital, demo day prep. $150-$500 per asset.",
        ),
        "year_in_review" => Some(
            "- **year_in_review** — personalized year-in-review wrapped-style recap videos. Best fit: creators and brands recapping annual highlights. $100-$400 per asset.",
        ),
        "isometric_explainer" => Some(
            "- **isometric_explainer** — isometric 3D perspective explainer videos with geometric shapes and modern motion graphics. Best fit: product explainers, architectural concepts, tech demos. $100-$400 per asset.",
        ),
        _ => None,
    }
}

/// Builds `(menu_text, must_line)` for AI scoring prompts from currently
/// enabled services. Falls back to the clipping pair when nothing is enabled
/// so discovery never produces an empty menu.
pub async fn scoring_menu(pool: &sqlx::PgPool) -> (String, String) {
    let mut enabled = enabled_services(pool).await;
    if enabled.is_empty() {
        enabled = clipping_services().iter().map(|s| s.to_string()).collect();
    }
    let menu = enabled
        .iter()
        .filter_map(|s| service_menu_line(s))
        .collect::<Vec<_>>()
        .join("\n");
    let must = format!(
        "`service` MUST be one of: {}. No other values are valid.",
        enabled.join(", ")
    );
    (menu, must)
}

/// Coerce an AI-chosen service into the enabled set. Anything disabled (or
/// unknown) falls back by prospect shape — and NEVER returns a disabled
/// service: clipper-like → kick_auto_clipper, Twitch platform → twitch_clipping,
/// YouTube platform → youtube_clipping, else the first enabled clipping service.
/// Last resort is the original value (downstream gates 503 it at zero cost)
/// so prospects are parked honestly instead of misrouted.
pub async fn constrain_service(
    pool: &sqlx::PgPool,
    service: &str,
    clipper_like: bool,
    platform: &str,
) -> String {
    let s = service.to_ascii_lowercase();
    if service_enabled(pool, &s).await {
        return s;
    }
    let p = platform.to_ascii_lowercase();
    let twitch_shaped = p == "twitch" || p.contains("twitch") || s.contains("twitch");
    let youtube_shaped =
        p == "youtube" || p.contains("youtube") || s.contains("youtube") || s == "clipping";
    if clipper_like && service_enabled(pool, "kick_auto_clipper").await {
        return "kick_auto_clipper".to_string();
    }
    if twitch_shaped && service_enabled(pool, "twitch_clipping").await {
        return "twitch_clipping".to_string();
    }
    if youtube_shaped && service_enabled(pool, "youtube_clipping").await {
        return "youtube_clipping".to_string();
    }
    for candidate in clipping_services() {
        if service_enabled(pool, candidate).await {
            return candidate.to_string();
        }
    }
    s
}
