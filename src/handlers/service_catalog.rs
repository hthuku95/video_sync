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
        "blender-scene-pack" => "3D/2D Animation Scene Pack",
        "voice-audio-pack" => "Voice & Audio Production Pack",
        "mixed-agency-bundle" => "Mixed Agency Production Bundle",
        "creator-manager-fulfillment" => "Agency Production Backend",
        "x402-asset-api" => "Programmable Payments",
        "kick-com-clipping" => "Kick.com Clipping Service",
        _ => "VideoSync custom sample",
    };

    let focus = match service_slug {
        "saas-launch-pack" => "Create a founder-ready demo sample from a website/app URL, screenshots, app recording, or launch brief. The default buyer-facing promise is simple: turn the product into a polished demo/promo video in 24 hours. Most rush demos are 30-60s; longer explainers and walkthroughs are available.",
        "clipper-enhancement-pack" => "Create a premium visual-packaging sample with stronger thumbnails, motion graphics, mockups, overlays, narration support, or branded polish that improves how the final offer looks before anyone clicks play.",
        "thumbnail-hero-pack" => "Create click-focused thumbnail and hero visual concepts with hooks, variants, and QA so the buyer can test stronger first impressions quickly.",
        "product-mockup-pack" => "Create UI mockup scenes and short product videos from a URL, screenshots, Figma exports, app recordings, or a written workflow.",
        "education-explainer-pack" => "Create a lesson or explainer using animated math/science visuals, diagrams, narration, and long-form assembly when the topic needs more than a static asset.",
        "blender-scene-pack" => "Create animated 2D/3D support visuals, product scenes, motion graphics, data visualizations, or cinematic loops.",
        "voice-audio-pack" => "Create voice/audio assets with scripts, VibeVoice narration, summaries, audio visualizers, or narration-backed videos.",
        "mixed-agency-bundle" => "Create a website-to-video agency package: agencies send client websites or app URLs, and VideoSync produces client-ready demo/promo videos they can resell with delivery/download links.",
        "creator-manager-fulfillment" => "Create an agency-backend sample that shows how repeatable client video deliverables could be produced through VideoSync without exposing internal operations language to the buyer.",
        "x402-asset-api" => "Create a technical-buyer sample that shows how paid access, delivery unlocks, or a custom media integration would work in practice without turning the explanation into internal product language.",
        "kick-com-clipping" => "Clip and download Kick.com VODs and livestreams into ready-to-use highlight clips, shorts, and social media formats. The agent receives the Kick.com VOD URL and produces the requested clips with captions, thumbnails, and hooks.",
        _ => "Create a high-quality custom sample that matches the requested service.",
    };
    let production_stack = match service_slug {
        "saas-launch-pack" => "Use long-form assembly when the brief asks for a 30s+ product story, demo, explainer, launch trailer, or multi-cutdown campaign. Pull from UI mockups, product screenshots, animated scenes, VibeVoice narration, Pexels support footage, captions, thumbnails, and Gemini multimodal QA as needed. Default deliverable split: $299 basic rush demo = one tightly scoped short demo, usually 30-60s; $499 full pack = longer or more polished video when needed plus hooks/captions, thumbnail or hero concept, and delivery/download page.",
        "clipper-enhancement-pack" => "Use long-form assembly when the request asks for a clip pack, recap, narrated summary, or multiple enhanced cutdowns. Combine clipping/enhancement tools with captions, thumbnail/hero assets, motion graphics, audio cleanup, and QA instead of treating it as a single raw clip.",
        "thumbnail-hero-pack" => "Use thumbnail/hero generation first, then add short motion loops or product mockups when a static visual alone will not sell the offer.",
        "product-mockup-pack" => "Use UI mockup and long-form assembly when the buyer needs browser/device scenes, app walkthroughs, launch cutdowns, or homepage hero video.",
        "education-explainer-pack" => "Use animated math/science visuals, narration, diagrams, and resumable long-form segments for lessons, tutorials, formulas, and technical explanations.",
        "blender-scene-pack" => "Use animated 3D/2D scenes, render QA, and optional video assembly when 2D/3D motion can make the result more premium than stock footage.",
        "voice-audio-pack" => "Use VibeVoice narration, audio cleanup/visualization, summaries, and optional video assembly depending on whether the deliverable is audio-only or narrated media.",
        "mixed-agency-bundle" => "Use the full canonical tool stack only where it helps the agency deliver client websites as videos. Default package: $999 for 3 client website/app demo videos, each with a delivery/download page and optional hooks, thumbnail/hero concept, mockups, or narration.",
        "creator-manager-fulfillment" => "Use bundle-style long-form assembly for mixed agency packages: demos, clips, thumbnails, product mockups, narration, delivery pages, and review artifacts. The agent should choose the right tool mix instead of forcing one asset type.",
        "x402-asset-api" => "Use long-form assembly for buyer-facing technical demos when a walkthrough needs script, UI mockup, narration, diagrams, and delivery-page proof. Keep the story commercial, not just architectural.",
        "kick-com-clipping" => "Use the manual clipping pipeline with platform detection for Kick.com VODs. Download the VOD via yt-dlp (Kick URLs are supported), detect highlight moments, extract clips, add captions/thumbnails, and assemble into a final deliverable with a delivery page. The agent can also clip highlights from live Kick streams when the VOD becomes available. Use `generate_image` for thumbnails if no existing thumbnail is suitable.",
        _ => "Use long-form assembly whenever the buyer asks for a longer video, a multi-part asset pack, or a package that combines editing, animated 3D/2D scenes, math/science visuals, thumbnails, mockups, voice/audio, QA, and delivery-page output.",
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
        "Service page request for {service_label}.\n{focus}\n{production_stack}\n{url_line}{contact_line}Project brief: {brief}\n\n## Generation Instructions\nNo source files are provided. You must generate everything from scratch:\n- For thumbnails/hero images: call `generate_image` with the brief description as the prompt and output_file=\"outputs/{service_slug}.png\"\n- For videos: call `auto_generate_video` with the brief as the topic\n- For audio: call `generate_text_to_speech` with the script or content\n- Do NOT ask for uploaded files. Do NOT report that no files exist. Just generate.\n- After generating, call `submit_final_answer` with the output file paths."
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
        "blender-scene-pack" => "Animation Scene",
        "voice-audio-pack" => "Voice Audio",
        "mixed-agency-bundle" => "Agency Bundle",
        "creator-manager-fulfillment" => "Agency Backend",
        "x402-asset-api" => "Payments",
        "kick-com-clipping" => "Kick Clipping",
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
        "thumbnail-hero-pack" => json!({
            "section_title": "Request a thumbnail or hero visual",
            "section_copy": "Tell us the topic, product, or campaign goal. No URL needed — just describe what you need clicked on. Our agent generates thumbnail variants, hero images, ad stills, and campaign graphics from your description.",
            "source_label": "Topic, product, or campaign name",
            "source_placeholder": "e.g. AI tech review video, SaaS pricing page launch, finance channel",
            "contact_label": "Brand or channel name",
            "contact_placeholder": "YouTube channel, startup name, or campaign brand",
            "format_label": "Asset type",
            "format_placeholder": "YouTube thumbnail, hero image, ad still, campaign graphic set",
            "outcome_label": "Where this will be used",
            "outcome_placeholder": "YouTube click-through, landing page hero, LinkedIn/Twitter ad",
            "brief_label": "Describe the visual you want",
            "brief_placeholder": "Example: create 3 thumbnail variants for an AI tech review video — one with a shocked face closeup, one with split-screen before/after, and one with bold text overlay. Use blue/orange color scheme.",
            "launch_label": "Generate thumbnails",
            "status_idle": "Your request opens the AI workspace with a structured brief so the agent generates real thumbnail/hero candidates from scratch.",
            "anon_badge": "Sign in to generate your first thumbnails.",
            "helper_title": "What these visuals should prove",
            "helper_copy": "Use this to test whether the agent can produce click-worthy thumbnails and hero images from a simple description, without needing a source URL.",
            "helper_bullets": [
                "No URL or media needed — describe the topic and goal.",
                "Generates multiple variants for A/B testing.",
                "Outputs include hero images, thumbnails, ad stills, and campaign graphics.",
                "Your included visuals stay available before checkout appears."
            ],
            "included_unit_singular": "visual",
            "included_unit_plural": "visuals",
            "limit_reached_badge": "Your included visual packs are used up. Upgrade to keep generating.",
            "example_heading": "Example thumbnail request",
            "example_request": "Create 3 YouTube thumbnail variants for a finance channel video about crypto market trends — one with a shocked expression and green/red chart, one with bold 'BTC $100K' text overlay, and one minimalist with a glowing bitcoin icon on dark background.",
            "upgrade_href": "/subscribe",
            "upgrade_label": "Continue with paid visuals"
        }),
        "product-mockup-pack" => json!({
            "section_title": "Request a product mockup video",
            "section_copy": "Paste a URL, screenshots, Figma exports, or describe the app flow. Our agent turns it into animated UI mockups, device scenes, and short product videos for demos and landing pages.",
            "source_label": "Product URL, screenshots, or app flow",
            "source_placeholder": "https://yourapp.com, Figma link, screenshot folder, or app description",
            "contact_label": "Product or company name",
            "contact_placeholder": "Your product, startup, or client name",
            "format_label": "Target format",
            "format_placeholder": "Device mockup scene, app walkthrough, promo clip, landing page hero",
            "outcome_label": "Where this mockup will be used",
            "outcome_placeholder": "Landing page, Product Hunt, demo video, sales deck, social ad",
            "brief_label": "Describe the mockup video you want",
            "brief_placeholder": "Example: create an iPhone mockup scene showing our app's onboarding flow, with device tilt animation and callout highlights for the key features. End with a clean app store-style frame.",
            "launch_label": "Generate mockup video",
            "status_idle": "Your request opens the AI workspace so the agent can generate a real product mockup video from your source material.",
            "anon_badge": "Sign in to generate your first product mockup.",
            "helper_title": "What this mockup should prove",
            "helper_copy": "Use this mockup to see how the agent turns URLs, screenshots, or descriptions into polished device scenes that make your product look premium.",
            "helper_bullets": [
                "Supports URLs, screenshots, Figma exports, or written app descriptions.",
                "Generates browser/device mockups with motion and callouts.",
                "Can include narration, music, and branded overlays.",
                "Your included mockups stay available before checkout appears."
            ],
            "included_unit_singular": "mockup",
            "included_unit_plural": "mockups",
            "limit_reached_badge": "Your included mockups are used up. Upgrade to keep generating.",
            "example_heading": "Example mockup request",
            "example_request": "Turn videosync.video into an iPhone mockup scene — show the landing page as a device screen with a slight tilt reveal animation, add callout highlights for the key value props, and end with the pricing card in focus.",
            "upgrade_href": "/subscribe",
            "upgrade_label": "Continue with paid mockups"
        }),
        "education-explainer-pack" => json!({
            "section_title": "Request an explainer or lesson video",
            "section_copy": "Describe the topic, lesson outline, or concept you want explained. No URL needed — our agent uses animated math/science visuals, diagrams, narration, and custom graphics to turn any topic into a clear educational video.",
            "source_label": "Topic, lesson, or concept to explain",
            "source_placeholder": "e.g. How blockchain works, quadratic formula proof, Python decorators tutorial",
            "contact_label": "Course or channel name",
            "contact_placeholder": "Your YouTube channel, course name, or brand",
            "format_label": "Video style and length",
            "format_placeholder": "5-min narrated explainer, 15-min lesson, animated formula proof, tutorial",
            "outcome_label": "Who this is for",
            "outcome_placeholder": "YouTube audience, course students, B2B buyers, technical team",
            "brief_label": "Describe the lesson or explainer you want",
            "brief_placeholder": "Example: create a 5-minute narrated explainer on how transformers work in machine learning — open with a simple analogy, show the attention mechanism with animated math/science visuals, include formula visuals, and end with a real-world use case.",
            "launch_label": "Generate explainer video",
            "status_idle": "Your request opens the AI workspace so the agent can generate a real educational video from your topic description.",
            "anon_badge": "Sign in to generate your first explainer.",
            "helper_title": "What this explainer should prove",
            "helper_copy": "Use this to test whether the agent can produce a clear, engaging educational video from just a topic description — no source media needed.",
            "helper_bullets": [
                "No URL or media required — just describe the topic.",
                "Uses animated math/science visuals for technical animations and formulas.",
                "Supports narration, diagrams, stock footage, and long-form assembly.",
                "Your included explainers stay available before checkout appears."
            ],
            "included_unit_singular": "explainer",
            "included_unit_plural": "explainers",
            "limit_reached_badge": "Your included explainers are used up. Upgrade to keep generating.",
            "example_heading": "Example explainer request",
            "example_request": "Create a 4-minute narrated explainer on how GPT models work — start with the concept of next-token prediction using a simple sentence completion example, show the transformer architecture with animated math/science diagram animations, display the attention formula with formula visuals, and end with real applications like chat and code generation.",
            "upgrade_href": "/subscribe",
            "upgrade_label": "Continue with paid explainers"
        }),
        "blender-scene-pack" => json!({
            "section_title": "Request an animated scene",
            "section_copy": "Describe the 3D scene, product animation, or visual you need. No URL needed — our agent generates professional animated scenes from your description. Product animations, abstract visuals, cinematic scenes, and explainer support assets.",
            "source_label": "Scene description or style reference",
            "source_placeholder": "e.g. futuristic city skyline at night, product rotation of a sneaker, abstract network visualization",
            "contact_label": "Project or brand name",
            "contact_placeholder": "Your brand, campaign, or project name",
            "format_label": "Scene type",
            "format_placeholder": "Product animation, cinematic landscape, abstract background, 3D explainer scene",
            "outcome_label": "Where this scene will be used",
            "outcome_placeholder": "Video background, product demo, title sequence, social media asset",
            "brief_label": "Describe the 3D scene you want rendered",
            "brief_placeholder": "Example: create a cinematic product animation of a luxury watch floating in a dark studio with soft rim lighting — slow 360 rotation, shallow depth of field, with a subtle particle sparkle background. 10-second loop.",
            "launch_label": "Generate animated scene",
            "status_idle": "Your request opens the AI workspace so the agent can generate a professional animated scene from your description.",
            "anon_badge": "Sign in to generate your first animated scene.",
            "helper_title": "What this scene should prove",
            "helper_copy": "Use this to test whether the agent can produce production-ready 3D renders from a text description alone — no 3D modeling experience needed.",
            "helper_bullets": [
                "No URL or source media needed — describe the scene.",
                "Supports product animations, abstract visuals, landscapes, and logos.",
                "Renders are uploaded to cloud storage with delivery links.",
                "Your included scenes stay available before checkout appears."
            ],
            "included_unit_singular": "scene",
            "included_unit_plural": "scenes",
            "limit_reached_badge": "Your included animated scenes are used up. Upgrade to keep generating.",
            "example_heading": "Example animated scene request",
            "example_request": "Create a 15-second cinematic product animation of a modern smartwatch — floating in a dark studio with blue LED rim lighting, smooth 360 rotation, brushed metal texture, with floating UI elements fading in around the watch face. 4K resolution.",
            "upgrade_href": "/subscribe",
            "upgrade_label": "Continue with paid scenes"
        }),
        "voice-audio-pack" => json!({
            "section_title": "Request voice or audio production",
            "section_copy": "Share a script, topic, article, or rough notes. No source media needed — our agent writes scripts, generates voiceovers, creates audio summaries, and produces narrated video packages from your description.",
            "source_label": "Script, topic, or source material",
            "source_placeholder": "e.g. 60-second product ad script, article link, podcast outline, YouTube video notes",
            "contact_label": "Project or brand name",
            "contact_placeholder": "Your brand, channel, or project name",
            "format_label": "Audio format",
            "format_placeholder": "Voiceover, podcast audio, narrated video, audio summary, radio ad",
            "outcome_label": "Where this audio will be used",
            "outcome_placeholder": "YouTube video, podcast episode, social clip, sales asset, course narration",
            "brief_label": "Describe the audio or narration you want",
            "brief_placeholder": "Example: create a 90-second narrated product ad script and voiceover for a project management SaaS — energetic, conversational tone, open with the pain of scattered workflows, show the solution, end with a soft CTA. Include background music.",
            "launch_label": "Generate audio",
            "status_idle": "Your request opens the AI workspace so the agent can generate real voice/audio assets from your script or topic.",
            "anon_badge": "Sign in to generate your first audio production.",
            "helper_title": "What this audio should prove",
            "helper_copy": "Use this to test whether the agent can write scripts, generate natural-sounding voiceovers, and produce professional audio from just a topic or brief.",
            "helper_bullets": [
                "No source media needed — describe the script or topic.",
                "AI writes scripts and generates voiceovers in multiple styles.",
                "Supports audio visualizers, narrated videos, and podcast-style production.",
                "Your included audio productions stay available before checkout appears."
            ],
            "included_unit_singular": "audio",
            "included_unit_plural": "audio files",
            "limit_reached_badge": "Your included audio productions are used up. Upgrade to keep generating.",
            "example_heading": "Example audio request",
            "example_request": "Create a 60-second narrated ad script and voiceover for a productivity app — conversational but confident tone, open with the pain of context switching, describe the solution benefits, end with 'Start your free trial today.' Include soft background music.",
            "upgrade_href": "/subscribe",
            "upgrade_label": "Continue with paid audio"
        }),
        "mixed-agency-bundle" => json!({
            "section_title": "Request an agency demo pack",
            "section_copy": "Send up to 3 client websites or app URLs. Our agent produces client-ready demo/promo videos you can resell under your own brand. Each client gets a delivery page with preview and downloads.",
            "source_label": "Client website URLs (up to 3)",
            "source_placeholder": "https://client1.com, https://client2.com, https://client3.com",
            "contact_label": "Agency or account name",
            "contact_placeholder": "Your agency name or client account",
            "format_label": "Video style per client",
            "format_placeholder": "Short promo (30s), walkthrough (60s), narrated demo, or mixed",
            "outcome_label": "Resale plan",
            "outcome_placeholder": "Upsell to existing clients, new business offer, monthly retainer package",
            "brief_label": "Describe the agency pack you want",
            "brief_placeholder": "Example: produce 3 client demo videos — a 30-second homepage promo for each, with device mockups, clean motion, no narration, and delivery pages the clients can preview and download. Agency white-label branding.",
            "launch_label": "Generate agency pack",
            "status_idle": "Your request opens the AI workspace so the agent can produce a real 3-client agency demo pack from the URLs you provide.",
            "anon_badge": "Sign in to generate your first agency pack.",
            "helper_title": "What this pack should prove",
            "helper_copy": "Use this to test whether the pipeline can turn multiple client URLs into distinct, polished demo videos suitable for agency resale.",
            "helper_bullets": [
                "Send up to 3 client website URLs.",
                "Each client gets their own delivery page with preview and downloads.",
                "Supports narration, mockups, thumbnails, and white-label branding.",
                "Your included packs stay available before checkout appears."
            ],
            "included_unit_singular": "client video",
            "included_unit_plural": "client videos",
            "limit_reached_badge": "Your included agency packs are used up. Upgrade to keep generating.",
            "example_heading": "Example agency pack request",
            "example_request": "Produce 3 client demo videos: client 1 is a fintech landing page — 30s short promo with device mockup, client 2 is a SaaS dashboard — 45s narrated walkthrough explaining the workflow, client 3 is an ecommerce store — 30s promo with product shots. Each with a delivery page.",
            "upgrade_href": "/subscribe",
            "upgrade_label": "Continue with paid agency packs"
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
                "Technical demos can use UI mockups, animated math/science diagrams, narration, and long-form assembly when needed.",
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
        "kick-com-clipping" => json!({
            "section_title": "Request Kick.com clips",
            "section_copy": "Paste a Kick.com VOD, channel, or clip link and tell us what highlights you want extracted. Our agent downloads the source video, clips the best moments, and delivers ready-to-use shorts, highlights, and social clips.",
            "source_label": "Kick.com VOD or channel link",
            "source_placeholder": "https://kick.com/video/xxxxx or https://kick.com/channelname",
            "contact_label": "Creator or streamer name",
            "contact_placeholder": "Name of the Kick streamer or brand",
            "format_label": "Clip format and platform",
            "format_placeholder": "YouTube Shorts, TikTok, Twitter clip, Instagram Reel, raw highlight",
            "outcome_label": "Type of clips you want",
            "outcome_placeholder": "Top plays, chat reactions, funny moments, educational highlights, full VOD summary",
            "brief_label": "Describe the clips you want extracted",
            "brief_placeholder": "Example: extract the top 3 highlights from this 2-hour Kick stream — the clutch win at 45:00, the donation read at 1:12:00, and the rage moment at 1:45:00. Make each into a 30-second vertical short with captions.",
            "launch_label": "Extract my clips",
            "status_idle": "Your request opens the main AI workspace with a structured clipping brief so the agent can download the Kick VOD, extract highlights, add captions/thumbnails, and deliver the clips.",
            "anon_badge": "Sign in to clip your first Kick.com VOD.",
            "helper_title": "What these clip packages include",
            "helper_copy": "Use this service to get professional highlight clips from any Kick.com VOD — ready to post on YouTube, TikTok, Twitter, or Instagram. No more screen recording from the browser.",
            "helper_bullets": [
                "Supports any public Kick.com VOD URL — just paste and describe the moments you want.",
                "Each clip comes with clean captions, a thumbnail, and hook text.",
                "Delivered as downloadable video files with a shareable delivery page.",
                "Great for clip channels, streamer editors, compilations, and social media managers."
            ],
            "included_unit_singular": "clip",
            "included_unit_plural": "clips",
            "limit_reached_badge": "Your included Kick clipping credits are used up. Upgrade to keep extracting clips.",
            "example_heading": "Example Kick clipping request",
            "example_request": "Extract 3 highlight clips from this Kick streamer's latest 3-hour VOD — one big play, one funny chat interaction, and one donation readout. Make each 25-40 seconds with dynamic captions, a hook title card, and a consistent thumbnail style.",
            "upgrade_href": "/subscribe",
            "upgrade_label": "Continue with paid clipping"
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
                "Supports clip packs, thumbnails, product mockups, education videos, animated 3D/2D scenes, voice/audio, and bundles.",
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

pub struct ServicePortfolioBrief {
    pub service_slug: &'static str,
    pub name: &'static str,
    pub brief: &'static str,
    pub description: &'static str,
}

pub fn get_service_portfolio_briefs(service_slug: &str) -> Vec<ServicePortfolioBrief> {
    match service_slug {
        "saas-launch-pack" => vec![
            ServicePortfolioBrief {
                service_slug: "saas-launch-pack",
                name: "SaaS Product Demo",
                description: "A 60-second narrated product demo showing the full SaaS workflow from landing page to key features.",
                brief: "Create a 60-second narrated product demo for a fictional project management SaaS called 'FlowForge'. The demo should open with the pain point of scattered team communication, show the clean kanban board interface (generate a UI mockup for this), include a short device mockup scene, explain the key workflow with on-screen captions, and end with a CTA. Use background music and professional voiceover narration throughout. The video should be polished enough to use on a homepage or Product Hunt launch page.",
            },
            ServicePortfolioBrief {
                service_slug: "saas-launch-pack",
                name: "App Walkthrough Explainer",
                description: "A 90-second walkthrough that explains a mobile app's value proposition with device mockups and narration.",
                brief: "Create a 90-second app walkthrough video for a fitness tracking app called 'FitPulse'. Generate an iPhone UI mockup scene showing the app's main dashboard with daily stats. The walkthrough should: open with a problem statement (hard to stay consistent), show the app's simple check-in flow with motion callouts, display a progress chart animation, and end with results/testimonials. Include voiceover narration, background music, and captions throughout. Deliver as a delivery page URL.",
            },
        ],
        "mixed-agency-bundle" => vec![
            ServicePortfolioBrief {
                service_slug: "mixed-agency-bundle",
                name: "3-Client Agency Demo Pack",
                description: "Three 30-second client demos for different industries — fintech, ecommerce, and SaaS — showing agency resale capability.",
                brief: "Create a 3-client agency demo pack suitable for resale. Client 1 is a fintech landing page called 'VaultPay': produce a 30-second promo with device mockup, clean motion, and soft background music — no narration, keep it sleek. Client 2 is an ecommerce store called 'GreenLeaf Market': create a 30-second product showcase with smooth transitions and product imagery. Client 3 is a SaaS dashboard called 'TeamSync': create a 30-second narrated walkthrough explaining the workflow. Each client should have their own delivery page with preview and download links. White-label, no VideoSync branding.",
            },
        ],
        "product-mockup-pack" => vec![
            ServicePortfolioBrief {
                service_slug: "product-mockup-pack",
                name: "iPhone App Mockup Scene",
                description: "Animated iPhone mockup with reveal animation showcasing a food delivery app.",
                brief: "Create an iPhone mockup scene for a food delivery app called 'QuickBite'. Generate an iPhone device frame with a reveal animation showing the app's home screen with restaurant listings. Add motion callouts for the key features: one-tap ordering, real-time tracking, and saved favorites. The scene should be 15 seconds, include soft background music, and end with a clean app store-style frame. Deliver as a video with delivery page.",
            },
        ],
        "education-explainer-pack" => vec![
            ServicePortfolioBrief {
                service_slug: "education-explainer-pack",
                name: "Machine Learning Explainer",
                description: "A 4-minute narrated explainer on how neural networks work, using animated math/science visuals and formula visuals.",
                brief: "Create a 4-minute narrated educational explainer on how neural networks work. Start with a simple analogy (brain neurons vs artificial neurons), show the network architecture with animated math/science visuals (input layer → hidden layers → output layer), display key formulas with formula visuals (activation function, weight update), include a real example of digit recognition with visual demonstrations, and end with practical applications. The video should have: clear voiceover narration, background music, on-screen captions, and animated math/science diagrams. Suitable for a YouTube educational channel or course module.",
            },
            ServicePortfolioBrief {
                service_slug: "education-explainer-pack",
                name: "Math Formula Proof",
                description: "Step-by-step animated proof of the quadratic formula using formula and animated visuals.",
                brief: "Create a 3-minute animated math lesson proving the quadratic formula. Start with the standard form ax²+bx+c=0, walk through completing the square step by step with formula visuals, derive the final formula x = (-b±√(b²-4ac))/2a, and show a concrete example. Use animated math/science visuals for smooth math animations, color-coded steps, and a clean educational style. Include voiceover narration and background music. Deliver as a complete video with delivery page.",
            },
        ],
        "blender-scene-pack" => vec![
            ServicePortfolioBrief {
                service_slug: "blender-scene-pack",
                name: "Product Animation — Smartwatch",
                description: "Cinematic 15-second product animation of a modern smartwatch with rim lighting and rotation.",
                brief: "Create a cinematic 15-second product animation of a modern smartwatch floating in a dark studio. The watch should have a brushed metal finish with blue LED rim lighting, perform a slow 360-degree rotation, and have subtle floating UI elements fading in around the watch face. Background should be dark with soft volumetric lighting. 4K resolution. Include soft cinematic background music. Output as a rendered video file with delivery link.",
            },
            ServicePortfolioBrief {
                service_slug: "blender-scene-pack",
                name: "Abstract Tech Background",
                description: "Abstract futuristic network visualization with glowing nodes and data streams.",
                brief: "Create a 10-second abstract futuristic network visualization animation. Glowing blue nodes connected by light beams on a dark background with floating data particles. The camera should slowly orbit the network. Include cinematic ambient music. Suitable for use as a tech video background or intro sequence. Deliver as a rendered video with delivery page.",
            },
        ],
        "thumbnail-hero-pack" => vec![
            ServicePortfolioBrief {
                service_slug: "thumbnail-hero-pack",
                name: "AI Tech Review Thumbnail Set",
                description: "3 YouTube thumbnail variants for an AI technology review video with different visual approaches.",
                brief: "Create 3 YouTube thumbnail variants for an AI technology review video. Variant 1: shocked face closeup with blue/neon glow effect and bold 'MINDBLOWING' text overlay. Variant 2: split-screen showing 'before' (old tech) vs 'after' (AI) with dramatic lighting difference. Variant 3: minimalist — glowing AI brain icon on pure dark background with 'THE FUTURE' in clean sans-serif text. All should be 1280x720, high contrast, with vibrant colors optimized for click-through.",
            },
        ],
        "clipper-enhancement-pack" => vec![
            ServicePortfolioBrief {
                service_slug: "clipper-enhancement-pack",
                name: "Gaming Highlight to Premium Short",
                description: "Turn a gaming clip into a premium short with faster hook, cleaner captions, and motion graphics.",
                brief: "Create a premium short-form video suitable for YouTube Shorts or TikTok. Take a gaming highlight concept: a player clutches a 1v3 situation in a competitive match. The short should open with a fast hook ('THIS is why they call him the clutch king'), show the highlight with clean motion-tracked captions highlighting key moments (health low, last bullet, victory), add a branded lower third for the streamer name, end with a subscribe animation. Total length: 30-45 seconds. Include energetic background music and sound effects. Deliver as a video file with delivery page.",
            },
        ],
        "voice-audio-pack" => vec![
            ServicePortfolioBrief {
                service_slug: "voice-audio-pack",
                name: "Narrated Product Ad",
                description: "60-second narrated product ad script and voiceover with background music for a productivity app.",
                brief: "Create a 60-second narrated product advertisement for a productivity app called 'FocusFlow'. The script should: open with the pain of constant distractions ('Your to-do list is growing but your focus is shrinking'), present the solution (one-tap focus mode, smart scheduling, progress tracking), describe the benefits with a conversational but confident tone, and end with a soft CTA ('Start your free trial today — FocusFlow'). Generate the voiceover using a professional male voice, include soft uplifting background music throughout, and produce an audio visualizer video to go with it. Deliver as both an audio file and a narrated video.",
            },
        ],
        "kick-com-clipping" => vec![
            ServicePortfolioBrief {
                service_slug: "kick-com-clipping",
                name: "Kick Gaming Highlights Pack",
                description: "Extract 3 highlight clips from a 2-hour Kick gaming VOD with captions and thumbnails.",
                brief: "Download a 2-hour Kick.com gaming VOD and extract the 3 best highlight moments. Clip 1: the final circle clutch win where the player wins 1v3 with low health — add fast-paced captions highlighting key HP callouts and the winning shot. Clip 2: a funny donation readout mid-match with chat reaction overlay — include clean captions of the donation message. Clip 3: an educational moment where the player explains their strategy — pull a 45-second segment with clear strategy callout captions. Each clip should be 25-45 seconds, have a hook title card, dynamic captions, and a thumbnail. Deliver as three separate video files with a delivery page.",
            },
            ServicePortfolioBrief {
                service_slug: "kick-com-clipping",
                name: "Kick IRL Stream Compilation",
                description: "Create a 60-second compilation of the best moments from a Kick IRL (in real life) stream VOD.",
                brief: "Download a Kick.com IRL stream VOD and create a 60-second compilation of the top moments. Select 4-6 short segments that show the funniest, most entertaining, or most outrageous moments from the stream. Arrange them in a high-energy sequence with: a hook intro title card, fast cuts, dynamic captions identifying each moment, background music that matches the energy, and an end screen with 'Subscribe' call-to-action. Include a thumbnail that captures the best expression or scene from the compilation. Deliver as a single compilation video with delivery page.",
            },
        ],
        _ => vec![],
    }
}

pub fn all_service_portfolio_briefs() -> Vec<ServicePortfolioBrief> {
    let mut all = Vec::new();
    for slug in &[
        "saas-launch-pack",
        "mixed-agency-bundle",
        "product-mockup-pack",
        "education-explainer-pack",
        "blender-scene-pack",
        "thumbnail-hero-pack",
        "clipper-enhancement-pack",
        "voice-audio-pack",
        "kick-com-clipping",
    ] {
        all.extend(get_service_portfolio_briefs(slug));
    }
    all
}
