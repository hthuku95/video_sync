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

pub const SERVICE_OFFERS: [ServiceOffer; 7] = [
    ServiceOffer {
        key: "clipping",
        title: "SHORT-FORM CLIPPING",
        what_you_offer: "turn their long videos / podcasts / streams into 20-40 vertical Shorts/Reels/TikToks per month.",
        pricing_tiers: "$297 (30 clips/mo) -> $497 (50 clips/mo) -> $899 (unlimited + 48h SLA).",
        best_fit: "podcasters, long-form YouTubers, streamers.",
    },
    ServiceOffer {
        key: "animations",
        title: "AI-DRIVEN BLENDER ANIMATIONS",
        what_you_offer: "explainer scenes, data visualisations, LaTeX equations, lower-thirds, title cards.",
        pricing_tiers: "$50-$150 per 15-60s animation, or $400/month for 5 animations.",
        best_fit: "educators, finance/crypto channels.",
    },
    ServiceOffer {
        key: "thumbnails",
        title: "AI-OPTIMISED YOUTUBE THUMBNAILS",
        what_you_offer: "3-frame extract -> AI picks the strongest -> branded title overlay -> CTR-tested.",
        pricing_tiers: "$25-$50 per thumbnail, or $300/month for 30 thumbnails.",
        best_fit: "growing YouTubers (5k-100k).",
    },
    ServiceOffer {
        key: "ugc",
        title: "UGC / PRODUCT-DEMO VIDEOS",
        what_you_offer: "vertical-first ad-style demo videos for ecommerce / SaaS founders.",
        pricing_tiers: "$200-$500 per 30-60s UGC video.",
        best_fit: "Shopify and SaaS founders.",
    },
    ServiceOffer {
        key: "product_mockup",
        title: "3D PRODUCT MOCKUPS",
        what_you_offer: "photorealistic Blender renders of their product on a device or lifestyle scene - Gemini-generated product shot + cinematic camera move.",
        pricing_tiers: "$100-$300 per mockup, or $600 for a pack of 4 with variations.",
        best_fit: "ecommerce, hardware, app launches.",
    },
    ServiceOffer {
        key: "landing_page",
        title: "ANIMATED LANDING PAGE HERO",
        what_you_offer: "cinematic 10-15s animated hero mockup for their SaaS/startup - if they have a live site, we scrape the hero image; otherwise we generate one with Gemini and animate it in Blender.",
        pricing_tiers: "$200-$600 per hero video. Ideal for Product Hunt launches / YC demos.",
        best_fit: "indie founders and pre-launch SaaS teams.",
    },
    ServiceOffer {
        key: "full_stack",
        title: "FULL-STACK PRODUCTION BUNDLE",
        what_you_offer: "clipping + thumbnails + animations + mockups + landing-page heroes + delivery, all in one retainer.",
        pricing_tiers: "$1,500-$3,000/month.",
        best_fit: "teams that want a full content production partner.",
    },
];

pub fn delivery_unlock_price_summary() -> &'static str {
    "$19-$97 for lightweight Blender samples; $197+ for website-driven presentation videos."
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

    let mut menu = String::from("Pick the strongest-fit service from this menu (mention only ONE in the DM):\n");
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
- Paid deliveries: delivery-specific unlocks on /delivery/:id pages. Typical range: {}.\n\n\
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
        delivery_unlock_price_summary()
    )
}
