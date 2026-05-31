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
        what_you_offer: "turn long-form videos, podcasts, or streams into a managed short-form package when the workflow is actively supervised.",
        pricing_tiers: "$297-$899/month depending on clip count, review level, and turnaround.",
        best_fit: "podcasters, long-form YouTubers, and streamers with enough source material to justify an ongoing package.",
    },
    ServiceOffer {
        key: "animations",
        title: "AI-DRIVEN BLENDER ANIMATIONS",
        what_you_offer: "explainer scenes, data visualizations, title cards, lower thirds, motion graphics, and visual support assets.",
        pricing_tiers: "$75-$400 per asset, or custom monthly retainers for recurring production.",
        best_fit: "educators, finance/crypto creators, product marketers, and agencies that need premium visual support.",
    },
    ServiceOffer {
        key: "thumbnails",
        title: "AI-OPTIMISED YOUTUBE THUMBNAILS",
        what_you_offer: "click-focused thumbnail concepts, branded title treatments, and premium packaging around YouTube content or launch media.",
        pricing_tiers: "$25-$75 per thumbnail, or recurring monthly packaging retainers.",
        best_fit: "growing channels, creators, and teams that want stronger presentation before a viewer clicks play.",
    },
    ServiceOffer {
        key: "ugc",
        title: "UGC / PRODUCT-DEMO VIDEOS",
        what_you_offer: "vertical-first product videos, demo edits, and ad-ready promo assets for ecommerce and software brands.",
        pricing_tiers: "$200-$900 per video depending on scope, voiceover, and asset complexity.",
        best_fit: "Shopify operators, SaaS founders, and marketers testing paid acquisition.",
    },
    ServiceOffer {
        key: "product_mockup",
        title: "3D PRODUCT MOCKUPS",
        what_you_offer: "rendered product visuals, device mockups, and motion-enhanced presentation assets for launches, promos, and demos.",
        pricing_tiers: "$100-$600 per asset or multi-asset package.",
        best_fit: "ecommerce, hardware, app launches, and product marketing teams.",
    },
    ServiceOffer {
        key: "landing_page",
        title: "ANIMATED LANDING PAGE HERO",
        what_you_offer: "homepage hero videos, narrated product demos, launch cutdowns, and landing-page motion built from a real product site or app flow.",
        pricing_tiers: "$299-$1,500+ depending on whether the buyer needs a hero loop, a narrated demo, or a fuller launch package.",
        best_fit: "indie founders, SaaS teams, launch marketers, and agencies selling product video.",
    },
    ServiceOffer {
        key: "full_stack",
        title: "FULL-STACK PRODUCTION BUNDLE",
        what_you_offer: "a private production backend covering product demos, thumbnails, motion graphics, mockups, and recurring delivery under the buyer's own brand.",
        pricing_tiers: "$1,000-$3,000+/month depending on output mix, review load, and turnaround.",
        best_fit: "boutique agencies, creator managers, and operators who want backend production support instead of a one-off asset.",
    },
];

pub fn delivery_unlock_price_summary() -> &'static str {
    "$19-$97 for lightweight sample assets; $197+ for stronger website-driven presentation videos and higher-value delivery unlocks."
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
