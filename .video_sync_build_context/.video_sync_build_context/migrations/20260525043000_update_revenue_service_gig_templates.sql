-- Align Content Machine gig templates with the current VideoSync services.
-- These rows are used by marketers/whitelisted users to copy offers and generate proof samples.

UPDATE gig_templates
SET
    service_type = 'saas_demo',
    display_name = 'SaaS/App Demo Video Pack',
    tagline = 'Turn a product URL into a polished buyer-facing demo video',
    description = E'I create polished SaaS/app demo videos from your website, app URL, screenshots, Loom, or short brief.\n\nThis is built for founders, indie hackers, product marketers, and agencies that need launch-ready video assets fast without waiting weeks for a traditional studio.\n\nWhat I can deliver:\n• Product demo or promo video\n• Animated UI/browser/device mockups\n• Captions, callouts, and product story structure\n• Optional narration and launch hooks\n• Delivery page with preview and download links\n\nBest for Product Hunt launches, homepage videos, X/LinkedIn posts, sales pages, onboarding, and investor updates.',
    basic_price = 399,
    basic_delivery_days = 2,
    basic_includes = 'Starter demo up to 60-90s, product story, captions/callouts, HD delivery, 1 revision',
    standard_price = 699,
    standard_delivery_days = 2,
    standard_includes = 'Launch demo pack: polished demo, hooks/captions, thumbnail or hero concept, delivery page, 2 revisions',
    premium_price = 1200,
    premium_delivery_days = 4,
    premium_includes = 'Walkthrough or campaign pack: longer video, multiple variants, stronger motion/narration polish, priority review',
    keywords = '["saas demo video","app demo video","product demo video","startup launch video","product hunt video","website to video","software explainer"]',
    gig_titles = '["I will turn your SaaS or app website into a polished product demo video","I will create a launch-ready SaaS demo video from your product URL","I will make a professional app walkthrough or promo video for your startup"]',
    sample_prompts = '["Create a 60-second Cal.com style scheduling SaaS demo showing pain, product workflow, proof, and CTA","Create a product launch demo for an AI meeting notes app using browser mockups, captions, and narration","Create a homepage SaaS demo for a CRM tool with UI callouts, buyer benefits, and LinkedIn-ready framing"]',
    sort_order = 1
WHERE display_name = 'YouTube Automation Full Video Package';

UPDATE gig_templates
SET
    service_type = 'agency_pack',
    display_name = 'Website-to-Video Agency Pack',
    tagline = '3 client websites turned into 3 resellable demo videos',
    description = E'I help agencies turn client websites, landing pages, and SaaS apps into polished video deliverables they can resell.\n\nSend 3 client URLs and I will create 3 client-ready demo/promo videos with delivery pages and downloadable assets.\n\nBest for Webflow, Framer, SaaS, marketing, and no-code agencies that already have clients and want a high-value video upsell without hiring a full production team.',
    basic_price = 1500,
    basic_delivery_days = 5,
    basic_includes = '3 client website/app demo videos, delivery pages, HD downloads, 1 revision per video',
    standard_price = 2500,
    standard_delivery_days = 7,
    standard_includes = '5 client videos, thumbnails/hero concepts, hooks/captions, delivery pages, 2 revisions',
    premium_price = 5000,
    premium_delivery_days = 14,
    premium_includes = 'Monthly agency backend starter: recurring client videos, priority queue, mixed assets, private fulfillment support',
    keywords = '["website to video","agency video fulfillment","webflow agency video","framer agency video","client demo videos","white label video production"]',
    gig_titles = '["I will turn your agency client websites into resellable demo videos","I will create 3 client-ready website demo videos for your agency","I will be your private website-to-video production backend"]',
    sample_prompts = '["Create 3 short demo video concepts for Webflow agency clients using their website URLs and buyer outcomes","Create a Framer agency resale sample showing how a client landing page becomes a polished promo video","Create an agency backend fulfillment sample with delivery page copy and video proof positioning"]',
    sort_order = 2
WHERE display_name = 'Animated Title Card / Channel Intro';

UPDATE gig_templates
SET
    service_type = 'product_mockup',
    display_name = 'Product Mockup Video Pack',
    tagline = 'Screenshots and product flows turned into animated mockup videos',
    description = E'I create animated UI/product mockup videos from websites, screenshots, app flows, or product briefs.\n\nThis is ideal when your product is useful but you need it to look premium for ads, launch pages, sales decks, or social posts.\n\nDeliverables can include browser scenes, device mockups, animated callouts, short promo clips, and delivery/download pages.',
    basic_price = 299,
    basic_delivery_days = 2,
    basic_includes = '1 short product mockup clip up to 30s, one device/browser scene, HD delivery',
    standard_price = 599,
    standard_delivery_days = 3,
    standard_includes = 'Product mockup video up to 60-90s, multiple scenes, callouts, 2 revisions',
    premium_price = 900,
    premium_delivery_days = 5,
    premium_includes = 'Full app-flow mockup pack with multiple variants, hero visual, captions/hooks, delivery page',
    keywords = '["product mockup video","ui mockup animation","app promo video","device mockup video","saas mockup","browser mockup video"]',
    gig_titles = '["I will create an animated product mockup video from your app screenshots","I will turn your website or app flow into a polished UI promo video","I will make device and browser mockup videos for your SaaS or app"]',
    sample_prompts = '["Create an animated browser mockup video for a project management SaaS showing dashboard, tasks, and reporting","Create an iPhone app promo mockup for a fitness tracker with 3 scenes and benefit callouts","Create a product mockup sales clip from screenshots for a no-code booking app"]',
    sort_order = 3
WHERE display_name = 'UI Mockup / Device Demo Video';

UPDATE gig_templates
SET
    service_type = 'education_explainer',
    display_name = 'Education Explainer Pack',
    tagline = 'Technical lessons with diagrams, Manim, LaTeX, narration, and long-form assembly',
    description = E'I create visual education explainers for technical topics, courses, YouTube lessons, and B2B product education.\n\nSend a topic, outline, script, paper, or rough notes. I can turn it into diagrams, Manim/LaTeX scenes, narrated explainers, and longer educational videos.\n\nBest for course creators, technical YouTubers, founders, educators, and teams that need hard ideas explained clearly.',
    basic_price = 300,
    basic_delivery_days = 3,
    basic_includes = 'Short visual explainer up to 90s, diagrams or equations, HD delivery, 1 revision',
    standard_price = 750,
    standard_delivery_days = 5,
    standard_includes = '3-5 minute explainer, Manim/LaTeX or diagram scenes, narration, captions, 2 revisions',
    premium_price = 1500,
    premium_delivery_days = 10,
    premium_includes = 'Long-form lesson/module pack with segmented workflow, reusable assets, QA, delivery page',
    keywords = '["education explainer","technical explainer video","manim animation","latex animation","course video","stem animation","visual lesson"]',
    gig_titles = '["I will create a visual educational explainer video for your technical topic","I will animate your lesson with Manim, LaTeX, diagrams, and narration","I will turn your course topic into a polished long-form explainer video"]',
    sample_prompts = '["Create a 3-minute visual explainer for vector databases using diagrams, simple examples, and narration","Create a Manim-style lesson explaining derivatives with step-by-step formula animation","Create a technical API explainer for developers with diagrams, code callouts, and voiceover"]',
    sort_order = 4
WHERE display_name IN ('Educational Explainer Video', 'LaTeX Math & Physics Animation');

UPDATE gig_templates
SET
    service_type = 'blender_scene',
    display_name = 'Blender 2D/3D Scene Pack',
    tagline = 'Cinematic product scenes, 3D explainers, animated models, and support visuals',
    description = E'I create Blender-based 2D/3D scenes and product animations for videos, demos, explainers, and campaigns.\n\nThis is for buyers who need something more distinctive than stock footage or template motion graphics: product scenes, cinematic loops, 3D explainers, animated objects, title cards, and support visuals that can be inserted into larger videos.',
    basic_price = 500,
    basic_delivery_days = 4,
    basic_includes = '1 short rendered scene or product animation up to 20-30s, HD, 1 revision',
    standard_price = 1200,
    standard_delivery_days = 7,
    standard_includes = 'Scene pack up to 60s, multiple shots, lighting/camera polish, 2 revisions',
    premium_price = 2500,
    premium_delivery_days = 14,
    premium_includes = 'Premium 3D visual package with multiple scenes, stronger art direction, 4K where useful, priority review',
    keywords = '["blender animation","3d product animation","3d explainer","product scene","cinematic product video","3d marketing video"]',
    gig_titles = '["I will create cinematic Blender product scenes for your brand or video","I will make a premium 2D or 3D explainer scene in Blender","I will design animated 3D support visuals for your product demo or course"]',
    sample_prompts = '["Create a premium 3D product scene for a cybersecurity SaaS using abstract shields, dashboards, and dark blue lighting","Create a Blender support scene showing a physical product rotating on a clean studio surface","Create an animated 3D concept visual for cloud infrastructure with nodes, data flow, and cinematic camera movement"]',
    sort_order = 5
WHERE display_name = '3D Blender Scene / Product Animation';

UPDATE gig_templates
SET
    service_type = 'thumbnail_hero',
    display_name = 'Thumbnail & Hero Visual Pack',
    tagline = 'Click-focused thumbnails, hero images, ad stills, and campaign visuals',
    description = E'I create high-impact thumbnails and hero visuals for YouTube videos, SaaS launches, ads, newsletters, landing pages, and campaigns.\n\nThis is not just a cheap thumbnail gig. The goal is a buyer-facing visual that improves first impression, click intent, and campaign clarity.\n\nDeliverables can include thumbnail variants, hero concepts, ad stills, hook/caption options, and download-ready assets.',
    basic_price = 75,
    basic_delivery_days = 1,
    basic_includes = '1 polished thumbnail or hero visual, 1 revision, download-ready PNG/JPG',
    standard_price = 150,
    standard_delivery_days = 2,
    standard_includes = '3 visual variants, hook/caption options, 2 revisions',
    premium_price = 300,
    premium_delivery_days = 3,
    premium_includes = 'Campaign visual pack with thumbnails, hero visual, ad stills, and source/variant handoff',
    keywords = '["youtube thumbnail","hero image design","saas hero visual","ad creative","campaign graphics","thumbnail design"]',
    gig_titles = '["I will create click-focused thumbnails and hero visuals for your campaign","I will design premium YouTube thumbnails or SaaS hero visuals","I will create ad-ready visual concepts for your product or video"]',
    sample_prompts = '["Create 3 thumbnail concepts for a SaaS launch video with bold contrast, product UI, and clear hook text","Create a hero visual for an AI customer support product landing page","Create a campaign visual pack for a creator education video about AI tools"]',
    sort_order = 6
WHERE display_name = '3D YouTube Thumbnail';

UPDATE gig_templates
SET
    service_type = 'clip_enhancement',
    display_name = 'Clip Enhancement Pack',
    tagline = 'Turn raw clips into polished social-ready videos with captions, title cards, and motion',
    description = E'I enhance raw clips, highlights, podcast moments, screen recordings, and short videos into polished social-ready assets.\n\nDeliverables can include captions, title cards, lower thirds, motion graphics, thumbnails, exports, and multiple platform variants.\n\nBest for creators, agencies, coaches, founders, and brands that already have footage but need it packaged professionally.',
    basic_price = 250,
    basic_delivery_days = 3,
    basic_includes = '3 polished clips with captions/title cards, HD delivery, 1 revision',
    standard_price = 600,
    standard_delivery_days = 5,
    standard_includes = '10 clip pack with captions, graphics, thumbnails, platform variants, 2 revisions',
    premium_price = 1200,
    premium_delivery_days = 7,
    premium_includes = 'Monthly-style content pack with 20 clips, visual system, thumbnails, delivery links, priority review',
    keywords = '["short form video editing","clip editing","captioned reels","youtube shorts editing","tiktok editing","motion graphics clips"]',
    gig_titles = '["I will turn your raw clips into polished captioned social videos","I will create a professional short-form clip pack from your footage","I will edit your podcast or long-form content into social-ready clips"]',
    sample_prompts = '["Create a polished 3-clip social pack from a podcast highlight with captions, title cards, and platform-safe framing","Create a creator clip enhancement sample with hook text, lower thirds, and thumbnail frame","Create a B2B founder clip package from a screen recording and talking-head intro"]',
    sort_order = 7
WHERE display_name = 'Lower Thirds & Broadcast Graphics Bundle';

UPDATE gig_templates
SET
    service_type = 'voice_audio',
    display_name = 'Voice & Audio Production Pack',
    tagline = 'Narration, voiceovers, podcast-style audio, summaries, and audio-backed videos',
    description = E'I create narration and audio-backed content for product videos, explainers, summaries, podcasts, courses, and sales assets.\n\nSend a script, topic, article, video, or rough notes. I can produce voiceover-ready scripts, narrated summaries, audio visualizers, and videos that combine narration with motion assets.\n\nBest for founders, educators, agencies, newsletter operators, and creators who need clean audio content fast.',
    basic_price = 99,
    basic_delivery_days = 2,
    basic_includes = 'Short narration or audio summary up to 2 minutes, script polish, downloadable file',
    standard_price = 300,
    standard_delivery_days = 3,
    standard_includes = 'Narrated explainer/audio-backed video up to 5 minutes, script, voiceover, visual pairing',
    premium_price = 750,
    premium_delivery_days = 5,
    premium_includes = 'Longer audio/video package, multiple sections, summaries, delivery page, priority review',
    keywords = '["voiceover service","audio production","narrated explainer","podcast summary","audio backed video","ai narration"]',
    gig_titles = '["I will create professional narration or voiceover-backed video assets","I will turn your script, article, or video into a narrated summary","I will produce audio-backed explainers and summaries for your brand or course"]',
    sample_prompts = '["Create a narrated 3-minute product summary from a SaaS landing page and turn it into an audio-backed video outline","Create a podcast-style audio summary for a technical blog post with intro, sections, and CTA","Create a voiceover-backed education explainer from a short lesson outline"]',
    sort_order = 8
WHERE display_name = 'Animated Data Visualization';
