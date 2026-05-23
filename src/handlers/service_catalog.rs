use serde_json::json;

pub fn build_service_sample_prompt(
    service_slug: &str,
    reference_url: Option<&str>,
    prospect_name: Option<&str>,
    brief: &str,
) -> String {
    let service_label = match service_slug {
        "saas-launch-pack" => "SaaS Launch Pack",
        "clipper-enhancement-pack" => "Thumbnail & Motion Graphics Pack",
        "thumbnail-hero-pack" => "Thumbnail & Hero Visual Pack",
        "product-mockup-pack" => "Product Mockup Video Pack",
        "education-explainer-pack" => "Education Explainer Pack",
        "blender-scene-pack" => "Blender 2D/3D Scene Pack",
        "voice-audio-pack" => "Voice & Audio Production Pack",
        "mixed-agency-bundle" => "Mixed Agency Production Bundle",
        "creator-manager-fulfillment" => "Agency Production Backend",
        "x402-asset-api" => "Programmable Payments",
        _ => "VideoSync custom sample",
    };

    let focus = match service_slug {
        "saas-launch-pack" => "Create a founder-ready demo sample from a website/app URL, screenshots, app recording, or launch brief. The default buyer-facing promise is simple: turn the product into a polished 30-60s demo/promo video in 24 hours.",
        "clipper-enhancement-pack" => "Create a premium visual-packaging sample with stronger thumbnails, motion graphics, mockups, overlays, narration support, or branded polish that improves how the final offer looks before anyone clicks play.",
        "thumbnail-hero-pack" => "Create click-focused thumbnail and hero visual concepts with hooks, variants, and QA so the buyer can test stronger first impressions quickly.",
        "product-mockup-pack" => "Create UI mockup scenes and short product videos from a URL, screenshots, Figma exports, app recordings, or a written workflow.",
        "education-explainer-pack" => "Create a lesson or explainer using Manim, LaTeX, diagrams, narration, and long-form assembly when the topic needs more than a static asset.",
        "blender-scene-pack" => "Create Blender 2D/3D support visuals, animated models, product scenes, lower thirds, data visuals, or cinematic loops.",
        "voice-audio-pack" => "Create voice/audio assets with scripts, VibeVoice narration, summaries, audio visualizers, or narration-backed videos.",
        "mixed-agency-bundle" => "Create a website-to-video agency package: agencies send client websites or app URLs, and VideoSync produces client-ready demo/promo videos they can resell with delivery/download links.",
        "creator-manager-fulfillment" => "Create an agency-backend sample that shows how repeatable client video deliverables could be produced through VideoSync without exposing internal operations language to the buyer.",
        "x402-asset-api" => "Create a technical-buyer sample that shows how paid access, delivery unlocks, or a custom media integration would work in practice without turning the explanation into internal product language.",
        _ => "Create a high-quality custom sample that matches the requested service.",
    };
    let production_stack = match service_slug {
        "saas-launch-pack" => "Use long-form assembly when the brief asks for a 30s+ product story, demo, explainer, launch trailer, or multi-cutdown campaign. Pull from UI mockups, product screenshots, Blender scenes, VibeVoice narration, Pexels support footage, captions, thumbnails, and Gemini multimodal QA as needed. Default deliverable split: $299 basic rush demo = one polished 30-60s video; $499 full pack = video plus hooks/captions, thumbnail or hero concept, and delivery/download page.",
        "clipper-enhancement-pack" => "Use long-form assembly when the request asks for a clip pack, recap, narrated summary, or multiple enhanced cutdowns. Combine clipping/enhancement tools with captions, thumbnail/hero assets, motion graphics, audio cleanup, and QA instead of treating it as a single raw clip.",
        "thumbnail-hero-pack" => "Use thumbnail/hero generation first, then add short motion loops or product mockups when a static visual alone will not sell the offer.",
        "product-mockup-pack" => "Use UI mockup and long-form assembly when the buyer needs browser/device scenes, app walkthroughs, launch cutdowns, or homepage hero video.",
        "education-explainer-pack" => "Use Manim/LaTeX, narration, diagrams, and resumable long-form segments for lessons, tutorials, formulas, and technical explanations.",
        "blender-scene-pack" => "Use BlenderMCP scenes, render QA, and optional video assembly when 2D/3D motion can make the result more premium than stock footage.",
        "voice-audio-pack" => "Use VibeVoice narration, audio cleanup/visualization, summaries, and optional video assembly depending on whether the deliverable is audio-only or narrated media.",
        "mixed-agency-bundle" => "Use the full canonical tool stack only where it helps the agency deliver client websites as videos. Default package: $999 for 3 client website/app demo videos, each with a delivery/download page and optional hooks, thumbnail/hero concept, mockups, or narration.",
        "creator-manager-fulfillment" => "Use bundle-style long-form assembly for mixed agency packages: demos, clips, thumbnails, product mockups, narration, delivery pages, and review artifacts. The agent should choose the right tool mix instead of forcing one asset type.",
        "x402-asset-api" => "Use long-form assembly for buyer-facing technical demos when a walkthrough needs script, UI mockup, narration, diagrams, and delivery-page proof. Keep the story commercial, not just architectural.",
        _ => "Use long-form assembly whenever the buyer asks for a longer video, a multi-part asset pack, or a package that combines editing, Blender/Manim/LaTeX, thumbnails, mockups, voice/audio, QA, and delivery-page output.",
    };

    let url_line = reference_url
        .map(str::trim)
        .filter(|value| !value.is_empty())
        .map(|value| format!("Reference URL or media: {value}\n"))
        .unwrap_or_default();
    let contact_line = prospect_name
        .map(str::trim)
        .filter(|value| !value.is_empty())
        .map(|value| format!("Prospect or brand: {value}\n"))
        .unwrap_or_default();

    format!(
        "Service page request for {service_label}.\n{focus}\n{production_stack}\n{url_line}{contact_line}Project brief: {brief}\n\nDeliver a concrete, buyer-facing plan and then use the full production stack to generate the requested output."
    )
}

pub fn build_service_sample_chat_title(
    service_slug: &str,
    brief: &str,
    prospect_name: Option<&str>,
) -> String {
    let prefix = match service_slug {
        "saas-launch-pack" => "SaaS Launch",
        "clipper-enhancement-pack" => "Motion Pack",
        "thumbnail-hero-pack" => "Thumbnail Pack",
        "product-mockup-pack" => "Product Mockup",
        "education-explainer-pack" => "Education Explainer",
        "blender-scene-pack" => "Blender Scene",
        "voice-audio-pack" => "Voice Audio",
        "mixed-agency-bundle" => "Agency Bundle",
        "creator-manager-fulfillment" => "Agency Backend",
        "x402-asset-api" => "Payments",
        _ => "Video Request",
    };

    let trimmed_brief = brief.split_whitespace().collect::<Vec<_>>().join(" ");
    let subject = if let Some(name) = prospect_name
        .map(str::trim)
        .filter(|value| !value.is_empty())
    {
        format!("{name} - {trimmed_brief}")
    } else {
        trimmed_brief
    };

    let composed = format!("{prefix}: {subject}");
    if composed.chars().count() <= 90 {
        return composed;
    }

    let shortened: String = composed.chars().take(90).collect();
    match shortened.rfind(' ') {
        Some(idx) if idx > 28 => format!("{}...", shortened[..idx].trim_end()),
        _ => format!("{}...", shortened.trim_end()),
    }
}

pub fn service_sample_ui_config(service_slug: &str) -> serde_json::Value {
    match service_slug {
        "saas-launch-pack" => json!({
            "section_title": "Request a launch video",
            "section_copy": "Paste a product URL, app recording, screenshots, or a launch brief and describe the exact video you want. The agent can turn the same product into a homepage hero, a narrated demo, a founder pitch explainer, or a longer campaign video.",
            "source_label": "Product URL, app recording, or source asset",
            "source_placeholder": "https://yourapp.com, Loom demo, Figma export, screenshot folder, or product brief",
            "contact_label": "Product or company name",
            "contact_placeholder": "Your startup, client, or product name",
            "format_label": "Target length and format",
            "format_placeholder": "15-30s homepage hero, 45s narrated demo, 90s explainer, or custom",
            "outcome_label": "Primary use case",
            "outcome_placeholder": "Homepage, Product Hunt, LinkedIn ads, onboarding, sales demo, investor update",
            "brief_label": "Describe the video you want",
            "brief_placeholder": "Example: build a 60-second narrated product story that opens with the market pain, shows our workflow clearly, includes device mockups, and ends with a CTA for B2B buyers.",
            "launch_label": "Generate my video",
            "status_idle": "Your request opens the main AI chat with a structured production brief so the agent can generate a real buyer-facing launch video from scratch.",
            "anon_badge": "Sign in to generate your first product video.",
            "helper_title": "What this video should prove",
            "helper_copy": "Use this launch video to test how clearly VideoSync understands your product, the right length for your campaign, and the quality of the visual direction before you buy more output.",
            "helper_bullets": [
                "Supports website URLs, screenshots, app recordings, or raw launch briefs.",
                "You can ask for homepage heroes, narrated walkthroughs, investor explainers, or launch cutdowns.",
                "Longer requests can use long-form assembly with UI mockups, narration, motion scenes, QA, and cutdowns.",
                "Your included videos stay available before checkout appears."
            ],
            "included_unit_singular": "video",
            "included_unit_plural": "videos",
            "limit_reached_badge": "Your included launch videos are used up. Upgrade to keep generating.",
            "example_heading": "Example buyer-ready request",
            "example_request": "Create a 60-second narrated launch video for VideoSync that opens with the pain of slow video production, shows the workflow clearly with website and product visuals, and ends with a CTA for founders who need faster marketing output.",
            "upgrade_href": "/subscribe",
            "upgrade_label": "Continue with paid videos"
        }),
        "clipper-enhancement-pack" => json!({
            "section_title": "Request upgraded clips",
            "section_copy": "Send the source video, VOD, creator link, or existing clip and describe the premium upgrade you want. This page is for motion hooks, captions, narration, inserts, thumbnails, and resale-ready polish that editors and agencies can package under their own brand.",
            "source_label": "Source video, VOD, channel, or clip link",
            "source_placeholder": "YouTube, Twitch, Drive, or any clip/VOD reference link",
            "contact_label": "Creator or client name",
            "contact_placeholder": "Creator, client, or agency name",
            "format_label": "Platform and clip style",
            "format_placeholder": "YouTube Shorts, TikTok, Twitch recap, podcast highlight, finance explainer",
            "outcome_label": "Upgrade you want to test",
            "outcome_placeholder": "Hooks, motion graphics, narration, title cards, thumbnails, branded polish",
            "brief_label": "Describe the upgrade sample you want",
            "brief_placeholder": "Example: turn this gaming highlight into a premium short with a faster first hook, cleaner captions, streamer branding, and one narrated context insert before the payoff.",
            "launch_label": "Generate my clips",
            "status_idle": "Your request opens the main AI workspace with a structured visual-packaging brief so the agent can build premium clips from scratch.",
            "anon_badge": "Sign in to generate your first clip package.",
            "helper_title": "What these clips should prove",
            "helper_copy": "Use this clip package to test whether the upgrade gives you something you can resell for more money, not just another ordinary clip.",
            "helper_bullets": [
                "Built for editors, short-form operators, and agencies selling premium output.",
                "Best for motion hooks, branded title cards, narration, thumbnails, and explainer inserts.",
                "Clip packs can include manual/automatic clipping, enhancement, animated summaries, and thumbnail/hero assets.",
                "Your included clips stay available before checkout appears."
            ],
            "included_unit_singular": "clip",
            "included_unit_plural": "clips",
            "limit_reached_badge": "Your included clips are used up. Upgrade to keep generating.",
            "example_heading": "Example upgrade request",
            "example_request": "Turn this 45-minute creator video into three YouTube Shorts with stronger first hooks, cleaner captions, branded motion graphics, and one thumbnail concept that looks premium enough to resell to a client.",
            "upgrade_href": "/subscribe",
            "upgrade_label": "Continue with paid clip work"
        }),
        "creator-manager-fulfillment" => json!({
            "section_title": "Request a production package",
            "section_copy": "Paste the client site, product, campaign brief, or source media and describe the production package you want to test. This page is for agencies and operators who need a reliable backend for demos, cutdowns, thumbnails, and delivery links they can reuse across client accounts.",
            "source_label": "Client site, campaign, or source asset link",
            "source_placeholder": "Website URL, Drive folder, YouTube channel, campaign brief, or product asset link",
            "contact_label": "Client or account name",
            "contact_placeholder": "Client brand, product, or campaign name",
            "format_label": "Package or deliverable mix",
            "format_placeholder": "Homepage demo, creator bundle, launch cutdowns, thumbnail pack, mixed deliverables",
            "outcome_label": "Business goal",
            "outcome_placeholder": "Client delivery, white-label backend test, agency proof-of-capability, recurring package",
            "brief_label": "Describe the package you want",
            "brief_placeholder": "Example: build a sample production package for a fintech client with one homepage demo, two social cutdowns, and one thumbnail concept that shows what a repeatable backend relationship could look like.",
            "launch_label": "Generate my package",
            "status_idle": "Your request opens the main AI workspace with a structured fulfillment brief so the agent can generate a real backend-ready production package.",
            "anon_badge": "Sign in to generate your first production package.",
            "helper_title": "What this package should prove",
            "helper_copy": "Use this package to see if VideoSync can behave like a reliable white-label production backend rather than a one-off editor.",
            "helper_bullets": [
                "Best for agencies, creator managers, operators, and white-label fulfillment teams.",
                "Supports mixed deliverables like demos, cutdowns, thumbnails, and delivery pages.",
                "Bundle requests can combine long-form video, clips, thumbnails, mockups, education scenes, 3D scenes, and VibeVoice.",
                "Your included packages stay available before checkout appears."
            ],
            "included_unit_singular": "package",
            "included_unit_plural": "packages",
            "limit_reached_badge": "Your included production packages are used up. Upgrade to keep generating.",
            "example_heading": "Example backend request",
            "example_request": "Create a backend-ready sample package for a finance client that includes one homepage demo, two creator cutdowns, and one thumbnail concept that proves we can deliver repeatable premium output without exposing internal ops language.",
            "upgrade_href": "/subscribe",
            "upgrade_label": "Continue with paid production work"
        }),
        "x402-asset-api" => json!({
            "section_title": "Request a payments demo",
            "section_copy": "Paste the app, API concept, product, or integration brief and describe the technical-buyer demo you want. This page is for wallet-native access, delivery unlocks, and programmable media/payment flows that still need to be sold as concrete outputs.",
            "source_label": "Product, API, or integration link",
            "source_placeholder": "Website URL, docs, prototype, app flow, or integration brief",
            "contact_label": "Company or technical buyer",
            "contact_placeholder": "Team, partner, or buyer name",
            "format_label": "Demo format",
            "format_placeholder": "Integration walkthrough, landing-page demo, API explainer, founder sales asset",
            "outcome_label": "Commercial use case",
            "outcome_placeholder": "Paid access, unlock flow, partner integration, creator payout system, technical sales asset",
            "brief_label": "Describe the payments demo you want",
            "brief_placeholder": "Example: create a short buyer-facing demo that shows how a wallet-native USDC payment unlocks a protected media workflow without turning the script into internal engineering language.",
            "launch_label": "Generate my demo",
            "status_idle": "Your request opens the main AI workspace with a structured programmable-payments brief so the agent can generate a real technical-buyer demo.",
            "anon_badge": "Sign in to generate your first payments demo.",
            "helper_title": "What this demo should prove",
            "helper_copy": "Use this demo to show how programmable payments connect to media delivery and access control in a way a buyer can understand quickly.",
            "helper_bullets": [
                "Best for developer products, partner demos, and wallet-native commercial flows.",
                "Turns API/payment logic into a buyer-facing narrative instead of raw internal architecture language.",
                "Technical demos can use UI mockups, Manim/LaTeX diagrams, narration, and long-form assembly when needed.",
                "Your included demos stay available before checkout appears."
            ],
            "included_unit_singular": "demo",
            "included_unit_plural": "demos",
            "limit_reached_badge": "Your included payment demos are used up. Upgrade to keep generating.",
            "example_heading": "Example integration request",
            "example_request": "Create a short buyer-facing demo for VideoSync that shows how a USDC wallet payment on Base unlocks a premium media delivery flow and why that matters for developer-led sales.",
            "upgrade_href": "/subscribe",
            "upgrade_label": "Continue with paid demos"
        }),
        _ => json!({
            "section_title": "Request a custom output",
            "section_copy": "Describe the outcome you want and the agent will open the main production workspace with a structured brief.",
            "source_label": "Reference link or source asset",
            "source_placeholder": "Website URL, Drive folder, video link, screenshot set, or brief",
            "contact_label": "Project or brand name",
            "contact_placeholder": "Name of the project, client, or brand",
            "format_label": "Target format",
            "format_placeholder": "Short clip, demo video, thumbnail concept, animation, or custom output",
            "outcome_label": "Use case",
            "outcome_placeholder": "Where this output will be used and what it should achieve",
            "brief_label": "Describe the result you want",
            "brief_placeholder": "Example: create a polished sample that clearly shows the outcome we can deliver from this source material.",
            "launch_label": "Generate my output",
            "status_idle": "Your request opens the main AI workspace with a structured custom-production brief.",
            "anon_badge": "Sign in to generate your first custom output.",
            "helper_title": "What this output should prove",
            "helper_copy": "Use this path when your use case does not fit one of the standard service offers but still needs a real production deliverable.",
            "helper_bullets": [
                "Works for custom production experiments and buyer-facing samples.",
                "The agent still uses the same full production stack.",
                "Supports clip packs, thumbnails, product mockups, education videos, Blender scenes, voice/audio, and bundles.",
                "Your included outputs stay available before checkout appears."
            ],
            "included_unit_singular": "output",
            "included_unit_plural": "outputs",
            "limit_reached_badge": "Your included outputs are used up. Upgrade to keep generating.",
            "example_heading": "Example custom request",
            "example_request": "Create a polished buyer-facing sample that proves what this system can deliver from the provided source material.",
            "upgrade_href": "/subscribe",
            "upgrade_label": "Continue with paid outputs"
        }),
    }
}
