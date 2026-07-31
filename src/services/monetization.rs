pub const CREATOR_MONTHLY_USDC_CENTS: u64 = 1500;
pub const AGENCY_STARTER_USDC_CENTS: u64 = 9900;
pub const AGENCY_PRO_USDC_CENTS: u64 = 19900;

#[derive(Debug, Clone, Copy)]
pub struct ServiceOffer {
    pub key: &'static str,
    pub title: &'static str,
    pub what_you_offer: &'static str,
    pub pricing_tiers: &'static str,
    pub best_fit: &'static str,
}

pub const SERVICE_OFFERS: [ServiceOffer; 12] = [
    ServiceOffer {
        key: "clipping",
        title: "SHORT-FORM CLIPPING",
        what_you_offer: "automated daily clip generation from long-form videos, podcasts, or streams — posted to your connected social accounts.",
        pricing_tiers: "$297/mo for a daily campaign with up to 3 posts/day across your connected platforms.",
        best_fit: "podcasters, streamers, and YouTubers with regular source content who want a daily short-form presence.",
    },
    ServiceOffer {
        key: "kick_auto_clipper",
        title: "KICK AUTO-CLIPPER",
        what_you_offer: "automated daily clips from Kick streamer VODs — discover, clip, caption, and post to your connected social accounts.",
        pricing_tiers: "$297/mo for a daily campaign with up to 3 posts/day.",
        best_fit: "clipping channels that repost big Kick streamers' content and need fresh clips daily.",
    },
    ServiceOffer {
        key: "landing_page",
        title: "ANIMATED LANDING PAGE HERO",
        what_you_offer: "animated homepage hero videos, narrated product demos, and brand videos generated from your website URL — posted to your connected social accounts.",
        pricing_tiers: "$149/mo for a daily campaign with up to 3 posts/day.",
        best_fit: "SaaS founders, business owners, consultants, and marketers who want a daily video presence.",
    },
    ServiceOffer {
        key: "education",
        title: "EDUCATIONAL ANIMATED VIDEOS",
        what_you_offer: "Manim/LaTeX-powered educational explainer videos — math, science, finance, coding — posted to your connected social accounts.",
        pricing_tiers: "$199/mo for a daily campaign with up to 3 posts/day.",
        best_fit: "educators, course creators, edu-YouTubers, and academic content channels.",
    },
    ServiceOffer {
        key: "manim_explainer",
        title: "MANIM EXPLAINER VIDEOS",
        what_you_offer: "Manim-animated explainer videos on any topic — generated from a brief, posted daily to your connected social accounts.",
        pricing_tiers: "$149/mo for a daily campaign with up to 3 posts/day.",
        best_fit: "creators who want animated explainer content without learning Manim themselves.",
    },
    ServiceOffer {
        key: "whiteboard_animation",
        title: "WHITEBOARD ANIMATION CAMPAIGN",
        what_you_offer: "hand-drawn style whiteboard animations generated from your brief — posted daily to your connected social accounts.",
        pricing_tiers: "$149/mo for a daily campaign with up to 3 posts/day.",
        best_fit: "educators, trainers, and B2B marketers who want explainer-style whiteboard content.",
    },
    ServiceOffer {
        key: "kinetic_typography",
        title: "KINETIC TYPOGRAPHY CAMPAIGN",
        what_you_offer: "text-driven kinetic typography videos — quotes, key messages, lyric-style motion text — posted daily to your connected social accounts.",
        pricing_tiers: "$149/mo for a daily campaign with up to 3 posts/day.",
        best_fit: "quote pages, motivational content creators, and brands wanting text-driven video content.",
    },
    ServiceOffer {
        key: "animated_infographic",
        title: "ANIMATED INFOGRAPHIC CAMPAIGN",
        what_you_offer: "animated data visualizations and infographics from your data or brief — posted daily to your connected social accounts.",
        pricing_tiers: "$149/mo for a daily campaign with up to 3 posts/day.",
        best_fit: "data-driven creators, finance channels, and businesses with metrics to visualize.",
    },
    ServiceOffer {
        key: "algorithm_viz",
        title: "ALGORITHM VISUALIZATION CAMPAIGN",
        what_you_offer: "animated algorithm visualizations — sorting, searching, graph traversal, data structures — posted daily to your connected social accounts.",
        pricing_tiers: "$149/mo for a daily campaign with up to 3 posts/day.",
        best_fit: "coding bootcamps, computer science educators, and tech content creators.",
    },
    ServiceOffer {
        key: "investor_pitch",
        title: "INVESTOR PITCH DECK CAMPAIGN",
        what_you_offer: "animated investor pitch videos from your deck or brief — posted daily to your connected social accounts.",
        pricing_tiers: "$149/mo for a daily campaign with up to 3 posts/day.",
        best_fit: "startup founders preparing for fundraising who want video-enhanced pitches.",
    },
    ServiceOffer {
        key: "year_in_review",
        title: "YEAR-IN-REVIEW CAMPAIGN",
        what_you_offer: "animated year-in-review or wrapped-style recap videos — posted to your connected social accounts.",
        pricing_tiers: "$149/mo for a daily campaign with up to 3 posts/day.",
        best_fit: "creators, brands, and channels wanting regular recap/annual-style content.",
    },
    ServiceOffer {
        key: "isometric_explainer",
        title: "ISOMETRIC EXPLAINER CAMPAIGN",
        what_you_offer: "isometric 3D-style animated explainer videos from your brief — posted daily to your connected social accounts.",
        pricing_tiers: "$149/mo for a daily campaign with up to 3 posts/day.",
        best_fit: "tech companies, product marketers, and creators wanting distinctive isometric visuals.",
    },
];

pub fn delivery_unlock_price_summary() -> &'static str {
    "All sample deliveries are free — no payment required to view or download."
}

pub fn service_offer_prompt(service: Option<&str>) -> String {
    if let Some(service_key) = service {
        if let Some(offer) = SERVICE_OFFERS.iter().find(|offer| offer.key == service_key) {
            return format!(
                "Service to pitch: {}.\n  - What you offer: {}\n  - Pricing tiers: {}",
                offer.title, offer.what_you_offer, offer.pricing_tiers
            );
        }
    }

    let mut menu = String::from(
        "Pick the strongest-fit service from this menu (mention only ONE in the DM):\n",
    );
    for offer in SERVICE_OFFERS.iter() {
        menu.push_str(&format!(
            "  - {} - Best fit: {} Pricing: {}\n",
            offer.title, offer.best_fit, offer.pricing_tiers
        ));
    }
    menu.trim_end().to_string()
}

pub fn telegram_system_pitch() -> String {
    format!(
        "You are the sales assistant for VideoSync - an AI video production platform. You answer incoming Telegram DMs on behalf of @videosync_sales_bot.\n\n\
VideoSync offers:\n\
- Regular users: 7-day free trial, then ${}/mo USDC for AI thumbnails, Blender animations (title cards, data viz, LaTeX, lower thirds, UI mockups), full agent video pipeline, and FFmpeg tool API.\n\
- Agencies: API access - ${}/mo Starter (1k clips + 500 thumbs + 50 animations), ${}/mo Pro (5k clips + 2.5k thumbs + 200 animations + white-label delivery pages).\n\
- Managed campaign subscriptions: ${}/mo for clipping/kick-auto-clipper, ${}/mo for education, ${}/mo for all other services (landing page, manim, whiteboard, etc.) — daily content generation + auto-posting to connected social accounts.\n\n\
All payments are USDC on Base (Phantom, MetaMask, Coinbase Wallet). No Stripe, no contracts.\n\n\
Sign up: https://www.videosync.video\n\
Subscribe: https://www.videosync.video/subscribe\n\
Agency API: https://www.videosync.video/api-access\n\n\
RULES:\n\
1. Keep replies under 80 words unless asked for detail.\n\
2. If asked about clipping: say \"clipping is reserved for our internal team today - we'll open it up soon; follow the site for updates\".\n\
3. If asked something you don't know, say you'll forward the question to a human and end with \"tag @hthuku if urgent\".\n\
4. Sound like a founder. Casual, lowercase ok, no corporate fluff.\n\
5. Never invent features or prices beyond the ranges and offers listed above.",
        CREATOR_MONTHLY_USDC_CENTS / 100,
        AGENCY_STARTER_USDC_CENTS / 100,
        AGENCY_PRO_USDC_CENTS / 100,
        297,
        199,
        149
    )
}
