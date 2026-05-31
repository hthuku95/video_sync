-- Gig Templates system (Apr 1, 2026)
-- Stores Fiverr/PPH gig template info + generated sample videos.

CREATE TABLE IF NOT EXISTS gig_templates (
    id                     UUID        PRIMARY KEY DEFAULT gen_random_uuid(),
    service_type           TEXT        NOT NULL,   -- scene | thumbnail | title_card | data_viz | lower_third | latex | ui_mockup | auto_video
    display_name           TEXT        NOT NULL,
    tagline                TEXT        NOT NULL,
    description            TEXT        NOT NULL,   -- full Fiverr gig description (copy-paste ready)
    basic_price            INT         NOT NULL,
    basic_delivery_days    INT         NOT NULL,
    basic_includes         TEXT        NOT NULL,
    standard_price         INT         NOT NULL,
    standard_delivery_days INT         NOT NULL,
    standard_includes      TEXT        NOT NULL,
    premium_price          INT         NOT NULL,
    premium_delivery_days  INT         NOT NULL,
    premium_includes       TEXT        NOT NULL,
    keywords               JSONB       NOT NULL DEFAULT '[]',
    gig_titles             JSONB       NOT NULL DEFAULT '[]',
    sample_prompts         JSONB       NOT NULL DEFAULT '[]',
    sort_order             INT         NOT NULL DEFAULT 0,
    created_at             TIMESTAMPTZ NOT NULL DEFAULT NOW()
);

CREATE TABLE IF NOT EXISTS gig_sample_videos (
    id              UUID        PRIMARY KEY DEFAULT gen_random_uuid(),
    template_id     UUID        NOT NULL REFERENCES gig_templates(id) ON DELETE CASCADE,
    title           TEXT        NOT NULL,
    prompt_used     TEXT        NOT NULL,
    status          TEXT        NOT NULL DEFAULT 'pending',
    output_r2_url   TEXT,
    output_filename TEXT,
    error_message   TEXT,
    created_at      TIMESTAMPTZ NOT NULL DEFAULT NOW(),
    completed_at    TIMESTAMPTZ
);

CREATE INDEX IF NOT EXISTS idx_gig_samples_template ON gig_sample_videos(template_id);

-- ─── SEED DATA ───────────────────────────────────────────────────────────────

INSERT INTO gig_templates (service_type, display_name, tagline, description,
    basic_price, basic_delivery_days, basic_includes,
    standard_price, standard_delivery_days, standard_includes,
    premium_price, premium_delivery_days, premium_includes,
    keywords, gig_titles, sample_prompts, sort_order)
VALUES

-- 1. 3D Blender Scene / Product Animation
('scene',
 '3D Blender Scene / Product Animation',
 'Jaw-dropping 3D product animations delivered in 24–48 hours',
 E'Looking for a jaw-dropping 3D animation that makes your brand unforgettable?\n\nI create professional Blender 3D animations for products, brands, and corporate presentations — using AI-powered rendering that delivers studio quality at freelance speed.\n\n✅ What you get:\n• Photorealistic 3D Blender rendering\n• Custom camera movements and dynamic lighting\n• Professional color grading\n• MP4 delivery up to 4K quality\n• Commercial license included\n• Source file available on request (Premium)\n\n🎯 Perfect for:\n• Amazon & Shopify product showcases\n• Brand intros and promotional videos\n• Corporate presentations\n• Social media content\n• NFT and digital art\n\n⚡ Why choose me:\nMost 3D animators take 2–4 weeks. My AI-assisted pipeline delivers the same quality in 24–48 hours. No compromises on quality.',
 150, 3, '1 x 3D scene clip (up to 30s), 1920×1080 HD, 1 revision',
 400, 2, '1 x 3D scene clip (up to 60s), 1920×1080 HD, dynamic camera, custom lighting, 2 revisions',
 750, 1, '1 x full 3D animation (up to 90s), 4K, custom camera sequence, color grade, music-ready audio markers, unlimited revisions',
 '["blender 3d animation","product animation","3d product video","motion graphics blender","3d scene animation","product showcase video","blender render","3d marketing video"]',
 '["I will create a stunning 3D product animation video in Blender","I will make a cinematic 3D Blender animation for your product or brand","I will design a professional 3D motion graphics product showcase video"]',
 '["Futuristic tech gadget floating in space with dynamic camera orbit and neon accent lighting","Luxury skincare bottle on marble surface with warm studio lighting and slow 360 rotation","Abstract corporate logo reveal with flowing particle effects and metallic sheen","Sports equipment product showcase with energetic camera movements and dramatic rim lighting","Minimalist premium tech product on dark reflective surface with sleek studio atmosphere"]',
 1),

-- 2. YouTube Thumbnail (3D)
('thumbnail',
 '3D YouTube Thumbnail',
 'Eye-catching AI-rendered 3D thumbnails that boost click-through rates',
 E'Stop losing clicks to boring thumbnails.\n\nI create stunning 3D-rendered YouTube thumbnails using Blender and AI that grab attention in a crowded feed and dramatically improve your click-through rate.\n\n✅ What you get:\n• High-resolution 3D rendered thumbnail (1280×720 PNG)\n• Custom title text overlay\n• Multiple style options (dark, vibrant, minimal, dramatic)\n• Commercial license included\n• PNG + layered PSD available (Premium)\n\n🎯 Perfect for:\n• YouTubers wanting to stand out\n• Tech, finance, gaming, and lifestyle channels\n• Faceless YouTube automation channels\n• Course creators and educators\n\n⚡ Most Fiverr thumbnails are flat 2D Photoshop designs. Mine are rendered in 3D Blender — a completely different visual league.',
 35, 1, '1 thumbnail (1280×720 PNG), custom title text, 1 style, 1 revision',
 90, 1, '3 thumbnail variations, 2 style options, A/B testing pack, 2 revisions',
 200, 1, '5 thumbnails, all style variants, PNG + source file, priority queue, unlimited revisions',
 '["youtube thumbnail","3d thumbnail","thumbnail design","youtube click through rate","eye catching thumbnail","blender thumbnail","ai thumbnail","youtube channel art"]',
 '["I will create a stunning 3D rendered YouTube thumbnail that boosts clicks","I will design an eye-catching AI-powered 3D YouTube thumbnail for your channel","I will make a professional Blender 3D thumbnail that stands out in YouTube search"]',
 '["Top 10 AI Tools 2026 — tech channel thumbnail, dark background, bold white text, dramatic neon lighting","How I Made $10K in 30 Days — finance thumbnail, gold and black, money imagery","The TRUTH About Crypto — dramatic thumbnail, red warning aesthetic, chart graphics","I Tried This for 30 Days and It Changed My Life — lifestyle vlog thumbnail, warm colors","5 Programming Languages to Learn in 2026 — coding thumbnail, terminal aesthetic, green matrix style"]',
 2),

-- 3. Animated Title Card
('title_card',
 'Animated Title Card / Channel Intro',
 'Professional broadcast-quality animated title cards for YouTube and streaming',
 E'First impressions matter. Give your channel a professional identity with a custom animated title card.\n\nI create broadcast-quality animated title cards and channel intros using 3D Blender — the kind of production value you see on major YouTube channels and TV broadcasts.\n\n✅ What you get:\n• Smooth 3D animated title card\n• Custom text (title + subtitle)\n• Loopable or one-shot animation\n• Transparent background version (Alpha channel)\n• MP4 + MOV delivery\n• Commercial license included\n\n🎯 Perfect for:\n• YouTube channel intros and outros\n• Podcast episode openers\n• Twitch stream overlays\n• Corporate presentation openers\n• Course module intros',
 75, 2, '1 animated title card (up to 5s), HD 1080p, 1 revision',
 180, 1, '1 title card (up to 8s) + 2 lower thirds, 1080p, transparent bg version, 2 revisions',
 350, 1, 'Full graphics pack: title card + 3 lower thirds + outro (up to 10s each), 4K, unlimited revisions',
 '["animated title card","channel intro animation","youtube intro","3d title animation","channel graphics","motion graphics","lower third animation","stream overlay","channel branding"]',
 '["I will create a professional 3D animated title card for your YouTube channel","I will make a broadcast-quality animated intro and title card for your channel","I will design a stunning animated channel identity pack with title card and lower thirds"]',
 '["TechBro Studios — modern tech channel title card, glowing circuit board background, blue and white","The Daily Briefing — professional news-style animated title card, clean typography, dark background","LEVEL UP Gaming — gaming channel intro title card with particle explosion and fire effects","Learn With Me — educational channel minimal animated title card, clean white and purple aesthetic","Growth Hacks Daily — corporate finance channel animated opener, gold and navy color scheme"]',
 3),

-- 4. Data Visualization Animation
('data_viz',
 'Animated Data Visualization',
 'Transform boring spreadsheets into stunning animated chart videos',
 E'Data is powerful. Animated data is unforgettable.\n\nI turn your raw numbers and spreadsheets into stunning animated chart and graph videos — perfect for business presentations, YouTube channels, social media, and investor pitches.\n\n✅ What you get:\n• Animated bar charts, line graphs, pie charts, counters, or scatter plots\n• Custom colors matching your brand\n• Smooth professional animations\n• MP4 delivery, HD or 4K\n• Commercial license included\n\n🎯 Perfect for:\n• Finance and business YouTubers\n• Startup investor presentations\n• Marketing performance reports\n• Educational data journalism\n• Social media infographic videos\n\n📊 I support: bar chart races, animated counters, line chart reveals, pie chart builds, scatter animations and more.',
 100, 3, '1 animated chart (up to 30s), 1 chart type, custom colors, 1 revision',
 280, 2, '3 animated charts (up to 60s total), custom branding, background music, 2 revisions',
 550, 1, 'Full data video (up to 90s), animated dashboard with multiple chart types, brand colors, music, unlimited revisions',
 '["data visualization animation","animated chart","bar chart animation","data infographic video","animated statistics","business analytics video","motion graphics data","animated graph"]',
 '["I will create a stunning animated data visualization video for your business or channel","I will make a professional animated bar chart or graph video from your data","I will produce an eye-catching animated infographic video from your statistics"]',
 '["Q1 to Q4 2025 Revenue Growth — animated bar chart with corporate blue color scheme, smooth reveal","Global Market Share 2026 — animated pie chart with labeled segments and percentage counters","Bitcoin Price 2022-2026 — animated line graph with dramatic reveal and milestone markers","Company KPI Dashboard — animated counter metrics showing users, revenue, growth, retention","Sales Performance by Region — animated grouped bar chart comparison with transitions"]',
 4),

-- 5. Educational Explainer Video
('scene',
 'Educational Explainer Video',
 'AI-animated explainer videos that make complex topics simple and engaging',
 E'Make your ideas impossible to misunderstand.\n\nI create professional AI-animated educational explainer videos perfect for courses, YouTube channels, corporate training, and presentations — delivered in 24–48 hours.\n\n✅ What you get:\n• Full 3D animated explainer video\n• AI-generated visuals matched to your topic\n• Professional text overlays\n• Background music (royalty-free)\n• MP4 delivery, HD or 4K\n• Commercial license included\n• Script writing available (add-on)\n\n🎯 Perfect for:\n• Online course creators (Udemy, Teachable, Kajabi)\n• YouTube education channels\n• Corporate training videos\n• Startup explainer videos\n• School and university content\n\n⚡ Turn your script into a fully animated video. No voiceover needed — the visuals do the talking.',
 150, 3, 'Animated explainer up to 60s, HD 1080p, text overlays, background music, 1 revision',
 400, 2, 'Up to 2-minute explainer, 1080p, custom animated scenes, music, 2 revisions',
 800, 1, 'Up to 5-minute fully animated explainer, 4K, multiple animated scenes, music sync, unlimited revisions',
 '["explainer video","educational animation","course video animation","3d explainer","animated tutorial","educational content","ai explainer video","training video animation","e-learning video"]',
 '["I will create a professional AI-animated educational explainer video in 24 hours","I will make a 3D animated explainer video for your course, business or YouTube channel","I will produce a stunning animated educational video that makes your topic unforgettable"]',
 '["How Artificial Intelligence Works — 2-minute educational explainer with 3D neural network visualization","The Science of Blockchain Explained Simply — educational explainer with chain and block animations","5 Steps to Financial Freedom — motivational finance explainer with animated wealth building visuals","How the Human Brain Processes Information — educational 3D brain anatomy explainer","The Complete Guide to Starting a Business — animated startup journey explainer video"]',
 5),

-- 6. Lower Thirds & Title Cards Bundle
('lower_third',
 'Lower Thirds & Broadcast Graphics Bundle',
 'Broadcast-quality animated lower thirds for YouTube, streaming, and corporate video',
 E'Level up your production value with broadcast-quality animated lower thirds.\n\nI create professional animated lower thirds used by top YouTubers, streamers, and corporate video producers — smooth 3D animations that make your content look like it came from a TV studio.\n\n✅ What you get:\n• Animated lower third overlays\n• Custom name/title text\n• Multiple professional styles\n• Transparent background (Alpha channel)\n• MP4 + MOV format\n• Loopable animations\n• Commercial license included\n\n🎯 Perfect for:\n• YouTube interview and documentary videos\n• Podcast episode videos\n• Twitch and live stream overlays\n• Corporate webinars and presentations\n• News and journalism content',
 75, 2, '3 animated lower thirds, custom text, HD, transparent bg, 1 revision',
 160, 1, '6 animated lower thirds + 2 title cards, custom branding, 2 styles, 2 revisions',
 320, 1, 'Full broadcast pack: 10 lower thirds + 5 title cards + 1 intro, 4K, all styles, unlimited revisions',
 '["lower third animation","animated lower third","broadcast graphics","youtube overlay","name title animation","stream overlay graphics","motion graphics lower third","subtitle animation","name plate animation"]',
 '["I will create professional animated lower thirds for your YouTube or corporate video","I will make broadcast-quality animated name titles and lower thirds for your content","I will design a complete animated graphics overlay pack for your YouTube channel or stream"]',
 '["CEO John Smith — TechCorp, professional blue gradient broadcast lower third with smooth slide-in","Breaking News — urgent red ticker lower third with animated highlight bar","xXGameMaster Pro Player — gaming stream name overlay with energy burst effect","Dr Sarah Johnson PhD — academic white and gold speaker introduction lower third","Episode 47 — Chapter marker lower third for podcast or tutorial series, minimal design"]',
 6),

-- 7. LaTeX / Math Animation
('latex',
 'LaTeX Math & Physics Animation',
 'Beautiful step-by-step equation animations for educators and content creators',
 E'Make your equations come alive.\n\nI create professional animated LaTeX math and physics equation videos — step-by-step reveals that make complex formulas elegant and easy to follow. Perfect for educators, YouTubers, and researchers.\n\n✅ What you get:\n• Animated LaTeX equation step-by-step reveal\n• Custom background (dark/light/transparent)\n• Smooth professional animation\n• Multiple animation styles (appear, morph, step-by-step)\n• MP4 delivery, HD or 4K\n• Commercial license included\n\n🎯 Perfect for:\n• Math and physics YouTube channels (like 3Blue1Brown style)\n• Online course creators teaching STEM subjects\n• Academic presentations and lectures\n• Explainer videos for technical topics\n• University and school content\n\n📐 I handle: calculus, algebra, physics formulas, statistics, linear algebra, geometry proofs and more.',
 100, 2, '1 equation animation (up to 30s), dark or light background, step-by-step style, 1 revision',
 260, 1, 'Up to 5 equations (60s total), custom background color, 2 animation styles, 2 revisions',
 480, 1, 'Full math derivation sequence (up to 90s), custom background, all styles, narration markers, unlimited revisions',
 '["math animation","latex animation","equation animation","manim animation","physics equation video","calculus animation","step by step math","3blue1brown style","educational math video","stem animation"]',
 '["I will create a beautiful animated LaTeX math or physics equation video","I will make a professional step-by-step animated math equation video like 3Blue1Brown","I will animate your mathematical equations or physics formulas into a stunning video"]',
 '["E=mc^2 Einstein mass-energy equivalence, step by step reveal with glow effect on dark background","\\\\frac{d}{dx}[x^n] = nx^{n-1} power rule derivative animation with highlight progression","a^2 + b^2 = c^2 Pythagorean theorem proof with geometric visualization","\\\\int_{-\\\\infty}^{\\\\infty} e^{-x^2}\\\\,dx = \\\\sqrt{\\\\pi} Gaussian integral step by step","F = ma Newton second law of motion physics animation with force diagram"]',
 7),

-- 8. UI Mockup Video
('ui_mockup',
 'UI Mockup / Device Demo Video',
 '3D device mockup animations for apps, SaaS, and digital products',
 E'Showcase your app or website inside a stunning 3D device mockup.\n\nI create professional animated device mockup videos — iPhone, MacBook, iPad, and browser — that showcase your app or SaaS product with the polish of an Apple product launch.\n\n✅ What you get:\n• 3D animated device mockup (iPhone, MacBook, browser, or iPad)\n• Your screenshot or UI shown inside the device\n• Professional reveal, scroll, or tilt animation\n• Custom background colors\n• MP4 delivery, HD or 4K\n• Commercial license included\n\n🎯 Perfect for:\n• App Store and Google Play promotional videos\n• SaaS landing pages and pitch decks\n• Product Hunt launches\n• Startup investor demos\n• Social media product showcases\n\n📱 Devices: iPhone 15 Pro, MacBook Pro, iPad Pro, Browser Window — or request custom.',
 150, 2, '1 device mockup video (up to 15s), 1 device type, reveal animation, HD, 1 revision',
 350, 1, '2 device mockup videos (up to 30s each), 2 device types, custom animation, 2 revisions',
 650, 1, 'Full device showcase (up to 60s), up to 4 device types, custom animation sequence, 4K, unlimited revisions',
 '["iphone mockup video","device mockup animation","app demo video","saas demo video","product mockup","ui animation","app store preview","phone animation","macbook mockup","product showcase video"]',
 '["I will create a professional 3D iPhone or device mockup animation video for your app","I will make a stunning animated device mockup video for your app, SaaS or digital product","I will produce an Apple-quality 3D device showcase video for your product launch"]',
 '["Modern fitness tracking app with dashboard on iPhone 15 Pro, smooth reveal animation, dark background","SaaS analytics platform on MacBook Pro, professional scroll walkthrough, light corporate theme","Fashion e-commerce store on browser window, 3D tilt rotation with product grid","Productivity and task management app on iPad, clean reveal with content animation","Mobile puzzle game on iPhone with colorful UI, spinning reveal with particle burst"]',
 8),

-- 9. YouTube Automation Full Package
('auto_video',
 'YouTube Automation Full Video Package',
 'Complete AI-powered video production — script to published video — for faceless channels',
 E'Run a profitable YouTube channel without showing your face.\n\nI deliver the complete YouTube automation pipeline: topic research, script, AI-animated video, custom thumbnail, SEO metadata — everything you need to publish and grow.\n\n✅ What you get:\n• Topic research and script writing\n• Full AI-animated 3D video (your choice of duration)\n• Custom 3D YouTube thumbnail\n• SEO-optimized title, description, and tags\n• Ready-to-upload MP4\n• Commercial license included\n\n🎯 Perfect for:\n• Passive income YouTube channel builders\n• Content entrepreneurs running faceless channels\n• Businesses wanting consistent YouTube presence without hiring a team\n• Agencies reselling content production\n\n🚀 Niches I specialize in: Tech, Finance, Education, AI, Crypto, Science, Business, Self-improvement\n\n⚡ My AI pipeline produces what takes a human team days — in hours.',
 200, 5, '1 complete video (up to 3 min), script + animation + thumbnail + SEO, 1080p, 1 revision',
 600, 7, '5 videos/month (up to 5 min each), script + animation + thumbnail + SEO, 1080p, priority queue, 2 revisions each',
 1000, 7, '10 videos/month (up to 10 min each), all assets + Shorts cuts + channel management advice, 4K, unlimited revisions',
 '["youtube automation","faceless youtube channel","ai youtube video","youtube content creation","automated youtube","youtube video production","ai animated youtube","faceless channel video","youtube passive income","ai video creator"]',
 '["I will create complete AI-animated YouTube videos for your faceless automation channel","I will produce professional AI-powered YouTube videos with thumbnails and SEO optimization","I will set up and run an AI-driven YouTube content production pipeline for your channel"]',
 '["5 Ways Artificial Intelligence is Changing the World in 2026 — 4-minute educational explainer","The Complete Guide to Building Passive Income Online — 5-minute finance education video","Top 10 Programming Languages to Learn in 2026 — 3-minute tech education countdown","How Blockchain and Cryptocurrency Actually Works — 4-minute animated explainer","The Future of Space Exploration and Mars Colonization — 5-minute science documentary style"]',
 9)

ON CONFLICT DO NOTHING;
